//! QUIC data plane forwarding pipeline and handshake protocol.
//! Part of Nexus Remote Desktop Platform.

use std::sync::Arc;

use nexus_common::id::SessionId;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::session::{RelaySession, SessionPairingError};
use crate::token::{EndpointRole, RelayToken, RelayTokenError};

/// Errors arising during relay forwarding or QUIC connection handling.
#[derive(Debug, Error)]
pub enum RelayServerError {
    /// Low-level I/O error on sockets or streams.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// Quinn QUIC connection error.
    #[error("QUIC connection error: {0}")]
    Quic(#[from] quinn::ConnectionError),

    /// Quinn write stream error.
    #[error("QUIC write error: {0}")]
    WriteError(#[from] quinn::WriteError),

    /// Quinn read stream error.
    #[error("QUIC read error: {0}")]
    ReadError(#[from] quinn::ReadError),

    /// Quinn stream closed error.
    #[error("QUIC stream closed: {0}")]
    ClosedStream(#[from] quinn::ClosedStream),

    /// TLS configuration error.
    #[error("TLS configuration error: {0}")]
    Tls(String),

    /// Certificate generation error.
    #[error("certificate error: {0}")]
    Certificate(String),

    /// Relay token validation or signature error.
    #[error("token error: {0}")]
    Token(#[from] RelayTokenError),

    /// Relay session pairing error.
    #[error("session error: {0}")]
    Session(#[from] SessionPairingError),

    /// Protocol framing or deserialization error.
    #[error("protocol error: {0}")]
    Protocol(String),

    /// Handshake rejected by server with explicit error message.
    #[error("handshake rejected: {0}")]
    HandshakeRejected(String),

    /// Server was closed or shut down.
    #[error("server closed")]
    Closed,
}

/// Request payload sent by connecting endpoint during relay handshake.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelayHandshakeRequest {
    /// Signed relay authorization token.
    pub token: RelayToken,
}

/// Response payload returned by relay server after evaluating handshake.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelayHandshakeResponse {
    /// Whether the handshake and token verification succeeded.
    pub success: bool,
    /// Verified session identifier.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<SessionId>,
    /// Verified role of the connecting endpoint.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<EndpointRole>,
    /// Whether both endpoints are now paired and active.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_paired: Option<bool>,
    /// Error message if handshake failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Maximum permitted size of a serialized handshake message (64 KB).
const MAX_HANDSHAKE_MSG_SIZE: usize = 64 * 1024;

/// Reads and parses a [`RelayToken`] from an incoming handshake bi-stream.
/// Supports 4-byte big-endian length-prefixed JSON or raw JSON stream until EOF.
pub async fn read_handshake_token(
    recv: &mut quinn::RecvStream,
) -> Result<RelayToken, RelayServerError> {
    // Read up to 4 bytes to check for length prefix or JSON start character '{'
    let mut initial = [0u8; 4];
    let mut read_bytes = 0;
    while read_bytes < 4 {
        match recv.read(&mut initial[read_bytes..]).await? {
            Some(0) | None => break,
            Some(n) => read_bytes += n,
        }
    }

    if read_bytes == 0 {
        return Err(RelayServerError::Protocol(
            "empty handshake stream".to_string(),
        ));
    }

    let payload = if initial[0] == b'{' {
        // Direct JSON stream without length prefix
        let mut buf = Vec::with_capacity(512);
        buf.extend_from_slice(&initial[..read_bytes]);
        let mut chunk = [0u8; 1024];
        loop {
            match recv.read(&mut chunk).await? {
                Some(0) | None => break,
                Some(n) => {
                    if buf.len() + n > MAX_HANDSHAKE_MSG_SIZE {
                        return Err(RelayServerError::Protocol(
                            "handshake message exceeded maximum size".to_string(),
                        ));
                    }
                    buf.extend_from_slice(&chunk[..n]);
                }
            }
        }
        buf
    } else {
        // 4-byte BE length-prefixed
        if read_bytes < 4 {
            return Err(RelayServerError::Protocol(
                "incomplete length prefix in handshake stream".to_string(),
            ));
        }
        let length = u32::from_be_bytes(initial) as usize;
        if length == 0 || length > MAX_HANDSHAKE_MSG_SIZE {
            return Err(RelayServerError::Protocol(format!(
                "invalid handshake payload length: {length}"
            )));
        }

        let mut buf = vec![0u8; length];
        recv.read_exact(&mut buf).await.map_err(|e| {
            RelayServerError::Protocol(format!("failed to read handshake payload: {e}"))
        })?;
        buf
    };

    // Try parsing as RelayHandshakeRequest, fallback to RelayToken
    if let Ok(req) = serde_json::from_slice::<RelayHandshakeRequest>(&payload) {
        Ok(req.token)
    } else {
        serde_json::from_slice::<RelayToken>(&payload)
            .map_err(|e| RelayServerError::Protocol(format!("failed to parse RelayToken: {e}")))
    }
}

/// Writes a [`RelayHandshakeResponse`] to the handshake stream with 4-byte length prefix.
pub async fn write_handshake_response(
    send: &mut quinn::SendStream,
    response: &RelayHandshakeResponse,
) -> Result<(), RelayServerError> {
    let bytes = serde_json::to_vec(response)
        .map_err(|e| RelayServerError::Protocol(format!("failed to serialize response: {e}")))?;
    let len = (bytes.len() as u32).to_be_bytes();
    send.write_all(&len).await?;
    send.write_all(&bytes).await?;
    send.finish()?;
    Ok(())
}

/// Sends a handshake request from a client endpoint and receives the server response.
pub async fn perform_client_handshake(
    conn: &quinn::Connection,
    token: &RelayToken,
) -> Result<RelayHandshakeResponse, RelayServerError> {
    let (mut send, mut recv) = conn.open_bi().await?;

    let req = RelayHandshakeRequest {
        token: token.clone(),
    };
    let bytes = serde_json::to_vec(&req)
        .map_err(|e| RelayServerError::Protocol(format!("failed to serialize request: {e}")))?;
    let len = (bytes.len() as u32).to_be_bytes();
    send.write_all(&len).await?;
    send.write_all(&bytes).await?;
    send.finish()?;

    // Read response (4-byte length + payload)
    let mut len_buf = [0u8; 4];
    recv.read_exact(&mut len_buf).await.map_err(|e| {
        RelayServerError::Protocol(format!("failed to read handshake response length: {e}"))
    })?;
    let resp_len = u32::from_be_bytes(len_buf) as usize;
    if resp_len > MAX_HANDSHAKE_MSG_SIZE {
        return Err(RelayServerError::Protocol(
            "handshake response payload too large".to_string(),
        ));
    }

    let mut resp_buf = vec![0u8; resp_len];
    recv.read_exact(&mut resp_buf).await.map_err(|e| {
        RelayServerError::Protocol(format!("failed to read handshake response body: {e}"))
    })?;

    let resp: RelayHandshakeResponse = serde_json::from_slice(&resp_buf).map_err(|e| {
        RelayServerError::Protocol(format!("failed to parse handshake response: {e}"))
    })?;

    if !resp.success {
        return Err(RelayServerError::HandshakeRejected(
            resp.error
                .unwrap_or_else(|| "handshake rejected".to_string()),
        ));
    }

    Ok(resp)
}

/// Bridges bidirectional reliable streams between two paired endpoints.
/// Transparently copies data in both directions and updates traffic metrics.
pub async fn bridge_bistreams(
    mut client_side_send: quinn::SendStream,
    mut client_side_recv: quinn::RecvStream,
    mut peer_side_send: quinn::SendStream,
    mut peer_side_recv: quinn::RecvStream,
    session: Arc<RelaySession>,
    initiator_role: EndpointRole,
) {
    let peer_role = initiator_role.peer();

    // Copy client_side -> peer_side
    let fwd_to_peer = {
        let session = Arc::clone(&session);
        async move {
            let mut buf = vec![0u8; 64 * 1024];
            loop {
                match client_side_recv.read(&mut buf).await {
                    Ok(Some(n)) => {
                        tracing::info!(initiator_role = ?initiator_role, bytes = n, "forwarding client_side to peer_side");
                        if let Err(e) = peer_side_send.write_all(&buf[..n]).await {
                            tracing::warn!(error = %e, "peer_side_send write_all failed");
                            break;
                        }
                        session.record_forward(initiator_role, n as u64);
                    }
                    Ok(None) => {
                        tracing::info!("client_side_recv EOF, finishing peer_side_send");
                        let _ = peer_side_send.finish();
                        break;
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "client_side_recv read error, resetting peer_side_send");
                        let _ = peer_side_send.reset(quinn::VarInt::from_u32(0));
                        break;
                    }
                }
            }
        }
    };

    // Copy peer_side -> client_side
    let fwd_to_client = {
        let session = Arc::clone(&session);
        async move {
            let mut buf = vec![0u8; 64 * 1024];
            loop {
                match peer_side_recv.read(&mut buf).await {
                    Ok(Some(n)) => {
                        tracing::info!(peer_role = ?peer_role, bytes = n, "forwarding peer_side to client_side");
                        if let Err(e) = client_side_send.write_all(&buf[..n]).await {
                            tracing::warn!(error = %e, "client_side_send write_all failed");
                            break;
                        }
                        session.record_forward(peer_role, n as u64);
                    }
                    Ok(None) => {
                        tracing::info!("peer_side_recv EOF, finishing client_side_send");
                        let _ = client_side_send.finish();
                        break;
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "peer_side_recv read error, resetting client_side_send");
                        let _ = client_side_send.reset(quinn::VarInt::from_u32(0));
                        break;
                    }
                }
            }
        }
    };

    tokio::spawn(async move {
        let _ = tokio::join!(fwd_to_peer, fwd_to_client);
    });
}

/// Forwards QUIC streams opened by either Client or Host to the paired peer.
pub async fn forward_streams(
    client_conn: quinn::Connection,
    host_conn: quinn::Connection,
    session: Arc<RelaySession>,
) {
    let client_accept_loop = {
        let client_conn = client_conn.clone();
        let host_conn = host_conn.clone();
        let session = Arc::clone(&session);
        async move {
            loop {
                match client_conn.accept_bi().await {
                    Ok((recv_send, recv_recv)) => match host_conn.open_bi().await {
                        Ok((opened_send, opened_recv)) => {
                            tracing::info!("bridging bistreams from client to host");
                            tokio::spawn(bridge_bistreams(
                                recv_send,
                                recv_recv,
                                opened_send,
                                opened_recv,
                                Arc::clone(&session),
                                EndpointRole::Client,
                            ));
                        }
                        Err(e) => {
                            tracing::warn!(error = %e, "failed to open bi stream on host");
                            break;
                        }
                    },
                    Err(e) => {
                        tracing::warn!(error = %e, "client accept_bi loop terminated");
                        break;
                    }
                }
            }
        }
    };

    let host_accept_loop = {
        let client_conn = client_conn.clone();
        let host_conn = host_conn.clone();
        let session = Arc::clone(&session);
        async move {
            while let Ok((recv_send, recv_recv)) = host_conn.accept_bi().await {
                match client_conn.open_bi().await {
                    Ok((opened_send, opened_recv)) => {
                        tokio::spawn(bridge_bistreams(
                            recv_send,
                            recv_recv,
                            opened_send,
                            opened_recv,
                            Arc::clone(&session),
                            EndpointRole::Host,
                        ));
                    }
                    Err(e) => {
                        tracing::debug!(error = %e, "failed to open bi stream on client");
                        break;
                    }
                }
            }
        }
    };

    let _ = tokio::join!(client_accept_loop, host_accept_loop);
}

/// Forwards QUIC datagrams between Client and Host concurrently while updating traffic metrics.
pub async fn forward_datagrams(
    client_conn: quinn::Connection,
    host_conn: quinn::Connection,
    session: Arc<RelaySession>,
) {
    let client_to_host = {
        let client_conn = client_conn.clone();
        let host_conn = host_conn.clone();
        let session = Arc::clone(&session);
        async move {
            while let Ok(dgram) = client_conn.read_datagram().await {
                session.record_forward(EndpointRole::Client, dgram.len() as u64);
                if let Err(e) = host_conn.send_datagram(dgram) {
                    if matches!(e, quinn::SendDatagramError::ConnectionLost(_)) {
                        break;
                    }
                }
            }
        }
    };

    let host_to_client = {
        let client_conn = client_conn.clone();
        let host_conn = host_conn.clone();
        let session = Arc::clone(&session);
        async move {
            while let Ok(dgram) = host_conn.read_datagram().await {
                session.record_forward(EndpointRole::Host, dgram.len() as u64);
                if let Err(e) = client_conn.send_datagram(dgram) {
                    if matches!(e, quinn::SendDatagramError::ConnectionLost(_)) {
                        break;
                    }
                }
            }
        }
    };

    let _ = tokio::join!(client_to_host, host_to_client);
}

/// Orchestrates full forwarding pipeline (streams + datagrams) for a paired session.
pub async fn forward_session(
    client_conn: quinn::Connection,
    host_conn: quinn::Connection,
    session: Arc<RelaySession>,
) {
    let streams_handle = tokio::spawn(forward_streams(
        client_conn.clone(),
        host_conn.clone(),
        Arc::clone(&session),
    ));
    let datagrams_handle = tokio::spawn(forward_datagrams(
        client_conn.clone(),
        host_conn.clone(),
        Arc::clone(&session),
    ));

    tokio::select! {
        _ = client_conn.closed() => {},
        _ = host_conn.closed() => {},
    }

    streams_handle.abort();
    datagrams_handle.abort();

    session.terminate("session closed");
    client_conn.close(quinn::VarInt::from_u32(0), b"session closed");
    host_conn.close(quinn::VarInt::from_u32(0), b"session closed");
}
