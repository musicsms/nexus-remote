#![cfg(windows)]

use nexus_client::{InputController, WindowCommand, WindowConfig, WindowController, WindowEvent};
use nexus_input::InputEvent;
use nexus_protocol::MonitorInfo;
use std::time::{Duration, Instant};
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_MOUSEMOVE};

#[test]
#[ignore = "requires an interactive Windows desktop"]
fn creates_window_routes_controlled_input_and_closes() {
    let mut window = WindowController::start(WindowConfig::default()).unwrap();
    let monitor = MonitorInfo {
        id: 1,
        origin_x: 0,
        origin_y: 0,
        width: 1280,
        height: 720,
        scale: 1.0,
    };
    let mut input = InputController::new(monitor).unwrap();
    window.try_send(WindowCommand::RequestFocus).unwrap();
    let mut focused = false;
    for _ in 0..250 {
        if let Some(event) = window.try_next_event() {
            input.handle_window_event(&event).unwrap();
            if matches!(
                event,
                WindowEvent::FocusChanged(true) | WindowEvent::Focused(true)
            ) {
                focused = true;
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(4));
    }
    assert!(focused, "native window did not report focus");

    let hwnd = HWND(window.native_handle().expect("native HWND") as *mut core::ffi::c_void);
    let point = LPARAM((10u32 | (10u32 << 16)) as isize);
    unsafe { PostMessageW(hwnd, WM_MOUSEMOVE, WPARAM(0), point).unwrap() };
    let mut translated = false;
    for _ in 0..250 {
        if let Some(event) = window.try_next_event() {
            input.handle_window_event(&event).unwrap();
            if matches!(event, WindowEvent::Input(InputEvent::MouseMove { .. })) {
                translated = true;
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(4));
    }
    assert!(translated, "native mouse message was not translated");
    assert!(input.try_next_control().is_some());

    window.try_send(WindowCommand::Close).unwrap();
    window
        .shutdown(Instant::now() + Duration::from_secs(2))
        .unwrap();
}
