use fuel_core_client::client::FuelClient;
use serde::Serialize;
use utoipa::ToSchema;

use crate::{AppError, AppJson, ErrorResponse};

#[derive(Debug, Serialize, ToSchema)]
pub struct Health {
    up: bool,
}

#[utoipa::path(
    get,
    path = "/health",
    responses(
        (status = OK, description = "Success", body = Health),
        (status = BAD_GATEWAY, description = "Request to fuel-core failed", body = ErrorResponse),
    ),
)]
pub async fn route(client: FuelClient) -> Result<AppJson<Health>, AppError> {
    let up = client.health().await.map_err(|_| AppError::Health)?;
    Ok(AppJson(Health { up }))
}
