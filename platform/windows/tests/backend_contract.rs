#[cfg(not(windows))]
use platform_windows::WindowsCursorSource;
use platform_windows::{BackendErrorKind, CursorSnapshot};

fn cursor() -> CursorSnapshot {
    CursorSnapshot {
        visible: true,
        x: 40,
        y: 80,
        width: 2,
        height: 2,
        hotspot_x: 1,
        hotspot_y: 1,
        rgba: vec![0xff; 16],
    }
}

#[test]
fn cursor_accepts_exact_rgba_payload_with_in_bounds_hotspot() {
    assert_eq!(cursor().validate(), Ok(()));
}

#[test]
fn cursor_rejects_hotspot_outside_visible_bounds() {
    let mut snapshot = cursor();
    snapshot.hotspot_x = 2;

    assert_eq!(
        snapshot.validate().unwrap_err().kind(),
        BackendErrorKind::HotspotOutOfBounds
    );
}

#[test]
fn cursor_rejects_rgba_payload_with_wrong_length() {
    let mut snapshot = cursor();
    snapshot.rgba.pop();

    assert_eq!(
        snapshot.validate().unwrap_err().kind(),
        BackendErrorKind::CursorPayloadLength
    );
}

#[cfg(not(windows))]
#[test]
fn system_cursor_snapshot_fails_closed_off_windows() {
    let mut cursor = WindowsCursorSource::system();

    assert_eq!(
        cursor.snapshot().unwrap_err().kind(),
        BackendErrorKind::UnsupportedPlatform
    );
}
