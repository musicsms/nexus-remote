//! Audit log sink abstractions and implementations.
//!
//! Provides the [`AuditSink`] trait for asynchronous audit recording,
//! [`MemoryAuditSink`] for in-memory capturing and testing,
//! [`BroadcastAuditSink`] for multi-destination forwarding, and
//! [`SinkError`] for error handling.

use async_trait::async_trait;
use std::sync::{Arc, RwLock};

use crate::chain::ChainedAuditEvent;

/// Errors that can occur when recording or flushing audit events to a sink.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SinkError {
    /// An I/O error occurred during sink operations.
    #[error("I/O error: {0}")]
    IoError(String),

    /// The audit channel or queue is closed and cannot accept new events.
    #[error("audit channel closed")]
    ChannelClosed,

    /// A storage or database error occurred.
    #[error("storage error: {0}")]
    StorageError(String),

    /// A custom error from a sink implementation.
    #[error("custom sink error: {0}")]
    Custom(String),
}

impl From<std::io::Error> for SinkError {
    fn from(err: std::io::Error) -> Self {
        Self::IoError(err.to_string())
    }
}

/// Asynchronous destination for chained audit events.
#[async_trait]
pub trait AuditSink: Send + Sync {
    /// Records a chained audit event to the sink.
    async fn record(&self, event: &ChainedAuditEvent) -> Result<(), SinkError>;

    /// Flushes any buffered audit events to persistent storage.
    async fn flush(&self) -> Result<(), SinkError> {
        Ok(())
    }
}

/// An in-memory audit sink backed by an `Arc<std::sync::RwLock<Vec<ChainedAuditEvent>>>`.
///
/// Useful for testing, validation, and in-memory aggregation.
#[derive(Debug, Clone)]
pub struct MemoryAuditSink {
    events: Arc<RwLock<Vec<ChainedAuditEvent>>>,
}

impl Default for MemoryAuditSink {
    fn default() -> Self {
        Self::new()
    }
}

impl MemoryAuditSink {
    /// Creates a new, empty in-memory audit sink.
    #[must_use]
    pub fn new() -> Self {
        Self {
            events: Arc::new(RwLock::new(Vec::new())),
        }
    }

    /// Returns a copy of all recorded chained audit events.
    #[must_use]
    pub fn events(&self) -> Vec<ChainedAuditEvent> {
        self.events
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// Returns a copy of all recorded chained audit events asynchronously.
    pub async fn events_async(&self) -> Vec<ChainedAuditEvent> {
        self.events()
    }

    /// Returns the number of events recorded in the sink.
    #[must_use]
    pub fn len(&self) -> usize {
        self.events
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .len()
    }

    /// Returns `true` if the sink contains no events.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Clears all events stored in the sink.
    pub fn clear(&self) {
        let mut guard = self
            .events
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        guard.clear();
    }

    /// Clears all events stored in the sink asynchronously.
    pub async fn clear_async(&self) {
        self.clear();
    }
}

#[async_trait]
impl AuditSink for MemoryAuditSink {
    async fn record(&self, event: &ChainedAuditEvent) -> Result<(), SinkError> {
        let mut guard = self
            .events
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        guard.push(event.clone());
        Ok(())
    }

    async fn flush(&self) -> Result<(), SinkError> {
        Ok(())
    }
}

/// Composite audit sink that broadcasts events to multiple underlying sinks.
pub struct BroadcastAuditSink {
    sinks: Vec<Arc<dyn AuditSink>>,
}

impl Default for BroadcastAuditSink {
    fn default() -> Self {
        Self::new(Vec::new())
    }
}

impl BroadcastAuditSink {
    /// Creates a new broadcast audit sink with the provided child sinks.
    #[must_use]
    pub fn new(sinks: Vec<Arc<dyn AuditSink>>) -> Self {
        Self { sinks }
    }

    /// Adds an additional child sink to the broadcast list.
    pub fn add_sink(&mut self, sink: Arc<dyn AuditSink>) {
        self.sinks.push(sink);
    }

    /// Returns the number of child sinks configured.
    #[must_use]
    pub fn len(&self) -> usize {
        self.sinks.len()
    }

