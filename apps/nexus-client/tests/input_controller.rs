use nexus_client::{
    DecodedFrameJob, InputController, InputControllerError, WindowCommand, WindowConfig,
    WindowController, WindowError, WindowEvent, MAX_INPUT_RATE_PER_SECOND,
};
use nexus_input::{InputEvent, KeyAction, Modifiers};
use nexus_protocol::{MonitorInfo, MouseMove, TextInput};
use nexus_transport::control::decode_framed_control;
use std::thread;
use std::time::Duration;

fn monitor() -> MonitorInfo {
    MonitorInfo {
        id: 7,
        origin_x: -100,
        origin_y: 20,
        width: 200,
        height: 100,
        scale: 1.0,
    }
}

#[test]
fn keyboard_and_text_events_are_encoded_after_focus() {
    let mut controller = InputController::new(monitor()).unwrap();
    assert_eq!(
        controller.handle_window_event(WindowEvent::Input(InputEvent::Key {
            physical_code: 0x04,
            logical_code: 0x04,
            action: KeyAction::Down,
            modifiers: Modifiers::CTRL,
        })),
        Err(InputControllerError::NotFocused)
    );

    controller
        .handle_window_event(WindowEvent::FocusChanged(true))
        .unwrap();
    controller
        .handle_window_event(WindowEvent::Input(InputEvent::Key {
            physical_code: 0x04,
            logical_code: 0x04,
            action: KeyAction::Down,
            modifiers: Modifiers::CTRL,
        }))
        .unwrap();
    controller
        .handle_window_event(WindowEvent::Input(InputEvent::Text("hello".into())))
        .unwrap();
    let key = controller.try_next_control().unwrap();
    let text = controller.try_next_control().unwrap();
    assert!(key.len() > 4);
    assert!(text.len() > 4);
    let decoded: TextInput = decode_framed_control(&text).unwrap();
    assert_eq!(decoded.text, "hello");
}

#[test]
fn mouse_coordinates_are_clamped_to_monitor_bounds() {
    let mut controller = InputController::new(monitor()).unwrap();
    controller.set_focused(true);
    controller
        .handle_window_event(WindowEvent::Input(InputEvent::MouseMove {
            x: i32::MIN,
            y: i32::MAX,
        }))
        .unwrap();
    let bytes = controller.try_next_control().unwrap();
    let move_event: MouseMove = decode_framed_control(&bytes).unwrap();
    assert_eq!((move_event.x, move_event.y), (-100, 119));
}

#[test]
fn queue_rate_and_payload_limits_are_enforced() {
    let mut controller = InputController::with_limits(monitor(), 1, 2).unwrap();
    controller.set_focused(true);
    controller
        .handle_window_event(WindowEvent::Input(InputEvent::Text("x".into())))
        .unwrap();
    assert_eq!(
        controller.handle_window_event(WindowEvent::Input(InputEvent::Text("y".into()))),
        Err(InputControllerError::RateLimited)
    );
    assert!(controller.try_next_control().is_some());
}

#[test]
fn close_and_expiry_stop_input() {
    let mut controller = InputController::new(monitor()).unwrap();
    controller.set_focused(true);
    controller.handle_window_event(WindowEvent::Closed).unwrap();
    assert!(controller.is_shutdown());
    assert_eq!(
        controller.handle_window_event(WindowEvent::Input(InputEvent::Text("x".into()))),
        Err(InputControllerError::Shutdown)
    );
    controller.shutdown();
    assert!(controller.is_shutdown());
}

#[test]
fn cursor_snapshot_validation_is_reused() {
    let controller = InputController::new(monitor()).unwrap();
    let invalid = platform_windows::CursorSnapshot {
        visible: true,
        x: 0,
        y: 0,
        width: 2,
        height: 2,
        hotspot_x: 2,
        hotspot_y: 0,
        rgba: vec![0; 16],
    };
    assert!(matches!(
        controller.validate_cursor_snapshot(&invalid),
        Err(InputControllerError::InvalidCursor(_))
    ));
}

#[test]
fn input_queue_rejects_overflow_without_unbounded_growth() {
    let mut controller = InputController::with_limits(monitor(), 10, 1).unwrap();
    controller.set_focused(true);
    controller
        .handle_window_event(WindowEvent::Input(InputEvent::Text("a".into())))
        .unwrap();
    assert_eq!(
        controller.handle_window_event(WindowEvent::Input(InputEvent::Text("b".into()))),
        Err(InputControllerError::QueueFull)
    );
}

#[test]
fn input_queue_rejects_unbounded_capacity_before_allocation() {
    assert_eq!(
        InputController::with_limits(monitor(), 10, usize::MAX).unwrap_err(),
        InputControllerError::InvalidQueueCapacity
    );
}

#[test]
fn input_rate_limit_is_capped_before_timestamp_storage() {
    assert_eq!(
        InputController::with_limits(monitor(), MAX_INPUT_RATE_PER_SECOND + 1, 1).unwrap_err(),
        InputControllerError::InvalidRateLimit
    );
}

#[test]
fn window_rejects_render_payload_before_enqueueing_it() {
    let config = WindowConfig {
        max_render_payload: 1,
        ..WindowConfig::default()
    };
    let window = WindowController::start(config).unwrap();
    let result = window.try_send(WindowCommand::Render(DecodedFrameJob {
        frame_id: 1,
        timestamp_us: 0,
        keyframe: true,
        access_unit: vec![1, 2],
    }));
    assert_eq!(result, Err(WindowError::RenderPayloadTooLarge));
    drop(window);
}

#[test]
fn window_forwards_render_commands_to_task_three_queue() {
    let window = WindowController::start(WindowConfig::default()).unwrap();
    window
        .try_send(WindowCommand::Render(DecodedFrameJob {
            frame_id: 9,
            timestamp_us: 10,
            keyframe: true,
            access_unit: vec![7],
        }))
        .unwrap();
    let queue = window.render_queue();
    for _ in 0..20 {
        if queue.take_latest().is_some() {
            return;
        }
        thread::sleep(Duration::from_millis(2));
    }
    panic!("window thread did not forward render command");
}
