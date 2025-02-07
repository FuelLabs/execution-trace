use std::collections::HashMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct TraceBlock {
    /// The abi json files are taken as strings to avoid client having to re-serialize them
    abis: HashMap<String, String>,
    /// The block number to trace
    height: u32,
    /// The options for the trace
    trace: TraceOptions,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TraceOptions {
    callret: bool,
}

#[tokio::main]
async fn main() -> Result<(), reqwest::Error> {
    let client = reqwest::Client::new();

    let file = std::fs::read("/Users/hannes/Projects/fuel/mira-v1-core/contracts/mira_amm_contract/out/debug/mira_amm_contract-abi.json").unwrap();
    let abi = String::from_utf8(file).unwrap();
    let mut abis = HashMap::new();
    abis.insert(
        "0x2e40f2b244b98ed6b8204b3de0156c6961f98525c8162f80162fcf53eebd90e7".to_string(),
        abi,
    );

    let mut height = 13023074;
    while height > 0 {
        let body = TraceBlock {
            abis: abis.clone(),
            height,
            trace: TraceOptions { callret: true },
        };

        let resp = client
            .post("http://localhost:4001/v1/trace")
            .json(&body)
            .send()
            .await?;

        let status = resp.status();
        let text = resp.text().await?;

        if status != 200 {
            panic!("block {height} errors: {text}");
        }

        dbg!(text);

        height -= 1;
    }

    Ok(())
}
