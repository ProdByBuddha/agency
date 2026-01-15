use axum::{
    extract::{State, Json},
    routing::{get, post},
    Router,
};
use anyhow::Result;
use crate::memory::{Memory, MemoryEntry, vector::LocalVectorMemory};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::info;
use std::env;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

pub struct MemoryServerState {
    pub memory: LocalVectorMemory,
}

#[derive(Deserialize)]
struct StoreRequest {
    entry: MemoryEntry,
}

#[derive(Deserialize)]
struct SearchRequest {
    query: String,
    top_k: usize,
    context: Option<String>,
    kind: Option<crate::orchestrator::Kind>,
}

#[derive(Serialize)]
struct StoreResponse {
    id: String,
}

#[derive(Serialize)]
struct SearchResponse {
    entries: Vec<MemoryEntry>,
}

struct ServerError(anyhow::Error);

impl IntoResponse for ServerError {
    fn into_response(self) -> Response {
        let (status, message) = (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Memory Server Error: {}", self.0),
        );
        (status, Json(serde_json::json!({ "error": message }))).into_response()
    }
}

impl<E> From<E> for ServerError where E: Into<anyhow::Error> {
    fn from(err: E) -> Self { Self(err.into()) }
}

pub async fn run_memory_server() -> Result<()> {
    info!("🧠 Starting Integrated Memory Server...");

    let memory_path = env::var("AGENCY_MEMORY_PATH").unwrap_or_else(|_| "memory.json".to_string());
    let memory = LocalVectorMemory::new(memory_path.into())?;
    memory.wake().await?;

    let state = Arc::new(MemoryServerState { memory });

    let app = Router::new()
        .route("/health", get(|| async { "OK" }))
        .route("/store", post(store_handler))
        .route("/search", post(search_handler))
        .route("/persist", post(persist_handler))
        .route("/hibernate", post(hibernate_handler))
        .route("/wake", post(wake_handler))
        .route("/count", get(count_handler))
        .with_state(state);

    let port = env::var("AGENCY_MEMORY_PORT").unwrap_or_else(|_| "3001".to_string());
    let addr = format!("0.0.0.0:{}", port);
    info!("🚀 Memory Server listening at http://{}", addr);

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

async fn store_handler(
    State(state): State<Arc<MemoryServerState>>,
    Json(payload): Json<StoreRequest>,
) -> Result<Json<StoreResponse>, ServerError> {
    let id = state.memory.store(payload.entry).await?;
    Ok(Json(StoreResponse { id }))
}

async fn search_handler(
    State(state): State<Arc<MemoryServerState>>,
    Json(payload): Json<SearchRequest>,
) -> Result<Json<SearchResponse>, ServerError> {
    let entries = state.memory.search(
        &payload.query, 
        payload.top_k, 
        payload.context.as_deref(), 
        payload.kind
    ).await?;
    Ok(Json(SearchResponse { entries }))
}

async fn persist_handler(State(state): State<Arc<MemoryServerState>>) -> Result<Json<serde_json::Value>, ServerError> {
    state.memory.persist().await?;
    Ok(Json(serde_json::json!({ "status": "ok" })))
}

async fn hibernate_handler(State(state): State<Arc<MemoryServerState>>) -> Result<Json<serde_json::Value>, ServerError> {
    state.memory.hibernate().await?;
    Ok(Json(serde_json::json!({ "status": "ok" })))
}

async fn wake_handler(State(state): State<Arc<MemoryServerState>>) -> Json<serde_json::Value> {
    match state.memory.wake().await {
        Ok(_) => Json(serde_json::json!({ "status": "ok" })),
        Err(e) => Json(serde_json::json!({ "error": e.to_string() }))
    }
}

async fn count_handler(State(state): State<Arc<MemoryServerState>>) -> Result<Json<serde_json::Value>, ServerError> {
    let count = state.memory.count().await?;
    Ok(Json(serde_json::json!({ "count": count })))
}
