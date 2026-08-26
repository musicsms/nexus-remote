//! Audit event types, data model, and canonical serialization.
//!
//! Provides the core audit event definitions for the Nexus Remote Desktop Platform
//! as specified in Spec Section 35 (Audit Model).

use std::fmt;
use std::str::FromStr;

use nexus_common::id::{DeviceId, SessionId, TenantId, UserId};
use nexus_common::time::UnixTimestamp;
use serde::{Deserialize, Serialize};

/// Strongly typed audit event categories per Spec Section 35.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum AuditEventType {
    #[serde(rename = "user.login")]
    UserLogin,
    #[serde(rename = "user.mfa")]
    UserMfa,
    #[serde(rename = "device.enroll")]
    DeviceEnroll,
    #[serde(rename = "device.revoke")]
    DeviceRevoke,
    #[serde(rename = "session.request")]
    SessionRequest,
    #[serde(rename = "session.authorize")]
    SessionAuthorize,
    #[serde(rename = "session.deny")]
    SessionDeny,
    #[serde(rename = "session.start")]
    SessionStart,
    #[serde(rename = "session.disconnect")]
    SessionDisconnect,
    #[serde(rename = "session.end")]
    SessionEnd,
    #[serde(rename = "clipboard.read")]
    ClipboardRead,
    #[serde(rename = "clipboard.write")]
    ClipboardWrite,
    #[serde(rename = "file.upload")]
    FileUpload,
    #[serde(rename = "file.download")]
    FileDownload,
    #[serde(rename = "access.request")]
    AccessRequest,
    #[serde(rename = "access.approve")]
    AccessApprove,
    #[serde(rename = "policy.update")]
    PolicyUpdate,
}

impl AuditEventType {
    /// Array containing all 17 standard [`AuditEventType`] variants.
    pub const ALL: [AuditEventType; 17] = [
        Self::UserLogin,
        Self::UserMfa,
        Self::DeviceEnroll,
        Self::DeviceRevoke,
        Self::SessionRequest,
        Self::SessionAuthorize,
        Self::SessionDeny,
        Self::SessionStart,
        Self::SessionDisconnect,
        Self::SessionEnd,
        Self::ClipboardRead,
        Self::ClipboardWrite,
        Self::FileUpload,
        Self::FileDownload,
        Self::AccessRequest,
        Self::AccessApprove,
        Self::PolicyUpdate,
    ];

    /// Returns the canonical string representation of the audit event type.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::UserLogin => "user.login",
            Self::UserMfa => "user.mfa",
            Self::DeviceEnroll => "device.enroll",
            Self::DeviceRevoke => "device.revoke",
            Self::SessionRequest => "session.request",
            Self::SessionAuthorize => "session.authorize",
            Self::SessionDeny => "session.deny",
            Self::SessionStart => "session.start",
            Self::SessionDisconnect => "session.disconnect",
            Self::SessionEnd => "session.end",
            Self::ClipboardRead => "clipboard.read",
            Self::ClipboardWrite => "clipboard.write",
            Self::FileUpload => "file.upload",
            Self::FileDownload => "file.download",
            Self::AccessRequest => "access.request",
            Self::AccessApprove => "access.approve",
            Self::PolicyUpdate => "policy.update",
        }
    }
}

impl fmt::Display for AuditEventType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Error returned when parsing an audit event type fails.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("unknown audit event type: {0}")]
pub struct EventParseError(pub String);

impl FromStr for AuditEventType {
    type Err = EventParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "user.login" => Ok(Self::UserLogin),
            "user.mfa" => Ok(Self::UserMfa),
            "device.enroll" => Ok(Self::DeviceEnroll),
            "device.revoke" => Ok(Self::DeviceRevoke),
            "session.request" => Ok(Self::SessionRequest),
            "session.authorize" => Ok(Self::SessionAuthorize),
            "session.deny" => Ok(Self::SessionDeny),
            "session.start" => Ok(Self::SessionStart),
            "session.disconnect" => Ok(Self::SessionDisconnect),
            "session.end" => Ok(Self::SessionEnd),
            "clipboard.read" => Ok(Self::ClipboardRead),
            "clipboard.write" => Ok(Self::ClipboardWrite),
            "file.upload" => Ok(Self::FileUpload),
            "file.download" => Ok(Self::FileDownload),
            "access.request" => Ok(Self::AccessRequest),
            "access.approve" => Ok(Self::AccessApprove),
            "policy.update" => Ok(Self::PolicyUpdate),
            _ => Err(EventParseError(s.to_string())),
        }
    }
}

