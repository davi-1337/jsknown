use anyhow::Result;
use axum::{Json, Router, extract::State, http::StatusCode, routing::get, routing::post};
use jsknown_core::{config::Config, ingest::IngestionRequest, processor::Processor};
use serde::Serialize;
use std::{net::SocketAddr, sync::Arc};
use tower_http::trace::TraceLayer;

#[derive(Clone)]
struct AppState {
    processor: Arc<Processor>,
}

#[derive(Serialize)]
struct Health {
    status: &'static str,
}

pub async fn serve(config: Config) -> Result<()> {
    let addr: SocketAddr = format!("{}:{}", config.host, config.port).parse()?;
    let processor = Arc::new(Processor::new(config).await?);
    let state = AppState { processor };
    let app = Router::new()
        .route("/health", get(health))
        .route("/ingest", post(ingest))
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    tracing::info!("jsknown listening on http://{addr}");
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

async fn health() -> Json<Health> {
    Json(Health { status: "ok" })
}

async fn ingest(
    State(state): State<AppState>,
    Json(payload): Json<IngestionRequest>,
) -> Result<StatusCode, (StatusCode, String)> {
    state
        .processor
        .process_ingestion(payload)
        .await
        .map_err(|err| (StatusCode::BAD_REQUEST, err.to_string()))?;
    Ok(StatusCode::ACCEPTED)
}
