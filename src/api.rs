use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use std::collections::HashSet;
use std::sync::Arc;
use std::process::Command;
use std::time::{Duration, Instant};
use tracing::{info, error};
use serde::{Deserialize, Serialize};
use crate::can::{BuRole, DecodedMessageSnapshot, DecodedSignalValue, SharedState};
use crate::dbc::{bu_matches_message, find_bus, lookup_message};

const STATUS_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Serialize)]
struct VinResponse {
    vin: Option<String>,
}

pub fn create_router(state: Arc<SharedState>) -> Router {
    Router::new()
        .route("/vin", get(get_vin))
        .route("/status", get(get_status))
        .route("/can/:interface/:bu", get(get_can_data_by_bu))
        .route("/shutdown", post(shutdown))
        .with_state(state)
}

#[derive(Debug, Deserialize)]
struct CanQuery {
    messages: Option<String>,
}

#[derive(Debug, Serialize)]
struct CanBuResponse {
    interface: String,
    bu: String,
    messages: Vec<CanMessageResponse>,
}

#[derive(Debug, Serialize)]
struct CanMessageResponse {
    message_name: String,
    dlc: u8,
    timestamp_ms: u128,
    signals: std::collections::BTreeMap<String, DecodedSignalValue>,
}

async fn get_vin(State(state): State<Arc<SharedState>>) -> impl IntoResponse {
    let vin = state.vin.read().await.clone();
    Json(VinResponse { vin })
}

async fn get_status(State(state): State<Arc<SharedState>>) -> impl IntoResponse {
    let mut status = state.status.read().await.clone();
    let now = Instant::now();

    if let Some(last_time) = status.last_battery_time {
        if now.duration_since(last_time) > STATUS_TIMEOUT {
            status.battery_voltage = None;
        }
    } else {
        status.battery_voltage = None;
    }

    if let Some(last_time) = status.last_ignition_time {
        if now.duration_since(last_time) > STATUS_TIMEOUT {
            status.ignition_sts = None;
        }
    } else {
        status.ignition_sts = None;
    }

    Json(status)
}

async fn get_can_data_by_bu(
    Path((interface, bu)): Path<(String, String)>,
    Query(query): Query<CanQuery>,
    State(state): State<Arc<SharedState>>,
) -> impl IntoResponse {
    let Some(bus) = find_bus(&interface) else {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({
                "error": format!("unknown interface: {}", interface),
            })),
        )
            .into_response();
    };

    if !bus.bus_units.iter().any(|known_bu| *known_bu == bu) {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({
                "error": format!("unknown BU '{}' for interface {}", bu, interface),
            })),
        )
            .into_response();
    }

    // Endpoint is intentionally TX-only: return only source messages for the requested BU.
    let role = BuRole::Tx;

    let message_filter: Option<HashSet<String>> = query.messages.as_ref().map(|items| {
        items
            .split(',')
            .map(|m| m.trim().to_string())
            .filter(|m| !m.is_empty())
            .collect()
    });

    let decoded = state.decoded.read().await;
    let snapshots = decoded.get(&interface);
    let mut messages = snapshots
        .map(|map| {
            map.values()
                .filter(|snapshot| {
                    if let Some(allowlist) = &message_filter {
                        allowlist.contains(snapshot.message_name.as_str())
                    } else {
                        true
                    }
                })
                .filter(|snapshot| {
                    lookup_message(&interface, snapshot.frame_id)
                        .map(|message_def| bu_matches_message(message_def, &bu, role))
                        .unwrap_or(false)
                })
                .cloned()
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    messages.sort_by(|a, b| a.message_name.cmp(&b.message_name));

    let messages = messages
        .into_iter()
        .map(|snapshot| map_snapshot_to_response(snapshot))
        .collect::<Vec<_>>();

    Json(CanBuResponse {
        interface,
        bu,
        messages,
    })
    .into_response()
}

fn map_snapshot_to_response(snapshot: DecodedMessageSnapshot) -> CanMessageResponse {
    CanMessageResponse {
        message_name: snapshot.message_name,
        dlc: snapshot.dlc,
        timestamp_ms: snapshot.timestamp_ms,
        signals: snapshot.signals,
    }
}

async fn shutdown() -> impl IntoResponse {
    info!("Shutdown request received");
    
    // Security: We execute the shutdown command using std::process::Command.
    // In a real production environment, the user running this process must have
    // the necessary permissions (e.g., via sudoers) to execute 'shutdown'.
    match Command::new("shutdown").arg("-h").arg("now").spawn() {
        Ok(_) => {
            info!("Shutdown command initiated successfully");
            (StatusCode::ACCEPTED, "Shutting down...")
        }
        Err(e) => {
            error!("Failed to execute shutdown command: {}", e);
            (StatusCode::INTERNAL_SERVER_ERROR, "Failed to initiate shutdown")
        }
    }
}
