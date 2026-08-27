use super::{SqliteStorage, StorageError};
use nexus_common::id::{DeviceId, SessionId, TenantId, UserId};
use nexus_common::time::UnixTimestamp;
use serde::{Deserialize, Serialize};
use sqlx::Row;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthorizedSessionRecord {
    pub session_id: SessionId,
    pub organization_id: TenantId,
    pub user_id: UserId,
    pub client_device_id: DeviceId,
    pub target_device_id: DeviceId,
    pub relay_id: String,
    pub permissions: Vec<String>,
    pub created_at: UnixTimestamp,
    pub status: String,
}

impl SqliteStorage {
    pub async fn insert_authorized_session(
        &self,
        record: &AuthorizedSessionRecord,
    ) -> Result<(), StorageError> {
        if record.status != "authorized" {
            return Err(StorageError::CorruptRow(
                "session status must be authorized".into(),
            ));
        }
        let policy = serde_json::to_string(&record.permissions)?;
        let mut transaction = self.pool.begin().await?;
        sqlx::query(
            "INSERT INTO organizations (id, created_at) VALUES (?, ?) ON CONFLICT(id) DO NOTHING",
        )
        .bind(record.organization_id.as_str())
        .bind(
            i64::try_from(record.created_at.as_secs())
                .map_err(|_| StorageError::CorruptRow("created_at".into()))?,
        )
        .execute(&mut *transaction)
        .await?;
        sqlx::query("INSERT INTO sessions (id, organization_id, user_id, client_device_id, target_device_id, status, connection_mode, relay_id, created_at, started_at, ended_at, policy_snapshot_json, termination_reason) VALUES (?, ?, ?, ?, ?, 'authorized', 'relay', ?, ?, NULL, NULL, ?, NULL)")
            .bind(record.session_id.as_str()).bind(record.organization_id.as_str()).bind(record.user_id.as_str())
            .bind(record.client_device_id.as_str()).bind(record.target_device_id.as_str()).bind(&record.relay_id)
            .bind(i64::try_from(record.created_at.as_secs()).map_err(|_| StorageError::CorruptRow("created_at".into()))?)
            .bind(policy).execute(&mut *transaction).await?;
        transaction.commit().await?;
        Ok(())
    }

    pub async fn get_session(
        &self,
        session_id: &SessionId,
    ) -> Result<Option<AuthorizedSessionRecord>, StorageError> {
        let row = sqlx::query("SELECT * FROM sessions WHERE id = ?")
            .bind(session_id.as_str())
            .fetch_optional(&self.pool)
            .await?;
        row.map(|r| {
            let sid = SessionId::new(
                r.try_get::<String, _>("id")
                    .map_err(|e| StorageError::CorruptRow(e.to_string()))?,
            )
            .map_err(|e| StorageError::CorruptRow(e.to_string()))?;
            let org = TenantId::new(
                r.try_get::<String, _>("organization_id")
                    .map_err(|e| StorageError::CorruptRow(e.to_string()))?,
            )
            .map_err(|e| StorageError::CorruptRow(e.to_string()))?;
            let user = UserId::new(
                r.try_get::<String, _>("user_id")
                    .map_err(|e| StorageError::CorruptRow(e.to_string()))?,
            )
            .map_err(|e| StorageError::CorruptRow(e.to_string()))?;
            let client = DeviceId::new(
                r.try_get::<String, _>("client_device_id")
                    .map_err(|e| StorageError::CorruptRow(e.to_string()))?,
            )
            .map_err(|e| StorageError::CorruptRow(e.to_string()))?;
            let target = DeviceId::new(
                r.try_get::<String, _>("target_device_id")
                    .map_err(|e| StorageError::CorruptRow(e.to_string()))?,
            )
            .map_err(|e| StorageError::CorruptRow(e.to_string()))?;
            let status: String = r
                .try_get("status")
                .map_err(|e| StorageError::CorruptRow(e.to_string()))?;
            if status != "authorized" {
                return Err(StorageError::CorruptRow("status".into()));
            }
            let mode: String = r
                .try_get("connection_mode")
                .map_err(|e| StorageError::CorruptRow(e.to_string()))?;
            if mode != "relay" {
                return Err(StorageError::CorruptRow("connection_mode".into()));
            }
            let ts: i64 = r
                .try_get("created_at")
                .map_err(|e| StorageError::CorruptRow(e.to_string()))?;
            let created_at = UnixTimestamp::from_secs(
                u64::try_from(ts).map_err(|_| StorageError::CorruptRow("created_at".into()))?,
            );
            let permissions: Vec<String> = serde_json::from_str(
                &r.try_get::<String, _>("policy_snapshot_json")
                    .map_err(|e| StorageError::CorruptRow(e.to_string()))?,
            )
            .map_err(|e| StorageError::CorruptRow(format!("permissions: {e}")))?;
            Ok(AuthorizedSessionRecord {
                session_id: sid,
                organization_id: org,
                user_id: user,
                client_device_id: client,
                target_device_id: target,
                relay_id: r
                    .try_get("relay_id")
                    .map_err(|e| StorageError::CorruptRow(e.to_string()))?,
                permissions,
                created_at,
                status,
            })
        })
        .transpose()
    }
}
