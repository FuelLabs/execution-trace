use std::collections::HashMap;

use fuel_execution_trace::{MemoryReader, Vm};
use fuel_vm::prelude::{ContractId, Receipt};
use fuels::{
    core::codec::{ABIDecoder, DecoderConfig},
    types::{param_types::ParamType, Token},
};

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
                        .map(|type_| {
                            decoder
                                .decode(&type_, args_reader.clone())
                                .map(|t| t.to_string())
                        })
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
                    decoder
                        .decode(&return_type, reader)
                        .map(|t| t.to_string())
                        .ok()
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
