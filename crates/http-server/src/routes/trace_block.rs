use fuel_execution_trace::trace_block;
use fuel_core_client::client::FuelClient;
use fuel_vm::{fuel_types::BlockHeight, prelude::ContractId};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use utoipa::ToSchema;

use crate::{
    tracers::{self, Abi, TraceEvent},
    AppError, AppJson, ErrorResponse,
};

#[derive(Deserialize, Debug, ToSchema)]
pub struct TraceBlock {
    /// The abi json files are taken as strings to avoid client having to re-serialize them
    #[serde(default)]
    #[schema(value_type = Object, examples(json!({
        "3aa298739660ff73d0a6d8d93f58620a88a504d8bb4b43632cfd52fa82d408cc": "..",
    })))]
    abis: HashMap<ContractId, String>,
    /// The block number to trace
    height: u32,
    /// The options for the trace
    trace: tracers::TraceOptions,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct BlockTrace {
    events: Vec<TraceEvent>,
}

#[utoipa::path(
    post,
    path = "/v1/trace",
    request_body = TraceBlock,
    responses(
        (status = OK, description = "Tracing successful", body = BlockTrace),
        (status = NOT_FOUND, description = "Requested block was not found", body = ErrorResponse),
        (status = BAD_GATEWAY, description = "Request to fuel-core failed", body = ErrorResponse),
        (status = BAD_REQUEST, description = "Malformed request", body = ErrorResponse),
    ),
)]
pub async fn route(
    client: FuelClient,
    AppJson(payload): AppJson<TraceBlock>,
) -> Result<AppJson<BlockTrace>, AppError> {
    let block_height = BlockHeight::from(payload.height);

    let mut abis = HashMap::new();
    for (contract, abi_json) in payload.abis {
        let abi = Abi::from_json(&abi_json).map_err(|err| AppError::InvalidAbiJson {
            contract,
            error: err,
        })?;
        abis.insert(contract, abi);
    }

    let mut tracers = payload.trace.initialize(abis);

    trace_block(&client, block_height, |vm| tracers.callback(vm)).await?;

    let events = tracers.into_events();
    Ok(AppJson(BlockTrace { events }))
}
