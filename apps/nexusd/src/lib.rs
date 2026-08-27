//! nexusd control plane library.
//! Part of Nexus Remote Desktop Platform.

pub mod config;
pub mod routes;
pub mod server;
pub mod state;
pub mod storage;

pub use config::{DatabaseConfig, DatabaseDriver};
pub use storage::{AuthorizedSessionRecord, EnrollmentError, SqliteStorage, StorageError};

pub use routes::{
    create_router, ErrorResponse, HealthResponse, SessionAuthorizationResponse,
    SessionRequestPayload,
};
pub use server::ControlPlaneServer;
pub use state::{AppState, RegisteredDevice, StateError, TrackedEnrollmentToken};
