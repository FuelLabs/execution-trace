use fuels::prelude::*;
use std::str::FromStr;

use clap::{Parser, Subcommand};

/// Execution tracing demo
#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
pub struct Args {
    pub counter_contract: ContractId,
    #[command(subcommand)]
    pub cmd: Command,
}


#[derive(Subcommand, Debug)]
pub enum Command {
    Count,
    Increment,
    Chain {
        contract_id: ContractId,
        count: u64,
    },
}


#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    let provider = Provider::connect("localhost:4000").await.unwrap();

    let address =
        Address::from_str("0x6b63804cfbf9856e68e5b6e7aef238dc8311ec55bec04df774003a2c96e0418e")
            .unwrap();
    let address = Bech32Address::from(address);
    let impersonator = ImpersonatedAccount::new(address, Some(provider.clone()));

    abigen!(
        Contract(
            name = "TestContract",
            abi = "../test-contract/out/release/test-contract-abi.json"
        ),
        Contract(
            name = "MyContract",
            abi = "../test-chain-contract/out/release/test-chain-contract-abi.json"
        ),
    );
    
    let counter = TestContract::new(args.counter_contract, impersonator.clone());
    match args.cmd {
        Command::Count => {
            let response = counter.methods().count().call().await?;
            println!(
                "Counter value is {:?} after tx {:?}",
                response.value,
                response.tx_id.unwrap()
            );
        }
        Command::Increment => {
            let response = counter.methods().increment().call().await?;
            println!(
                "Counter value is {:?} after tx {:?}",
                response.value,
                response.tx_id.unwrap()
            );
        }
        Command::Chain { contract_id, count } => {
            let chain = MyContract::new(contract_id, impersonator.clone());
        
            let response = chain.methods().invoke(count).with_contracts(&[&counter]).call().await?;
            println!(
                "Response {:?} after tx {:?}",
                response.value,
                response.tx_id.unwrap()
            );

        }
    }



    Ok(())
}
