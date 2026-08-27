use super::{SqliteStorage, StorageError};
use nexus_audit::{AuditEvent, AuditEventType, ChainedAuditEvent};
use nexus_common::id::{DeviceId, SessionId, TenantId, UserId};
use sqlx::Row;
use std::str::FromStr;

impl SqliteStorage {
    pub async fn insert_audit_event(
        &self,
        chained: &ChainedAuditEvent,
    ) -> Result<(), StorageError> {
        let event = &chained.event;
        let json = event.canonical_json()?;
        sqlx::query(
            "INSERT INTO organizations (id, created_at) VALUES (?, ?) ON CONFLICT(id) DO NOTHING",
        )
        .bind(event.organization_id.as_str())
        .bind(
            i64::try_from(event.timestamp.as_secs())
                .map_err(|_| StorageError::CorruptRow("timestamp".into()))?,
        )
        .execute(&self.pool)
        .await?;
        sqlx::query("INSERT INTO audit_events (event_id, organization_id, user_id, device_id, session_id, event_type, sequence, event_json, previous_hash, hash, timestamp) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)")
            .bind(&event.event_id).bind(event.organization_id.as_str()).bind(event.user_id.as_ref().map(UserId::as_str)).bind(event.device_id.as_ref().map(DeviceId::as_str)).bind(event.session_id.as_ref().map(SessionId::as_str)).bind(event.event_type.as_str()).bind(i64::try_from(chained.sequence).map_err(|_| StorageError::CorruptRow("sequence".into()))?).bind(json).bind(chained.previous_hash.as_bytes()).bind(chained.hash.as_bytes()).bind(i64::try_from(event.timestamp.as_secs()).map_err(|_| StorageError::CorruptRow("timestamp".into()))?).execute(&self.pool).await?;
        Ok(())
    }

    pub async fn list_audit_events(
        &self,
        organization_id: &TenantId,
    ) -> Result<Vec<ChainedAuditEvent>, StorageError> {
        let rows =
            sqlx::query("SELECT * FROM audit_events WHERE organization_id = ? ORDER BY sequence")
                .bind(organization_id.as_str())
                .fetch_all(&self.pool)
                .await?;
        rows.into_iter()
            .map(|r| {
                let seq = u64::try_from(
                    r.try_get::<i64, _>("sequence")
                        .map_err(|e| StorageError::CorruptRow(e.to_string()))?,
                )
                .map_err(|_| StorageError::CorruptRow("sequence".into()))?;
                let event: AuditEvent = serde_json::from_str(
                    &r.try_get::<String, _>("event_json")
                        .map_err(|e| StorageError::CorruptRow(e.to_string()))?,
                )
                .map_err(|e| StorageError::CorruptRow(format!("event_json: {e}")))?;
                let event_id: String = r
                    .try_get("event_id")
                    .map_err(|e| StorageError::CorruptRow(e.to_string()))?;
                if event.event_id != event_id || event.organization_id != *organization_id {
                    return Err(StorageError::CorruptRow(
                        "event columns inconsistent".into(),
                    ));
                }
                let stored_ts = u64::try_from(
                    r.try_get::<i64, _>("timestamp")
                        .map_err(|e| StorageError::CorruptRow(e.to_string()))?,
                )
                .map_err(|_| StorageError::CorruptRow("timestamp".into()))?;
                if event.timestamp.as_secs() != stored_ts
                    || event.organization_id.as_str() != organization_id.as_str()
                {
                    return Err(StorageError::CorruptRow("timestamp/organization_id".into()));
                }
                let stored_user: Option<String> = r
                    .try_get("user_id")
                    .map_err(|e| StorageError::CorruptRow(e.to_string()))?;
                if stored_user.as_deref() != event.user_id.as_ref().map(UserId::as_str) {
                    return Err(StorageError::CorruptRow("user_id".into()));
                }
                let stored_device: Option<String> = r
                    .try_get("device_id")
                    .map_err(|e| StorageError::CorruptRow(e.to_string()))?;
                if stored_device.as_deref() != event.device_id.as_ref().map(DeviceId::as_str) {
                    return Err(StorageError::CorruptRow("device_id".into()));
                }
                let stored_session: Option<String> = r
                    .try_get("session_id")
                    .map_err(|e| StorageError::CorruptRow(e.to_string()))?;
                if stored_session.as_deref() != event.session_id.as_ref().map(SessionId::as_str) {
                    return Err(StorageError::CorruptRow("session_id".into()));
                }
                let et = AuditEventType::from_str(
                    &r.try_get::<String, _>("event_type")
                        .map_err(|e| StorageError::CorruptRow(e.to_string()))?,
                )
                .map_err(|e| StorageError::CorruptRow(e.to_string()))?;
                if event.event_type != et {
                    return Err(StorageError::CorruptRow("event_type".into()));
                }
                let prev = String::from_utf8(
                    r.try_get::<Vec<u8>, _>("previous_hash")
                        .map_err(|e| StorageError::CorruptRow(e.to_string()))?,
                )
                .map_err(|e| StorageError::CorruptRow(e.to_string()))?;
                let hash = String::from_utf8(
                    r.try_get::<Vec<u8>, _>("hash")
                        .map_err(|e| StorageError::CorruptRow(e.to_string()))?,
                )
                .map_err(|e| StorageError::CorruptRow(e.to_string()))?;
                Ok(ChainedAuditEvent {
                    sequence: seq,
                    event,
                    previous_hash: prev,
                    hash,
                })
            })
            .collect()
    }
}
