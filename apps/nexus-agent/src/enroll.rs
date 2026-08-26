//! Remote enrollment client connecting to nexusd control plane.
//! Part of Nexus Remote Desktop Platform.

use nexus_auth::credential::{DeviceCredential, DeviceRegistrationRequest};
use nexus_auth::enrollment::EnrollmentToken;
use nexus_common::time::{Clock, SystemClock};
use thiserror::Error;

use crate::identity::AgentIdentity;

/// Errors arising during remote enrollment.
#[derive(Debug, Error)]
pub enum EnrollmentClientError {
    #[error("Identity error: {0}")]
    Identity(#[from] crate::identity::IdentityError),

    #[error("HTTP error communicating with control plane: {0}")]
    Http(String),

    #[error("Control plane rejected enrollment: status {0}, message: {1}")]
    Rejected(u16, String),

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
}

/// Helper client to execute device enrollment against `nexusd`.
pub struct EnrollmentClient {
    control_plane_url: String,
}

impl EnrollmentClient {
    /// Creates a new `EnrollmentClient` with control plane base URL (e.g. `http://127.0.0.1:8080`).
    pub fn new(control_plane_url: impl Into<String>) -> Self {
        Self {
            control_plane_url: control_plane_url.into(),
        }
    }

    /// Enrolls this agent with the control plane using the given pre-signed enrollment token.
    pub async fn enroll(
        &self,
        identity: &mut AgentIdentity,
        enrollment_token: EnrollmentToken,
        hostname: impl Into<String>,
        os: impl Into<String>,
        architecture: impl Into<String>,
        agent_version: impl Into<String>,
    ) -> Result<DeviceCredential, EnrollmentClientError> {
        let now = SystemClock.now();
        let pub_key = identity.verifying_key().to_bytes().to_vec();

        let mut req = DeviceRegistrationRequest {
            enrollment_token,
            device_public_key: pub_key,
            os: os.into(),
            architecture: architecture.into(),
            hostname: hostname.into(),
            agent_version: agent_version.into(),
            requested_at: now,
            proof_signature: Vec::new(),
        };

        req.sign_proof(identity.signing_key());

        let url = format!("{}/api/v1/devices/enroll", self.control_plane_url);
        let client = reqwest::Client::new();
        let resp = client
            .post(&url)
            .json(&req)
            .send()
            .await
            .map_err(|e| EnrollmentClientError::Http(e.to_string()))?;

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            return Err(EnrollmentClientError::Rejected(status, body));
        }

        let credential: DeviceCredential = resp
            .json()
            .await
            .map_err(|e| EnrollmentClientError::Http(e.to_string()))?;

        identity.set_credential(credential.clone())?;
        Ok(credential)
    }
}
