//! Authentication and capability replay-protection primitives.

mod replay;

pub use replay::NonceReplayCache;

pub fn init() {
    // Initializer stub for nexus-auth
}
