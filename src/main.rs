mod memory_reader;
mod routes;
mod tracers;

use anyhow::Context;
use axum::{
    extract::{rejection::JsonRejection, FromRequest},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
    Router,
};
use clap::Parser;
use fuel_core_client::client::FuelClient;
use fuel_vm::prelude::ContractId;
use local_trace_client::TraceError;
use serde::Serialize;

#[derive(FromRequest)]
#[from_request(via(axum::Json), rejection(AppError))]
struct AppJson<T>(T);

impl<T> IntoResponse for AppJson<T>
where
    axum::Json<T>: IntoResponse,
{
    fn into_response(self) -> Response {
        axum::Json(self.0).into_response()
    }
}

enum AppError {
    JsonRejection(JsonRejection),
    InvalidAbiJson { contract: ContractId, error: String },
    Trace(TraceError),
    Health,
}

impl From<JsonRejection> for AppError {
    fn from(rejection: JsonRejection) -> Self {
        Self::JsonRejection(rejection)
    }
}

impl From<TraceError> for AppError {
    fn from(trace: TraceError) -> Self {
        Self::Trace(trace)
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        #[derive(Serialize)]
        struct ErrorResponse {
            message: String,
        }

        let (status, message) = match self {
            AppError::JsonRejection(rejection) => (rejection.status(), rejection.body_text()),
            AppError::Health => (
                StatusCode::BAD_GATEWAY,
                format!("request to fuel-core instance failed"),
            ),
            AppError::InvalidAbiJson { contract, error } => (
                StatusCode::BAD_REQUEST,
                format!("Invalid ABI JSON for contract {}: {}", contract, error),
            ),
            AppError::Trace(err) => match err {
                TraceError::Network(error) => (
                    StatusCode::BAD_GATEWAY,
                    format!("request to fuel-core instance failed: {}", error),
                ),
                TraceError::NoSuchBlock => (StatusCode::NOT_FOUND, format!("Block not found")),
                TraceError::ReceiptsMismatch(_) => (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Receipts mismatch"),
                ),
                other => (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("unable to process: {other:?}"),
                ),
            },
        };

        (status, AppJson(ErrorResponse { message })).into_response()
    }
}

/// Execution tracing demo
#[derive(Parser, Debug)]
#[command(version, about)]
pub struct Args {
    /// Fuel core GraphQL endopoint
    #[clap(long, env = "FUEL_CORE")]
    pub fuel_core: String,
    /// Address to bind to
    #[clap(short, long, env = "TRACING_BIND")]
    pub bind: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    let client = FuelClient::new(args.fuel_core).context("Failed to create FuelClient")?;

    let app = Router::new()
        .route(
            "/health",
            get({
                let client = client.clone();
                move || routes::health::route(client)
            }),
        )
        .route(
            "/v1/trace",
            post(|path| routes::trace_block::route(client, path)),
        )
        .fallback((StatusCode::NOT_FOUND, "404 NOT FOUND"));
    let listener = tokio::net::TcpListener::bind(args.bind).await?;
    tracing::debug!("Listening on {}", listener.local_addr().unwrap());
    axum::serve(listener, app).await.unwrap();
    Ok(())
}
