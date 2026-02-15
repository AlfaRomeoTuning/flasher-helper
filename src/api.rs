use axum::{
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use std::sync::Arc;
use std::process::Command;
use tracing::{info, warn, error};
use serde::Serialize;
use crate::can::SharedState;

#[derive(Serialize)]
struct VinResponse {
    vin: Option<String>,
}

pub fn create_router(state: Arc<SharedState>) -> Router {
    Router::new()
        .route("/vin", get(get_vin))
        .route("/shutdown", post(shutdown))
        .with_state(state)
}

async fn get_vin(State(state): State<Arc<SharedState>>) -> impl IntoResponse {
    let vin = state.vin.read().unwrap().clone();
    Json(VinResponse { vin })
}

async fn shutdown() -> impl IntoResponse {
    info!("Shutdown request received");
    
    // Security: We execute the shutdown command using std::process::Command.
    // In a real production environment, the user running this process must have
    // the necessary permissions (e.g., via sudoers) to execute 'shutdown'.
    match Command::new("shutdown").arg("-h").arg("now").spawn() {
        Ok(_) => {
            info!("Shutdown command initiated successfully");
            (StatusCode::OK, "Shutting down...")
        }
        Err(e) => {
            error!("Failed to execute shutdown command: {}", e);
            (StatusCode::INTERNAL_SERVER_ERROR, "Failed to initiate shutdown")
        }
    }
}
