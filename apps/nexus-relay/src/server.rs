//! QUIC Relay Server implementation and lifecycle management.
//! Part of Nexus Remote Desktop Platform.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use nexus_common::id::SessionId;
use nexus_common::time::{Clock, SystemClock};
use quinn::{ClientConfig, Endpoint, ServerConfig, TransportConfig};
use rustls::pki_types::{CertificateDer, PrivatePkcs8KeyDer};
use tokio::sync::{oneshot, Mutex, Notify};

use crate::forwarder::{
    forward_session, read_handshake_token, write_handshake_response, RelayHandshakeResponse,
    RelayServerError,
};
use crate::session::RelaySessionTable;
use crate::token::{EndpointRole, RelayTokenVerifier};

/// Installs the default `rustls` crypto provider (Ring) if not already installed.
pub fn ensure_crypto_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

/// Builds a standard QUIC [`TransportConfig`] with timeouts and datagram buffer settings.
pub fn default_transport_config() -> TransportConfig {
    let mut config = TransportConfig::default();
    config.max_idle_timeout(Some(
        Duration::from_secs(30).try_into().expect("fits in VarInt"),
    ));
    config.datagram_receive_buffer_size(Some(64 * 1024));
    config.datagram_send_buffer_size(64 * 1024);
    config
}

/// Generates a self-signed TLS certificate and constructs a [`ServerConfig`] for QUIC.
pub fn generate_self_signed_server_config(
    subject_alt_names: Vec<String>,
) -> Result<(ServerConfig, CertificateDer<'static>), RelayServerError> {
    ensure_crypto_provider();
    let san = if subject_alt_names.is_empty() {
        vec!["localhost".to_string()]
    } else {
        subject_alt_names
    };
    let generated = rcgen::generate_simple_self_signed(san)
        .map_err(|e| RelayServerError::Certificate(e.to_string()))?;
    let cert_der = CertificateDer::from(generated.cert);
    let key_der = PrivatePkcs8KeyDer::from(generated.key_pair.serialize_der());

    let mut server_config = ServerConfig::with_single_cert(vec![cert_der.clone()], key_der.into())
        .map_err(|e| RelayServerError::Tls(e.to_string()))?;
    server_config.transport_config(Arc::new(default_transport_config()));
    Ok((server_config, cert_der))
}

/// Creates a [`ClientConfig`] that trusts the provided server certificate.
pub fn make_client_config(
    server_cert: &CertificateDer<'static>,
) -> Result<ClientConfig, RelayServerError> {
    ensure_crypto_provider();
    let mut roots = rustls::RootCertStore::empty();
    roots
        .add(server_cert.clone())
        .map_err(|e| RelayServerError::Tls(e.to_string()))?;

    let client_crypto = rustls::ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();

    let quic_client_config = quinn::crypto::rustls::QuicClientConfig::try_from(client_crypto)
        .map_err(|e| RelayServerError::Tls(e.to_string()))?;
    let mut client_config = ClientConfig::new(Arc::new(quic_client_config));
    client_config.transport_config(Arc::new(default_transport_config()));
    Ok(client_config)
}

/// Creates a client QUIC [`Endpoint`] trusting the provided server certificate.
pub fn make_client_endpoint(
    bind_addr: SocketAddr,
    server_cert: &CertificateDer<'static>,
) -> Result<Endpoint, RelayServerError> {
    let client_config = make_client_config(server_cert)?;
    let mut endpoint = Endpoint::client(bind_addr)?;
    endpoint.set_default_client_config(client_config);
    Ok(endpoint)
}

type WaitingPeer = (
    EndpointRole,
    quinn::Connection,
    oneshot::Sender<(EndpointRole, quinn::Connection)>,
);

