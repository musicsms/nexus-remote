//! nexusd binary entrypoint
//! Part of Nexus Remote Desktop Platform

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();
    tracing::info!("Starting nexusd v{}", env!("CARGO_PKG_VERSION"));
    Ok(())
}
