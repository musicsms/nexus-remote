use nexus_input::{InputEvent, KeyAction, MouseButton};

use crate::{BackendErrorKind, BackendResult};

/// Inclusive virtual-desktop bounds used to map mouse positions to Windows absolute coordinates.
///
/// Both minimum and maximum endpoints are included because the native adapter
/// uses `MOUSEEVENTF_VIRTUALDESK` absolute coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MonitorBounds {
    pub min_x: i32,
    pub min_y: i32,
    pub max_x: i32,
    pub max_y: i32,
}

/// A platform-neutral record ready for the narrow native input boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputRecord {
    ScanCode {
        scan_code: u16,
        extended: bool,
        key_up: bool,
    },
    Unicode {
        code_unit: u16,
        key_up: bool,
    },
    MouseMove {
        absolute_x: u16,
        absolute_y: u16,
    },
    MouseButton {
        button: MouseButton,
        pressed: bool,
    },
    MouseWheel {
        delta_x: i32,
        delta_y: i32,
    },
}

/// The narrow native boundary used by deterministic input contract tests.
pub trait NativeInputApi {
    fn send(&mut self, records: &[InputRecord]) -> BackendResult<usize>;
}

/// Validates and translates semantic input before submitting it to a native API.
pub struct InputInjector<A> {
    native: A,
    monitor: MonitorBounds,
}

impl<A> InputInjector<A>
where
    A: NativeInputApi,
{
    pub(crate) const fn new(native: A, monitor: MonitorBounds) -> Self {
        Self { native, monitor }
    }

    pub(crate) fn inject_records(&mut self, event: &InputEvent) -> BackendResult<usize> {
        event
            .validate()
            .map_err(|_| BackendErrorKind::InvalidInput)?;

        let records = translate(event, self.monitor)?;
        let submitted = self.native.send(&records)?;
        if submitted != records.len() {
            return Err(BackendErrorKind::NativeFailure.into());
        }
        Ok(submitted)
    }

    #[cfg(test)]
    pub(crate) fn into_inner(self) -> A {
        self.native
    }
}

