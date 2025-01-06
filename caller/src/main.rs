use fuels::prelude::*;
use std::str::FromStr;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
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

    let contract_id =
        ContractId::from_str("fc51eadbb8962d9ac7a22e887f82abcf797457bb99fac8a2f634cf1fa08284c8")
            .unwrap();
    let contract_instance = TestContract::new(contract_id, impersonator.clone());

    // let response = contract_instance.methods().increment().call().await?;
    // println!(
    //     "Counter value is {:?} after tx {:?}",
    //     response.value,
    //     response.tx_id.unwrap()
    // );

    let contract_id2 =
        ContractId::from_str("4995a7693e4c7af5bced3f7e3e4f2d7859139f54fa8d393092c89a3a8fb77177")
            .unwrap();
    let contract_instance2 = MyContract::new(contract_id2, impersonator.clone());

    let response = contract_instance2.methods().invoke(5).with_contracts(&[&contract_instance]).call().await?;
    println!(
        "Response {:?} after tx {:?}",
        response.value,
        response.tx_id.unwrap()
    );
    Ok(())
}
