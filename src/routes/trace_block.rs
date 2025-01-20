use ::local_trace_client::trace_block;
use fuel_core_client::client::FuelClient;
use fuel_vm::{fuel_types::BlockHeight, prelude::ContractId};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::{
    tracers::{self, Abi, TraceEvent},
    AppError, AppJson,
};

#[derive(Deserialize, Debug)]
pub struct TraceBlock {
    /// The abi json files are taken as strings to avoid client having to re-serialize them
    #[serde(default)]
    abis: HashMap<ContractId, String>,
    /// The block number to trace
    height: BlockHeight,
    /// The options for the trace
    trace: tracers::TraceOptions,
}

#[derive(Debug, Serialize)]
pub struct BlockTrace {
    events: Vec<TraceEvent>,
}

pub async fn route(
    client: FuelClient,
    AppJson(payload): AppJson<TraceBlock>,
) -> Result<AppJson<BlockTrace>, AppError> {
    let mut abis = HashMap::new();
    for (contract, abi_json) in payload.abis {
        let abi = Abi::from_json(&abi_json).map_err(|err| AppError::InvalidAbiJson {
            contract,
            error: err,
        })?;
        abis.insert(contract, abi);
    }

    let mut tracers = payload.trace.initialize(abis);

    trace_block(&client, payload.height, |vm| tracers.callback(vm)).await?;

    let events = tracers.into_events();
    Ok(AppJson(BlockTrace { events }))
}