fn translate(event: &InputEvent, monitor: MonitorBounds) -> BackendResult<Vec<InputRecord>> {
    match event {
        InputEvent::Key {
            physical_code,
            action,
            ..
        } => {
            let scan_code = map_hid_usage(*physical_code)?;
            Ok(vec![InputRecord::ScanCode {
                scan_code: scan_code.value,
                extended: scan_code.extended,
                key_up: matches!(action, KeyAction::Up),
            }])
        }
        InputEvent::Text(text) => Ok(text
            .encode_utf16()
            .flat_map(|code_unit| {
                [
                    InputRecord::Unicode {
                        code_unit,
                        key_up: false,
                    },
                    InputRecord::Unicode {
                        code_unit,
                        key_up: true,
                    },
                ]
            })
            .collect()),
        InputEvent::MouseMove { x, y } => Ok(vec![InputRecord::MouseMove {
            absolute_x: normalize_coordinate(*x, monitor.min_x, monitor.max_x)?,
            absolute_y: normalize_coordinate(*y, monitor.min_y, monitor.max_y)?,
        }]),
        InputEvent::MouseButton { button, pressed } => Ok(vec![InputRecord::MouseButton {
            button: *button,
            pressed: *pressed,
        }]),
        InputEvent::MouseWheel { delta_x, delta_y } => {
            let mut records = Vec::with_capacity(2);
            if *delta_y != 0 {
                records.push(InputRecord::MouseWheel {
                    delta_x: 0,
                    delta_y: *delta_y,
                });
            }
            if *delta_x != 0 {
                records.push(InputRecord::MouseWheel {
                    delta_x: *delta_x,
                    delta_y: 0,
                });
            }
            Ok(records)
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct WindowsScanCode {
    value: u16,
    extended: bool,
}

const fn scan_code(value: u16) -> WindowsScanCode {
    WindowsScanCode {
        value,
        extended: false,
    }
}

const fn extended_scan_code(value: u16) -> WindowsScanCode {
    WindowsScanCode {
        value,
        extended: true,
    }
}

fn map_hid_usage(usage: u32) -> BackendResult<WindowsScanCode> {
    let scan_code = match usage {
        0x04 => scan_code(0x1e),
        0x05 => scan_code(0x30),
        0x06 => scan_code(0x2e),
        0x07 => scan_code(0x20),
        0x08 => scan_code(0x12),
        0x09 => scan_code(0x21),
        0x0a => scan_code(0x22),
        0x0b => scan_code(0x23),
        0x0c => scan_code(0x17),
        0x0d => scan_code(0x24),
        0x0e => scan_code(0x25),
        0x0f => scan_code(0x26),
        0x10 => scan_code(0x32),
        0x11 => scan_code(0x31),
        0x12 => scan_code(0x18),
        0x13 => scan_code(0x19),
        0x14 => scan_code(0x10),
        0x15 => scan_code(0x13),
        0x16 => scan_code(0x1f),
        0x17 => scan_code(0x14),
        0x18 => scan_code(0x16),
        0x19 => scan_code(0x2f),
        0x1a => scan_code(0x11),
        0x1b => scan_code(0x2d),
        0x1c => scan_code(0x15),
        0x1d => scan_code(0x2c),
        0x1e..=0x27 => scan_code((usage - 0x1e + 0x02) as u16),
        0x28 => scan_code(0x1c),
        0x29 => scan_code(0x01),
        0x2a => scan_code(0x0e),
        0x2b => scan_code(0x0f),
        0x2c => scan_code(0x39),
        0x2d => scan_code(0x0c),
        0x2e => scan_code(0x0d),
        0x2f => scan_code(0x1a),
        0x30 => scan_code(0x1b),
        0x31 => scan_code(0x2b),
        0x32 => scan_code(0x2b),
        0x33 => scan_code(0x27),
        0x34 => scan_code(0x28),
        0x35 => scan_code(0x29),
        0x36 => scan_code(0x33),
        0x37 => scan_code(0x34),
        0x38 => scan_code(0x35),
        0x39 => scan_code(0x3a),
        0x3a..=0x43 => scan_code((usage - 0x3a + 0x3b) as u16),
        0x44 => scan_code(0x57),
        0x45 => scan_code(0x58),
        0x47 => scan_code(0x46),
        0x49 => extended_scan_code(0x52),
        0x4a => extended_scan_code(0x47),
        0x4b => extended_scan_code(0x49),
        0x4c => extended_scan_code(0x53),
        0x4d => extended_scan_code(0x4f),
        0x4e => extended_scan_code(0x51),
        0x4f => extended_scan_code(0x4d),
        0x50 => extended_scan_code(0x4b),
        0x51 => extended_scan_code(0x50),
        0x52 => extended_scan_code(0x48),
        0x53 => scan_code(0x45),
        0x54 => extended_scan_code(0x35),
        0x55 => scan_code(0x37),
        0x56 => scan_code(0x4a),
        0x57 => scan_code(0x4e),
        0x58 => extended_scan_code(0x1c),
        0x59 => scan_code(0x4f),
        0x5a => scan_code(0x50),
        0x5b => scan_code(0x51),
        0x5c => scan_code(0x4b),
        0x5d => scan_code(0x4c),
        0x5e => scan_code(0x4d),
        0x5f => scan_code(0x47),
        0x60 => scan_code(0x48),
        0x61 => scan_code(0x49),
        0x62 => scan_code(0x52),
        0x63 => scan_code(0x53),
        0x64 => scan_code(0x56),
        0x65 => extended_scan_code(0x5d),
        0xe0 => scan_code(0x1d),
        0xe1 => scan_code(0x2a),
        0xe2 => scan_code(0x38),
        0xe3 => extended_scan_code(0x5b),
        0xe4 => extended_scan_code(0x1d),
        0xe5 => scan_code(0x36),
        0xe6 => extended_scan_code(0x38),
        0xe7 => extended_scan_code(0x5c),
        _ => return Err(BackendErrorKind::InvalidInput.into()),
    };
    Ok(scan_code)
}

fn normalize_coordinate(value: i32, minimum: i32, maximum: i32) -> BackendResult<u16> {
    let range = (maximum as i64)
        .checked_sub(minimum as i64)
        .ok_or(BackendErrorKind::InvalidInput)?;
    if range <= 0 {
        return Err(BackendErrorKind::InvalidInput.into());
    }

    let offset = (value as i64)
        .checked_sub(minimum as i64)
        .ok_or(BackendErrorKind::InvalidInput)?;
    let scaled = offset
        .checked_mul(65_535)
        .ok_or(BackendErrorKind::InvalidInput)?
        / range;

    Ok(scaled.clamp(0, 65_535) as u16)
}

/// The production adapter, which calls `SendInput` only on Windows.
#[cfg(windows)]
pub struct SystemInputApi(native::WindowsSendInputApi);

#[cfg(not(windows))]
pub struct SystemInputApi;

impl SystemInputApi {
    fn new() -> Self {
        #[cfg(windows)]
        {
            Self(native::WindowsSendInputApi)
        }

        #[cfg(not(windows))]
        {
            Self
        }
    }
}

impl InputInjector<SystemInputApi> {
    pub fn system(monitor: MonitorBounds) -> Self {
        #[cfg(windows)]
        {
            Self::new(SystemInputApi::new(), monitor)
        }

        #[cfg(not(windows))]
        {
            Self::new(SystemInputApi::new(), monitor)
        }
    }

    /// Injects a validated semantic event through the system input adapter.
    pub fn inject(&mut self, event: &InputEvent) -> BackendResult<usize> {
        self.inject_records(event)
    }
}

#[cfg(not(windows))]
impl NativeInputApi for SystemInputApi {
    fn send(&mut self, _records: &[InputRecord]) -> BackendResult<usize> {
        Err(BackendErrorKind::UnsupportedPlatform.into())
    }
}

#[cfg(windows)]
impl NativeInputApi for SystemInputApi {
    fn send(&mut self, records: &[InputRecord]) -> BackendResult<usize> {
        self.0.send(records)
    }
}

#[cfg(windows)]
mod native {
    use std::mem::size_of;

    use nexus_input::MouseButton;
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT, KEYBD_EVENT_FLAGS,
        KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, KEYEVENTF_SCANCODE, KEYEVENTF_UNICODE,
        MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_HWHEEL, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP,
        MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP, MOUSEEVENTF_MOVE, MOUSEEVENTF_RIGHTDOWN,
        MOUSEEVENTF_RIGHTUP, MOUSEEVENTF_VIRTUALDESK, MOUSEEVENTF_WHEEL, MOUSEEVENTF_XDOWN,
        MOUSEEVENTF_XUP, MOUSEINPUT, MOUSE_EVENT_FLAGS, VIRTUAL_KEY,
    };

    use super::{InputRecord, NativeInputApi};
    use crate::{BackendErrorKind, BackendResult};

    pub(super) struct WindowsSendInputApi;

    impl NativeInputApi for WindowsSendInputApi {
        fn send(&mut self, records: &[InputRecord]) -> BackendResult<usize> {
            let inputs: Vec<INPUT> = records.iter().copied().map(to_windows_input).collect();
            let input_size = size_of::<INPUT>() as i32;
            // SAFETY: `inputs` is fully initialized and remains valid for this synchronous call.
            let submitted = unsafe { SendInput(&inputs, input_size) } as usize;
            if submitted != inputs.len() {
                return Err(BackendErrorKind::NativeFailure.into());
            }
            Ok(submitted)
        }
    }

    fn to_windows_input(record: InputRecord) -> INPUT {
        match record {
            InputRecord::ScanCode {
                scan_code,
                extended,
                key_up,
            } => keyboard_input(
                scan_code,
                KEYEVENTF_SCANCODE
                    | if extended {
                        KEYEVENTF_EXTENDEDKEY
                    } else {
                        KEYBD_EVENT_FLAGS(0)
                    }
                    | if key_up {
                        KEYEVENTF_KEYUP
                    } else {
                        KEYBD_EVENT_FLAGS(0)
                    },
            ),
            InputRecord::Unicode { code_unit, key_up } => keyboard_input(
                code_unit,
                KEYEVENTF_UNICODE
                    | if key_up {
                        KEYEVENTF_KEYUP
                    } else {
                        KEYBD_EVENT_FLAGS(0)
                    },
            ),
            InputRecord::MouseMove {
                absolute_x,
                absolute_y,
            } => mouse_input(
                absolute_x as i32,
                absolute_y as i32,
                0,
                MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK,
            ),
            InputRecord::MouseButton { button, pressed } => {
                let (flags, mouse_data) = mouse_button_flags(button, pressed);
                mouse_input(0, 0, mouse_data, flags)
            }
            InputRecord::MouseWheel {
                delta_x: _,
                delta_y,
            } if delta_y != 0 => mouse_input(0, 0, delta_y as u32, MOUSEEVENTF_WHEEL),
            InputRecord::MouseWheel { delta_x, .. } => {
                mouse_input(0, 0, delta_x as u32, MOUSEEVENTF_HWHEEL)
            }
        }
    }

    fn keyboard_input(scan_code: u16, flags: KEYBD_EVENT_FLAGS) -> INPUT {
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: VIRTUAL_KEY(0),
                    wScan: scan_code,
                    dwFlags: flags,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        }
    }

    fn mouse_input(dx: i32, dy: i32, mouse_data: u32, flags: MOUSE_EVENT_FLAGS) -> INPUT {
        INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: INPUT_0 {
                mi: MOUSEINPUT {
                    dx,
                    dy,
                    mouseData: mouse_data,
                    dwFlags: flags,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        }
    }

    fn mouse_button_flags(button: MouseButton, pressed: bool) -> (MOUSE_EVENT_FLAGS, u32) {
        match (button, pressed) {
            (MouseButton::Left, true) => (MOUSEEVENTF_LEFTDOWN, 0),
            (MouseButton::Left, false) => (MOUSEEVENTF_LEFTUP, 0),
            (MouseButton::Middle, true) => (MOUSEEVENTF_MIDDLEDOWN, 0),
            (MouseButton::Middle, false) => (MOUSEEVENTF_MIDDLEUP, 0),
            (MouseButton::Right, true) => (MOUSEEVENTF_RIGHTDOWN, 0),
            (MouseButton::Right, false) => (MOUSEEVENTF_RIGHTUP, 0),
            (MouseButton::Back, true) => (MOUSEEVENTF_XDOWN, 1),
            (MouseButton::Back, false) => (MOUSEEVENTF_XUP, 1),
            (MouseButton::Forward, true) => (MOUSEEVENTF_XDOWN, 2),
            (MouseButton::Forward, false) => (MOUSEEVENTF_XUP, 2),
        }
    }
}

#[cfg(test)]
mod tests {
    use nexus_input::{InputEvent, KeyAction, Modifiers};

    use super::{InputInjector, InputRecord, MonitorBounds, NativeInputApi};
    use crate::{BackendErrorKind, BackendResult};

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

    struct PartialNativeInputApi;

    impl NativeInputApi for PartialNativeInputApi {
        fn send(&mut self, records: &[InputRecord]) -> BackendResult<usize> {
            Ok(records.len().saturating_sub(1))
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

    fn key(physical_code: u32) -> InputEvent {
        InputEvent::Key {
            physical_code,
            logical_code: 0,
            action: KeyAction::Down,
            modifiers: Modifiers::NONE,
        }
    }

    #[test]
    fn translates_hid_key_to_windows_scan_code() {
        let mut injector = injector();

        assert_eq!(injector.inject_records(&key(0x1e)), Ok(1));
        assert_eq!(
            injector.into_inner().records,
            vec![InputRecord::ScanCode {
                scan_code: 0x02,
                extended: false,
                key_up: false,
            }]
        );
    }

    #[test]
    fn translates_hid_usage_0x32_to_windows_scan_code_0x2b() {
        let mut injector = injector();

        assert_eq!(injector.inject_records(&key(0x32)), Ok(1));
        assert_eq!(
            injector.into_inner().records,
            vec![InputRecord::ScanCode {
                scan_code: 0x2b,
                extended: false,
                key_up: false,
            }]
        );
    }

    #[test]
    fn translates_hid_usage_0x64_to_windows_scan_code_0x56() {
        let mut injector = injector();

        assert_eq!(injector.inject_records(&key(0x64)), Ok(1));
        assert_eq!(
            injector.into_inner().records,
            vec![InputRecord::ScanCode {
                scan_code: 0x56,
                extended: false,
                key_up: false,
            }]
        );
    }

    #[test]
    fn translates_extended_hid_key_with_extended_metadata() {
        let mut injector = injector();

        assert_eq!(injector.inject_records(&key(0x4f)), Ok(1));
        assert_eq!(
            injector.into_inner().records,
            vec![InputRecord::ScanCode {
                scan_code: 0x4d,
                extended: true,
                key_up: false,
            }]
        );
    }

    #[test]
    fn rejects_unsupported_hid_key() {
        assert_eq!(
            injector().inject_records(&key(0x48)).unwrap_err().kind(),
            BackendErrorKind::InvalidInput
        );
    }

    #[test]
    fn translates_text_to_utf16_down_up_pair() {
        let mut injector = injector();

        assert_eq!(
            injector.inject_records(&InputEvent::Text("é".into())),
            Ok(2)
        );
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
    fn normalizes_inclusive_monitor_endpoints_to_windows_absolute_coordinates() {
        let mut injector = injector();

        assert_eq!(
            injector.inject_records(&InputEvent::MouseMove { x: 100, y: 600 }),
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
    fn clamps_mouse_coordinates_outside_inclusive_monitor_bounds() {
        let mut injector = injector();

        assert_eq!(
            injector.inject_records(&InputEvent::MouseMove { x: 50, y: 700 }),
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
    fn normalizes_extreme_i32_coordinates_without_overflow() {
        let mut injector = InputInjector::new(
            RecordingNativeInputApi::default(),
            MonitorBounds {
                min_x: i32::MIN,
                min_y: i32::MIN,
                max_x: i32::MAX,
                max_y: i32::MAX,
            },
        );

        assert_eq!(
            injector.inject_records(&InputEvent::MouseMove {
                x: i32::MAX,
                y: i32::MIN,
            }),
            Ok(1)
        );
        assert_eq!(
            injector.into_inner().records,
            vec![InputRecord::MouseMove {
                absolute_x: 65_535,
                absolute_y: 0,
            }]
        );
    }

    #[test]
    fn rejects_invalid_monitor_bounds_before_native_submission() {
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
                .inject_records(&InputEvent::MouseMove { x: 10, y: 50 })
                .unwrap_err()
                .kind(),
            BackendErrorKind::InvalidInput
        );
        assert!(injector.into_inner().records.is_empty());
    }

    #[test]
    fn reports_partial_native_submission_as_a_failure() {
        let mut injector = InputInjector::new(
            PartialNativeInputApi,
            MonitorBounds {
                min_x: 0,
                min_y: 0,
                max_x: 1,
                max_y: 1,
            },
        );

        assert_eq!(
            injector.inject_records(&key(0x04)).unwrap_err().kind(),
            BackendErrorKind::NativeFailure
        );
    }

    #[test]
    fn translates_each_nonzero_wheel_axis_without_dropping_a_delta() {
        let mut injector = injector();

        assert_eq!(
            injector.inject_records(&InputEvent::MouseWheel {
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
    fn system_input_adapter_fails_closed_off_windows_for_empty_batches() {
        let monitor = MonitorBounds {
            min_x: 0,
            min_y: 0,
            max_x: 1,
            max_y: 1,
        };

        for event in [
            InputEvent::Text(String::new()),
            InputEvent::MouseWheel {
                delta_x: 0,
                delta_y: 0,
            },
            InputEvent::MouseMove { x: 0, y: 0 },
        ] {
            assert_eq!(
                InputInjector::system(monitor)
                    .inject(&event)
                    .unwrap_err()
                    .kind(),
                BackendErrorKind::UnsupportedPlatform
            );
        }
    }
}
