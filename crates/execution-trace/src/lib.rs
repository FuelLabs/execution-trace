#![deny(unsafe_code)]
#![deny(unused_must_use)]
#![deny(unused_crate_dependencies)]
#![deny(
    clippy::arithmetic_side_effects,
    clippy::cast_sign_loss,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::string_slice
)]

mod memory_reader;
mod shallow_storage;

pub use memory_reader::MemoryReader;

use std::cell::RefCell;

use field::{InputContract, MintGasPrice};
use fuel_core_client::client::{
    types::{TransactionStatus, TransactionType},
    FuelClient,
};
use fuel_vm::{
    checked_transaction::{CheckError, IntoChecked},
    fuel_types::BlockHeight,
    interpreter::{InterpreterParams, NotSupportedEcal},
    prelude::*,
};

use shallow_storage::ShallowStorage;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum TraceError {
    #[error("Request to fuel-core failed")]
    Network(#[from] std::io::Error),
    #[error("Requested block doesn't exist")]
    NoSuchBlock,
    #[error("Requested block is malformed")]
    MalformedBlock,
    #[error("Couldn't get consensus parameters for the block")]
    NoConsensusParameters,
    #[error("Couldn't get transaction that's in a block")]
    MissingTransaction(TxId),
    #[error("Block contained unknown transaction type")]
    UnknownTransactionType(TxId),
    #[error("Transaction failed checking")]
    CheckTransaction(TxId, CheckError),
    #[error("Local execution produced different receipts")]
    ReceiptsMismatch(TxId, Vec<Receipt>),
}

/// The VM type used for tracing
pub type Vm = Interpreter<MemoryInstance, ShallowStorage, Script, NotSupportedEcal>;

pub async fn trace_block<Callback>(
    client: &FuelClient,
    block_height: BlockHeight,
    mut on_instruction: Callback,
) -> Result<(), TraceError>
where
    Callback: FnMut(&Vm),
{
    let block = client
        .block_by_height(block_height)
        .await?
        .ok_or(TraceError::NoSuchBlock)?;

    let storage_reads = client.storage_read_replay(&block_height).await?;

    let mint_tx_id = block
        .transactions
        .last()
        .ok_or(TraceError::MalformedBlock)?;
    let mint_tx = client
        .transaction(&mint_tx_id)
        .await?
        .ok_or_else(|| TraceError::MissingTransaction((*mint_tx_id).into()))?;
    let TransactionType::Known(Transaction::Mint(mint_tx)) = mint_tx.transaction else {
        return Err(TraceError::MalformedBlock);
    };

    let gas_price = *mint_tx.gas_price();
    let coinbase = mint_tx.input_contract().contract_id;

    let consensus_params = client
        .consensus_parameters(block.header.consensus_parameters_version as i32)
        .await?
        .ok_or(TraceError::NoConsensusParameters)?;

    let mut storage = ShallowStorage {
        block_height,
        timestamp: block.header.time,
        consensus_parameters_version: block.header.consensus_parameters_version,
        state_transition_version: block.header.state_transition_bytecode_version,
        coinbase,
        storage: RefCell::new(ShallowStorage::initial_storage(storage_reads)),
    };

    for tx_id in block.transactions.iter().take(block.transactions.len() - 1) {
        let tx = client
            .transaction(&tx_id)
            .await?
            .ok_or_else(|| TraceError::MissingTransaction((*tx_id).into()))?;

        let receipts = match tx.status {
            TransactionStatus::Success { receipts, .. } => receipts,
            TransactionStatus::Failure { .. } => continue,
            TransactionStatus::Submitted { .. } | TransactionStatus::SqueezedOut { .. } => {
                return Err(TraceError::MalformedBlock)
            }
        };

        let TransactionType::Known(tx) = tx.transaction else {
            return Err(TraceError::UnknownTransactionType((*tx_id).into()));
        };

        let Transaction::Script(script_tx) = tx else {
            continue;
        };

        let script_tx = script_tx
            .into_checked_basic(block_height, &consensus_params)
            .map_err(|err| TraceError::CheckTransaction((*tx_id).into(), err))?
            .into_ready(
                gas_price,
                consensus_params.gas_costs(),
                consensus_params.fee_params(),
                Some(block_height),
            )
            .map_err(|err| TraceError::CheckTransaction((*tx_id).into(), err))?;

        let mut vm = Interpreter::<_, _, Script>::with_storage(
            MemoryInstance::new(),
            storage.clone(),
            InterpreterParams::new(gas_price, &consensus_params),
        );
        vm.set_single_stepping(true);

        let mut t = *vm.transact(script_tx).expect("panicked").state();
        loop {
            on_instruction(&vm);
            match t {
                ProgramState::Return(_) | ProgramState::ReturnData(_) | ProgramState::Revert(_) => {
                    break
                }
                ProgramState::RunProgram(_) | ProgramState::VerifyPredicate(_) => {
                    t = vm.resume().expect("panicked");
                }
            }
        }

        if vm.receipts() != receipts {
            return Err(TraceError::ReceiptsMismatch(
                (*tx_id).into(),
                vm.receipts().to_vec(),
            ));
        }

        storage = vm.as_ref().clone();
    }

    Ok(())
}
