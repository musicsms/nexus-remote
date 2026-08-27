use super::{SqliteStorage, StorageError};
use crate::state::RegisteredDevice;
use nexus_common::id::{DeviceId, TenantId};
use nexus_common::time::UnixTimestamp;
use sqlx::Row;

pub(crate) fn device_from_row(
    row: &sqlx::sqlite::SqliteRow,
) -> Result<RegisteredDevice, StorageError> {
    let device_id: String = row
        .try_get("id")
        .map_err(|e| StorageError::CorruptRow(format!("device_id: {e}")))?;
    let organization_id: String = row
        .try_get("organization_id")
        .map_err(|e| StorageError::CorruptRow(format!("organization_id: {e}")))?;
    let credential_json: String = row
        .try_get("credential_json")
        .map_err(|e| StorageError::CorruptRow(format!("credential_json: {e}")))?;
    let device_id = DeviceId::new(device_id)
        .map_err(|e| StorageError::CorruptRow(format!("device_id: {e}")))?;
    let organization_id = TenantId::new(organization_id)
        .map_err(|e| StorageError::CorruptRow(format!("organization_id: {e}")))?;
    let credential: nexus_auth::credential::DeviceCredential =
        serde_json::from_str(&credential_json)
            .map_err(|e| StorageError::CorruptRow(format!("credential_json: {e}")))?;
    if credential.device_id != device_id {
        return Err(StorageError::CorruptRow("credential.device_id".into()));
    }
    if credential.organization_id != organization_id {
        return Err(StorageError::CorruptRow(
            "credential.organization_id".into(),
        ));
    }
    let enrolled_at: i64 = row
        .try_get("created_at")
        .map_err(|e| StorageError::CorruptRow(format!("created_at: {e}")))?;
    let enrolled_at =
        u64::try_from(enrolled_at).map_err(|_| StorageError::CorruptRow("created_at".into()))?;
    let hostname: String = row
        .try_get("hostname")
        .map_err(|e| StorageError::CorruptRow(format!("hostname: {e}")))?;
    let agent_version: String = row
        .try_get("agent_version")
        .map_err(|e| StorageError::CorruptRow(format!("agent_version: {e}")))?;
    let status: String = row
        .try_get("status")
        .map_err(|e| StorageError::CorruptRow(format!("status: {e}")))?;
    let is_active = match status.as_str() {
        "active" => true,
        "revoked" => false,
        _ => return Err(StorageError::CorruptRow("status".into())),
    };
    Ok(RegisteredDevice {
        credential,
        hostname,
        agent_version,
        enrolled_at: UnixTimestamp::from_secs(enrolled_at),
        is_active,
    })
}

impl SqliteStorage {
    pub async fn get_device(
        &self,
        device_id: &DeviceId,
    ) -> Result<Option<RegisteredDevice>, StorageError> {
        let row = sqlx::query("SELECT d.*, c.credential_json FROM devices d JOIN device_credentials c ON c.device_id = d.id WHERE d.id = ?")
            .bind(device_id.as_str()).fetch_optional(&self.pool).await?;
        row.as_ref().map(device_from_row).transpose()
    }

    pub async fn list_devices(
        &self,
        organization_id: &TenantId,
    ) -> Result<Vec<RegisteredDevice>, StorageError> {
        let rows = sqlx::query("SELECT d.*, c.credential_json FROM devices d JOIN device_credentials c ON c.device_id = d.id WHERE d.organization_id = ? ORDER BY d.id")
            .bind(organization_id.as_str()).fetch_all(&self.pool).await?;
        rows.iter().map(device_from_row).collect()
    }

    pub async fn count_devices(&self) -> Result<u64, StorageError> {
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM devices")
            .fetch_one(&self.pool)
            .await?;
        u64::try_from(count).map_err(|_| StorageError::CorruptRow("device count".into()))
    }
}
