use nexus_input::{InputEvent, KeyAction, Modifiers};
use platform_windows::{
    BackendErrorKind, BackendResult, CursorSnapshot, InputInjector, InputRecord, MonitorBounds,
    NativeInputApi,
};

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

#[derive(Default)]
struct RecordingNativeInputApi {
    records: Vec<InputRecord>,
}

impl NativeInputApi for RecordingNativeInputApi {
    fn send(&mut self, records: &[InputRecord]) -> BackendResult<usize> {
        self.records.extend_from_slice(records);
        Ok(records.len())
    }
}

fn injector() -> InputInjector<RecordingNativeInputApi> {
    InputInjector::new(
        RecordingNativeInputApi::default(),
        MonitorBounds {
            min_x: 100,
            min_y: 200,
            max_x: 300,
            max_y: 600,
        },
    )
}

#[test]
fn input_translates_physical_key_down_to_a_scan_code_record() {
    let event = InputEvent::Key {
        physical_code: 0x1e,
        logical_code: 0,
        action: KeyAction::Down,
        modifiers: Modifiers::NONE,
    };
    let mut injector = injector();

    assert_eq!(injector.inject(&event), Ok(1));
    assert_eq!(
        injector.into_inner().records,
        vec![InputRecord::ScanCode {
            scan_code: 0x1e,
            key_up: false,
        }]
    );
}

#[test]
fn input_translates_text_to_utf16_down_up_pair() {
    let mut injector = injector();

    assert_eq!(injector.inject(&InputEvent::Text("é".into())), Ok(2));
    assert_eq!(
        injector.into_inner().records,
        vec![
            InputRecord::Unicode {
                code_unit: 0x00e9,
                key_up: false,
            },
            InputRecord::Unicode {
                code_unit: 0x00e9,
                key_up: true,
            },
        ]
    );
}

#[test]
fn input_normalizes_monitor_endpoints_to_windows_absolute_coordinates() {
    let mut injector = injector();

    assert_eq!(
        injector.inject(&InputEvent::MouseMove { x: 100, y: 600 }),
        Ok(1)
    );
    assert_eq!(
        injector.into_inner().records,
        vec![InputRecord::MouseMove {
            absolute_x: 0,
            absolute_y: 65_535,
        }]
    );
}

#[test]
fn input_rejects_invalid_monitor_bounds_before_native_submission() {
    let mut injector = InputInjector::new(
        RecordingNativeInputApi::default(),
        MonitorBounds {
            min_x: 10,
            min_y: 0,
            max_x: 10,
            max_y: 100,
        },
    );

    assert_eq!(
        injector
            .inject(&InputEvent::MouseMove { x: 10, y: 50 })
            .unwrap_err()
            .kind(),
        BackendErrorKind::InvalidInput
    );
    assert!(injector.into_inner().records.is_empty());
}

#[test]
fn input_translates_each_nonzero_wheel_axis_without_dropping_a_delta() {
    let mut injector = injector();

    assert_eq!(
        injector.inject(&InputEvent::MouseWheel {
            delta_x: 120,
            delta_y: -120,
        }),
        Ok(2)
    );
    assert_eq!(
        injector.into_inner().records,
        vec![
            InputRecord::MouseWheel {
                delta_x: 0,
                delta_y: -120,
            },
            InputRecord::MouseWheel {
                delta_x: 120,
                delta_y: 0,
            },
        ]
    );
}

#[cfg(not(windows))]
#[test]
fn system_input_adapter_fails_closed_off_windows() {
    let mut injector = InputInjector::system(MonitorBounds {
        min_x: 0,
        min_y: 0,
        max_x: 1,
        max_y: 1,
    });

    assert_eq!(
        injector
            .inject(&InputEvent::MouseMove { x: 0, y: 0 })
            .unwrap_err()
            .kind(),
        BackendErrorKind::UnsupportedPlatform
    );
}
