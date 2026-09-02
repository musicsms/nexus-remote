//! Portable client lifecycle primitives.

pub mod receiver;
pub mod session;

pub use receiver::{
    ClientInputError, ClientInputSender, ClientReceiver, ClientReceiverError, DecodedFrameJob,
};
