use super::{SqliteStorage, StorageError};
use crate::state::RegisteredDevice;
use nexus_auth::enrollment::EnrollmentToken;
use nexus_common::time::UnixTimestamp;
use sqlx::Row;

#[derive(Debug, thiserror::Error)]
pub enum EnrollmentError {
    #[error("enrollment token {token_id} not found")]
    NotFound { token_id: String },
    #[error("enrollment token expired at {expired_at}, current time {current_time}")]
    Expired {
        expired_at: UnixTimestamp,
        current_time: UnixTimestamp,
    },
    #[error("enrollment token not active until {not_before}, current time {current_time}")]
    NotYetActive {
        not_before: UnixTimestamp,
        current_time: UnixTimestamp,
    },
    #[error("enrollment token {token_id} has exceeded maximum uses ({uses_count}/{max_uses})")]
    Exhausted {
        token_id: String,
        uses_count: u32,
        max_uses: u32,
    },
    #[error(transparent)]
    Storage(#[from] StorageError),
}

impl From<sqlx::Error> for EnrollmentError {
    fn from(value: sqlx::Error) -> Self {
        Self::Storage(StorageError::Database(value))
    }
}
impl From<serde_json::Error> for EnrollmentError {
    fn from(value: serde_json::Error) -> Self {
        Self::Storage(StorageError::Serialization(value))
    }
}

impl SqliteStorage {
    pub async fn store_enrollment_token(
        &self,
        token: &EnrollmentToken,
    ) -> Result<(), StorageError> {
        let json = serde_json::to_string(token)?;
        let mut tx = self.pool.begin().await?;
        sqlx::query(
            "INSERT INTO organizations (id, created_at) VALUES (?, ?) ON CONFLICT(id) DO NOTHING",
        )
        .bind(token.organization_id.as_str())
        .bind(
            i64::try_from(token.not_before.as_secs())
                .map_err(|_| StorageError::CorruptRow("not_before".into()))?,
        )
        .execute(&mut *tx)
        .await?;
        sqlx::query("INSERT OR REPLACE INTO enrollment_tokens (token_id, organization_id, token_json, not_before, expires_at, max_uses, uses_count) VALUES (?, ?, ?, ?, ?, ?, 0)")
            .bind(&token.token_id).bind(token.organization_id.as_str()).bind(json)
            .bind(i64::try_from(token.not_before.as_secs()).map_err(|_| StorageError::CorruptRow("not_before".into()))?)
            .bind(i64::try_from(token.expires_at.as_secs()).map_err(|_| StorageError::CorruptRow("expires_at".into()))?)
            .bind(i64::from(token.max_uses)).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn enroll_device(
        &self,
        token_id: &str,
        now: UnixTimestamp,
        device: RegisteredDevice,
    ) -> Result<EnrollmentToken, EnrollmentError> {
        let mut tx = self.pool.begin().await?;
        let row = sqlx::query("SELECT token_json, uses_count, max_uses, not_before, expires_at FROM enrollment_tokens WHERE token_id = ?")
            .bind(token_id).fetch_optional(&mut *tx).await?;
        let row = row.ok_or_else(|| EnrollmentError::NotFound {
            token_id: token_id.to_string(),
        })?;
        let token_json: String = row
            .try_get("token_json")
            .map_err(|e| StorageError::CorruptRow(format!("token_json: {e}")))?;
        let token: EnrollmentToken = serde_json::from_str(&token_json)
            .map_err(|e| StorageError::CorruptRow(format!("token_json: {e}")))?;
        let uses: i64 = row
            .try_get("uses_count")
            .map_err(|e| StorageError::CorruptRow(format!("uses_count: {e}")))?;
        let max: i64 = row
            .try_get("max_uses")
            .map_err(|e| StorageError::CorruptRow(format!("max_uses: {e}")))?;
        let not_before_db: i64 = row
            .try_get("not_before")
            .map_err(|e| StorageError::CorruptRow(format!("not_before: {e}")))?;
        let expires_at_db: i64 = row
            .try_get("expires_at")
            .map_err(|e| StorageError::CorruptRow(format!("expires_at: {e}")))?;
        let not_before_db = u64::try_from(not_before_db)
            .map_err(|_| StorageError::CorruptRow("not_before".into()))?;
        let expires_at_db = u64::try_from(expires_at_db)
            .map_err(|_| StorageError::CorruptRow("expires_at".into()))?;
        let uses_u =
            u32::try_from(uses).map_err(|_| StorageError::CorruptRow("uses_count".into()))?;
        let max_u = u32::try_from(max).map_err(|_| StorageError::CorruptRow("max_uses".into()))?;
        let expires_at = UnixTimestamp::from_secs(expires_at_db);
        let not_before = UnixTimestamp::from_secs(not_before_db);
        if now < not_before {
            return Err(EnrollmentError::NotYetActive {
                not_before,
                current_time: now,
            });
        }
        if now > expires_at {
            return Err(EnrollmentError::Expired {
                expired_at: expires_at,
                current_time: now,
            });
        }
        let _ = not_before_db;
        if uses_u >= max_u {
            return Err(EnrollmentError::Exhausted {
                token_id: token_id.to_string(),
                uses_count: uses_u,
                max_uses: max_u,
            });
        }
        let updated = match sqlx::query("UPDATE enrollment_tokens SET uses_count = uses_count + 1 WHERE token_id = ? AND uses_count < max_uses AND expires_at >= ?")
            .bind(token_id).bind(i64::try_from(now.as_secs()).map_err(|_| StorageError::CorruptRow("current_time".into()))?).execute(&mut *tx).await {
            Ok(result) => result,
            Err(e) if e.as_database_error().and_then(|d| d.code()).as_deref() == Some("5") => {
                return Err(EnrollmentError::Exhausted { token_id: token_id.to_string(), uses_count: uses_u, max_uses: max_u });
            }
            Err(e) => return Err(EnrollmentError::Storage(StorageError::Database(e))),
        };
        if updated.rows_affected() != 1 {
            return Err(EnrollmentError::Exhausted {
                token_id: token_id.to_string(),
                uses_count: uses_u,
                max_uses: max_u,
            });
        }
        let cred = &device.credential;
        if cred.organization_id != token.organization_id {
            return Err(EnrollmentError::Storage(StorageError::CorruptRow(
                "organization_id mismatch".into(),
            )));
        }
        let cred_json = serde_json::to_string(cred)?;
        sqlx::query(
            "INSERT INTO organizations (id, created_at) VALUES (?, ?) ON CONFLICT(id) DO NOTHING",
        )
        .bind(cred.organization_id.as_str())
        .bind(
            i64::try_from(device.enrolled_at.as_secs())
                .map_err(|_| StorageError::CorruptRow("enrolled_at".into()))?,
        )
        .execute(&mut *tx)
        .await?;
        sqlx::query("INSERT INTO devices (id, organization_id, hostname, os, os_version, architecture, agent_version, public_key, status, capabilities_json, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)")
            .bind(cred.device_id.as_str()).bind(cred.organization_id.as_str()).bind(&device.hostname).bind(&cred.os).bind("").bind(&cred.architecture).bind(&device.agent_version).bind(&cred.public_key).bind(if device.is_active {"active"} else {"revoked"}).bind(serde_json::to_string(&cred.capabilities)?).bind(i64::try_from(device.enrolled_at.as_secs()).map_err(|_| StorageError::CorruptRow("enrolled_at".into()))?).execute(&mut *tx).await?;
        sqlx::query("INSERT INTO device_credentials (device_id, organization_id, credential_json, issued_at, expires_at, public_key) VALUES (?, ?, ?, ?, ?, ?)")
            .bind(cred.device_id.as_str()).bind(cred.organization_id.as_str()).bind(cred_json).bind(i64::try_from(cred.issued_at.as_secs()).map_err(|_| StorageError::CorruptRow("issued_at".into()))?).bind(i64::try_from(cred.expires_at.as_secs()).map_err(|_| StorageError::CorruptRow("expires_at".into()))?).bind(&cred.public_key).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(token)
    }
}
