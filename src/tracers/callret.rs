use std::collections::HashMap;

use fuel_vm::prelude::{ContractId, Receipt};
use fuels::{
    core::codec::{ABIDecoder, DecoderConfig},
    types::{param_types::ParamType, Token},
};
use local_trace_client::Vm;

use crate::memory_reader::MemoryReader;

use super::{Abi, TraceEvent, Tracer};

#[derive(Default)]
pub struct CallRetTracer {
    seen_receipt_count: usize,
    return_type_callstack: Vec<StackFrame>,
}

enum StackFrame {
    KnownAbi(ParamType),
    UnknownAbi,
}

impl Tracer for CallRetTracer {
    fn callback(&mut self, vm: &Vm, abis: &HashMap<ContractId, Abi>) -> Vec<TraceEvent> {
        let mut result = Vec::new();
        while self.seen_receipt_count < vm.receipts().len() {
            result.extend(self.handle_latest_receipt(vm, abis).into_iter());
            self.seen_receipt_count += 1;
        }
        result
    }
}

impl CallRetTracer {
    fn handle_latest_receipt(
        &mut self,
        vm: &Vm,
        abis: &HashMap<ContractId, Abi>,
    ) -> Option<TraceEvent> {
        let decoder = ABIDecoder::new(DecoderConfig::default());

        match vm.receipts()[self.seen_receipt_count] {
            Receipt::Call {
                to, param1, param2, ..
            } => {
                let method = match decoder
                    .decode(&ParamType::String, MemoryReader::new(vm.memory(), param1))
                {
                    Ok(Token::String(method)) => Some(method),
                    _ => None,
                };

                let arguments = if let Some(Signature {
                    parameters,
                    returns,
                }) = method
                    .as_ref()
                    .and_then(|m| Signature::try_from_abi(abis.get(&to)?, m.as_str()))
                {
                    self.return_type_callstack
                        .push(StackFrame::KnownAbi(returns));
                    let args_reader = MemoryReader::new(vm.memory(), param2);
                    parameters
                        .iter()
                        .map(|type_| decoder.decode(&type_, args_reader.clone()))
                        .collect::<Result<_, _>>()
                        .ok()
                } else {
                    self.return_type_callstack.push(StackFrame::UnknownAbi);
                    None
                };

                Some(TraceEvent::Call {
                    receipt: self.seen_receipt_count,
                    method,
                    arguments,
                })
            }

            Receipt::Return { .. } if !self.return_type_callstack.is_empty() => {
                let _ = self.return_type_callstack.pop().unwrap();
                Some(TraceEvent::Return {
                    receipt: self.seen_receipt_count,
                    value: None,
                })
            }

            Receipt::ReturnData { ptr, .. } if !self.return_type_callstack.is_empty() => {
                let return_value = if let StackFrame::KnownAbi(return_type) =
                    self.return_type_callstack.pop().unwrap()
                {
                    let reader = MemoryReader::new(vm.memory(), ptr);
                    decoder.decode(&return_type, reader).ok()
                } else {
                    None
                };

                Some(TraceEvent::Return {
                    receipt: self.seen_receipt_count,
                    value: return_value,
                })
            }

            _ => None,
        }
    }
}

struct Signature {
    parameters: Vec<ParamType>,
    returns: ParamType,
}
impl Signature {
    fn try_from_abi(abi: &Abi, method: &str) -> Option<Self> {
        let func = abi.unified.functions.iter().find(|f| f.name == *method)?;

        let mut parameters = Vec::new();
        for param in &func.inputs {
            parameters.push(ParamType::try_from_type_application(&param, &abi.type_lookup).ok()?);
        }

        let returns = ParamType::try_from_type_application(&func.output, &abi.type_lookup).ok()?;
        Some(Self {
            parameters,
            returns,
        })
    }
}

// let process_new_receipts = |vm: &Interpreter<_, _, Script>| {
//     while receipt_count < vm.receipts().len() {
//         match &vm.receipts()[receipt_count] {
//             Receipt::Call {
//                 to, param1, param2, ..
//             } => {
//                 let Ok(Token::String(method_name)) =
//                     decoder.decode(&ParamType::String, MemoryReader::new(vm.memory(), *param1))
//                 else {
//                     panic!("Expected string");
//                 };

//                 if let Some(abi) = abi_by_contract_id.get(to) {
//                     let args_reader = MemoryReader::new(vm.memory(), *param2);
//                     let unified_abi = UnifiedProgramABI::from_counterpart(abi).unwrap();

//                     let type_lookup = unified_abi
//                         .types
//                         .into_iter()
//                         .map(|decl| (decl.type_id, decl))
//                         .collect::<HashMap<_, _>>();

//                     let func = unified_abi
//                         .functions
//                         .iter()
//                         .find(|f| f.name == method_name)
//                         .unwrap();

//                     let return_type =
//                         ParamType::try_from_type_application(&func.output, &type_lookup)
//                             .unwrap();

//                     let mut args = Vec::new();
//                     for param in &func.inputs {
//                         let param_type =
//                             ParamType::try_from_type_application(&param, &type_lookup).unwrap();
//                         args.push(
//                             decoder
//                                 .decode(&param_type, args_reader.clone())
//                                 .unwrap()
//                                 .to_string(),
//                         );
//                     }

//                     println!(
//                         "{}call to {to} with method {method_name}({})",
//                         "  ".repeat(return_type_callstack.len()),
//                         args.join(", ")
//                     );
//                     return_type_callstack.push(Some(return_type));
//                 } else {
//                     println!(
//                         "{}call to {to} with method {method_name}({{unknown abi}})",
//                         "  ".repeat(return_type_callstack.len()),
//                     );
//                     return_type_callstack.push(None);
//                 }
//             }
//             Receipt::Return { val, .. } if !return_type_callstack.is_empty() => {
//                 println!(
//                     "{}-> returned {val}",
//                     "  ".repeat(return_type_callstack.len()),
//                 );
//                 let _ = return_type_callstack.pop().unwrap();
//             }
//             Receipt::ReturnData { id, ptr, len, .. } if !return_type_callstack.is_empty() => {
//                 if let Some(abi) = abi_by_contract_id.get(id) {
//                     if let Some(return_type) = return_type_callstack.pop().unwrap() {
//                         let reader = MemoryReader::new(vm.memory(), *ptr);
//                         let unified_abi = UnifiedProgramABI::from_counterpart(abi).unwrap();

//                         println!(
//                             "{}-> returned {:?}",
//                             "  ".repeat(return_type_callstack.len()),
//                             decoder.decode(&return_type, reader.clone()).unwrap()
//                         );
//                     } else {
//                         unreachable!("abi checked above");
//                     }
//                 } else {
//                     let _ = return_type_callstack.pop().unwrap();
//                     println!(
//                         "{}-> returned {{unknown abi}}",
//                         "  ".repeat(return_type_callstack.len()),
//                     );
//                 }
//             }
//             _ => {}
//         }
//         receipt_count += 1;
//     }
// };
