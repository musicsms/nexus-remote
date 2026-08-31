use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use nexus_capture::{CaptureSource, CapturedFrame};
use platform_windows::{
    BackendError, BackendErrorKind, BackendResult, CaptureApi, CaptureConfig, CaptureFactory,
    CaptureSession, CaptureState, WindowsCaptureSource,
};

#[derive(Clone, Copy)]
enum WgcOutcome {
    Error(BackendErrorKind),
}

struct RecordingFactory {
    calls: Arc<Mutex<Vec<CaptureApi>>>,
    wgc: WgcOutcome,
}

impl RecordingFactory {
    fn new(wgc: WgcOutcome) -> (Self, Arc<Mutex<Vec<CaptureApi>>>) {
        let calls = Arc::new(Mutex::new(Vec::new()));
        (
            Self {
                calls: Arc::clone(&calls),
                wgc,
            },
            calls,
        )
    }
}

impl CaptureFactory for RecordingFactory {
    fn start(&mut self, api: CaptureApi) -> BackendResult<Box<dyn CaptureSession>> {
        self.calls.lock().unwrap().push(api);
        match (api, self.wgc) {
            (CaptureApi::Dxgi, _) => Ok(Box::new(NoFrames)),
            (CaptureApi::Wgc, WgcOutcome::Error(kind)) => Err(kind.into()),
        }
    }
}

struct NoFrames;

impl CaptureSession for NoFrames {
    fn next_frame(&mut self) -> BackendResult<CapturedFrame> {
        Err(BackendErrorKind::Stopped.into())
    }
}

fn wgc_config(allow_dxgi_fallback: bool) -> CaptureConfig {
    CaptureConfig {
        preferred: CaptureApi::Wgc,
        allow_dxgi_fallback,
    }
}

#[test]
fn selection_attempts_wgc_first_then_dxgi_once_for_unsupported_api() {
    let (factory, calls) =
        RecordingFactory::new(WgcOutcome::Error(BackendErrorKind::UnsupportedApi));

    let source = WindowsCaptureSource::start_with_factory(wgc_config(true), factory).unwrap();

    assert_eq!(source.state(), CaptureState::Running(CaptureApi::Dxgi));
    assert_eq!(
        calls.lock().unwrap().as_slice(),
        &[CaptureApi::Wgc, CaptureApi::Dxgi]
    );
}

#[test]
fn selection_attempts_dxgi_once_for_wgc_initialization_device_loss() {
    let (factory, calls) = RecordingFactory::new(WgcOutcome::Error(BackendErrorKind::DeviceLost));

    let source = WindowsCaptureSource::start_with_factory(wgc_config(true), factory).unwrap();

    assert_eq!(source.state(), CaptureState::Running(CaptureApi::Dxgi));
    assert_eq!(
        calls.lock().unwrap().as_slice(),
        &[CaptureApi::Wgc, CaptureApi::Dxgi]
    );
}

#[test]
fn selection_does_not_fallback_after_permission_denial() {
    let (factory, calls) =
        RecordingFactory::new(WgcOutcome::Error(BackendErrorKind::PermissionDenied));

    let error = WindowsCaptureSource::start_with_factory(wgc_config(true), factory).unwrap_err();

    assert_eq!(error.kind(), BackendErrorKind::PermissionDenied);
    assert_eq!(calls.lock().unwrap().as_slice(), &[CaptureApi::Wgc]);
}

#[test]
fn selection_returns_original_wgc_error_when_fallback_is_disabled() {
    let (factory, calls) =
        RecordingFactory::new(WgcOutcome::Error(BackendErrorKind::UnsupportedApi));

    let error = WindowsCaptureSource::start_with_factory(wgc_config(false), factory).unwrap_err();

    assert_eq!(error, BackendError::new(BackendErrorKind::UnsupportedApi));
    assert_eq!(calls.lock().unwrap().as_slice(), &[CaptureApi::Wgc]);
}

struct SessionFactory {
    session: Option<ScriptedSession>,
    thread_names: Arc<Mutex<Vec<Option<String>>>>,
}

impl CaptureFactory for SessionFactory {
    fn start(&mut self, api: CaptureApi) -> BackendResult<Box<dyn CaptureSession>> {
        assert_eq!(api, CaptureApi::Wgc);
        self.thread_names
            .lock()
            .unwrap()
            .push(std::thread::current().name().map(str::to_owned));
        Ok(Box::new(self.session.take().unwrap()))
    }
}

