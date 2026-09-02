//! nexus-client binary entrypoint
//! Part of Nexus Remote Desktop Platform

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();
    tracing::info!("Starting nexus-client v{}", env!("CARGO_PKG_VERSION"));
    let configuration = nexus_client::ClientConfiguration::from_env()
        .map_err(|error| anyhow::anyhow!("invalid nexus-client configuration: {error}"))?;
    tracing::info!(
        server = %configuration.server,
        server_name = %configuration.server_name,
        "validated non-secret client configuration"
    );
    let cancellation = nexus_client::RuntimeCancellation::new();
    let ctrl_c_cancellation = cancellation.clone();
    tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            ctrl_c_cancellation.cancel();
        }
    });
    nexus_client::ClientRuntime::run_configured_with_cancellation(configuration, cancellation)
        .await
        .map(|_| ())
        .map_err(|error| anyhow::anyhow!("nexus-client runtime stopped: {error}"))
}
