use axum::{
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use std::sync::Arc;
use std::process::Command;
use tracing::{info, error};
use serde::Serialize;
use crate::can::SharedState;

#[derive(Serialize)]
struct VinResponse {
    vin: Option<String>,
}

pub fn create_router(state: Arc<SharedState>) -> Router {
    Router::new()
        .route("/vin", get(get_vin))
        .route("/status", get(get_status))
        .route("/shutdown", post(shutdown))
        .with_state(state)
}

async fn get_vin(State(state): State<Arc<SharedState>>) -> impl IntoResponse {
    let vin = state.vin.read().unwrap().clone();
    Json(VinResponse { vin })
}

async fn get_status(State(state): State<Arc<SharedState>>) -> impl IntoResponse {
    let mut status = state.status.read().unwrap().clone();
    let now = std::time::Instant::now();
    let timeout = std::time::Duration::from_secs(5);

    if let Some(last_time) = status.last_battery_time {
        if now.duration_since(last_time) > timeout {
            status.battery_voltage = None;
        }
    } else {
        status.battery_voltage = None;
    }

    if let Some(last_time) = status.last_ignition_time {
        if now.duration_since(last_time) > timeout {
            status.ignition_sts = None;
        }
    } else {
        status.ignition_sts = None;
    }

    Json(status)
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
