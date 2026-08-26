//! HTTP API routes and request handlers for nexusd control plane.
//! Part of Nexus Remote Desktop Platform.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::Json;
use ed25519_dalek::Signer;
use nexus_audit::event::{AuditEvent, AuditEventType};
use nexus_auth::credential::{DeviceCredential, DeviceRegistrationRequest};
use nexus_common::id::{DeviceId, SessionId, TenantId, UserId};
use nexus_common::time::{Clock, SystemClock, UnixTimestamp};
use nexus_policy::{Action, EvaluationDecision, ResourceContext, SubjectContext};
use nexus_protocol::SessionCapability;
use nexus_relay::token::{EndpointRole, RelayToken};
use prost::Message;
use serde::{Deserialize, Serialize};

use crate::state::{AppState, RegisteredDevice};

/// Response returned by the health check endpoint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HealthResponse {
    pub status: String,
    pub version: String,
    pub control_plane_id: String,
}

/// JSON payload returned when an API error occurs.
#[derive(Serialize)]
pub struct ErrorResponse {
    pub error: String,
}

impl IntoResponse for ErrorResponse {
    fn into_response(self) -> Response {
        (StatusCode::BAD_REQUEST, Json(self)).into_response()
    }
}

/// Request body for initiating a remote desktop session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionRequestPayload {
    pub organization_id: TenantId,
    pub subject_user_id: UserId,
    pub client_device_id: DeviceId,
    pub target_device_id: DeviceId,
    pub requested_actions: Vec<String>,
    pub client_ephemeral_public_key: Vec<u8>,
}

/// Response body returned when a session request is authorized.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionAuthorizationResponse {
    pub session_id: SessionId,
    pub capability_bytes: Vec<u8>,
    pub relay_id: String,
    pub client_relay_token: RelayToken,
    pub host_relay_token: RelayToken,
    pub expires_at: UnixTimestamp,
}

/// Query parameters for listing devices.
#[derive(Debug, Deserialize)]
pub struct ListDevicesQuery {
    pub organization_id: Option<String>,
}

/// Builds the Axum router for the `nexusd` control plane.
pub fn create_router(state: AppState) -> axum::Router {
    axum::Router::new()
        .route("/healthz", get(healthz_handler))
        .route("/api/v1/devices/enroll", post(enroll_device_handler))
        .route("/api/v1/devices", get(list_devices_handler))
        .route("/api/v1/sessions/request", post(request_session_handler))
        .with_state(state)
}

/// `GET /healthz` - Health check and system metadata.
pub async fn healthz_handler(State(state): State<AppState>) -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok".into(),
        version: env!("CARGO_PKG_VERSION").into(),
        control_plane_id: state.control_plane_id.clone(),
    })
}

