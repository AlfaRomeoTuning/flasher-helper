use crate::can::{BuRole, DecodedMessageSnapshot, DecodedSignalValue, SharedState};
use crate::dbc::{bu_matches_message, find_bus, lookup_message, metadata};
use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::{error, info, warn};

const STATUS_TIMEOUT: Duration = Duration::from_secs(5);
const SHUTDOWN_TOKEN_ENV: &str = "FLASHER_SHUTDOWN_TOKEN";
const SHUTDOWN_TOKEN_HEADER: &str = "x-shutdown-token";

#[derive(Serialize)]
struct VinResponse {
    vin: Option<String>,
}

pub fn create_router(state: Arc<SharedState>) -> Router {
    Router::new()
        .route("/vin", get(get_vin))
        .route("/status", get(get_status))
        .route("/can/:interface/:bu", get(get_can_data_by_bu))
        .route("/can/:interface/metadata", get(get_can_metadata))
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
    message_name: &'static str,
    dlc: u8,
    timestamp_ms: u128,
    signals: std::collections::BTreeMap<&'static str, DecodedSignalValue>,
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

    let message_filter: Option<HashSet<&str>> = query.messages.as_deref().map(|items| {
        items
            .split(',')
            .map(str::trim)
            .filter(|m| !m.is_empty())
            .collect()
    });

    let mut messages = {
        let decoded = state.decoded.read().await;
        decoded
            .get(interface.as_str())
            .map(|map| {
                map.values()
                    .filter(|snapshot| {
                        if let Some(allowlist) = &message_filter {
                            allowlist.contains(snapshot.message_name)
                        } else {
                            true
                        }
                    })
                    .cloned()
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    };

    messages.retain(|snapshot| {
        lookup_message(&interface, snapshot.frame_id)
            .map(|message_def| bu_matches_message(message_def, &bu, role))
            .unwrap_or(false)
    });

    messages.sort_by(|a, b| a.message_name.cmp(b.message_name));

    let messages = messages
        .into_iter()
        .map(map_snapshot_to_response)
        .collect::<Vec<_>>();

    Json(CanBuResponse {
        interface,
        bu,
        messages,
    })
    .into_response()
}

#[derive(Debug, Serialize)]
struct CanMetadataResponse {
    interface: String,
    bus_units: Vec<&'static str>,
    messages_by_bu: BTreeMap<&'static str, Vec<&'static str>>,
}

async fn get_can_metadata(
    Path(interface): Path<String>,
    State(_state): State<Arc<SharedState>>,
) -> impl IntoResponse {
    let Some(metadata) = metadata(&interface) else {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({
                "error": format!("unknown interface: {}", interface),
            })),
        )
            .into_response();
    };

    let messages_by_bu = metadata
        .messages_by_bu
        .iter()
        .map(|(bu, names)| (*bu, names.clone()))
        .collect::<BTreeMap<_, _>>();

    Json(CanMetadataResponse {
        interface,
        bus_units: metadata.bus_units.clone(),
        messages_by_bu,
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

async fn shutdown(headers: HeaderMap) -> impl IntoResponse {
    let expected_token = match std::env::var(SHUTDOWN_TOKEN_ENV) {
        Ok(token) if !token.is_empty() => token,
        _ => {
            error!(
                "{} is not set; refusing shutdown request",
                SHUTDOWN_TOKEN_ENV
            );
            return (StatusCode::FORBIDDEN, "Shutdown is disabled");
        }
    };

    let provided_token = headers
        .get(SHUTDOWN_TOKEN_HEADER)
        .and_then(|value| value.to_str().ok());

    if provided_token != Some(expected_token.as_str()) {
        warn!("Unauthorized shutdown request");
        return (StatusCode::UNAUTHORIZED, "Unauthorized");
    }

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
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to initiate shutdown",
            )
        }
    }
}
