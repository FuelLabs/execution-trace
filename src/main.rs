#![allow(unused_imports, unused_variables, dead_code)]

pub mod memory_reader;
mod shallow_storage;

use std::{cell::RefCell, collections::HashMap, str::FromStr};

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
        .transaction(
            &TxId::from_str("4b712d5e7208c55f440821d05d85d57f3bdc28a36c5c03cd27697881deaec6a0")
                .unwrap(),
        )
        .await
        .expect("Failed to get transaction")
        .expect("no such tx");
    dbg!(&script_tx.status);
    let TransactionStatus::Success { block_height, .. } = script_tx.status else {
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
        .test_into_ready();

    let storage_reads = client
        .storage_read_replay(&block_height)
        .await
        .expect("Failed to get reads");
    let reads = storage_reads[0].clone(); // TODO: index
    dbg!(&reads);

    let mut vm = Interpreter::<_, _, Script>::with_storage(
        MemoryInstance::new(),
        ShallowStorage {
            block_height,
            timestamp: block.header.time,
            consensus_parameters_version: block.header.consensus_parameters_version,
            state_transition_version: block.header.state_transition_bytecode_version,
            coinbase,
            reads: RefCell::new(reads),
            client,
        },
        InterpreterParams::new(gas_price, &consensus_params),
    );
    vm.set_single_stepping(true);

    let mut receipt_count = 0;
    let mut t = *vm.transact(script_tx).expect("panicked").state();
    loop {
        if vm.receipts().len() > receipt_count {
            receipt_count = vm.receipts().len();
            if let Some(Receipt::Call { to, .. }) = vm.receipts().last() {
                let fp = vm.registers()[RegId::FP];
                let call_frame_bytes = vm.memory().read(fp, CallFrame::serialized_size()).unwrap();
                let call_frame = CallFrame::from_bytes(&call_frame_bytes).unwrap();
                let decoder = ABIDecoder::new(DecoderConfig::default());
                let Ok(Token::String(method_name)) = decoder.decode(
                    &ParamType::String,
                    MemoryReader::new(vm.memory(), call_frame.a()),
                ) else {
                    panic!("Expected string");
                };

                let args_reader = MemoryReader::new(vm.memory(), call_frame.b());
                let abi = &abi_by_contract_id[call_frame.to()];
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
                    "call to {to} with method {method_name}({})",
                    args.join(", ")
                );
            }
        }

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
                        println!(
                            "at {:>4} reg[0x20] = {:4}, next instruction: {}",
                            bp.pc(),
                            &vm.registers()[0x20],
                            get_next_instruction(&vm)
                                .map(|i| format!("{i:?}"))
                                .unwrap_or_else(|| "???".to_owned()),
                        );
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
}