/// Stateless Encrypted Relay Server hosting QUIC endpoints for Client and Host connections.
#[derive(Clone)]
pub struct RelayServer {
    endpoint: Endpoint,
    session_table: Arc<RelaySessionTable>,
    verifier: Arc<RelayTokenVerifier>,
    server_cert_der: Option<CertificateDer<'static>>,
    waiting_peers: Arc<Mutex<HashMap<SessionId, WaitingPeer>>>,
    shutdown_notify: Arc<Notify>,
}

impl RelayServer {
    /// Binds a new [`RelayServer`] on `bind_addr` using a self-signed TLS certificate
    /// and the provided [`RelayTokenVerifier`].
    pub fn bind(
        bind_addr: SocketAddr,
        verifier: RelayTokenVerifier,
    ) -> Result<Self, RelayServerError> {
        let (server_config, cert_der) =
            generate_self_signed_server_config(vec!["localhost".to_string()])?;
        Self::with_server_config(bind_addr, verifier, server_config, Some(cert_der))
    }

    /// Binds a new [`RelayServer`] on `bind_addr` using custom [`ServerConfig`].
    pub fn with_server_config(
        bind_addr: SocketAddr,
        verifier: RelayTokenVerifier,
        server_config: ServerConfig,
        cert_der: Option<CertificateDer<'static>>,
    ) -> Result<Self, RelayServerError> {
        ensure_crypto_provider();
        let endpoint = Endpoint::server(server_config, bind_addr)?;
        Ok(Self {
            endpoint,
            session_table: Arc::new(RelaySessionTable::new()),
            verifier: Arc::new(verifier),
            server_cert_der: cert_der,
            waiting_peers: Arc::new(Mutex::new(HashMap::new())),
            shutdown_notify: Arc::new(Notify::new()),
        })
    }

    /// Returns the local socket address this server is listening on.
    pub fn local_addr(&self) -> std::io::Result<SocketAddr> {
        self.endpoint.local_addr()
    }

    /// Returns a reference to the underlying Quinn [`Endpoint`].
    pub fn endpoint(&self) -> &Endpoint {
        &self.endpoint
    }

    /// Returns a reference to the shared [`RelaySessionTable`].
    pub fn session_table(&self) -> &Arc<RelaySessionTable> {
        &self.session_table
    }

    /// Returns a reference to the [`RelayTokenVerifier`].
    pub fn verifier(&self) -> &RelayTokenVerifier {
        &self.verifier
    }

