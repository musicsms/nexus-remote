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
    Err(anyhow::anyhow!(
        "authenticated session bootstrap is not configured; capability, relay token, certificate, and frame key must be supplied by the control plane before ClientRuntime::connect"
    ))
}
