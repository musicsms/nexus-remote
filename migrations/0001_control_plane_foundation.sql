PRAGMA foreign_keys = ON;

CREATE TABLE organizations (
    id TEXT PRIMARY KEY,
    created_at BIGINT NOT NULL
);

CREATE TABLE enrollment_tokens (
    token_id TEXT PRIMARY KEY,
    organization_id TEXT NOT NULL REFERENCES organizations(id),
    token_json TEXT NOT NULL,
    not_before BIGINT NOT NULL,
    expires_at BIGINT NOT NULL,
    max_uses INTEGER NOT NULL,
    uses_count INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX idx_enrollment_tokens_organization ON enrollment_tokens(organization_id);

CREATE TABLE devices (
    id TEXT PRIMARY KEY,
    organization_id TEXT NOT NULL REFERENCES organizations(id),
    hostname TEXT NOT NULL,
    os TEXT NOT NULL,
    os_version TEXT NOT NULL,
    architecture TEXT NOT NULL,
    agent_version TEXT NOT NULL,
    public_key BLOB NOT NULL,
    last_seen_at BIGINT,
    status TEXT NOT NULL,
    capabilities_json TEXT NOT NULL,
    created_at BIGINT NOT NULL,
    revoked_at BIGINT
);
CREATE INDEX idx_devices_organization ON devices(organization_id);

CREATE TABLE device_credentials (
    device_id TEXT PRIMARY KEY REFERENCES devices(id),
    organization_id TEXT NOT NULL REFERENCES organizations(id),
    credential_json TEXT NOT NULL,
    issued_at BIGINT NOT NULL,
    expires_at BIGINT NOT NULL,
    public_key BLOB NOT NULL
);
CREATE INDEX idx_device_credentials_organization ON device_credentials(organization_id);

CREATE TABLE sessions (
    id TEXT PRIMARY KEY,
    organization_id TEXT NOT NULL REFERENCES organizations(id),
    user_id TEXT NOT NULL,
    client_device_id TEXT,
    target_device_id TEXT REFERENCES devices(id),
    status TEXT NOT NULL,
    connection_mode TEXT NOT NULL,
    relay_id TEXT,
    created_at BIGINT NOT NULL,
    started_at BIGINT,
    ended_at BIGINT,
    policy_snapshot_json TEXT NOT NULL,
    termination_reason TEXT
);
CREATE INDEX idx_sessions_organization ON sessions(organization_id);
CREATE INDEX idx_sessions_target_device ON sessions(target_device_id);

CREATE TABLE audit_events (
    event_id TEXT PRIMARY KEY,
    organization_id TEXT NOT NULL REFERENCES organizations(id),
    user_id TEXT,
    device_id TEXT REFERENCES devices(id),
    session_id TEXT REFERENCES sessions(id),
    event_type TEXT NOT NULL,
    sequence BIGINT NOT NULL,
    event_json TEXT NOT NULL,
    previous_hash BLOB,
    hash BLOB NOT NULL,
    timestamp BIGINT NOT NULL
);
CREATE INDEX idx_audit_events_organization_sequence ON audit_events(organization_id, sequence);
