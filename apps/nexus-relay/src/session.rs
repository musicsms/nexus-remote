use crate::token::{EndpointRole, RelayToken};
use nexus_common::{id::SessionId, time::UnixTimestamp};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum SessionPairingError {
    #[error("endpoint role already connected for session")]
    RoleAlreadyConnected,
    #[error("session has incompatible endpoint identities")]
    IdentityMismatch,
    #[error("session token expired")]
    Expired,
}

#[derive(Debug, Default)]
pub struct RelayMetrics {
    pub client_to_host_bytes: u64,
    pub host_to_client_bytes: u64,
    pub client_to_host_packets: u64,
    pub host_to_client_packets: u64,
}
#[derive(Debug)]
pub struct RelaySession {
    pub session_id: SessionId,
    client_device_id: String,
    target_device_id: String,
    state: Mutex<SessionState>,
    metrics: Mutex<RelayMetrics>,
}
#[derive(Debug, Default)]
struct SessionState {
    client: bool,
    host: bool,
    terminated: bool,
}
impl RelaySession {
    fn new(token: &RelayToken) -> Self {
        Self {
            session_id: token.session_id.clone(),
            client_device_id: token.client_device_id.to_string(),
            target_device_id: token.target_device_id.to_string(),
            state: Mutex::new(SessionState::default()),
            metrics: Mutex::new(RelayMetrics::default()),
        }
    }
    pub fn record_forward(&self, role: EndpointRole, bytes: u64) {
        let mut m = self.metrics.lock().unwrap();
        match role {
            EndpointRole::Client => {
                m.client_to_host_bytes += bytes;
                m.client_to_host_packets += 1
            }
            EndpointRole::Host => {
                m.host_to_client_bytes += bytes;
                m.host_to_client_packets += 1
            }
        }
    }
    pub fn snapshot(&self) -> RelayMetrics {
        let m = self.metrics.lock().unwrap();
        RelayMetrics {
            client_to_host_bytes: m.client_to_host_bytes,
            host_to_client_bytes: m.host_to_client_bytes,
            client_to_host_packets: m.client_to_host_packets,
            host_to_client_packets: m.host_to_client_packets,
        }
    }
    pub fn terminate(&self, _reason: &str) {
        self.state.lock().unwrap().terminated = true;
    }
}
pub struct RelaySessionTable {
    sessions: Mutex<HashMap<SessionId, Arc<RelaySession>>>,
}
impl Default for RelaySessionTable {
    fn default() -> Self {
        Self::new()
    }
}

impl RelaySessionTable {
    pub fn new() -> Self {
        Self {
            sessions: Mutex::new(HashMap::new()),
        }
    }
    pub fn len(&self) -> usize {
        self.sessions.lock().unwrap().len()
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    pub fn active_session_count(&self) -> usize {
        self.len()
    }
    pub fn get_session(&self, id: &SessionId) -> Option<Arc<RelaySession>> {
        self.sessions.lock().unwrap().get(id).cloned()
    }
    pub fn remove_session(&self, id: &SessionId) {
        self.sessions.lock().unwrap().remove(id);
    }
    pub fn register_or_join(
        &self,
        token: &RelayToken,
        now: UnixTimestamp,
    ) -> Result<(Arc<RelaySession>, bool), SessionPairingError> {
        if token.expires_at <= now {
            return Err(SessionPairingError::Expired);
        };
        let mut all = self.sessions.lock().unwrap();
        let session = all
            .entry(token.session_id.clone())
            .or_insert_with(|| Arc::new(RelaySession::new(token)))
            .clone();
        if session.client_device_id != token.client_device_id.to_string()
            || session.target_device_id != token.target_device_id.to_string()
        {
            return Err(SessionPairingError::IdentityMismatch);
        };
        let is_paired = {
            let mut state = session.state.lock().unwrap();
            let slot = match token.role {
                EndpointRole::Client => &mut state.client,
                EndpointRole::Host => &mut state.host,
            };
            if *slot {
                return Err(SessionPairingError::RoleAlreadyConnected);
            }
            *slot = true;
            state.client && state.host
        };
        Ok((session, is_paired))
    }
}
