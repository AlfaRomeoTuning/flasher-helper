mod can;
mod api;

use std::sync::{Arc, RwLock};
use std::net::SocketAddr;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
use crate::can::SharedState;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize logging
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| "flasher_helper=info,tower_http=debug".into()))
        .with(tracing_subscriber::fmt::layer())
        .init();

    let state = Arc::new(SharedState {
        vin: RwLock::new(None),
        status: RwLock::new(crate::can::StatusData::default()),
    });

    // Spawn CAN listener task
    let can_state = Arc::clone(&state);
    tokio::spawn(async move {
        if let Err(e) = can::run_can_listener("can-c1", can_state).await {
            tracing::error!("CAN listener task failed: {}", e);
        }
    });

    // Build the API router
    let app = api::create_router(state);

    // Parse port from command-line arguments (default: 3000)
    let port: u16 = std::env::args()
        .nth(1)
        .and_then(|arg| arg.parse().ok())
        .unwrap_or(3000);

    // Run the HTTP server
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    tracing::info!("Starting HTTP API on http://{}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
