#![cfg(windows)]

use nexus_input::InputEvent;
use platform_windows::{InputInjector, MonitorBounds, WindowsCursorSource};
use windows::Win32::Foundation::POINT;
use windows::Win32::UI::WindowsAndMessaging::{
    GetCursorPos, GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN,
    SM_YVIRTUALSCREEN,
};

#[test]
#[ignore = "requires an interactive Windows desktop"]
fn captures_the_current_cursor_shape_and_position() {
    let mut source = WindowsCursorSource::system();
    let snapshot = source
        .snapshot()
        .expect("interactive desktop cursor capture should succeed");
    snapshot
        .validate()
        .expect("native cursor snapshot should be bounded and valid");

    println!(
        "visible={} position=({}, {}) size={}x{} hotspot=({}, {})",
        snapshot.visible,
        snapshot.x,
        snapshot.y,
        snapshot.width,
        snapshot.height,
        snapshot.hotspot_x,
        snapshot.hotspot_y
    );
}

#[test]
#[ignore = "requires an interactive Windows desktop and moves the pointer only to its current position"]
fn send_input_reasserts_the_current_pointer_position() {
    let mut point = POINT::default();
    // SAFETY: `point` is valid writable storage for the synchronous API call.
    unsafe { GetCursorPos(&mut point) }.expect("current pointer position should be available");
    // SAFETY: GetSystemMetrics has no pointer arguments and only reads system metrics.
    let min_x = unsafe { GetSystemMetrics(SM_XVIRTUALSCREEN) };
    // SAFETY: GetSystemMetrics has no pointer arguments and only reads system metrics.
    let min_y = unsafe { GetSystemMetrics(SM_YVIRTUALSCREEN) };
    // SAFETY: GetSystemMetrics has no pointer arguments and only reads system metrics.
    let width = unsafe { GetSystemMetrics(SM_CXVIRTUALSCREEN) };
    // SAFETY: GetSystemMetrics has no pointer arguments and only reads system metrics.
    let height = unsafe { GetSystemMetrics(SM_CYVIRTUALSCREEN) };
    assert!(width > 0 && height > 0, "virtual desktop must have an area");
    let monitor = MonitorBounds {
        min_x,
        min_y,
        max_x: min_x
            .checked_add(width - 1)
            .expect("virtual desktop x range should fit i32"),
        max_y: min_y
            .checked_add(height - 1)
            .expect("virtual desktop y range should fit i32"),
    };
    let mut injector = InputInjector::system(monitor);

    let submitted = injector
        .inject(&InputEvent::MouseMove {
            x: point.x,
            y: point.y,
        })
        .expect("SendInput should submit the one current-position mouse record");

    assert_eq!(submitted, 1);
}
