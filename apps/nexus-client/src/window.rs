//! Dedicated native-window ownership and bounded portable event boundary.

use crate::{DecodedFrameJob, RenderQueue};
use nexus_input::InputEvent;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use thiserror::Error;

pub const DEFAULT_WINDOW_CHANNEL_CAPACITY: usize = 16;
const CONTROL_EVENT_CAPACITY: usize = 4;
const MAX_WINDOW_TITLE_BYTES: usize = 256;

#[derive(Debug, Clone)]
pub struct WindowConfig {
    pub title: String,
    pub width: u32,
    pub height: u32,
    pub channel_capacity: usize,
    pub max_render_payload: usize,
}

impl Default for WindowConfig {
    fn default() -> Self {
        Self {
            title: "Nexus Remote Desktop".to_owned(),
            width: 1280,
            height: 720,
            channel_capacity: DEFAULT_WINDOW_CHANNEL_CAPACITY,
            max_render_payload: crate::renderer::MAX_RENDER_ACCESS_UNIT_SIZE,
        }
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum WindowError {
    #[error("window dimensions are invalid")]
    InvalidDimensions,
    #[error("window channel capacity is invalid")]
    InvalidChannelCapacity,
    #[error("window title is too long")]
    TitleTooLong,
    #[error("render payload is too large")]
    RenderPayloadTooLarge,
    #[error("window thread could not be started")]
    ThreadStart,
    #[error("window command queue is closed")]
    CommandClosed,
    #[error("window command queue is full")]
    CommandFull,
    #[error("window shutdown exceeded its deadline")]
    ShutdownTimeout,
    #[error("window startup timed out")]
    StartupTimeout,
}

#[derive(Debug)]
pub enum WindowCommand {
    Render(DecodedFrameJob),
    SetTitle(String),
    RequestFocus,
    Close,
    Shutdown,
}

/// Validating clone of the command endpoint. The raw `SyncSender` never
/// leaves this module, so callers cannot bypass payload/title bounds.
#[derive(Clone)]
pub struct WindowCommandSender {
    sender: SyncSender<WindowCommand>,
    max_render_payload: usize,
}

impl WindowCommandSender {
    pub fn try_send(&self, command: WindowCommand) -> Result<(), WindowError> {
        validate_command(&command, self.max_render_payload)?;
        self.sender.try_send(command).map_err(|error| match error {
            TrySendError::Full(_) => WindowError::CommandFull,
            TrySendError::Disconnected(_) => WindowError::CommandClosed,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WindowEvent {
    FocusChanged(bool),
    Focused(bool),
    Resized { width: u32, height: u32 },
    Closed,
    Input(InputEvent),
    RenderRequested,
    RenderRejected,
}

#[derive(Clone)]
struct EventSinks {
    control: Arc<ControlEventQueue>,
    normal: SyncSender<WindowEvent>,
}

struct EventReceivers {
    control: Arc<ControlEventQueue>,
    normal: Receiver<WindowEvent>,
}

struct ControlEventQueue {
    events: Mutex<VecDeque<WindowEvent>>,
}

impl ControlEventQueue {
    fn new() -> Self {
        Self {
            events: Mutex::new(VecDeque::with_capacity(CONTROL_EVENT_CAPACITY)),
        }
    }

    fn push(&self, event: WindowEvent) {
        let Ok(mut events) = self.events.lock() else {
            return;
        };
        let is_closed = matches!(&event, WindowEvent::Closed);
        let is_focus = matches!(
            &event,
            WindowEvent::FocusChanged(_) | WindowEvent::Focused(_)
        );
        if is_focus {
            events.retain(|existing| {
                !matches!(
                    existing,
                    WindowEvent::FocusChanged(_) | WindowEvent::Focused(_)
                )
            });
        }
        if events.len() >= CONTROL_EVENT_CAPACITY {
            if let Some(index) = events.iter().position(|existing| {
                matches!(
                    existing,
                    WindowEvent::FocusChanged(_) | WindowEvent::Focused(_)
                )
            }) {
                events.remove(index);
            } else if is_focus || is_closed {
                // Lifecycle controls always get a reserved slot. A render
                // diagnostic is disposable; an existing close is coalesced.
                if is_closed
                    && events
                        .iter()
                        .any(|existing| matches!(existing, WindowEvent::Closed))
                {
                    return;
                }
                if let Some(index) = events
                    .iter()
                    .position(|existing| !matches!(existing, WindowEvent::Closed))
                {
                    events.remove(index);
                } else {
                    return;
                }
            } else {
                return;
            }
        }
        events.push_back(event);
    }

    fn pop(&self) -> Option<WindowEvent> {
        self.events.lock().ok()?.pop_front()
    }
}

/// A command/event pair backed by bounded standard-library channels.
pub struct WindowController {
    commands: SyncSender<WindowCommand>,
    events: Arc<Mutex<EventReceivers>>,
    worker: Option<JoinHandle<()>>,
    render_queue: RenderQueue,
    max_render_payload: usize,
    native_handle: Arc<AtomicIsize>,
    shutdown_requested: Arc<std::sync::atomic::AtomicBool>,
}

impl WindowController {
    pub fn start(config: WindowConfig) -> Result<Self, WindowError> {
        validate_config(&config)?;
        let max_render_payload = config.max_render_payload;
        let (commands, command_rx) = mpsc::sync_channel(config.channel_capacity);
        let control = Arc::new(ControlEventQueue::new());
        let (normal_tx, normal_rx) = mpsc::sync_channel(config.channel_capacity);
        let (started_tx, started_rx) = mpsc::sync_channel(1);
        let render_queue = RenderQueue::new();
        let native_handle = Arc::new(AtomicIsize::new(0));
        let shutdown_requested = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let worker_render_queue = render_queue.clone();
        let worker_native_handle = Arc::clone(&native_handle);
        let worker_control = Arc::clone(&control);
        let worker_shutdown_requested = Arc::clone(&shutdown_requested);
        let worker = thread::Builder::new()
            .name("nexus-client-window".to_owned())
            .spawn(move || {
                window_thread(
                    config,
                    command_rx,
                    EventSinks {
                        control: worker_control,
                        normal: normal_tx,
                    },
                    worker_render_queue,
                    worker_native_handle,
                    worker_shutdown_requested,
                    started_tx,
                )
            })
            .map_err(|_| WindowError::ThreadStart)?;
        match started_rx.recv_timeout(Duration::from_secs(2)) {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                let _ = worker.join();
                return Err(error);
            }
            Err(_) => {
                // The native thread owns all handles. Keep the JoinHandle in
                // an explicit reaper instead of silently detaching it.
                shutdown_requested.store(true, Ordering::Release);
                reap_window_worker(worker);
                return Err(WindowError::StartupTimeout);
            }
        }
        Ok(Self {
            commands,
            events: Arc::new(Mutex::new(EventReceivers {
                control,
                normal: normal_rx,
            })),
            worker: Some(worker),
            render_queue,
            max_render_payload,
            native_handle,
            shutdown_requested,
        })
    }

    pub fn try_send(&self, command: WindowCommand) -> Result<(), WindowError> {
        self.command_sender().try_send(command)
    }

    pub fn command_sender(&self) -> WindowCommandSender {
        WindowCommandSender {
            sender: self.commands.clone(),
            max_render_payload: self.max_render_payload,
        }
    }

    pub fn try_next_event(&self) -> Option<WindowEvent> {
        let receiver = self.events.lock().ok()?;
        receiver
            .control
            .pop()
            .or_else(|| receiver.normal.try_recv().ok())
    }

    pub fn render_queue(&self) -> RenderQueue {
        self.render_queue.clone()
    }

    /// Replaces the pending render job directly in the depth-one handoff.
    /// Network receive paths must use this instead of the FIFO command queue,
    /// so a saturated window thread cannot retain stale frames.
    pub fn render_latest(&self, frame: DecodedFrameJob) -> Result<(), WindowError> {
        validate_command(
            &WindowCommand::Render(frame.clone()),
            self.max_render_payload,
        )?;
        self.render_queue
            .push_latest(frame)
            .map_err(|error| match error {
                crate::renderer::RenderQueueError::AccessUnitTooLarge { .. } => {
                    WindowError::RenderPayloadTooLarge
                }
                crate::renderer::RenderQueueError::Shutdown => WindowError::CommandClosed,
                crate::renderer::RenderQueueError::EmptyAccessUnit
                | crate::renderer::RenderQueueError::StateUnavailable => {
                    WindowError::RenderPayloadTooLarge
                }
            })
    }

    /// Returns the native window as an opaque integer for cfg(windows) smoke
    /// tooling. Core code never imports or stores an HWND type.
    pub fn native_handle(&self) -> Option<isize> {
        let handle = self.native_handle.load(Ordering::Acquire);
        (handle != 0).then_some(handle)
    }

    pub fn shutdown(&mut self, deadline: Instant) -> Result<(), WindowError> {
        self.shutdown_requested.store(true, Ordering::Release);
        let _ = self.try_send(WindowCommand::Shutdown);
        let Some(worker) = self.worker.take() else {
            return Ok(());
        };
        while !worker.is_finished() {
            if Instant::now() >= deadline {
                reap_window_worker(worker);
                return Err(WindowError::ShutdownTimeout);
            }
            thread::sleep(Duration::from_millis(1));
        }
        worker.join().map_err(|_| WindowError::ThreadStart)
    }
}

fn reap_window_worker(worker: JoinHandle<()>) {
    let slot = Arc::new(Mutex::new(Some(worker)));
    let reaper_slot = Arc::clone(&slot);
    if thread::Builder::new()
        .name("nexus-client-window-reaper".to_owned())
        .spawn(move || {
            if let Some(worker) = reaper_slot.lock().ok().and_then(|mut slot| slot.take()) {
                let _ = worker.join();
            }
        })
        .is_err()
    {
        // Thread creation failure is exceptional; retain the join guarantee
        // by joining synchronously instead of dropping the handle.
        if let Some(worker) = slot.lock().ok().and_then(|mut slot| slot.take()) {
            let _ = worker.join();
        }
    }
}

impl Drop for WindowController {
    fn drop(&mut self) {
        let _ = self.shutdown(Instant::now() + Duration::from_millis(250));
    }
}

fn validate_config(config: &WindowConfig) -> Result<(), WindowError> {
    if config.width == 0 || config.height == 0 || config.width > 16_384 || config.height > 16_384 {
        return Err(WindowError::InvalidDimensions);
    }
    if config.channel_capacity == 0 || config.channel_capacity > 1024 {
        return Err(WindowError::InvalidChannelCapacity);
    }
    if config.title.len() > 256 {
        return Err(WindowError::TitleTooLong);
    }
    if config.max_render_payload == 0
        || config.max_render_payload > crate::renderer::MAX_RENDER_ACCESS_UNIT_SIZE
    {
        return Err(WindowError::RenderPayloadTooLarge);
    }
    Ok(())
}

fn validate_command(command: &WindowCommand, max_render_payload: usize) -> Result<(), WindowError> {
    match command {
        WindowCommand::SetTitle(title) if title.len() > MAX_WINDOW_TITLE_BYTES => {
            Err(WindowError::TitleTooLong)
        }
        WindowCommand::Render(frame) if frame.access_unit.len() > max_render_payload => {
            Err(WindowError::RenderPayloadTooLarge)
        }
        _ => Ok(()),
    }
}

#[cfg(not(windows))]
fn window_thread(
    config: WindowConfig,
    commands: Receiver<WindowCommand>,
    events: EventSinks,
    render_queue: RenderQueue,
    native_handle: Arc<AtomicIsize>,
    shutdown_requested: Arc<std::sync::atomic::AtomicBool>,
    started: SyncSender<Result<(), WindowError>>,
) {
    let _ = started.send(Ok(()));
    loop {
        if shutdown_requested.load(Ordering::Acquire) {
            events.control.push(WindowEvent::Closed);
            native_handle.store(0, Ordering::Release);
            break;
        }
        let Ok(command) = commands.recv_timeout(Duration::from_millis(10)) else {
            continue;
        };
        match command {
            WindowCommand::Render(frame) => {
                if frame.access_unit.len() > config.max_render_payload
                    || render_queue.push_latest(frame).is_err()
                {
                    events.control.push(WindowEvent::RenderRejected);
                } else {
                    let _ = events.normal.try_send(WindowEvent::RenderRequested);
                }
            }
            WindowCommand::SetTitle(_) | WindowCommand::RequestFocus => {}
            WindowCommand::Close | WindowCommand::Shutdown => {
                events.control.push(WindowEvent::Closed);
                native_handle.store(0, Ordering::Release);
                break;
            }
        }
    }
}

#[cfg(windows)]
fn window_thread(
    config: WindowConfig,
    commands: Receiver<WindowCommand>,
    events: EventSinks,
    render_queue: RenderQueue,
    native_handle: Arc<AtomicIsize>,
    shutdown_requested: Arc<std::sync::atomic::AtomicBool>,
    started: SyncSender<Result<(), WindowError>>,
) {
    native::run(
        config,
        commands,
        events,
        render_queue,
        native_handle,
        shutdown_requested,
        started,
    );
}

#[cfg(windows)]
mod native {
    use super::*;
    use std::ffi::c_void;
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        GetKeyState, SetFocus, VK_CONTROL, VK_LMENU, VK_LSHIFT, VK_RMENU, VK_RSHIFT,
    };
    use windows::Win32::UI::WindowsAndMessaging::*;

    const CLASS_NAME: &[u16] = &[
        b'N' as u16,
        b'e' as u16,
        b'x' as u16,
        b'u' as u16,
        b's' as u16,
        0,
    ];

    struct State {
        events: EventSinks,
        pending_high_surrogate: Option<u16>,
    }

    pub(super) fn run(
        config: WindowConfig,
        commands: Receiver<WindowCommand>,
        events: EventSinks,
        render_queue: RenderQueue,
        native_handle: Arc<AtomicIsize>,
        shutdown_requested: Arc<std::sync::atomic::AtomicBool>,
        started: SyncSender<Result<(), WindowError>>,
    ) {
        unsafe {
            let instance = HINSTANCE(GetModuleHandleW(None).unwrap_or_default().0);
            let class = WNDCLASSW {
                lpfnWndProc: Some(window_proc),
                hInstance: instance,
                lpszClassName: PCWSTR(CLASS_NAME.as_ptr()),
                ..Default::default()
            };
            let _ = RegisterClassW(&class);
            let state = Box::new(State {
                events: events.clone(),
                pending_high_surrogate: None,
            });
            // Transfer ownership to the HWND userdata exactly once. The
            // message loop reclaims it only when it receives WM_QUIT.
            let state_ptr = Box::into_raw(state);
            let state_param = state_ptr as *mut c_void;
            let title: Vec<u16> = config.title.encode_utf16().chain(Some(0)).collect();
            let hwnd = match CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                PCWSTR(CLASS_NAME.as_ptr()),
                PCWSTR(title.as_ptr()),
                WS_OVERLAPPEDWINDOW | WS_VISIBLE,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                config.width as i32,
                config.height as i32,
                None,
                None,
                instance,
                Some(state_param),
            ) {
                Ok(hwnd) => hwnd,
                Err(_) => {
                    drop(Box::from_raw(state_ptr));
                    let _ = started.send(Err(WindowError::ThreadStart));
                    return;
                }
            };
            if hwnd.is_invalid() {
                drop(Box::from_raw(state_ptr));
                let _ = started.send(Err(WindowError::ThreadStart));
                return;
            }
            native_handle.store(hwnd.0 as isize, Ordering::Release);
            events.control.push(WindowEvent::FocusChanged(false));
            let _ = started.send(Ok(()));
            loop {
                if shutdown_requested.load(Ordering::Acquire) {
                    let _ = DestroyWindow(hwnd);
                }
                let mut command_disconnected = false;
                loop {
                    match commands.try_recv() {
                        Ok(command) => match command {
                            WindowCommand::Render(frame) => {
                                if frame.access_unit.len() > config.max_render_payload
                                    || render_queue.push_latest(frame).is_err()
                                {
                                    events.control.push(WindowEvent::RenderRejected);
                                }
                            }
                            WindowCommand::SetTitle(value) => {
                                let title: Vec<u16> = value.encode_utf16().chain(Some(0)).collect();
                                let _ = SetWindowTextW(hwnd, PCWSTR(title.as_ptr()));
                            }
                            WindowCommand::RequestFocus => {
                                let _ = SetFocus(hwnd);
                            }
                            WindowCommand::Close | WindowCommand::Shutdown => {
                                let _ = DestroyWindow(hwnd);
                            }
                        },
                        Err(std::sync::mpsc::TryRecvError::Empty) => break,
                        Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                            command_disconnected = true;
                            break;
                        }
                    }
                }
                if command_disconnected {
                    let _ = DestroyWindow(hwnd);
                }
                let mut message = MSG::default();
                while PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool() {
                    if message.message == WM_QUIT {
                        native_handle.store(0, Ordering::Release);
                        drop(Box::from_raw(state_ptr));
                        return;
                    }
                    let _ = TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
                thread::sleep(Duration::from_millis(4));
            }
        }
    }

    unsafe extern "system" fn window_proc(
        hwnd: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        let state = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut State;
        if message == WM_NCCREATE {
            let create = &*(lparam.0 as *const CREATESTRUCTW);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
            return DefWindowProcW(hwnd, message, wparam, lparam);
        }
        if !state.is_null() {
            let send = |event| {
                let _ = (*state).events.normal.try_send(event);
            };
            let send_control = |event| {
                // Focus loss and close are lifecycle controls. This bounded,
                // coalescing priority queue preserves them under normal-input
                // load; network workers never wait on this path.
                (*state).events.control.push(event);
            };
            match message {
                WM_SETFOCUS => send_control(WindowEvent::FocusChanged(true)),
                WM_KILLFOCUS => send_control(WindowEvent::FocusChanged(false)),
                WM_SIZE => send(WindowEvent::Resized {
                    width: (lparam.0 as u32 & 0xffff),
                    height: ((lparam.0 as u32 >> 16) & 0xffff),
                }),
                WM_CLOSE => {
                    let _ = DestroyWindow(hwnd);
                    return LRESULT(0);
                }
                WM_DESTROY => {
                    send_control(WindowEvent::Closed);
                    PostQuitMessage(0);
                    return LRESULT(0);
                }
                WM_KEYDOWN | WM_SYSKEYDOWN => {
                    if let Some(event) = key_event(wparam.0 as u32, lparam.0, true) {
                        send(WindowEvent::Input(event));
                    }
                }
                WM_KEYUP | WM_SYSKEYUP => {
                    if let Some(event) = key_event(wparam.0 as u32, lparam.0, false) {
                        send(WindowEvent::Input(event));
                    }
                }
                WM_CHAR => {
                    if let Some(text) =
                        decode_utf16_unit(&mut (*state).pending_high_surrogate, wparam.0 as u16)
                    {
                        send(WindowEvent::Input(InputEvent::Text(text)));
                    }
                }
                WM_MOUSEMOVE => send(WindowEvent::Input(InputEvent::MouseMove {
                    x: signed_low(lparam.0),
                    y: signed_high(lparam.0),
                })),
                WM_LBUTTONDOWN | WM_LBUTTONUP => {
                    send(WindowEvent::Input(InputEvent::MouseButton {
                        button: nexus_input::MouseButton::Left,
                        pressed: message == WM_LBUTTONDOWN,
                    }))
                }
                WM_RBUTTONDOWN | WM_RBUTTONUP => {
                    send(WindowEvent::Input(InputEvent::MouseButton {
                        button: nexus_input::MouseButton::Right,
                        pressed: message == WM_RBUTTONDOWN,
                    }))
                }
                WM_MBUTTONDOWN | WM_MBUTTONUP => {
                    send(WindowEvent::Input(InputEvent::MouseButton {
                        button: nexus_input::MouseButton::Middle,
                        pressed: message == WM_MBUTTONDOWN,
                    }))
                }
                WM_MOUSEWHEEL => send(WindowEvent::Input(InputEvent::MouseWheel {
                    delta_x: 0,
                    delta_y: ((wparam.0 >> 16) as u16 as i16 as i32),
                })),
                WM_MOUSEHWHEEL => send(WindowEvent::Input(InputEvent::MouseWheel {
                    delta_x: ((wparam.0 >> 16) as u16 as i16 as i32),
                    delta_y: 0,
                })),
                _ => {}
            }
        }
        DefWindowProcW(hwnd, message, wparam, lparam)
    }

    fn signed_low(value: isize) -> i32 {
        (value as i32 as u32 & 0xffff) as i16 as i32
    }
    fn signed_high(value: isize) -> i32 {
        ((value as i32 as u32 >> 16) & 0xffff) as i16 as i32
    }
    fn decode_utf16_unit(pending: &mut Option<u16>, unit: u16) -> Option<String> {
        if (0xD800..=0xDBFF).contains(&unit) {
            *pending = Some(unit);
            return None;
        }
        if (0xDC00..=0xDFFF).contains(&unit) {
            let high = pending.take()?;
            return String::from_utf16(&[high, unit]).ok();
        }
        // A non-surrogate after an unmatched high surrogate is malformed;
        // discard the pending unit and continue with the current code unit.
        pending.take();
        char::from_u32(u32::from(unit)).map(|character| character.to_string())
    }

    fn key_event(code: u32, lparam: isize, pressed: bool) -> Option<InputEvent> {
        let scan_code = ((lparam >> 16) & 0xff) as u8;
        let extended = ((lparam >> 24) & 1) != 0;
        let code = map_virtual_key_to_hid(code, scan_code, extended)?;
        let mut modifiers = nexus_input::Modifiers::NONE;
        if (unsafe { GetKeyState(VK_LSHIFT.0 as i32) } as u16 & 0x8000) != 0
            || (unsafe { GetKeyState(VK_RSHIFT.0 as i32) } as u16 & 0x8000) != 0
        {
            modifiers = nexus_input::Modifiers::from_bits(
                modifiers.bits() | nexus_input::Modifiers::SHIFT.bits(),
            );
        }
        if (unsafe { GetKeyState(VK_CONTROL.0 as i32) } as u16 & 0x8000) != 0 {
            modifiers = nexus_input::Modifiers::from_bits(
                modifiers.bits() | nexus_input::Modifiers::CTRL.bits(),
            );
        }
        if (unsafe { GetKeyState(VK_LMENU.0 as i32) } as u16 & 0x8000) != 0
            || (unsafe { GetKeyState(VK_RMENU.0 as i32) } as u16 & 0x8000) != 0
        {
            modifiers = nexus_input::Modifiers::from_bits(
                modifiers.bits() | nexus_input::Modifiers::ALT.bits(),
            );
        }
        Some(InputEvent::Key {
            physical_code: code,
            logical_code: code,
            action: if pressed {
                nexus_input::KeyAction::Down
            } else {
                nexus_input::KeyAction::Up
            },
            modifiers,
        })
    }

    fn map_virtual_key_to_hid(code: u32, scan_code: u8, extended: bool) -> Option<u32> {
        let usage = match code {
            0x41..=0x5A => code - 0x41 + 0x04,
            0x30..=0x39 => match code {
                0x30 => 0x27,
                value => value - 0x30 + 0x1D,
            },
            0x70..=0x7B => code - 0x70 + 0x3A,
            0x08 => 0x2A,
            0x09 => 0x2B,
            0x0D => {
                if extended {
                    0x58
                } else {
                    0x28
                }
            }
            0x0C => 0x5D,
            0x1B => 0x29,
            0x20 => 0x2C,
            0x2D => {
                if extended {
                    0x49
                } else {
                    0x62
                }
            }
            0x2E => {
                if extended {
                    0x4C
                } else {
                    0x63
                }
            }
            0x24 => {
                if extended {
                    0x4A
                } else {
                    0x5F
                }
            }
            0x23 => {
                if extended {
                    0x4D
                } else {
                    0x59
                }
            }
            0x21 => {
                if extended {
                    0x4B
                } else {
                    0x61
                }
            }
            0x22 => {
                if extended {
                    0x4E
                } else {
                    0x5B
                }
            }
            0x25 => {
                if extended {
                    0x50
                } else {
                    0x5C
                }
            }
            0x26 => {
                if extended {
                    0x52
                } else {
                    0x60
                }
            }
            0x27 => {
                if extended {
                    0x4F
                } else {
                    0x5E
                }
            }
            0x28 => {
                if extended {
                    0x51
                } else {
                    0x5A
                }
            }
            0x10 => {
                if scan_code == 0x36 {
                    0xE5
                } else {
                    0xE1
                }
            }
            0x11 => {
                if extended {
                    0xE4
                } else {
                    0xE0
                }
            }
            0x12 => {
                if extended {
                    0xE6
                } else {
                    0xE2
                }
            }
            0x5B => 0xE3,
            0x5C => 0xE7,
            0x60..=0x69 => match code {
                0x60 => 0x62,
                0x61 => 0x59,
                0x62 => 0x5A,
                0x63 => 0x5B,
                0x64 => 0x5C,
                0x65 => 0x5D,
                0x66 => 0x5E,
                0x67 => 0x5F,
                0x68 => 0x60,
                0x69 => 0x61,
                _ => unreachable!(),
            },
            0x6A => 0x55,
            0x6B => 0x57,
            0x6D => 0x56,
            0x6E => 0x63,
            0x6F => 0x54,
            0x90 => 0x53,
            0xBA => 0x33,
            0xBB => 0x2E,
            0xBC => 0x36,
            0xBD => 0x2D,
            0xBE => 0x37,
            0xBF => 0x38,
            0xC0 => 0x35,
            0xDB => 0x2F,
            0xDC => 0x31,
            0xDD => 0x30,
            0xDE => 0x34,
            _ => return None,
        };
        Some(usage)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn maps_left_right_modifiers_and_keypad_navigation() {
            assert_eq!(map_virtual_key_to_hid(0x10, 0x2A, false), Some(0xE1));
            assert_eq!(map_virtual_key_to_hid(0x10, 0x36, false), Some(0xE5));
            assert_eq!(map_virtual_key_to_hid(0x11, 0x1D, true), Some(0xE4));
            assert_eq!(map_virtual_key_to_hid(0x11, 0x1D, false), Some(0xE0));
            assert_eq!(map_virtual_key_to_hid(0x5B, 0x5B, false), Some(0xE3));
            assert_eq!(map_virtual_key_to_hid(0x5C, 0x5C, true), Some(0xE7));
            assert_eq!(map_virtual_key_to_hid(0x2D, 0x52, false), Some(0x62));
            assert_eq!(map_virtual_key_to_hid(0x2D, 0x52, true), Some(0x49));
            assert_eq!(map_virtual_key_to_hid(0x0D, 0x1C, true), Some(0x58));
            assert_eq!(map_virtual_key_to_hid(0x0D, 0x1C, false), Some(0x28));
            assert_eq!(map_virtual_key_to_hid(0x90, 0x45, false), Some(0x53));
            assert_eq!(map_virtual_key_to_hid(0x0C, 0x4C, false), Some(0x5D));
        }

        #[test]
        fn buffers_utf16_surrogate_pair() {
            let mut pending = None;
            assert_eq!(decode_utf16_unit(&mut pending, 0xD83D), None);
            assert_eq!(
                decode_utf16_unit(&mut pending, 0xDE00),
                Some("😀".to_owned())
            );
            assert_eq!(pending, None);
        }
    }
}
