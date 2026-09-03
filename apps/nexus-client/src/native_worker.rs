//! Bounded ownership of a native worker that may be blocked in a driver call.

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

/// Keeps a worker joinable even when its public owner reaches a deadline.
/// A background reaper owns the handle after timeout, so COM/D3D/MF cleanup
/// still happens on the worker when it eventually returns.
#[derive(Debug)]
pub(crate) struct WorkerLifecycle {
    worker: Mutex<Option<JoinHandle<()>>>,
    reaping: AtomicBool,
}

impl WorkerLifecycle {
    pub(crate) fn new(worker: JoinHandle<()>) -> Arc<Self> {
        Arc::new(Self {
            worker: Mutex::new(Some(worker)),
            reaping: AtomicBool::new(false),
        })
    }

    pub(crate) fn join_before(self: &Arc<Self>, deadline: Instant) {
        let worker = match self.worker.lock() {
            Ok(mut worker) => worker.take(),
            Err(_) => {
                self.reap_in_background();
                return;
            }
        };
        let Some(worker) = worker else {
            return;
        };
        while !worker.is_finished() {
            if Instant::now() >= deadline {
                if let Ok(mut slot) = self.worker.lock() {
                    *slot = Some(worker);
                }
                self.reap_in_background();
                return;
            }
            thread::sleep(Duration::from_millis(1));
        }
        let _ = worker.join();
    }

    pub(crate) fn reap_in_background(self: &Arc<Self>) {
        if self.reaping.swap(true, Ordering::AcqRel) {
            return;
        }
        let lifecycle = Arc::clone(self);
        if thread::Builder::new()
            .name("nexus-client-native-reaper".to_owned())
            .spawn(move || {
                let worker = lifecycle
                    .worker
                    .lock()
                    .ok()
                    .and_then(|mut worker| worker.take());
                if let Some(worker) = worker {
                    let _ = worker.join();
                }
            })
            .is_err()
        {
            self.reaping.store(false, Ordering::Release);
        }
    }
}