/// Error returned when building an [`AuditEvent`] fails due to missing required fields.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AuditEventBuildError {
    /// A required field was not provided to the builder.
    #[error("missing required field: {0}")]
    MissingField(&'static str),
}

/// A structured audit event representing a security- or operations-relevant action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditEvent {
    /// Unique identifier for the audit event.
    pub event_id: String,
    /// Unix timestamp when the event occurred.
    pub timestamp: UnixTimestamp,
    /// Tenant or organization identifier associated with the event.
    pub organization_id: TenantId,
    /// Optional user who initiated or was targeted by the event.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_id: Option<UserId>,
    /// Optional device associated with the event.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_id: Option<DeviceId>,
    /// Optional session associated with the event.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<SessionId>,
    /// Categorical audit event type.
    pub event_type: AuditEventType,
    /// Arbitrary structured metadata associated with the event.
    #[serde(default)]
    pub metadata: serde_json::Value,
}

impl AuditEvent {
    /// Creates a new `AuditEvent` with required fields and defaults for optional fields.
    pub fn new(
        event_id: impl Into<String>,
        timestamp: UnixTimestamp,
        organization_id: TenantId,
        event_type: AuditEventType,
    ) -> Self {
        Self {
            event_id: event_id.into(),
            timestamp,
            organization_id,
            user_id: None,
            device_id: None,
            session_id: None,
            event_type,
            metadata: serde_json::Value::Null,
        }
    }

    /// Returns a new `AuditEventBuilder` for constructing an audit event.
    #[must_use]
    pub fn builder() -> AuditEventBuilder {
        AuditEventBuilder::new()
    }

    /// Sets the user ID.
    #[must_use]
    pub fn with_user_id(mut self, user_id: impl Into<Option<UserId>>) -> Self {
        self.user_id = user_id.into();
        self
    }

    /// Sets the device ID.
    #[must_use]
    pub fn with_device_id(mut self, device_id: impl Into<Option<DeviceId>>) -> Self {
        self.device_id = device_id.into();
        self
    }

    /// Sets the session ID.
    #[must_use]
    pub fn with_session_id(mut self, session_id: impl Into<Option<SessionId>>) -> Self {
        self.session_id = session_id.into();
        self
    }

    /// Sets the structured metadata.
    #[must_use]
    pub fn with_metadata(mut self, metadata: serde_json::Value) -> Self {
        self.metadata = metadata;
        self
    }

    /// Returns the event ID.
    #[must_use]
    pub fn event_id(&self) -> &str {
        &self.event_id
    }

    /// Returns the timestamp.
    #[must_use]
    pub fn timestamp(&self) -> UnixTimestamp {
        self.timestamp
    }

    /// Returns the organization/tenant ID.
    #[must_use]
    pub fn organization_id(&self) -> &TenantId {
        &self.organization_id
    }

    /// Returns the user ID if present.
    #[must_use]
    pub fn user_id(&self) -> Option<&UserId> {
        self.user_id.as_ref()
    }

    /// Returns the device ID if present.
    #[must_use]
    pub fn device_id(&self) -> Option<&DeviceId> {
        self.device_id.as_ref()
    }

    /// Returns the session ID if present.
    #[must_use]
    pub fn session_id(&self) -> Option<&SessionId> {
        self.session_id.as_ref()
    }

    /// Returns the event type.
    #[must_use]
    pub fn event_type(&self) -> AuditEventType {
        self.event_type
    }

    /// Returns the metadata.
    #[must_use]
    pub fn metadata(&self) -> &serde_json::Value {
        &self.metadata
    }

    /// Serializes the audit event into canonical JSON bytes for deterministic hashing.
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, serde_json::Error> {
        serde_json::to_vec(self)
    }

    /// Serializes the audit event into canonical JSON string for deterministic hashing.
    pub fn canonical_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }
}

/// Builder for constructing an [`AuditEvent`].
#[derive(Debug, Clone, Default)]
pub struct AuditEventBuilder {
    event_id: Option<String>,
    timestamp: Option<UnixTimestamp>,
    organization_id: Option<TenantId>,
    user_id: Option<UserId>,
    device_id: Option<DeviceId>,
    session_id: Option<SessionId>,
    event_type: Option<AuditEventType>,
    metadata: Option<serde_json::Value>,
}