/// `POST /api/v1/devices/enroll` - Enrolls a new host or client device.
pub async fn enroll_device_handler(
    State(state): State<AppState>,
    Json(req): Json<DeviceRegistrationRequest>,
) -> Result<Json<DeviceCredential>, (StatusCode, Json<ErrorResponse>)> {
    let now = SystemClock.now();

    // 1. Verify that the enrollment token was issued by control plane & is valid
    let cp_verifying_key = state.signing_key.verifying_key();
    if let Err(e) = req.enrollment_token.verify(&cp_verifying_key, now) {
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(ErrorResponse {
                error: format!("enrollment token invalid: {e}"),
            }),
        ));
    }

    // 2. Consume one use from the enrollment token store
    if let Err(e) = state.consume_enrollment_token(&req.enrollment_token.token_id, now) {
        return Err((
            StatusCode::FORBIDDEN,
            Json(ErrorResponse {
                error: format!("enrollment token cannot be consumed: {e}"),
            }),
        ));
    }

    // 3. Verify proof of possession against the device's public key
    if let Err(e) = req.verify_proof() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!("invalid proof-of-possession: {e}"),
            }),
        ));
    }

    // 4. Generate unique DeviceId and build signed DeviceCredential
    let device_id_str = format!(
        "dev-{}-{}",
        req.enrollment_token.device_type.as_str(),
        &req.enrollment_token.token_id[..req.enrollment_token.token_id.len().min(8)]
    );
    let device_id = DeviceId::new(device_id_str).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("failed to generate device id: {e}"),
            }),
        )
    })?;

    let mut credential = DeviceCredential::builder()
        .device_id(device_id.clone())
        .organization_id(req.enrollment_token.organization_id.clone())
        .public_key(req.device_public_key.clone())
        .device_type(req.enrollment_token.device_type)
        .os(req.os.clone())
        .architecture(req.architecture.clone())
        .capabilities(vec!["desktop.view".into(), "desktop.control".into()])
        .issued_at(now)
        .expires_at(UnixTimestamp::from_secs(now.as_secs() + 365 * 24 * 3600))
        .build()
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("failed to build credential: {e}"),
                }),
            )
        })?;

    credential.sign(&state.signing_key).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("failed to sign credential: {e}"),
            }),
        )
    })?;

    // 5. Save registered device in state
    state.register_device(
        credential.clone(),
        req.hostname.clone(),
        req.agent_version.clone(),
        now,
    );

    // 6. Record audit event
    if let Ok(audit_evt) = AuditEvent::builder()
        .event_id(format!("evt-enroll-{}", device_id))
        .timestamp(now)
        .organization_id(req.enrollment_token.organization_id)
        .device_id(device_id)
        .event_type(AuditEventType::DeviceEnroll)
        .metadata(serde_json::json!({
            "hostname": req.hostname,
            "os": req.os,
            "arch": req.architecture,
            "token_id": req.enrollment_token.token_id,
        }))
        .build()
    {
        state.record_audit(audit_evt).await;
    }

    Ok(Json(credential))
}

/// `GET /api/v1/devices` - Lists devices registered in the control plane.
pub async fn list_devices_handler(
    State(state): State<AppState>,
    axum::extract::Query(query): axum::extract::Query<ListDevicesQuery>,
) -> Json<Vec<RegisteredDevice>> {
    let org_id_str = query
        .organization_id
        .unwrap_or_else(|| "default".to_string());
    let org_id =
        TenantId::new(org_id_str).unwrap_or_else(|_| TenantId::new("org-default").unwrap());
    let list = state.list_devices(&org_id);
    Json(list)
}

