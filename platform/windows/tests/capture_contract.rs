use platform_windows::CaptureConfig;

#[cfg(not(windows))]
#[test]
fn native_start_fails_closed_off_windows() {
    let error =
        platform_windows::WindowsCaptureSource::start(CaptureConfig::default()).unwrap_err();

    assert_eq!(
        error.kind(),
        platform_windows::BackendErrorKind::UnsupportedPlatform
    );
}

#[test]
fn config_defaults_to_allowing_initialization_fallback() {
    assert!(CaptureConfig::default().allow_dxgi_fallback);
}
