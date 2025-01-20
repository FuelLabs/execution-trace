#![allow(unused_imports, unused_variables, dead_code)]

pub mod args;
pub mod memory_reader;
mod shallow_storage;

use std::{
    cell::{Ref, RefCell},
    collections::HashMap,
    str::FromStr,
};

use args::Args;
use clap::Parser;
use field::{InputContract, MintGasPrice, OutputContract};
use fuel_abi_types::abi::{
    full_program::FullProgramABI, program::ProgramABI, unified_program::UnifiedProgramABI,
};
use fuel_core_client::client::{
    schema::schema::__fields::GasCosts::call,
    types::{TransactionStatus, TransactionType},
    FuelClient,
};
use fuel_vm::{
    checked_transaction::IntoChecked,
    fuel_asm::RawInstruction,
    fuel_types::canonical::Deserialize,
    interpreter::{InterpreterParams, NotSupportedEcal},
    prelude::*,
};
use fuels::{
    core::codec::{ABIDecoder, DecoderConfig},
    types::{param_types::ParamType, Token},
};

use memory_reader::MemoryReader;
use shallow_storage::ShallowStorage;

fn get_next_instruction<M, S, Tx>(
    vm: &Interpreter<M, S, Tx, NotSupportedEcal>,
) -> Option<Instruction>
where
    M: Memory,
{
    let pc = vm.registers()[RegId::PC];
    let instruction = RawInstruction::from_be_bytes(vm.memory().read_bytes(pc).ok()?);
    Instruction::try_from(instruction).ok()
}

// #[tokio::main]
async fn main() -> anyhow::Result<()> {
    pretty_env_logger::init();

    let args = Args::parse();

    let mut abi_by_contract_id = HashMap::new();
    for path in ["test-contract", "test-chain-contract"] {
        let deploys = std::fs::read_dir(format!("{path}/out/deployments"))
            .expect("Failed to read directory")
            .map(|entry| entry.expect("Failed to read entry").path())
            .map(|path| {
                ContractId::from_str(
                    path.file_stem()
                        .expect("no stem")
                        .to_str()
                        .unwrap()
                        .rsplit_once('-')
                        .unwrap()
                        .1,
                )
                .unwrap()
            })
            .collect::<Vec<_>>();

        let abi = std::fs::read(format!("{path}/out/release/{path}-abi.json")).unwrap();
        let abi: ProgramABI = serde_json::from_slice(&abi).unwrap();
        for d in deploys {
            abi_by_contract_id.insert(d, abi.clone());
        }
    }

    let client = FuelClient::new("http://localhost:4000").expect("Failed to create client");

    let script_tx = client
        .transaction(&(*args.tx_id).into())
        .await
        .expect("Failed to get transaction")
        .expect("no such tx");
    let TransactionStatus::Success { block_height, .. } = script_tx.status else {
        panic!("Transaction failed");
    };

    let decoder = ABIDecoder::new(DecoderConfig::default());
    let mut receipt_count = 0;
    let mut return_type_callstack = Vec::new();

    let process_new_receipts = |vm: &Interpreter<_, _, Script>| {
        while receipt_count < vm.receipts().len() {
            match &vm.receipts()[receipt_count] {
                Receipt::Call {
                    to, param1, param2, ..
                } => {
                    let Ok(Token::String(method_name)) =
                        decoder.decode(&ParamType::String, MemoryReader::new(vm.memory(), *param1))
                    else {
                        panic!("Expected string");
                    };

                    if let Some(abi) = abi_by_contract_id.get(to) {
                        let args_reader = MemoryReader::new(vm.memory(), *param2);
                        let unified_abi = UnifiedProgramABI::from_counterpart(abi).unwrap();

                        let type_lookup = unified_abi
                            .types
                            .into_iter()
                            .map(|decl| (decl.type_id, decl))
                            .collect::<HashMap<_, _>>();

                        let func = unified_abi
                            .functions
                            .iter()
                            .find(|f| f.name == method_name)
                            .unwrap();

                        let return_type =
                            ParamType::try_from_type_application(&func.output, &type_lookup)
                                .unwrap();

                        let mut args = Vec::new();
                        for param in &func.inputs {
                            let param_type =
                                ParamType::try_from_type_application(&param, &type_lookup).unwrap();
                            args.push(
                                decoder
                                    .decode(&param_type, args_reader.clone())
                                    .unwrap()
                                    .to_string(),
                            );
                        }

                        println!(
                            "{}call to {to} with method {method_name}({})",
                            "  ".repeat(return_type_callstack.len()),
                            args.join(", ")
                        );
                        return_type_callstack.push(Some(return_type));
                    } else {
                        println!(
                            "{}call to {to} with method {method_name}({{unknown abi}})",
                            "  ".repeat(return_type_callstack.len()),
                        );
                        return_type_callstack.push(None);
                    }
                }
                Receipt::Return { val, .. } if !return_type_callstack.is_empty() => {
                    println!(
                        "{}-> returned {val}",
                        "  ".repeat(return_type_callstack.len()),
                    );
                    let _ = return_type_callstack.pop().unwrap();
                }
                Receipt::ReturnData { id, ptr, len, .. } if !return_type_callstack.is_empty() => {
                    if let Some(abi) = abi_by_contract_id.get(id) {
                        if let Some(return_type) = return_type_callstack.pop().unwrap() {
                            let reader = MemoryReader::new(vm.memory(), *ptr);
                            let unified_abi = UnifiedProgramABI::from_counterpart(abi).unwrap();

                            println!(
                                "{}-> returned {:?}",
                                "  ".repeat(return_type_callstack.len()),
                                decoder.decode(&return_type, reader.clone()).unwrap()
                            );
                        } else {
                            unreachable!("abi checked above");
                        }
                    } else {
                        let _ = return_type_callstack.pop().unwrap();
                        println!(
                            "{}-> returned {{unknown abi}}",
                            "  ".repeat(return_type_callstack.len()),
                        );
                    }
                }
                _ => {}
            }
            receipt_count += 1;
        }
    };

    local_trace_client::trace_block(
        &client,
        &abi_by_contract_id,
        block_height,
        process_new_receipts,
    )
    .await?;
    Ok(())
}