/// `POST /api/v1/sessions/request` - Requests authorization for a remote desktop session.
pub async fn request_session_handler(
    State(state): State<AppState>,
    Json(payload): Json<SessionRequestPayload>,
) -> Result<Json<SessionAuthorizationResponse>, (StatusCode, Json<ErrorResponse>)> {
    let now = SystemClock.now();

    // 1. Verify target device exists and is active
    let target_device = state.get_device(&payload.target_device_id).ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: format!("target device {} not found", payload.target_device_id),
            }),
        )
    })?;

    if !target_device.is_active {
        return Err((
            StatusCode::FORBIDDEN,
            Json(ErrorResponse {
                error: "target device is not active".into(),
            }),
        ));
    }

    // 2. Evaluate access policy through PolicyEngine
    let subject = SubjectContext {
        user_id: payload.subject_user_id.clone(),
        roles: vec!["admin".into(), "operator".into()],
        mfa_authenticated: true,
        client_device_managed: true,
        client_ip: None,
    };

    let resource = ResourceContext {
        target_device_id: payload.target_device_id.clone(),
        device_labels: Default::default(),
        active_sessions_on_device: vec![],
    };

    let primary_action = Action::DesktopView;
    let decision = state
        .policy_engine
        .evaluate(&subject, &resource, primary_action);

    match decision {
        EvaluationDecision::Allowed {
            granted_actions,
            restrictions: _,
            matched_role: _,
        } => {
            let session_id_str = format!("sess-{}", uuid_simple());
            let session_id = SessionId::new(session_id_str).map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: format!("failed to generate session id: {e}"),
                    }),
                )
            })?;

            let ttl_secs = 120u64;
            let expires_at = UnixTimestamp::from_secs(now.as_secs() + ttl_secs);

            // 3. Build & sign SessionCapability (ADR-014, ADR-016)
            let mut capability = SessionCapability {
                version: 1,
                issuer: state.control_plane_id.clone(),
                session_id: session_id.to_string(),
                subject_user_id: payload.subject_user_id.to_string(),
                client_device_id: payload.client_device_id.to_string(),
                target_device_id: payload.target_device_id.to_string(),
                permissions: granted_actions
                    .into_iter()
                    .map(|a| a.as_str().to_string())
                    .collect(),
                restrictions: vec![],
                not_before: now.as_secs(),
                expires_at: expires_at.as_secs(),
                nonce: rand_bytes_16(),
                agent_min_protocol: 1,
                agent_max_protocol: 1,
                client_ephemeral_public_key: payload.client_ephemeral_public_key,
                signature: Vec::new(),
            };

            let mut raw_bytes = Vec::new();
            capability.encode(&mut raw_bytes).map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: format!("failed to encode capability: {e}"),
                    }),
                )
            })?;
            let sig = state.signing_key.sign(&raw_bytes);
            capability.signature = sig.to_bytes().to_vec();

            let mut signed_cap_bytes = Vec::with_capacity(capability.encoded_len());
            capability.encode(&mut signed_cap_bytes).map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: format!("failed to encode signed capability: {e}"),
                    }),
                )
            })?;

            // 4. Generate RelayTokens for Client and Host
            let mut client_relay_token = RelayToken::builder()
                .session_id(session_id.clone())
                .relay_id(&state.default_relay_id)
                .client_device_id(payload.client_device_id.clone())
                .target_device_id(payload.target_device_id.clone())
                .role(EndpointRole::Client)
                .expires_at(expires_at)
                .build()
                .map_err(|e| {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(ErrorResponse {
                            error: format!("failed to build client relay token: {e}"),
                        }),
                    )
                })?;
            client_relay_token.sign(&state.signing_key);

            let mut host_relay_token = RelayToken::builder()
                .session_id(session_id.clone())
                .relay_id(&state.default_relay_id)
                .client_device_id(payload.client_device_id.clone())
                .target_device_id(payload.target_device_id.clone())
                .role(EndpointRole::Host)
                .expires_at(expires_at)
                .build()
                .map_err(|e| {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(ErrorResponse {
                            error: format!("failed to build host relay token: {e}"),
                        }),
                    )
                })?;
            host_relay_token.sign(&state.signing_key);

            // 5. Audit session authorization
            if let Ok(audit_evt) = AuditEvent::builder()
                .event_id(format!("evt-auth-{}", session_id))
                .timestamp(now)
                .organization_id(payload.organization_id)
                .user_id(payload.subject_user_id)
                .device_id(payload.target_device_id)
                .session_id(session_id.clone())
                .event_type(AuditEventType::SessionAuthorize)
                .metadata(serde_json::json!({
                    "client_device_id": payload.client_device_id.as_str(),
                    "relay_id": state.default_relay_id,
                }))
                .build()
            {
                state.record_audit(audit_evt).await;
            }

            Ok(Json(SessionAuthorizationResponse {
                session_id,
                capability_bytes: signed_cap_bytes,
                relay_id: state.default_relay_id.clone(),
                client_relay_token,
                host_relay_token,
                expires_at,
            }))
        }
        EvaluationDecision::Denied { reason } => {
            if let Ok(audit_evt) = AuditEvent::builder()
                .event_id(format!("evt-deny-{}", now.as_secs()))
                .timestamp(now)
                .organization_id(payload.organization_id)
                .user_id(payload.subject_user_id)
                .device_id(payload.target_device_id)
                .event_type(AuditEventType::SessionDeny)
                .metadata(serde_json::json!({
                    "reason": reason.to_string(),
                }))
                .build()
            {
                state.record_audit(audit_evt).await;
            }

            Err((
                StatusCode::FORBIDDEN,
                Json(ErrorResponse {
                    error: format!("session access denied by policy: {reason}"),
                }),
            ))
        }
    }
}

fn rand_bytes_16() -> Vec<u8> {
    use std::time::SystemTime;
    let t = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let mut b = Vec::with_capacity(16);
    b.extend_from_slice(&t.to_be_bytes());
    b
}

fn uuid_simple() -> String {
    use std::time::SystemTime;
    let t = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    format!("{:032x}", t)
}
