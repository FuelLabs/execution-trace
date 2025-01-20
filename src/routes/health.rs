use fuel_core_client::client::FuelClient;
use serde::Serialize;

use crate::{AppError, AppJson};

#[derive(Debug, Serialize)]
pub struct Health {
    up: bool,
}

pub async fn route(client: FuelClient) -> Result<AppJson<Health>, AppError> {
    let up = client.health().await.map_err(|_| AppError::Health)?;
    Ok(AppJson(Health { up }))
}
