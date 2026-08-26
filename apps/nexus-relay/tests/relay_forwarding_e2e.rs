//! End-to-end integration tests for nexus-relay service.
//! Part of Nexus Remote Desktop Platform.

use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use ed25519_dalek::SigningKey;
use nexus_common::id::{DeviceId, SessionId};
use nexus_common::time::{Clock, SystemClock, UnixTimestamp};
use nexus_relay::forwarder::perform_client_handshake;
use nexus_relay::server::RelayServer;
use nexus_relay::token::{EndpointRole, RelayToken, RelayTokenVerifier};

fn make_client_endpoint(
    bind_addr: std::net::SocketAddr,
    server_cert_der: &[u8],
) -> Result<quinn::Endpoint, Box<dyn std::error::Error>> {
    let mut roots = rustls::RootCertStore::empty();
    roots.add(rustls::pki_types::CertificateDer::from(
        server_cert_der.to_vec(),
    ))?;

    let client_crypto = rustls::ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();

    let client_config = quinn::ClientConfig::new(Arc::new(
        quinn::crypto::rustls::QuicClientConfig::try_from(client_crypto)?,
    ));

    let mut endpoint = quinn::Endpoint::client(bind_addr)?;
    endpoint.set_default_client_config(client_config);
    Ok(endpoint)
}

#[tokio::test]
async fn test_relay_e2e_full_session_lifecycle() {
    // 1. Setup server and cryptographic verifier
    let signing_key = SigningKey::from_bytes(&[99u8; 32]);
    let verifying_key = signing_key.verifying_key();
    let verifier = RelayTokenVerifier::new(verifying_key, "relay-e2e-cluster-1");

    let server = RelayServer::bind("127.0.0.1:0".parse().unwrap(), verifier.clone()).unwrap();
    let server_addr = server.local_addr().unwrap();
    let cert_der = server.server_cert_der().unwrap().clone();

    let server_handle = {
        let server = server.clone();
        tokio::spawn(async move {
            server.run().await.unwrap();
        })
    };

    // 2. Generate signed tokens for Client and Host
    let session_id = SessionId::new("sess-e2e-relay-99").unwrap();
    let client_device = DeviceId::new("dev-client-desktop").unwrap();
    let host_device = DeviceId::new("dev-host-workstation").unwrap();
    let expires_at = UnixTimestamp::from_secs(SystemClock.now().as_secs() + 3600);

    let mut client_token = RelayToken::builder()
        .session_id(session_id.clone())
        .relay_id(verifier.relay_id())
        .client_device_id(client_device.clone())
        .target_device_id(host_device.clone())
        .role(EndpointRole::Client)
        .expires_at(expires_at)
        .build()
        .unwrap();
    client_token.sign(&signing_key);

    let mut host_token = RelayToken::builder()
        .session_id(session_id.clone())
        .relay_id(verifier.relay_id())
        .client_device_id(client_device)
        .target_device_id(host_device)
        .role(EndpointRole::Host)
        .expires_at(expires_at)
        .build()
        .unwrap();
    host_token.sign(&signing_key);

    // 3. Connect client & perform handshake
    let client_endpoint = make_client_endpoint("127.0.0.1:0".parse().unwrap(), &cert_der).unwrap();
    let client_conn = client_endpoint
        .connect(server_addr, "localhost")
        .unwrap()
        .await
        .unwrap();
    let client_hs = perform_client_handshake(&client_conn, &client_token)
        .await
        .unwrap();
    assert!(client_hs.success);
    assert_eq!(client_hs.is_paired, Some(false));

    // 4. Connect host & perform handshake (completes pairing)
    let host_endpoint = make_client_endpoint("127.0.0.1:0".parse().unwrap(), &cert_der).unwrap();
    let host_conn = host_endpoint
        .connect(server_addr, "localhost")
        .unwrap()
        .await
        .unwrap();
    let host_hs = perform_client_handshake(&host_conn, &host_token)
        .await
        .unwrap();
    assert!(host_hs.success);
    assert_eq!(host_hs.is_paired, Some(true));

    // 5. Test bi-stream multiplexing across relay
    let host_conn_task = host_conn.clone();
    let host_stream_task = tokio::spawn(async move {
        let (mut host_send, mut host_recv) = host_conn_task.accept_bi().await.unwrap();
        let mut msg_buf = [0u8; 11];
        host_recv.read_exact(&mut msg_buf).await.unwrap();
        assert_eq!(&msg_buf, b"hello relay");
        host_send.write_all(b"ack from host").await.unwrap();
        let _ = host_send.finish();
    });

    tokio::time::sleep(Duration::from_millis(100)).await;

    let (mut client_send, mut client_recv) = client_conn.open_bi().await.unwrap();
    client_send.write_all(b"hello relay").await.unwrap();
    client_send.finish().unwrap();

    let mut response_buf = [0u8; 13];
    client_recv.read_exact(&mut response_buf).await.unwrap();
    assert_eq!(&response_buf, b"ack from host");

    host_stream_task.await.unwrap();

    // 6. Test datagram forwarding across relay
    let client_dgram = Bytes::from_static(b"video-frame-payload-12345");
    client_conn.send_datagram(client_dgram.clone()).unwrap();
    let received_dgram = host_conn.read_datagram().await.unwrap();
    assert_eq!(received_dgram, client_dgram);

    let host_dgram = Bytes::from_static(b"fec-recovery-packet-67890");
    host_conn.send_datagram(host_dgram.clone()).unwrap();
    let client_received_dgram = client_conn.read_datagram().await.unwrap();
    assert_eq!(client_received_dgram, host_dgram);

    // 7. Verify session metrics
    let session = server.session_table().get_session(&session_id).unwrap();
    let metrics = session.snapshot();
    assert!(metrics.client_to_host_bytes >= 11 + client_dgram.len() as u64);
    assert!(metrics.host_to_client_bytes >= 13 + host_dgram.len() as u64);
    assert_eq!(metrics.client_to_host_packets, 2);
    assert_eq!(metrics.host_to_client_packets, 2);

    // 8. Graceful shutdown
    let _ = (&client_conn, &host_conn, &client_endpoint, &host_endpoint);
    server.shutdown();
    let _ = server_handle.await;
}
