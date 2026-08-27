use sqlx::sqlite::{SqlitePool, SqlitePoolOptions};

use crate::config::{DatabaseConfig, DatabaseDriver};

mod device;
mod enrollment;

#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("unsupported database driver: {0}")]
    UnsupportedDriver(&'static str),
    #[error("invalid database configuration: {0}")]
    InvalidConfig(String),
    #[error("database operation failed: {0}")]
    Database(#[from] sqlx::Error),
    #[error("database migration failed: {0}")]
    Migration(#[from] sqlx::migrate::MigrateError),
    #[error("stored data is invalid: {0}")]
    CorruptRow(String),
    #[error("serialization failed: {0}")]
    Serialization(#[from] serde_json::Error),
}

#[derive(Clone)]
pub struct SqliteStorage {
    pool: SqlitePool,
}

pub use enrollment::EnrollmentError;

impl SqliteStorage {
    pub async fn connect(config: &DatabaseConfig) -> Result<Self, StorageError> {
        config.validate()?;
        if config.driver != DatabaseDriver::Sqlite {
            return Err(StorageError::UnsupportedDriver("postgres"));
        }
        let pool = SqlitePoolOptions::new()
            .max_connections(config.max_connections)
            .connect(&config.url)
            .await?;
        sqlx::migrate!("../../migrations").run(&pool).await?;
        Ok(Self { pool })
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }
}
