//! Portable client lifecycle primitives.

pub mod receiver;
pub mod renderer;
pub mod session;

mod decoder;

pub use receiver::{
    ClientInputError, ClientInputSender, ClientReceiver, ClientReceiverError, DecodedFrameJob,
};
pub use renderer::{RenderQueue, RenderQueueError};

/// Starts the private Media Foundation and D3D11 adapters, then uploads one
/// synthetic decoded surface. It is only an interactive Windows smoke check;
/// the authenticated receiver-to-decoder flow is covered by the loopback task.
#[cfg(windows)]
pub fn interactive_windows_media_smoke() -> Result<(), String> {
    decoder::native_decoder_smoke().map_err(|error| error.to_string())?;
    renderer::native_renderer_smoke().map_err(|error| error.to_string())
}
