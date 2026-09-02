#![cfg(windows)]

use nexus_client::interactive_windows_media_smoke;

/// This test is intentionally kept off Linux and CI: it needs an interactive
/// Windows D3D11 + Media Foundation environment, not merely a cross-compile.
#[test]
#[ignore = "requires an interactive Windows D3D11 and Media Foundation environment"]
fn media_foundation_decoder_and_d3d_surface_can_start() {
    interactive_windows_media_smoke().unwrap();
}
