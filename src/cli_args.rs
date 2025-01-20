use std::path::PathBuf;

use clap::Parser;
use fuel_vm::fuel_types::TxId;

/// Execution tracing demo
#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
pub struct Args {
    /// Target transaction ID
    pub tx_id: TxId,

    /// ABI specification
    #[arg(short, long)]
    pub abi: Option<PathBuf>,
}
