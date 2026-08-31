#![cfg(windows)]

use std::time::Instant;

use nexus_capture::CaptureSource;
use platform_windows::{CaptureApi, CaptureConfig, CaptureState, WindowsCaptureSource};

#[test]
#[ignore = "requires an interactive Windows desktop"]
fn captures_one_real_desktop_frame() {
    let started = Instant::now();
    let mut source = WindowsCaptureSource::start(CaptureConfig::default())
        .expect("interactive desktop capture should start");
    let api = match source.state() {
        CaptureState::Running(api) => api,
        state => panic!("capture source did not enter Running: {state:?}"),
    };
    let frame = source
        .next_frame()
        .expect("interactive desktop should produce one frame");
    frame.validate().expect("native frame should be valid");

    println!(
        "{}x{} api={} elapsed_ms={}",
        frame.width,
        frame.height,
        api_name(api),
        started.elapsed().as_millis()
    );
}

fn api_name(api: CaptureApi) -> &'static str {
    match api {
        CaptureApi::Wgc => "WGC",
        CaptureApi::Dxgi => "DXGI",
    }
}
