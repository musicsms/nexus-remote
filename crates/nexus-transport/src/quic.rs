//! QUIC endpoint setup for local loopback (Spec Section 6, 14; ADR-003).
//!
//! Uses a self-signed certificate (`rcgen`) so client and server can
//! establish a TLS 1.3-secured QUIC connection without a real CA. This
//! is a Phase 0 PoC helper for same-process loopback testing — it is NOT
//! the production certificate model (real deployments use control-plane-
//! issued or CA-issued certs).

use std::{net::SocketAddr, sync::Arc, time::Duration};

use quinn::{ClientConfig, Endpoint, ServerConfig, TransportConfig};
use rustls::pki_types::{CertificateDer, PrivatePkcs8KeyDer};

pub struct ServerEndpoint {
    pub endpoint: Endpoint,
    pub cert_der: CertificateDer<'static>,
}

/// Installs the process-wide default `rustls` crypto provider on first
/// call. Both `ring` and `aws-lc-rs` are present in the dependency graph
/// (pulled in transitively by different crates), so `rustls` can no longer
/// auto-select one — it must be picked explicitly before any `ClientConfig`
/// or `ServerConfig` is built, or rustls panics at first use. Repeated
/// calls (e.g. server + client in the same process, as in the test below)
/// are safe: `install_default` failing because a provider is already
/// installed is not an error for our purposes.
fn ensure_crypto_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

fn transport_config() -> TransportConfig {
    let mut config = TransportConfig::default();
    config.max_idle_timeout(Some(
        Duration::from_secs(30).try_into().expect("fits in VarInt"),
    ));
    // Explicit, non-default receive buffer for video-style datagram
    // traffic (Spec Section 57 rule 1: no unbounded channels/queues).
    config.datagram_receive_buffer_size(Some(64 * 1024));
    config
}

/// Binds a QUIC server endpoint on `bind_addr` with a freshly generated
/// self-signed certificate for `localhost`.
pub fn make_server_endpoint(bind_addr: SocketAddr) -> anyhow::Result<ServerEndpoint> {
    ensure_crypto_provider();
    let generated = rcgen::generate_simple_self_signed(vec!["localhost".to_string()])?;
    let cert_der = CertificateDer::from(generated.cert);
    let key_der = PrivatePkcs8KeyDer::from(generated.key_pair.serialize_der());

    let mut server_config = ServerConfig::with_single_cert(vec![cert_der.clone()], key_der.into())?;
    server_config.transport_config(Arc::new(transport_config()));

    let endpoint = Endpoint::server(server_config, bind_addr)?;

    Ok(ServerEndpoint { endpoint, cert_der })
}

/// Binds a QUIC client endpoint on `bind_addr` that trusts exactly
/// `server_cert` (the certificate returned by `make_server_endpoint`).
pub fn make_client_endpoint(
    bind_addr: SocketAddr,
    server_cert: &CertificateDer<'static>,
) -> anyhow::Result<Endpoint> {
    ensure_crypto_provider();
    let mut roots = rustls::RootCertStore::empty();
    roots.add(server_cert.clone())?;

    let client_crypto = rustls::ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();

    let quic_client_config = quinn::crypto::rustls::QuicClientConfig::try_from(client_crypto)?;
    let mut client_config = ClientConfig::new(Arc::new(quic_client_config));
    client_config.transport_config(Arc::new(transport_config()));

    let mut endpoint = Endpoint::client(bind_addr)?;
    endpoint.set_default_client_config(client_config);

    Ok(endpoint)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn client_connects_to_server() {
        let server = make_server_endpoint("127.0.0.1:0".parse().unwrap()).unwrap();
        let server_addr = server.endpoint.local_addr().unwrap();
        let cert_der = server.cert_der.clone();

        let server_task = tokio::spawn(async move {
            let incoming = server
                .endpoint
                .accept()
                .await
                .expect("no incoming connection");
            let connection = incoming.await.expect("handshake failed");
            connection.remote_address()
        });

        let client = make_client_endpoint("127.0.0.1:0".parse().unwrap(), &cert_der).unwrap();
        let connection = client
            .connect(server_addr, "localhost")
            .expect("connect() setup failed")
            .await
            .expect("handshake failed");

        assert_eq!(connection.remote_address().port(), server_addr.port());

        let server_saw = server_task.await.unwrap();
        assert_eq!(
            server_saw.ip(),
            connection.local_ip().unwrap_or(server_saw.ip())
        );
    }
}
