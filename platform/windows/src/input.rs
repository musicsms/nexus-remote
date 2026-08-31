use nexus_input::{InputEvent, KeyAction, MouseButton};

use crate::{BackendErrorKind, BackendResult};

/// Desktop bounds used to map mouse positions to Windows absolute coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MonitorBounds {
    pub min_x: i32,
    pub min_y: i32,
    pub max_x: i32,
    pub max_y: i32,
}

/// A platform-neutral record ready for the narrow native input boundary.
#[doc(hidden)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputRecord {
    ScanCode { scan_code: u16, key_up: bool },
    Unicode { code_unit: u16, key_up: bool },
    MouseMove { absolute_x: u16, absolute_y: u16 },
    MouseButton { button: MouseButton, pressed: bool },
    MouseWheel { delta_x: i32, delta_y: i32 },
}

/// The narrow native boundary used by deterministic input contract tests.
#[doc(hidden)]
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
    pub const fn new(native: A, monitor: MonitorBounds) -> Self {
        Self { native, monitor }
    }

    pub fn inject(&mut self, event: &InputEvent) -> BackendResult<usize> {
        event
            .validate()
            .map_err(|_| BackendErrorKind::InvalidInput)?;

        let records = translate(event, self.monitor)?;
        if records.is_empty() {
            return Ok(0);
        }

        let submitted = self.native.send(&records)?;
        if submitted != records.len() {
            return Err(BackendErrorKind::NativeFailure.into());
        }
        Ok(submitted)
    }

    pub fn into_inner(self) -> A {
        self.native
    }
}

fn translate(event: &InputEvent, monitor: MonitorBounds) -> BackendResult<Vec<InputRecord>> {
    match event {
        InputEvent::Key {
            physical_code,
            action,
            ..
        } => Ok(vec![InputRecord::ScanCode {
            scan_code: *physical_code as u16,
            key_up: matches!(action, KeyAction::Up),
        }]),
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

#[cfg(windows)]
impl Default for SystemInputApi {
    fn default() -> Self {
        Self(native::WindowsSendInputApi)
    }
}

impl InputInjector<SystemInputApi> {
    pub fn system(monitor: MonitorBounds) -> Self {
        #[cfg(windows)]
        {
            Self::new(SystemInputApi::default(), monitor)
        }

        #[cfg(not(windows))]
        {
            Self::new(SystemInputApi, monitor)
        }
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
        KEYEVENTF_KEYUP, KEYEVENTF_SCANCODE, KEYEVENTF_UNICODE, MOUSEEVENTF_ABSOLUTE,
        MOUSEEVENTF_HWHEEL, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MIDDLEDOWN,
        MOUSEEVENTF_MIDDLEUP, MOUSEEVENTF_MOVE, MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP,
        MOUSEEVENTF_VIRTUALDESK, MOUSEEVENTF_WHEEL, MOUSEEVENTF_XDOWN, MOUSEEVENTF_XUP, MOUSEINPUT,
        MOUSE_EVENT_FLAGS, VIRTUAL_KEY,
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
            InputRecord::ScanCode { scan_code, key_up } => keyboard_input(
                scan_code,
                KEYEVENTF_SCANCODE
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