impl AuditEventBuilder {
    /// Creates a new, empty builder.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the event ID.
    #[must_use]
    pub fn event_id(mut self, event_id: impl Into<String>) -> Self {
        self.event_id = Some(event_id.into());
        self
    }

    /// Sets the timestamp.
    #[must_use]
    pub fn timestamp(mut self, timestamp: UnixTimestamp) -> Self {
        self.timestamp = Some(timestamp);
        self
    }

    /// Sets the organization/tenant ID.
    #[must_use]
    pub fn organization_id(mut self, organization_id: TenantId) -> Self {
        self.organization_id = Some(organization_id);
        self
    }

    /// Sets the user ID.
    #[must_use]
    pub fn user_id(mut self, user_id: impl Into<Option<UserId>>) -> Self {
        self.user_id = user_id.into();
        self
    }

    /// Sets the device ID.
    #[must_use]
    pub fn device_id(mut self, device_id: impl Into<Option<DeviceId>>) -> Self {
        self.device_id = device_id.into();
        self
    }

    /// Sets the session ID.
    #[must_use]
    pub fn session_id(mut self, session_id: impl Into<Option<SessionId>>) -> Self {
        self.session_id = session_id.into();
        self
    }

    /// Sets the event type.
    #[must_use]
    pub fn event_type(mut self, event_type: AuditEventType) -> Self {
        self.event_type = Some(event_type);
        self
    }

    /// Sets the metadata.
    #[must_use]
    pub fn metadata(mut self, metadata: serde_json::Value) -> Self {
        self.metadata = Some(metadata);
        self
    }