struct ScriptedSession {
    frames: VecDeque<BackendResult<CapturedFrame>>,
    stop_count: Arc<AtomicUsize>,
    thread_names: Arc<Mutex<Vec<Option<String>>>>,
}

impl CaptureSession for ScriptedSession {
    fn next_frame(&mut self) -> BackendResult<CapturedFrame> {
        self.thread_names
            .lock()
            .unwrap()
            .push(std::thread::current().name().map(str::to_owned));
        self.frames.pop_front().unwrap()
    }

    fn stop(&mut self) -> BackendResult<()> {
        self.stop_count.fetch_add(1, Ordering::SeqCst);
        self.thread_names
            .lock()
            .unwrap()
            .push(std::thread::current().name().map(str::to_owned));
        Ok(())
    }
}

fn frame(width: u32, height: u32, data: Vec<u8>) -> CapturedFrame {
    CapturedFrame {
        frame_id: 7,
        timestamp_us: 11,
        width,
        height,
        format: nexus_capture::PixelFormat::Bgra8,
        data: data.into(),
    }
}

struct SourceFixture {
    source: WindowsCaptureSource,
    stop_count: Arc<AtomicUsize>,
    thread_names: Arc<Mutex<Vec<Option<String>>>>,
}

fn source_with_frames(frames: Vec<BackendResult<CapturedFrame>>) -> SourceFixture {
    let stop_count = Arc::new(AtomicUsize::new(0));
    let thread_names = Arc::new(Mutex::new(Vec::new()));
    let factory = SessionFactory {
        session: Some(ScriptedSession {
            frames: frames.into(),
            stop_count: Arc::clone(&stop_count),
            thread_names: Arc::clone(&thread_names),
        }),
        thread_names: Arc::clone(&thread_names),
    };
    let source = WindowsCaptureSource::start_with_factory(wgc_config(false), factory).unwrap();
    SourceFixture {
        source,
        stop_count,
        thread_names,
    }
}

#[test]
fn lifecycle_returns_a_validated_frame_from_the_session() {
    let expected = frame(2, 1, vec![0x2a; 8]);
    let mut fixture = source_with_frames(vec![Ok(expected.clone())]);

    assert_eq!(fixture.source.next_frame().unwrap(), expected);
}

#[test]
fn lifecycle_maps_a_malformed_frame_to_invalid_frame() {
    let mut fixture = source_with_frames(vec![Ok(frame(2, 1, vec![0; 7]))]);

    assert_eq!(
        fixture.source.next_frame().unwrap_err().kind(),
        BackendErrorKind::InvalidFrame
    );
}

#[test]
fn lifecycle_classifies_device_loss_as_recoverable() {
    let mut fixture = source_with_frames(vec![Err(BackendErrorKind::DeviceLost.into())]);

    assert_eq!(
        fixture.source.next_frame().unwrap_err().kind(),
        BackendErrorKind::DeviceLost
    );
    assert_eq!(fixture.source.state(), CaptureState::RecoverableLoss);
}

#[test]
fn lifecycle_returns_stopped_after_shutdown_and_stops_the_session_once() {
    let mut fixture = source_with_frames(Vec::new());

    fixture.source.stop().unwrap();
    fixture.source.stop().unwrap();

    assert_eq!(fixture.source.state(), CaptureState::Stopped);
    assert_eq!(
        fixture.source.next_frame().unwrap_err().kind(),
        BackendErrorKind::Stopped
    );
    assert_eq!(fixture.stop_count.load(Ordering::SeqCst), 1);
}

#[test]
fn lifecycle_owns_initialization_acquisition_and_shutdown_on_one_named_thread() {
    let mut fixture = source_with_frames(vec![Ok(frame(1, 1, vec![0; 4]))]);

    fixture.source.next_frame().unwrap();
    fixture.source.stop().unwrap();

    assert_eq!(
        fixture.thread_names.lock().unwrap().as_slice(),
        &[
            Some("nexus-windows-capture".to_owned()),
            Some("nexus-windows-capture".to_owned()),
            Some("nexus-windows-capture".to_owned()),
        ]
    );
}

#[cfg(not(windows))]
#[test]
fn lifecycle_native_start_fails_closed_off_windows() {
    let error = WindowsCaptureSource::start(CaptureConfig::default()).unwrap_err();

    assert_eq!(error.kind(), BackendErrorKind::UnsupportedPlatform);
}
