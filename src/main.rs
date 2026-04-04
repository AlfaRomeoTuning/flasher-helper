mod can;
mod api;
mod dbc;

use std::sync::Arc;
use std::net::SocketAddr;
use std::str::FromStr;
use tokio::sync::RwLock;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
use crate::can::SharedState;

const DEFAULT_PORT: u16 = 3000;
const DEFAULT_BIND_ADDR: &str = "127.0.0.1";
const DEFAULT_INTERFACES: [&str; 3] = ["can-bh", "can-c1", "can-c2"];

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize logging
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| "flasher_helper=info".into()))
        .with(tracing_subscriber::fmt::layer())
        .init();

    let state = Arc::new(SharedState {
        vin: RwLock::new(None),
        status: RwLock::new(crate::can::StatusData::default()),
        decoded: RwLock::new(std::collections::HashMap::new()),
    });

    // Spawn one listener per CAN interface.
    for interface in DEFAULT_INTERFACES {
        let can_state = Arc::clone(&state);
        let interface_name = interface.to_string();
        tokio::spawn(async move {
            if let Err(e) = can::run_can_listener(&interface_name, can_state).await {
                tracing::error!("CAN listener task failed for {}: {}", interface_name, e);
            }
        });
    }

    // Build the API router
    let app = api::create_router(state);

    // Parse port from CLI arg, then env, then default.
    let port: u16 = std::env::args()
        .nth(1)
        .and_then(|arg| arg.parse().ok())
        .or_else(|| {
            std::env::var("FLASHER_PORT")
                .ok()
                .and_then(|arg| arg.parse().ok())
        })
        .unwrap_or(DEFAULT_PORT);

    let bind_addr = std::env::var("FLASHER_BIND_ADDR")
        .unwrap_or_else(|_| DEFAULT_BIND_ADDR.to_string());

    // Run the HTTP server
    let addr = SocketAddr::from_str(&format!("{}:{}", bind_addr, port))?;
    tracing::info!("Starting HTTP API on http://{}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