    /// Builds the [`AuditEvent`], returning an error if any required field is missing.
    pub fn build(self) -> Result<AuditEvent, AuditEventBuildError> {
        let event_id = self
            .event_id
            .ok_or(AuditEventBuildError::MissingField("event_id"))?;
        let timestamp = self
            .timestamp
            .ok_or(AuditEventBuildError::MissingField("timestamp"))?;
        let organization_id = self
            .organization_id
            .ok_or(AuditEventBuildError::MissingField("organization_id"))?;
        let event_type = self
            .event_type
            .ok_or(AuditEventBuildError::MissingField("event_type"))?;

        Ok(AuditEvent {
            event_id,
            timestamp,
            organization_id,
            user_id: self.user_id,
            device_id: self.device_id,
            session_id: self.session_id,
            event_type,
            metadata: self.metadata.unwrap_or(serde_json::Value::Null),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_audit_event_type_all_variants_as_str_and_display() {
        let expected_mappings = [
            (AuditEventType::UserLogin, "user.login"),
            (AuditEventType::UserMfa, "user.mfa"),
            (AuditEventType::DeviceEnroll, "device.enroll"),
            (AuditEventType::DeviceRevoke, "device.revoke"),
            (AuditEventType::SessionRequest, "session.request"),
            (AuditEventType::SessionAuthorize, "session.authorize"),
            (AuditEventType::SessionDeny, "session.deny"),
            (AuditEventType::SessionStart, "session.start"),
            (AuditEventType::SessionDisconnect, "session.disconnect"),
            (AuditEventType::SessionEnd, "session.end"),
            (AuditEventType::ClipboardRead, "clipboard.read"),
            (AuditEventType::ClipboardWrite, "clipboard.write"),
            (AuditEventType::FileUpload, "file.upload"),
            (AuditEventType::FileDownload, "file.download"),
            (AuditEventType::AccessRequest, "access.request"),
            (AuditEventType::AccessApprove, "access.approve"),
            (AuditEventType::PolicyUpdate, "policy.update"),
        ];

        assert_eq!(AuditEventType::ALL.len(), 17);
        assert_eq!(expected_mappings.len(), 17);

        for (variant, expected_str) in expected_mappings {
            assert_eq!(variant.as_str(), expected_str);
            assert_eq!(format!("{variant}"), expected_str);
        }
    }

    #[test]
    fn test_audit_event_type_from_str_and_parse_error() {
        for variant in AuditEventType::ALL {
            let s = variant.as_str();
            let parsed: AuditEventType = s.parse().expect("should parse valid event type");
            assert_eq!(parsed, variant);
        }

        let err = "unknown.event".parse::<AuditEventType>().unwrap_err();
        assert_eq!(err, EventParseError("unknown.event".to_string()));
        assert_eq!(format!("{err}"), "unknown audit event type: unknown.event");
    }

    #[test]
    fn test_audit_event_type_serde() {
        for variant in AuditEventType::ALL {
            let json_str = serde_json::to_string(&variant).expect("serialize");
            assert_eq!(json_str, format!("\"{}\"", variant.as_str()));
            let deserialized: AuditEventType =
                serde_json::from_str(&json_str).expect("deserialize");
            assert_eq!(deserialized, variant);
        }
    }

    #[test]
    fn test_audit_event_new_and_accessors() {
        let org_id = TenantId::new("org-acme").unwrap();
        let ts = UnixTimestamp::from_secs(1_700_000_000);
        let event = AuditEvent::new("evt-001", ts, org_id.clone(), AuditEventType::UserLogin);

        assert_eq!(event.event_id(), "evt-001");
        assert_eq!(event.timestamp(), ts);
        assert_eq!(event.organization_id(), &org_id);
        assert_eq!(event.event_type(), AuditEventType::UserLogin);
        assert_eq!(event.user_id(), None);
        assert_eq!(event.device_id(), None);
        assert_eq!(event.session_id(), None);
        assert_eq!(event.metadata(), &serde_json::Value::Null);
    }

    #[test]
    fn test_audit_event_with_setters() {
        let org_id = TenantId::new("org-acme").unwrap();
        let user_id = UserId::new("usr-123").unwrap();
        let device_id = DeviceId::new("dev-456").unwrap();
        let session_id = SessionId::new("sess-789").unwrap();
        let ts = UnixTimestamp::from_secs(1_700_000_000);

        let event = AuditEvent::new("evt-002", ts, org_id.clone(), AuditEventType::SessionStart)
            .with_user_id(user_id.clone())
            .with_device_id(device_id.clone())
            .with_session_id(session_id.clone())
            .with_metadata(json!({ "ip": "10.0.0.1", "auth_method": "fido2" }));

        assert_eq!(event.user_id(), Some(&user_id));
        assert_eq!(event.device_id(), Some(&device_id));
        assert_eq!(event.session_id(), Some(&session_id));
        assert_eq!(event.metadata()["ip"], "10.0.0.1");
        assert_eq!(event.metadata()["auth_method"], "fido2");
    }

    #[test]
    fn test_audit_event_builder_success() {
        let org_id = TenantId::new("org-enterprise").unwrap();
        let user_id = UserId::new("usr-999").unwrap();
        let ts = UnixTimestamp::from_secs(1_720_000_000);

        let event = AuditEvent::builder()
            .event_id("evt-builder-1")
            .timestamp(ts)
            .organization_id(org_id.clone())
            .user_id(user_id.clone())
            .event_type(AuditEventType::ClipboardRead)
            .metadata(json!({ "bytes_copied": 42 }))
            .build()
            .expect("valid build");

        assert_eq!(event.event_id(), "evt-builder-1");
        assert_eq!(event.timestamp(), ts);
        assert_eq!(event.organization_id(), &org_id);
        assert_eq!(event.user_id(), Some(&user_id));
        assert_eq!(event.device_id(), None);
        assert_eq!(event.session_id(), None);
        assert_eq!(event.event_type(), AuditEventType::ClipboardRead);
        assert_eq!(event.metadata()["bytes_copied"], 42);
    }

    #[test]
    fn test_audit_event_builder_missing_fields() {
        let org_id = TenantId::new("org-enterprise").unwrap();
        let ts = UnixTimestamp::from_secs(1_720_000_000);

        // Missing event_id
        let err = AuditEvent::builder()
            .timestamp(ts)
            .organization_id(org_id.clone())
            .event_type(AuditEventType::UserLogin)
            .build()
            .unwrap_err();
        assert_eq!(err, AuditEventBuildError::MissingField("event_id"));

        // Missing timestamp
        let err = AuditEvent::builder()
            .event_id("evt-1")
            .organization_id(org_id.clone())
            .event_type(AuditEventType::UserLogin)
            .build()
            .unwrap_err();
        assert_eq!(err, AuditEventBuildError::MissingField("timestamp"));

        // Missing organization_id
        let err = AuditEvent::builder()
            .event_id("evt-1")
            .timestamp(ts)
            .event_type(AuditEventType::UserLogin)
            .build()
            .unwrap_err();
        assert_eq!(err, AuditEventBuildError::MissingField("organization_id"));

        // Missing event_type
        let err = AuditEvent::builder()
            .event_id("evt-1")
            .timestamp(ts)
            .organization_id(org_id)
            .build()
            .unwrap_err();
        assert_eq!(err, AuditEventBuildError::MissingField("event_type"));
    }

    #[test]
    fn test_audit_event_serde_roundtrip() {
        let org_id = TenantId::new("org-corp").unwrap();
        let user_id = UserId::new("usr-alice").unwrap();
        let device_id = DeviceId::new("dev-laptop").unwrap();
        let session_id = SessionId::new("sess-live-01").unwrap();
        let ts = UnixTimestamp::from_secs(1_705_000_000);

        let event = AuditEvent::builder()
            .event_id("evt-serde-01")
            .timestamp(ts)
            .organization_id(org_id)
            .user_id(user_id)
            .device_id(device_id)
            .session_id(session_id)
            .event_type(AuditEventType::FileUpload)
            .metadata(json!({ "filename": "report.pdf", "size_bytes": 1024 }))
            .build()
            .unwrap();

        let json_str = serde_json::to_string(&event).expect("serialize event");
        let deserialized: AuditEvent = serde_json::from_str(&json_str).expect("deserialize event");
        assert_eq!(event, deserialized);
    }

    #[test]
    fn test_audit_event_deserialization_with_missing_optional_fields() {
        let raw_json = r#"{
            "event_id": "evt-min",
            "timestamp": 1705000000,
            "organization_id": "org-min",
            "event_type": "policy.update"
        }"#;

        let event: AuditEvent = serde_json::from_str(raw_json).expect("deserialize minimal");
        assert_eq!(event.event_id(), "evt-min");
        assert_eq!(event.timestamp(), UnixTimestamp::from_secs(1705000000));
        assert_eq!(event.organization_id().as_str(), "org-min");
        assert_eq!(event.event_type(), AuditEventType::PolicyUpdate);
        assert_eq!(event.user_id(), None);
        assert_eq!(event.device_id(), None);
        assert_eq!(event.session_id(), None);
        assert_eq!(event.metadata(), &serde_json::Value::Null);
    }

    #[test]
    fn test_canonical_bytes_and_json_determinism() {
        let org_id = TenantId::new("org-xyz").unwrap();
        let ts = UnixTimestamp::from_secs(1_700_123_456);

        // Construct metadata with keys inserted in different orders
        let mut map1 = serde_json::Map::new();
        map1.insert("zebra".to_string(), json!(1));
        map1.insert("alpha".to_string(), json!(2));
        map1.insert("beta".to_string(), json!(3));

        let mut map2 = serde_json::Map::new();
        map2.insert("alpha".to_string(), json!(2));
        map2.insert("beta".to_string(), json!(3));
        map2.insert("zebra".to_string(), json!(1));

        let event1 = AuditEvent::new(
            "evt-canon",
            ts,
            org_id.clone(),
            AuditEventType::AccessRequest,
        )
        .with_metadata(serde_json::Value::Object(map1));

        let event2 = AuditEvent::new("evt-canon", ts, org_id, AuditEventType::AccessRequest)
            .with_metadata(serde_json::Value::Object(map2));

        let bytes1 = event1.canonical_bytes().expect("canonical bytes 1");
        let bytes2 = event2.canonical_bytes().expect("canonical bytes 2");
        assert_eq!(bytes1, bytes2);

        let str1 = event1.canonical_json().expect("canonical json 1");
        let str2 = event2.canonical_json().expect("canonical json 2");
        assert_eq!(str1, str2);

        // Expected deterministic JSON string format
        let expected = "{\"event_id\":\"evt-canon\",\"timestamp\":1700123456,\"organization_id\":\"org-xyz\",\"event_type\":\"access.request\",\"metadata\":{\"alpha\":2,\"beta\":3,\"zebra\":1}}";
        assert_eq!(str1, expected);
        assert_eq!(bytes1, expected.as_bytes());
    }
}
