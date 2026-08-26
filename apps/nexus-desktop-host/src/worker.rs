use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::sleep;
use tracing::warn;

use nexus_capture::SyntheticCaptureSource;
use nexus_common::id::SessionId;

use crate::input_handler::HostInputHandler;
use crate::streamer::{HostVideoStreamer, StreamerError};

/// Active worker running in the user desktop session.
pub struct DesktopHostWorker {
    pub session_id: SessionId,
    pub input_handler: Arc<HostInputHandler>,
    running: Arc<AtomicBool>,
}

impl DesktopHostWorker {
    /// Creates a new `DesktopHostWorker`.
    pub fn new(session_id: SessionId) -> Self {
        Self {
            session_id,
            input_handler: Arc::new(HostInputHandler::new()),
            running: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Stops the worker loop.
    pub fn stop(&self) {
        self.running.store(false, Ordering::SeqCst);
    }

    /// Checks if worker is running.
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    /// Runs video streaming loop feeding generated datagrams to `datagram_tx`.
    pub async fn run_streamer(
        &self,
        aead_key: [u8; 32],
        width: u32,
        height: u32,
        datagram_tx: mpsc::Sender<Vec<u8>>,
    ) -> Result<(), StreamerError> {
        self.running.store(true, Ordering::SeqCst);
        let capture = SyntheticCaptureSource::new(width, height, 30);
        let mut streamer = HostVideoStreamer::new(capture, aead_key, width, height)?;

        while self.running.load(Ordering::Relaxed) {
            let datagrams = streamer.process_next_frame()?;
            for dg in datagrams {
                if datagram_tx.send(dg).await.is_err() {
                    warn!("Datagram receiver channel dropped; stopping streamer");
                    self.stop();
                    break;
                }
            }
            sleep(Duration::from_millis(33)).await; // ~30 FPS
        }

        Ok(())
    }
}
