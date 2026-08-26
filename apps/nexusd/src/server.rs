//! Server lifecycle and HTTP transport listener for nexusd.
//! Part of Nexus Remote Desktop Platform.

use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::Notify;

use crate::routes::create_router;
use crate::state::AppState;

/// Control plane HTTP server instance.
pub struct ControlPlaneServer {
    state: AppState,
    listener: tokio::net::TcpListener,
    shutdown_notify: Arc<Notify>,
}

impl ControlPlaneServer {
    /// Binds the server to the specified socket address.
    pub async fn bind(addr: SocketAddr, state: AppState) -> std::io::Result<Self> {
        let listener = tokio::net::TcpListener::bind(addr).await?;
        Ok(Self {
            state,
            listener,
            shutdown_notify: Arc::new(Notify::new()),
        })
    }

    /// Returns the local socket address this server is listening on.
    pub fn local_addr(&self) -> std::io::Result<SocketAddr> {
        self.listener.local_addr()
    }

    /// Triggers graceful server shutdown.
    pub fn shutdown(&self) {
        self.shutdown_notify.notify_waiters();
    }

    /// Runs the HTTP server until shutdown signal is received.
    pub async fn run(self) -> std::io::Result<()> {
        let app = create_router(self.state);
        let shutdown = self.shutdown_notify;

        axum::serve(self.listener, app)
            .with_graceful_shutdown(async move {
                shutdown.notified().await;
            })
            .await
    }
}
