use crate::storage::StorageError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DatabaseDriver {
    Sqlite,
    Postgres,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatabaseConfig {
    pub driver: DatabaseDriver,
    pub url: String,
    pub max_connections: u32,
}

impl DatabaseConfig {
    pub fn sqlite(url: impl Into<String>) -> Self {
        Self {
            driver: DatabaseDriver::Sqlite,
            url: url.into(),
            max_connections: 4,
        }
    }

    pub fn validate(&self) -> Result<(), StorageError> {
        if self.max_connections == 0 {
            return Err(StorageError::InvalidConfig(
                "max_connections must be positive".into(),
            ));
        }
        if self.url.trim().is_empty() {
            return Err(StorageError::InvalidConfig("url must not be empty".into()));
        }
        Ok(())
    }
}
