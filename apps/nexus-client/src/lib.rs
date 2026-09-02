//! Portable client lifecycle primitives.

pub mod receiver;
pub mod renderer;
pub mod session;

mod decoder;
#[cfg(windows)]
mod native_worker;

pub use receiver::{
    ClientInputError, ClientInputSender, ClientReceiver, ClientReceiverError, DecodedFrameJob,
};
pub use renderer::{RenderQueue, RenderQueueError};

/// Decodes an already authenticated H.264 job and presents it to the explicit
/// interactive HWND. This is Windows-only smoke plumbing, not a portable UI.
#[cfg(windows)]
pub fn interactive_windows_media_smoke(hwnd: isize, job: DecodedFrameJob) -> Result<(), String> {
    let surface = decoder::native_decoder_smoke(job).map_err(|error| error.to_string())?;
    renderer::native_renderer_smoke(hwnd, surface).map_err(|error| error.to_string())
}
