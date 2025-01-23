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
use fuel_execution_trace::TraceError;
use fuel_vm::prelude::ContractId;
use serde::Serialize;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};
use utoipa::{openapi::Server, OpenApi, ToSchema};
use utoipa_swagger_ui::SwaggerUi;

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

#[derive(Serialize, ToSchema)]
struct ErrorResponse {
    #[schema(examples("Error message"))]
    message: String,
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            AppError::JsonRejection(rejection) => (rejection.status(), rejection.body_text()),
            AppError::Health => (
                StatusCode::BAD_GATEWAY,
                format!("request to fuel-core failed"),
            ),
            AppError::InvalidAbiJson { contract, error } => (
                StatusCode::BAD_REQUEST,
                format!("Invalid ABI JSON for contract {}: {}", contract, error),
            ),
            AppError::Trace(err) => match err {
                TraceError::Network(error) => (
                    StatusCode::BAD_GATEWAY,
                    format!("request to fuel-core  ailed: {}", error),
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

#[derive(OpenApi)]
#[openapi(
    info(title = "Execution tracing proxy for fuel-core"),
    paths(routes::health::route, routes::trace_block::route,)
)]
struct ApiDoc;

/// Execution tracing demo
#[derive(Parser, Debug)]
#[command(version, about)]
pub struct Args {
    /// Fuel core GraphQL endopoint
    #[clap(
        long,
        env = "FUEL_CORE",
        default_value = "http://localhost:4000/graphql"
    )]
    pub fuel_core: String,
    /// Address to bind to
    #[clap(short, long, env = "TRACING_BIND", default_value = "0.0.0.0:4001")]
    pub bind: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer())
        .with(EnvFilter::from_default_env())
        .init();

    let args = Args::parse();

    let client = FuelClient::new(args.fuel_core).context("Failed to create FuelClient")?;
    let chain_info = client
        .chain_info()
        .await
        .context("Failed to connect to fuel-core")?;
    tracing::info!("Connected to {}", chain_info.name);

    let listener = tokio::net::TcpListener::bind(args.bind).await?;
    let addr = listener.local_addr()?;

    let mut api_doc = ApiDoc::openapi();
    api_doc.servers = Some(vec![Server::new(addr.to_string())]);
    let api_doc = api_doc.to_pretty_json().unwrap();

    let app = Router::new()
        .merge(SwaggerUi::new("/swagger-ui").url("/api-docs/openapi.json", ApiDoc::openapi()))
        .route("/docs", get(move || async { api_doc }))
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

    tracing::debug!("Serving on {}", addr);
    axum::serve(listener, app).await.unwrap();
    Ok(())
}
