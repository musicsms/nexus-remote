//! nexusd binary entrypoint
//! Part of Nexus Remote Desktop Platform

use ed25519_dalek::SigningKey;
use nexusd::server::ControlPlaneServer;
use nexusd::state::AppState;
use std::net::SocketAddr;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();
    tracing::info!(
        "Starting nexusd control plane v{}",
        env!("CARGO_PKG_VERSION")
    );

    let signing_key = SigningKey::from_bytes(&[100u8; 32]);
    let state = AppState::new(signing_key, "nexus-control-plane-primary");

    let bind_addr: SocketAddr = "0.0.0.0:8080".parse()?;
    let server = ControlPlaneServer::bind(bind_addr, state).await?;
    tracing::info!("nexusd listening on {}", server.local_addr()?);

    server.run().await?;
    Ok(())
}
