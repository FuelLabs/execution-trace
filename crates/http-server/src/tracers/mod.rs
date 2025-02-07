use fuel_execution_trace::Vm;
use std::collections::HashMap;

use fuel_abi_types::abi::{
    program::ProgramABI,
    unified_program::{UnifiedProgramABI, UnifiedTypeDeclaration},
};
use fuel_vm::prelude::ContractId;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

mod callret;

#[derive(Deserialize, Debug, ToSchema)]
pub struct TraceOptions {
    callret: bool,
}

impl TraceOptions {
    pub fn initialize(self, abis: HashMap<ContractId, Abi>) -> Tracers {
        let mut tracers: Vec<Box<dyn Tracer>> = Vec::new();
        if self.callret {
            tracers.push(Box::new(callret::CallRetTracer::default()));
        }
        Tracers {
            tracers,
            abis,
            output: Vec::new(),
        }
    }
}

trait Tracer: Send + Sync + 'static {
    fn callback(&mut self, vm: &Vm, abis: &HashMap<ContractId, Abi>) -> Vec<TraceEvent>;
}

pub struct Abi {
    #[allow(dead_code)]
    program: ProgramABI,
    unified: UnifiedProgramABI,
    type_lookup: HashMap<usize, UnifiedTypeDeclaration>,
}
impl Abi {
    pub fn from_json(json: &str) -> Result<Self, String> {
        let program: ProgramABI = serde_json::from_str(&json).map_err(|err| format!("{}", err))?;

        let unified = UnifiedProgramABI::from_counterpart(&program)
            .map_err(|err| format!("Conversion to unified format failed: {}", err))?;

        let type_lookup = unified
            .types
            .iter()
            .map(|decl| (decl.type_id, decl.clone()))
            .collect::<HashMap<_, _>>();

        Ok(Self {
            program,
            unified,
            type_lookup,
        })
    }
}

pub struct Tracers {
    abis: HashMap<ContractId, Abi>,
    tracers: Vec<Box<dyn Tracer>>,
    output: Vec<TraceEvent>,
}

impl Tracers {
    pub fn callback(&mut self, vm: &fuel_execution_trace::Vm) {
        for tracer in &mut self.tracers {
            self.output.extend(tracer.callback(vm, &self.abis));
        }
    }

    pub fn into_events(self) -> Vec<TraceEvent> {
        self.output
    }
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum TraceEvent {
    Call {
        /// Which receipt this call corresponds to.
        #[schema(examples(0))]
        receipt: usize,
        /// Method being called. `None` if param1 doesn't point to a string.
        #[schema(examples("method_name"))]
        method: Option<String>,
        /// Method being called. `None` if `method` couldn't be resolved,
        /// or if arguments couldn't be parsed due to unknown ABI or invalid form.
        #[schema(examples(json!(["U64(42)", "String(\"Limiting Factor\")"])))]
        arguments: Option<Vec<String>>,
    },
    Return {
        /// Which receipt this call corresponds to.
        #[schema(examples(1))]
        receipt: usize,
        /// Return value. `None` if unknown ABI or invalid form.
        /// Also contains `None` if no data is returned, i.e. using `ret` instead of `retd`.
        #[schema(examples(json!(["Array(U64(0), U64(1), U64(2))"])))]
        value: Option<String>,
    },
}