    /// Returns the self-signed server certificate DER if generated.
    pub fn server_cert_der(&self) -> Option<&CertificateDer<'static>> {
        self.server_cert_der.as_ref()
    }

    /// Runs the relay server accept loop, handling incoming connections asynchronously.
    pub async fn run(&self) -> Result<(), RelayServerError> {
        tracing::info!(
            local_addr = ?self.local_addr()?,
            relay_id = self.verifier.relay_id(),
            "relay server running"
        );

        loop {
            tokio::select! {
                incoming = self.endpoint.accept() => {
                    match incoming {
                        Some(incoming_conn) => {
                            let verifier = Arc::clone(&self.verifier);
                            let session_table = Arc::clone(&self.session_table);
                            let waiting_peers = Arc::clone(&self.waiting_peers);

                            tokio::spawn(async move {
                                match incoming_conn.await {
                                    Ok(conn) => {
                                        Self::handle_connection(conn, verifier, session_table, waiting_peers).await;
                                    }
                                    Err(e) => {
                                        tracing::debug!(error = %e, "incoming QUIC handshake failed");
                                    }
                                }
                            });
                        }
                        None => {
                            tracing::info!("relay server endpoint closed");
                            break;
                        }
                    }
                }
                _ = self.shutdown_notify.notified() => {
                    tracing::info!("relay server received shutdown signal");
                    break;
                }
            }
        }

        Ok(())
    }

    /// Shuts down the relay server endpoint.
    pub fn shutdown(&self) {
        self.endpoint
            .close(quinn::VarInt::from_u32(0), b"server shutdown");
        self.shutdown_notify.notify_waiters();
    }

    async fn handle_connection(
        conn: quinn::Connection,
        verifier: Arc<RelayTokenVerifier>,
        session_table: Arc<RelaySessionTable>,
        waiting_peers: Arc<Mutex<HashMap<SessionId, WaitingPeer>>>,
    ) {
        // Accept the handshake bi-stream with a timeout
        let (mut send, mut recv) =
            match tokio::time::timeout(Duration::from_secs(5), conn.accept_bi()).await {
                Ok(Ok(bi)) => bi,
                Ok(Err(e)) => {
                    tracing::debug!(error = %e, "failed to accept handshake stream");
                    conn.close(quinn::VarInt::from_u32(1), b"handshake stream error");
                    return;
                }
                Err(_) => {
                    tracing::debug!("timed out waiting for handshake stream");
                    conn.close(quinn::VarInt::from_u32(1), b"handshake timeout");
                    return;
                }
            };

        // Read and parse RelayToken
        let token = match read_handshake_token(&mut recv).await {
            Ok(t) => t,
            Err(e) => {
                tracing::warn!(error = %e, "failed to read handshake token");
                let resp = RelayHandshakeResponse {
                    success: false,
                    session_id: None,
                    role: None,
                    is_paired: None,
                    error: Some(e.to_string()),
                };
                let _ = write_handshake_response(&mut send, &resp).await;
                conn.close(quinn::VarInt::from_u32(2), b"invalid handshake token");
                return;
            }
        };

        // Validate token signature and constraints
        let now = SystemClock.now();
        if let Err(e) = verifier.verify(&token, now) {
            tracing::warn!(session_id = %token.session_id, error = %e, "relay token verification failed");
            let resp = RelayHandshakeResponse {
                success: false,
                session_id: Some(token.session_id.clone()),
                role: Some(token.role),
                is_paired: None,
                error: Some(e.to_string()),
            };
            let _ = write_handshake_response(&mut send, &resp).await;
            conn.close(quinn::VarInt::from_u32(3), b"token verification failed");
            return;
        }

        // Register or join session in session table
        let (session, is_paired) = match session_table.register_or_join(&token, now) {
            Ok(pair) => pair,
            Err(e) => {
                tracing::warn!(session_id = %token.session_id, error = %e, "session registration failed");
                let resp = RelayHandshakeResponse {
                    success: false,
                    session_id: Some(token.session_id.clone()),
                    role: Some(token.role),
                    is_paired: None,
                    error: Some(e.to_string()),
                };
                let _ = write_handshake_response(&mut send, &resp).await;
                conn.close(quinn::VarInt::from_u32(4), b"session pairing failed");
                return;
            }
        };

        // Send successful handshake response
        let resp = RelayHandshakeResponse {
            success: true,
            session_id: Some(token.session_id.clone()),
            role: Some(token.role),
            is_paired: Some(is_paired),
            error: None,
        };

        let waiting_rx = if !is_paired {
            let (tx, rx) = oneshot::channel();
            waiting_peers
                .lock()
                .await
                .insert(token.session_id.clone(), (token.role, conn.clone(), tx));
            Some(rx)
        } else {
            None
        };
        if let Err(e) = write_handshake_response(&mut send, &resp).await {
            tracing::warn!(session_id = %token.session_id, error = %e, "failed to write handshake response");
            conn.close(
                quinn::VarInt::from_u32(5),
                b"failed to send handshake response",
            );
            return;
        }

        let session_id = token.session_id.clone();
        let role = token.role;

        if !is_paired {
            // First endpoint to connect: wait for peer
            let rx = waiting_rx.expect("waiting receiver exists for first endpoint");

            let session_table_cloned = Arc::clone(&session_table);
            let session_cloned = Arc::clone(&session);
            let waiting_peers_cloned = Arc::clone(&waiting_peers);

            tokio::select! {
                peer_res = rx => {
                    match peer_res {
                        Ok((_peer_role, peer_conn)) => {
                            let (client_conn, host_conn) = match role {
                                EndpointRole::Client => (conn.clone(), peer_conn),
                                EndpointRole::Host => (peer_conn, conn.clone()),
                            };
                            forward_session(client_conn, host_conn, session_cloned).await;
                            session_table_cloned.remove_session(&session_id);
                        }
                        Err(_) => {
                            session_table_cloned.remove_session(&session_id);
                        }
                    }
                }
                _ = conn.closed() => {
                    tracing::info!(session_id = %session_id, role = %role, "endpoint disconnected while waiting for peer");
                    waiting_peers_cloned.lock().await.remove(&session_id);
                    session_table_cloned.remove_session(&session_id);
                }
            }
        } else {
            // Second endpoint to connect: notify the waiting peer
            let waiting = {
                let mut map = waiting_peers.lock().await;
                map.remove(&session_id)
            };

            if let Some((_peer_role, _peer_conn, tx)) = waiting {
                if tx.send((role, conn.clone())).is_err() {
                    tracing::warn!(session_id = %session_id, "failed to hand over peer connection; waiting peer dropped");
                    session_table.remove_session(&session_id);
                    conn.close(quinn::VarInt::from_u32(6), b"peer disconnected prematurely");
                    return;
                }
                // Keep the second connection task alive until the connection terminates
                let _ = conn.closed().await;
            } else {
                tracing::warn!(session_id = %session_id, "no waiting peer found despite is_paired=true");
                session_table.remove_session(&session_id);
                conn.close(quinn::VarInt::from_u32(7), b"pairing state mismatch");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::forwarder::perform_client_handshake;
    use crate::token::RelayToken;
    use ed25519_dalek::SigningKey;
    use nexus_common::id::{DeviceId, SessionId};
    use nexus_common::time::UnixTimestamp;

    fn setup_test_server() -> (RelayServer, SigningKey, RelayTokenVerifier) {
        let signing_key = SigningKey::from_bytes(&[42u8; 32]);
        let verifying_key = signing_key.verifying_key();
        let verifier = RelayTokenVerifier::new(verifying_key, "relay-test-1");

        let server = RelayServer::bind("127.0.0.1:0".parse().unwrap(), verifier.clone()).unwrap();
        (server, signing_key, verifier)
    }

    fn create_test_tokens(
        signing_key: &SigningKey,
        relay_id: &str,
    ) -> (RelayToken, RelayToken, SessionId) {
        let session_id = SessionId::new("sess-quic-test-1").unwrap();
        let client_device_id = DeviceId::new("dev-client-1").unwrap();
        let target_device_id = DeviceId::new("dev-host-1").unwrap();
        let expires_at = UnixTimestamp::from_secs(2_000_000_000);

        let mut client_token = RelayToken::builder()
            .session_id(session_id.clone())
            .relay_id(relay_id)
            .client_device_id(client_device_id.clone())
            .target_device_id(target_device_id.clone())
            .role(EndpointRole::Client)
            .expires_at(expires_at)
            .build()
            .unwrap();
        client_token.sign(signing_key);

        let mut host_token = RelayToken::builder()
            .session_id(session_id.clone())
            .relay_id(relay_id)
            .client_device_id(client_device_id)
            .target_device_id(target_device_id)
            .role(EndpointRole::Host)
            .expires_at(expires_at)
            .build()
            .unwrap();
        host_token.sign(signing_key);

        (client_token, host_token, session_id)
    }

    #[tokio::test]
    async fn test_server_bind_and_properties() {
        let (server, _signing_key, _verifier) = setup_test_server();
        let addr = server.local_addr().unwrap();
        assert_ne!(addr.port(), 0);
        assert!(server.server_cert_der().is_some());
        assert_eq!(server.session_table().len(), 0);
    }

    #[tokio::test]
    async fn test_handshake_success_and_pairing() {
        let (server, signing_key, verifier) = setup_test_server();
        let server_addr = server.local_addr().unwrap();
        let cert_der = server.server_cert_der().unwrap().clone();

        let server_handle = {
            let server = server.clone();
            tokio::spawn(async move {
                server.run().await.unwrap();
            })
        };

        let (client_token, host_token, session_id) =
            create_test_tokens(&signing_key, verifier.relay_id());

        // Connect Client
        let client_endpoint =
            make_client_endpoint("127.0.0.1:0".parse().unwrap(), &cert_der).unwrap();
        let client_conn = client_endpoint
            .connect(server_addr, "localhost")
            .unwrap()
            .await
            .unwrap();
        let client_resp = perform_client_handshake(&client_conn, &client_token)
            .await
            .unwrap();

        assert!(client_resp.success);
        assert_eq!(client_resp.session_id, Some(session_id.clone()));
        assert_eq!(client_resp.role, Some(EndpointRole::Client));
        assert_eq!(client_resp.is_paired, Some(false));

        // Connect Host
        let host_endpoint =
            make_client_endpoint("127.0.0.1:0".parse().unwrap(), &cert_der).unwrap();
        let host_conn = host_endpoint
            .connect(server_addr, "localhost")
            .unwrap()
            .await
            .unwrap();
        let host_resp = perform_client_handshake(&host_conn, &host_token)
            .await
            .unwrap();

        assert!(host_resp.success);
        assert_eq!(host_resp.session_id, Some(session_id.clone()));
        assert_eq!(host_resp.role, Some(EndpointRole::Host));
        assert_eq!(host_resp.is_paired, Some(true));

        // Wait brief moment for session table update
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(server.session_table().active_session_count(), 1);

        server.shutdown();
        let _ = server_handle.await;
    }

    #[tokio::test]
    async fn test_handshake_invalid_signature_rejected() {
        let (server, signing_key, verifier) = setup_test_server();
        let server_addr = server.local_addr().unwrap();
        let cert_der = server.server_cert_der().unwrap().clone();

        let server_handle = {
            let server = server.clone();
            tokio::spawn(async move {
                server.run().await.unwrap();
            })
        };

        let (mut client_token, _, _) = create_test_tokens(&signing_key, verifier.relay_id());
        // Corrupt signature
        if let Some(byte) = client_token.signature.first_mut() {
            *byte ^= 0xff;
        }

        let client_endpoint =
            make_client_endpoint("127.0.0.1:0".parse().unwrap(), &cert_der).unwrap();
        let client_conn = client_endpoint
            .connect(server_addr, "localhost")
            .unwrap()
            .await
            .unwrap();

        let result = perform_client_handshake(&client_conn, &client_token).await;
        assert!(result.is_err());

        server.shutdown();
        let _ = server_handle.await;
    }

    #[tokio::test]
    async fn test_handshake_expired_token_rejected() {
        let (server, signing_key, verifier) = setup_test_server();
        let server_addr = server.local_addr().unwrap();
        let cert_der = server.server_cert_der().unwrap().clone();

        let server_handle = {
            let server = server.clone();
            tokio::spawn(async move {
                server.run().await.unwrap();
            })
        };

        let session_id = SessionId::new("sess-expired-test").unwrap();
        let client_device_id = DeviceId::new("dev-client-1").unwrap();
        let target_device_id = DeviceId::new("dev-host-1").unwrap();

        let mut expired_token = RelayToken::builder()
            .session_id(session_id)
            .relay_id(verifier.relay_id())
            .client_device_id(client_device_id)
            .target_device_id(target_device_id)
            .role(EndpointRole::Client)
            .expires_at(UnixTimestamp::from_secs(1_000))
            .build()
            .unwrap();
        expired_token.sign(&signing_key);

        let client_endpoint =
            make_client_endpoint("127.0.0.1:0".parse().unwrap(), &cert_der).unwrap();
        let client_conn = client_endpoint
            .connect(server_addr, "localhost")
            .unwrap()
            .await
            .unwrap();

        let result = perform_client_handshake(&client_conn, &expired_token).await;
        assert!(result.is_err());

        server.shutdown();
        let _ = server_handle.await;
    }

    #[tokio::test]
    async fn test_handshake_relay_id_mismatch_rejected() {
        let (server, signing_key, _verifier) = setup_test_server();
        let server_addr = server.local_addr().unwrap();
        let cert_der = server.server_cert_der().unwrap().clone();

        let server_handle = {
            let server = server.clone();
            tokio::spawn(async move {
                server.run().await.unwrap();
            })
        };

        let (client_token, _, _) = create_test_tokens(&signing_key, "wrong-relay-id");

        let client_endpoint =
            make_client_endpoint("127.0.0.1:0".parse().unwrap(), &cert_der).unwrap();
        let client_conn = client_endpoint
            .connect(server_addr, "localhost")
            .unwrap()
            .await
            .unwrap();

        let result = perform_client_handshake(&client_conn, &client_token).await;
        assert!(result.is_err());

        server.shutdown();
        let _ = server_handle.await;
    }

    #[tokio::test]
    async fn test_bidirectional_stream_forwarding() {
        let _ = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::DEBUG)
            .with_test_writer()
            .try_init();

        let (server, signing_key, verifier) = setup_test_server();
        let server_addr = server.local_addr().unwrap();
        let cert_der = server.server_cert_der().unwrap().clone();

        let server_handle = {
            let server = server.clone();
            tokio::spawn(async move {
                server.run().await.unwrap();
            })
        };

        let (client_token, host_token, session_id) =
            create_test_tokens(&signing_key, verifier.relay_id());

        // Connect Client and Host
        let client_endpoint =
            make_client_endpoint("127.0.0.1:0".parse().unwrap(), &cert_der).unwrap();
        let client_conn = client_endpoint
            .connect(server_addr, "localhost")
            .unwrap()
            .await
            .unwrap();
        perform_client_handshake(&client_conn, &client_token)
            .await
            .unwrap();

        let host_endpoint =
            make_client_endpoint("127.0.0.1:0".parse().unwrap(), &cert_der).unwrap();
        let host_conn = host_endpoint
            .connect(server_addr, "localhost")
            .unwrap()
            .await
            .unwrap();
        perform_client_handshake(&host_conn, &host_token)
            .await
            .unwrap();

        // Host listens for bi-stream from Client
        let host_conn_for_task = host_conn.clone();
        let host_task = tokio::spawn(async move {
            let (mut host_send, mut host_recv) = host_conn_for_task.accept_bi().await.unwrap();
            let mut buf = [0u8; 18];
            host_recv.read_exact(&mut buf).await.unwrap();
            assert_eq!(&buf, b"ping from client!!");
            host_send.write_all(b"pong from host!!").await.unwrap();
            let _ = host_send.finish();
        });

        // Allow the relay's paired-session forwarder to install its stream
        // accept loops before opening the first application stream.
        tokio::time::sleep(Duration::from_millis(150)).await;

        // Client opens bi-stream to Host
        let (mut client_send, mut client_recv) = client_conn.open_bi().await.unwrap();
        client_send.write_all(b"ping from client!!").await.unwrap();
        client_send.finish().unwrap();

        let mut reply = [0u8; 16];
        client_recv.read_exact(&mut reply).await.unwrap();
        assert_eq!(&reply, b"pong from host!!");

        host_task.await.unwrap();

        // Keep connections and endpoints alive until test assertions finish
        let _ = (&client_conn, &host_conn, &client_endpoint, &host_endpoint);

        // Check metrics
        let session = server.session_table().get_session(&session_id).unwrap();
        let snapshot = session.snapshot();
        assert!(snapshot.client_to_host_bytes >= 18);
        assert!(snapshot.host_to_client_bytes >= 16);

        server.shutdown();
        let _ = server_handle.await;
    }

    #[tokio::test]
    async fn test_datagram_forwarding() {
        let (server, signing_key, verifier) = setup_test_server();
        let server_addr = server.local_addr().unwrap();
        let cert_der = server.server_cert_der().unwrap().clone();

        let server_handle = {
            let server = server.clone();
            tokio::spawn(async move {
                server.run().await.unwrap();
            })
        };

        let (client_token, host_token, session_id) =
            create_test_tokens(&signing_key, verifier.relay_id());

        // Connect Client and Host
        let client_endpoint =
            make_client_endpoint("127.0.0.1:0".parse().unwrap(), &cert_der).unwrap();
        let client_conn = client_endpoint
            .connect(server_addr, "localhost")
            .unwrap()
            .await
            .unwrap();
        perform_client_handshake(&client_conn, &client_token)
            .await
            .unwrap();

        let host_endpoint =
            make_client_endpoint("127.0.0.1:0".parse().unwrap(), &cert_der).unwrap();
        let host_conn = host_endpoint
            .connect(server_addr, "localhost")
            .unwrap()
            .await
            .unwrap();
        perform_client_handshake(&host_conn, &host_token)
            .await
            .unwrap();

        // Allow forwarder loops to start
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Client -> Host datagram
        let msg = bytes::Bytes::from_static(b"video packet client->host");
        client_conn.send_datagram(msg.clone()).unwrap();

        let received_on_host = host_conn.read_datagram().await.unwrap();
        assert_eq!(received_on_host, msg);

        // Host -> Client datagram
        let reply = bytes::Bytes::from_static(b"video ack host->client");
        host_conn.send_datagram(reply.clone()).unwrap();

        let received_on_client = client_conn.read_datagram().await.unwrap();
        assert_eq!(received_on_client, reply);

        // Metrics check
        let session = server.session_table().get_session(&session_id).unwrap();
        let snapshot = session.snapshot();
        assert_eq!(snapshot.client_to_host_packets, 1);
        assert_eq!(snapshot.host_to_client_packets, 1);
        assert_eq!(snapshot.client_to_host_bytes, msg.len() as u64);
        assert_eq!(snapshot.host_to_client_bytes, reply.len() as u64);

        server.shutdown();
        let _ = server_handle.await;
    }

    #[tokio::test]
    async fn test_peer_disconnect_cleans_up_session() {
        let (server, signing_key, verifier) = setup_test_server();
        let server_addr = server.local_addr().unwrap();
        let cert_der = server.server_cert_der().unwrap().clone();

        let server_handle = {
            let server = server.clone();
            tokio::spawn(async move {
                server.run().await.unwrap();
            })
        };

        let (client_token, host_token, session_id) =
            create_test_tokens(&signing_key, verifier.relay_id());

        let client_endpoint =
            make_client_endpoint("127.0.0.1:0".parse().unwrap(), &cert_der).unwrap();
        let client_conn = client_endpoint
            .connect(server_addr, "localhost")
            .unwrap()
            .await
            .unwrap();
        perform_client_handshake(&client_conn, &client_token)
            .await
            .unwrap();

        let host_endpoint =
            make_client_endpoint("127.0.0.1:0".parse().unwrap(), &cert_der).unwrap();
        let host_conn = host_endpoint
            .connect(server_addr, "localhost")
            .unwrap()
            .await
            .unwrap();
        perform_client_handshake(&host_conn, &host_token)
            .await
            .unwrap();

        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(server.session_table().active_session_count(), 1);

        // Close client connection
        client_conn.close(quinn::VarInt::from_u32(0), b"client left");

        // Wait for server to process disconnection
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Session table should be pruned / removed
        assert!(server.session_table().get_session(&session_id).is_none());
        assert_eq!(server.session_table().active_session_count(), 0);

        server.shutdown();
        let _ = server_handle.await;
    }
}