    /// Returns `true` if no child sinks are configured.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.sinks.is_empty()
    }

    /// Returns a slice of the child sinks.
    #[must_use]
    pub fn sinks(&self) -> &[Arc<dyn AuditSink>] {
        &self.sinks
    }
}

impl From<Vec<Arc<dyn AuditSink>>> for BroadcastAuditSink {
    fn from(sinks: Vec<Arc<dyn AuditSink>>) -> Self {
        Self::new(sinks)
    }
}

#[async_trait]
impl AuditSink for BroadcastAuditSink {
    async fn record(&self, event: &ChainedAuditEvent) -> Result<(), SinkError> {
        let mut first_error = None;
        for sink in &self.sinks {
            if let Err(err) = sink.record(event).await {
                if first_error.is_none() {
                    first_error = Some(err);
                }
            }
        }
        if let Some(err) = first_error {
            Err(err)
        } else {
            Ok(())
        }
    }

    async fn flush(&self) -> Result<(), SinkError> {
        let mut first_error = None;
        for sink in &self.sinks {
            if let Err(err) = sink.flush().await {
                if first_error.is_none() {
                    first_error = Some(err);
                }
            }
        }
        if let Some(err) = first_error {
            Err(err)
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chain::AuditChain;
    use crate::event::{AuditEvent, AuditEventType};
    use nexus_common::id::TenantId;
    use nexus_common::time::UnixTimestamp;

    fn sample_chained_event(id: &str, event_type: AuditEventType) -> ChainedAuditEvent {
        let mut chain = AuditChain::new(None);
        let event = AuditEvent::new(
            id,
            UnixTimestamp::from_secs(1_700_000_000),
            TenantId::new("tenant-corp-1").unwrap(),
            event_type,
        );
        chain.append(event).expect("append event")
    }

    #[tokio::test]
    async fn test_memory_audit_sink_basic_operations() {
        let sink = MemoryAuditSink::new();
        assert!(sink.is_empty());
        assert_eq!(sink.len(), 0);
        assert_eq!(sink.events().len(), 0);

        let ev1 = sample_chained_event("evt-1", AuditEventType::UserLogin);
        sink.record(&ev1).await.expect("record ev1");

        assert_eq!(sink.len(), 1);
        assert!(!sink.is_empty());
        assert_eq!(sink.events(), vec![ev1.clone()]);
        assert_eq!(sink.events_async().await, vec![ev1.clone()]);

        let ev2 = sample_chained_event("evt-2", AuditEventType::SessionStart);
        sink.record(&ev2).await.expect("record ev2");

        assert_eq!(sink.len(), 2);
        let events = sink.events();
        assert_eq!(events[0], ev1);
        assert_eq!(events[1], ev2);

        // Test flush
        assert!(sink.flush().await.is_ok());

        // Test clear
        sink.clear();
        assert_eq!(sink.len(), 0);
        assert!(sink.is_empty());
        assert!(sink.events().is_empty());

        // Test clear_async
        sink.record(&ev1).await.expect("record ev1");
        assert_eq!(sink.len(), 1);
        sink.clear_async().await;
        assert_eq!(sink.len(), 0);
    }

    #[tokio::test]
    async fn test_memory_audit_sink_clone_and_default() {
        let sink = MemoryAuditSink::default();
        let sink_clone = sink.clone();

        let ev = sample_chained_event("evt-1", AuditEventType::SessionEnd);
        sink.record(&ev).await.expect("record to original");

        // Clone shares underlying Arc RwLock
        assert_eq!(sink_clone.len(), 1);
        assert_eq!(sink_clone.events(), vec![ev]);
    }

    #[tokio::test]
    async fn test_broadcast_audit_sink_distribution() {
        let sink1 = Arc::new(MemoryAuditSink::new());
        let sink2 = Arc::new(MemoryAuditSink::new());
        let sink3 = Arc::new(MemoryAuditSink::new());

        let mut broadcaster = BroadcastAuditSink::new(vec![sink1.clone(), sink2.clone()]);
        assert_eq!(broadcaster.len(), 2);
        assert!(!broadcaster.is_empty());
        assert_eq!(broadcaster.sinks().len(), 2);

        // Add third sink dynamically
        broadcaster.add_sink(sink3.clone());
        assert_eq!(broadcaster.len(), 3);

        let ev1 = sample_chained_event("evt-1", AuditEventType::PolicyUpdate);
        let ev2 = sample_chained_event("evt-2", AuditEventType::AccessApprove);

        broadcaster.record(&ev1).await.expect("broadcast ev1");
        broadcaster.record(&ev2).await.expect("broadcast ev2");
        broadcaster.flush().await.expect("broadcast flush");

        // Verify all 3 sinks received both events
        for sink in [&sink1, &sink2, &sink3] {
            assert_eq!(sink.len(), 2);
            let events = sink.events();
            assert_eq!(events[0], ev1);
            assert_eq!(events[1], ev2);
        }
    }

    #[tokio::test]
    async fn test_broadcast_audit_sink_default_and_from() {
        let default_sink = BroadcastAuditSink::default();
        assert!(default_sink.is_empty());
        assert_eq!(default_sink.len(), 0);

        let child = Arc::new(MemoryAuditSink::new());
        let from_sink = BroadcastAuditSink::from(vec![child.clone() as Arc<dyn AuditSink>]);
        assert_eq!(from_sink.len(), 1);
    }

    struct FailingSink;

    #[async_trait]
    impl AuditSink for FailingSink {
        async fn record(&self, _event: &ChainedAuditEvent) -> Result<(), SinkError> {
            Err(SinkError::StorageError("disk failure".into()))
        }

        async fn flush(&self) -> Result<(), SinkError> {
            Err(SinkError::IoError("connection reset".into()))
        }
    }

    #[tokio::test]
    async fn test_broadcast_audit_sink_error_handling() {
        let good_sink = Arc::new(MemoryAuditSink::new());
        let fail_sink = Arc::new(FailingSink);

        let broadcaster = BroadcastAuditSink::new(vec![good_sink.clone(), fail_sink.clone()]);

        let ev = sample_chained_event("evt-1", AuditEventType::FileUpload);
        let rec_res = broadcaster.record(&ev).await;
        assert!(matches!(rec_res, Err(SinkError::StorageError(_))));

        // Good sink should still have received the event
        assert_eq!(good_sink.len(), 1);

        let flush_res = broadcaster.flush().await;
        assert!(matches!(flush_res, Err(SinkError::IoError(_))));
    }

    #[tokio::test]
    async fn test_concurrent_asynchronous_recording() {
        let sink = Arc::new(MemoryAuditSink::new());
        let total_tasks = 20;
        let events_per_task = 50;

        let mut handles = Vec::new();

        for task_idx in 0..total_tasks {
            let sink_clone = sink.clone();
            let handle = tokio::spawn(async move {
                for ev_idx in 0..events_per_task {
                    let ev_id = format!("evt-t{task_idx}-e{ev_idx}");
                    let ev = sample_chained_event(&ev_id, AuditEventType::ClipboardRead);
                    sink_clone.record(&ev).await.expect("record event");
                }
            });
            handles.push(handle);
        }

        for handle in handles {
            handle.await.expect("task join");
        }

        assert_eq!(sink.len(), total_tasks * events_per_task);
        assert_eq!(sink.events().len(), total_tasks * events_per_task);
    }

    #[test]
    fn test_sink_error_display_and_traits() {
        let err_io = SinkError::IoError("connection refused".into());
        assert_eq!(err_io.to_string(), "I/O error: connection refused");

        let err_closed = SinkError::ChannelClosed;
        assert_eq!(err_closed.to_string(), "audit channel closed");

        let err_storage = SinkError::StorageError("table locked".into());
        assert_eq!(err_storage.to_string(), "storage error: table locked");

        let err_custom = SinkError::Custom("plugin panic".into());
        assert_eq!(err_custom.to_string(), "custom sink error: plugin panic");

        let std_io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file missing");
        let from_io: SinkError = std_io_err.into();
        assert!(matches!(from_io, SinkError::IoError(_)));
    }
}
