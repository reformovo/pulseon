use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, SyncSender, TrySendError};
use std::sync::{Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::engine::EngineError;
use crate::engine::write::NativeWriteStore;
use crate::model::metric::{MetricKey, Step};
use crate::model::run::RunId;

const DEFAULT_METRIC_BUFFER_CAPACITY: usize = 65_536;
const WRITER_RUNNING: u8 = 0;
const WRITER_FAILED: u8 = 1;
const WRITER_CLOSED: u8 = 2;

#[derive(Clone)]
pub struct MetricReporter {
    inner: Arc<MetricReporterInner>,
}

impl MetricReporter {
    pub fn open(connection: Arc<Mutex<duckdb::Connection>>) -> Self {
        Self::open_with_capacity(connection, DEFAULT_METRIC_BUFFER_CAPACITY)
    }

    pub fn open_with_capacity(connection: Arc<Mutex<duckdb::Connection>>, capacity: usize) -> Self {
        let (sender, receiver) = mpsc::sync_channel(capacity);
        let diagnostics = Arc::new(MetricReporterDiagnosticsInner::default());
        let worker_diagnostics = Arc::clone(&diagnostics);
        let failure_diagnostics = Arc::clone(&diagnostics);
        let worker = std::thread::spawn(move || {
            let result = metric_worker(connection, receiver, worker_diagnostics);
            if result.is_err() {
                failure_diagnostics.set_writer_failed();
            }
        });

        Self {
            inner: Arc::new(MetricReporterInner {
                sender: Mutex::new(Some(sender)),
                diagnostics,
                worker: Mutex::new(Some(worker)),
            }),
        }
    }

    pub fn report_metric(
        &self,
        run_id: RunId,
        metric_key: MetricKey,
        step: Option<Step>,
        value_f64: f64,
    ) -> Result<(), EngineError> {
        let report = MetricReport {
            run_id,
            metric_key,
            step,
            value_f64,
        };
        let sender = match self.sender() {
            Some(sender) => sender,
            None => {
                return Ok(());
            }
        };
        self.inner.diagnostics.increment_pending();
        match sender.try_send(report) {
            Ok(()) => {
                self.inner.diagnostics.increment_accepted();
                Ok(())
            }
            Err(TrySendError::Full(_)) => {
                self.inner.diagnostics.decrement_pending();
                self.inner.diagnostics.increment_queue_full();
                Err(EngineError::MetricQueueFull)
            }
            Err(TrySendError::Disconnected(_)) => {
                self.inner.diagnostics.decrement_pending();
                Ok(())
            }
        }
    }

    pub fn diagnostics(&self) -> MetricReporterDiagnostics {
        self.inner.diagnostics.snapshot()
    }

    pub fn drain_for(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        while self.inner.diagnostics.pending_reports() > 0 {
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        true
    }

    pub fn shutdown_for(&self, timeout: Duration) -> bool {
        let drained = self.drain_for(timeout);
        if let Some(sender) = take_mutex_value(&self.inner.sender) {
            drop(sender);
        }
        if let Some(worker) = take_mutex_value(&self.inner.worker)
            && drained
        {
            let _ = worker.join();
        }
        if drained {
            self.inner.diagnostics.set_writer_closed();
        }
        drained
    }

    #[cfg(test)]
    pub(crate) fn blocked_for_test(capacity: usize) -> Self {
        let (sender, receiver) = mpsc::sync_channel(capacity);
        // Keep the receiver alive without draining it to simulate a blocked writer.
        std::mem::forget(receiver);
        Self {
            inner: Arc::new(MetricReporterInner {
                sender: Mutex::new(Some(sender)),
                diagnostics: Arc::new(MetricReporterDiagnosticsInner::default()),
                worker: Mutex::new(None),
            }),
        }
    }

    fn sender(&self) -> Option<SyncSender<MetricReport>> {
        let guard = self.inner.sender.lock().ok()?;
        guard.as_ref().cloned()
    }
}

struct MetricReporterInner {
    sender: Mutex<Option<SyncSender<MetricReport>>>,
    diagnostics: Arc<MetricReporterDiagnosticsInner>,
    worker: Mutex<Option<JoinHandle<()>>>,
}

impl Drop for MetricReporterInner {
    fn drop(&mut self) {
        if let Some(sender) = take_mutex_value(&self.sender) {
            drop(sender);
        }
        if let Some(worker) = take_mutex_value(&self.worker) {
            let _ = worker.join();
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MetricReporterDiagnostics {
    pub pending_reports: u64,
    pub queue_full_errors: u64,
    pub persisted_reports: u64,
    pub writer_state: &'static str,
    pub last_write_error: Option<String>,
    pub last_flush_run_id: Option<String>,
    pub last_flush_status: &'static str,
    pub last_flush_error: Option<String>,
}

#[derive(Default)]
struct MetricReporterDiagnosticsInner {
    pending_reports: AtomicU64,
    queue_full_errors: AtomicU64,
    persisted_reports: AtomicU64,
    writer_state: AtomicU64,
    last_write_error: Mutex<Option<String>>,
}

impl MetricReporterDiagnosticsInner {
    fn increment_accepted(&self) {
        self.writer_state
            .store(WRITER_RUNNING.into(), Ordering::Relaxed);
    }

    fn increment_queue_full(&self) {
        self.queue_full_errors.fetch_add(1, Ordering::Relaxed);
    }

    fn increment_persisted(&self) {
        self.persisted_reports.fetch_add(1, Ordering::Relaxed);
    }

    fn set_writer_failed(&self) {
        self.writer_state
            .store(WRITER_FAILED.into(), Ordering::Relaxed);
    }

    fn set_writer_closed(&self) {
        self.writer_state
            .store(WRITER_CLOSED.into(), Ordering::Relaxed);
    }

    fn set_last_write_error(&self, message: String) {
        if let Ok(mut last_write_error) = self.last_write_error.lock() {
            *last_write_error = Some(message);
        }
    }

    fn increment_pending(&self) {
        self.pending_reports.fetch_add(1, Ordering::Relaxed);
    }

    fn decrement_pending(&self) {
        self.pending_reports.fetch_sub(1, Ordering::Relaxed);
    }

    fn pending_reports(&self) -> u64 {
        self.pending_reports.load(Ordering::Relaxed)
    }

    fn snapshot(&self) -> MetricReporterDiagnostics {
        let pending_reports = self.pending_reports();
        let writer_state = match self.writer_state.load(Ordering::Relaxed) as u8 {
            WRITER_FAILED => "failed",
            WRITER_CLOSED => "closed",
            _ if pending_reports == 0 => "drained",
            _ => "running",
        };
        MetricReporterDiagnostics {
            pending_reports,
            queue_full_errors: self.queue_full_errors.load(Ordering::Relaxed),
            persisted_reports: self.persisted_reports.load(Ordering::Relaxed),
            writer_state,
            last_write_error: self
                .last_write_error
                .lock()
                .ok()
                .and_then(|last_write_error| last_write_error.clone()),
            last_flush_run_id: None,
            last_flush_status: "none",
            last_flush_error: None,
        }
    }
}

struct MetricReport {
    run_id: RunId,
    metric_key: MetricKey,
    step: Option<Step>,
    value_f64: f64,
}

fn take_mutex_value<T>(mutex: &Mutex<Option<T>>) -> Option<T> {
    let mut guard: MutexGuard<'_, Option<T>> = mutex.lock().ok()?;
    guard.take()
}

fn metric_worker(
    connection: Arc<Mutex<duckdb::Connection>>,
    receiver: mpsc::Receiver<MetricReport>,
    diagnostics: Arc<MetricReporterDiagnosticsInner>,
) -> Result<(), EngineError> {
    for report in receiver {
        let result = write_metric_report(&connection, &report);
        if let Err(error) = &result {
            diagnostics.set_last_write_error(error.to_string());
            diagnostics.set_writer_failed();
        } else {
            diagnostics.increment_persisted();
        }
        diagnostics.decrement_pending();
        result?;
    }
    Ok(())
}

fn write_metric_report(
    connection: &Arc<Mutex<duckdb::Connection>>,
    report: &MetricReport,
) -> Result<(), EngineError> {
    let connection = connection
        .lock()
        .map_err(|_| EngineError::ConnectionLockPoisoned)?;
    let store = NativeWriteStore::new(&connection);
    match report.step {
        Some(step) => store
            .log_metric_at_step(&report.run_id, &report.metric_key, step, report.value_f64)
            .map(|_| ()),
        None => store
            .log_metric(&report.run_id, &report.metric_key, report.value_f64)
            .map(|_| ()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_metric_returns_queue_full_when_buffer_is_full() {
        let reporter = MetricReporter::blocked_for_test(1);

        reporter
            .report_metric(
                RunId::from_string("run-1"),
                MetricKey::from_string("train/loss"),
                None,
                0.25,
            )
            .expect("first report should enter the queue");
        let result = reporter.report_metric(
            RunId::from_string("run-1"),
            MetricKey::from_string("train/loss"),
            None,
            0.125,
        );

        assert!(matches!(result, Err(EngineError::MetricQueueFull)));
        let diagnostics = reporter.diagnostics();
        assert_eq!(diagnostics.queue_full_errors, 1);
        assert_eq!(diagnostics.pending_reports, 1);
        assert_eq!(diagnostics.writer_state, "running");
    }

    #[test]
    fn report_metric_can_succeed_after_queue_full_error() {
        let (sender, receiver) = mpsc::sync_channel(1);
        let reporter = MetricReporter {
            inner: Arc::new(MetricReporterInner {
                sender: Mutex::new(Some(sender)),
                diagnostics: Arc::new(MetricReporterDiagnosticsInner::default()),
                worker: Mutex::new(None),
            }),
        };

        reporter
            .report_metric(
                RunId::from_string("run-1"),
                MetricKey::from_string("train/loss"),
                None,
                0.25,
            )
            .expect("first report should enter the queue");
        assert!(matches!(
            reporter.report_metric(
                RunId::from_string("run-1"),
                MetricKey::from_string("train/loss"),
                None,
                0.125,
            ),
            Err(EngineError::MetricQueueFull)
        ));

        receiver
            .try_recv()
            .expect("queued report should be available");
        reporter.inner.diagnostics.decrement_pending();
        reporter
            .report_metric(
                RunId::from_string("run-1"),
                MetricKey::from_string("train/loss"),
                None,
                0.0625,
            )
            .expect("later report should enter the queue after capacity is freed");

        let diagnostics = reporter.diagnostics();
        assert_eq!(diagnostics.queue_full_errors, 1);
        assert_eq!(diagnostics.pending_reports, 1);
    }

    #[test]
    fn drain_for_returns_false_when_pending_report_is_not_written() {
        let reporter = MetricReporter::blocked_for_test(1);
        reporter
            .report_metric(
                RunId::from_string("run-1"),
                MetricKey::from_string("train/loss"),
                None,
                0.25,
            )
            .expect("report should enter the queue");

        assert!(!reporter.drain_for(Duration::from_millis(1)));
        assert_eq!(reporter.diagnostics().pending_reports, 1);
    }

    #[test]
    fn shutdown_for_closes_report_sender() {
        let reporter = MetricReporter::blocked_for_test(1);

        assert!(reporter.shutdown_for(Duration::from_millis(1)));
        reporter
            .report_metric(
                RunId::from_string("run-1"),
                MetricKey::from_string("train/loss"),
                None,
                0.25,
            )
            .expect("closed reporter should retain current shutdown behavior");

        let diagnostics = reporter.diagnostics();
        assert_eq!(diagnostics.pending_reports, 0);
        assert_eq!(diagnostics.writer_state, "closed");
    }

    #[test]
    fn diagnostics_reports_drained_when_no_reports_are_pending() {
        let reporter = MetricReporter::blocked_for_test(1);

        let diagnostics = reporter.diagnostics();

        assert_eq!(diagnostics.pending_reports, 0);
        assert_eq!(diagnostics.writer_state, "drained");
        assert_eq!(diagnostics.last_write_error, None);
        assert_eq!(diagnostics.last_flush_status, "none");
        assert_eq!(diagnostics.last_flush_run_id, None);
        assert_eq!(diagnostics.last_flush_error, None);
    }
}
