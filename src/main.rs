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

#[tokio::main]
async fn main() {
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
    let TransactionStatus::Success {
        block_height,
        receipts,
        ..
    } = script_tx.status
    else {
        panic!("Transaction failed");
    };
    let TransactionType::Known(Transaction::Script(script_tx)) = script_tx.transaction else {
        panic!("Target tx must be script");
    };

    let block = client
        .block_by_height(block_height)
        .await
        .expect("Failed to get block")
        .expect("no such block");
    let mint_tx_id = block.transactions.last().unwrap();

    let mint_tx = client
        .transaction(&mint_tx_id)
        .await
        .expect("Failed to get transaction")
        .expect("no such tx");
    let TransactionType::Known(Transaction::Mint(mint_tx)) = mint_tx.transaction else {
        panic!("Last tx must be mint");
    };
    let gas_price = *mint_tx.gas_price();
    let coinbase = mint_tx.input_contract().contract_id;

    let consensus_params = client
        .consensus_parameters(block.header.consensus_parameters_version as i32)
        .await
        .expect("Failed to get consensus parameters")
        .expect("Failed to get consensus parameters");

    let script_tx = script_tx
        .into_checked_basic(block_height, &consensus_params)
        .expect("Failed to check tx")
        .into_ready(
            gas_price,
            consensus_params.gas_costs(),
            consensus_params.fee_params(),
            Some(block_height),
        )
        .expect("Failed to ready tx");

    let storage_reads = client
        .storage_read_replay(&block_height)
        .await
        .expect("Failed to get reads");

    // dbg!(&storage_reads);

    // Here we abuse the fact that the executor starts each transaction with a read of
    // the ProcessedTransactions column with the transaction ID as the key.
    // This allows us to find the reads for the transaction we're interested in.
    let mut reads_by_column = HashMap::new();
    let mut found_our_tx_start = false;
    for read in storage_reads.iter().cloned() {
        if read.column == "ProcessedTransactions" && read.key == *args.tx_id {
            found_our_tx_start = true;
            continue;
        }
        if !found_our_tx_start {
            continue;
        }
        reads_by_column
            .entry(read.column.clone())
            .or_insert_with(Vec::new)
            .push(read);
    }

    let mut vm = Interpreter::<_, _, Script>::with_storage(
        MemoryInstance::new(),
        ShallowStorage {
            block_height,
            timestamp: block.header.time,
            consensus_parameters_version: block.header.consensus_parameters_version,
            state_transition_version: block.header.state_transition_bytecode_version,
            coinbase,
            storage_write_mask: Default::default(),
            reads_by_column: RefCell::new(reads_by_column),
            storage_reads: RefCell::new(storage_reads),
            kludge: Default::default(),
            client,
        },
        InterpreterParams::new(gas_price, &consensus_params),
    );
    vm.set_single_stepping(true);

    let decoder = ABIDecoder::new(DecoderConfig::default());
    let mut receipt_count = 0;
    let mut return_type_callstack = Vec::new();

    let mut process_new_receipts = |vm: &Interpreter<_, _, Script>| {
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

    let mut t = *vm.transact(script_tx).expect("panicked").state();
    loop {
        process_new_receipts(&vm);
        match t {
            ProgramState::Return(r) => {
                println!("done: returned {r:?}");
                break;
            }
            ProgramState::ReturnData(r) => {
                println!("done: returned data {r:?}");
                break;
            }
            ProgramState::Revert(r) => {
                println!("done: reverted {r:?}");
                break;
            }
            ProgramState::RunProgram(d) => {
                match d {
                    DebugEval::Breakpoint(bp) => {
                        // println!(
                        //     "at {:>6} next instruction: {}",
                        //     bp.pc(),
                        //     get_next_instruction(&vm)
                        //         .map(|i| format!("{i:?}"))
                        //         .unwrap_or_else(|| "???".to_owned()),
                        // );
                    }
                    DebugEval::Continue => {}
                }
                t = vm.resume().expect("panicked");
            }
            ProgramState::VerifyPredicate(d) => {
                println!("paused on debugger {d:?} (in predicate)");
                t = vm.resume().expect("panicked");
            }
        }
    }

    assert_eq!(vm.receipts(), receipts);
}
