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

const DEFAULT_METRIC_BUFFER_CAPACITY: usize = 4096;

#[derive(Clone)]
pub struct MetricReporter {
    inner: Arc<MetricReporterInner>,
}

impl MetricReporter {
    pub fn open(connection: Arc<Mutex<duckdb::Connection>>) -> Self {
        Self::open_with_capacity(connection, DEFAULT_METRIC_BUFFER_CAPACITY)
    }

    fn open_with_capacity(connection: Arc<Mutex<duckdb::Connection>>, capacity: usize) -> Self {
        let (sender, receiver) = mpsc::sync_channel(capacity);
        let diagnostics = Arc::new(MetricReporterDiagnosticsInner::default());
        let worker_diagnostics = Arc::clone(&diagnostics);
        let failure_diagnostics = Arc::clone(&diagnostics);
        let worker = std::thread::spawn(move || {
            let result = metric_worker(connection, receiver, worker_diagnostics);
            if result.is_err() {
                failure_diagnostics.increment_failed();
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
    ) {
        let report = MetricReport {
            run_id,
            metric_key,
            step,
            value_f64,
        };
        let sender = match self.sender() {
            Some(sender) => sender,
            None => {
                self.inner.diagnostics.increment_failed();
                return;
            }
        };
        self.inner.diagnostics.increment_pending();
        match sender.try_send(report) {
            Ok(()) => self.inner.diagnostics.increment_accepted(),
            Err(TrySendError::Full(_)) => {
                self.inner.diagnostics.decrement_pending();
                self.inner.diagnostics.increment_dropped();
            }
            Err(TrySendError::Disconnected(_)) => {
                self.inner.diagnostics.decrement_pending();
                self.inner.diagnostics.increment_failed();
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
        drained
    }

    #[cfg(test)]
    fn blocked_for_test(capacity: usize) -> Self {
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
    pub accepted_reports: u64,
    pub dropped_reports: u64,
    pub failed_reports: u64,
    pub pending_reports: u64,
    pub writer_drained: bool,
    pub last_write_error: Option<String>,
}

#[derive(Default)]
struct MetricReporterDiagnosticsInner {
    accepted_reports: AtomicU64,
    dropped_reports: AtomicU64,
    failed_reports: AtomicU64,
    pending_reports: AtomicU64,
    last_write_error: Mutex<Option<String>>,
}

impl MetricReporterDiagnosticsInner {
    fn increment_accepted(&self) {
        self.accepted_reports.fetch_add(1, Ordering::Relaxed);
    }

    fn increment_dropped(&self) {
        self.dropped_reports.fetch_add(1, Ordering::Relaxed);
    }

    fn increment_failed(&self) {
        self.failed_reports.fetch_add(1, Ordering::Relaxed);
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
        MetricReporterDiagnostics {
            accepted_reports: self.accepted_reports.load(Ordering::Relaxed),
            dropped_reports: self.dropped_reports.load(Ordering::Relaxed),
            failed_reports: self.failed_reports.load(Ordering::Relaxed),
            pending_reports,
            writer_drained: pending_reports == 0,
            last_write_error: self
                .last_write_error
                .lock()
                .ok()
                .and_then(|last_write_error| last_write_error.clone()),
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
    fn report_metric_drops_immediately_when_buffer_is_full() {
        let reporter = MetricReporter::blocked_for_test(1);

        reporter.report_metric(
            RunId::from_string("run-1"),
            MetricKey::from_string("train/loss"),
            None,
            0.25,
        );
        reporter.report_metric(
            RunId::from_string("run-1"),
            MetricKey::from_string("train/loss"),
            None,
            0.125,
        );

        let diagnostics = reporter.diagnostics();
        assert_eq!(diagnostics.accepted_reports, 1);
        assert_eq!(diagnostics.dropped_reports, 1);
        assert_eq!(diagnostics.pending_reports, 1);
        assert!(!diagnostics.writer_drained);
    }

    #[test]
    fn drain_for_returns_false_when_pending_report_is_not_written() {
        let reporter = MetricReporter::blocked_for_test(1);
        reporter.report_metric(
            RunId::from_string("run-1"),
            MetricKey::from_string("train/loss"),
            None,
            0.25,
        );

        assert!(!reporter.drain_for(Duration::from_millis(1)));
        assert_eq!(reporter.diagnostics().pending_reports, 1);
    }

    #[test]
    fn shutdown_for_closes_report_sender() {
        let reporter = MetricReporter::blocked_for_test(1);

        assert!(reporter.shutdown_for(Duration::from_millis(1)));
        reporter.report_metric(
            RunId::from_string("run-1"),
            MetricKey::from_string("train/loss"),
            None,
            0.25,
        );

        assert_eq!(reporter.diagnostics().failed_reports, 1);
    }

    #[test]
    fn diagnostics_reports_drained_when_no_reports_are_pending() {
        let reporter = MetricReporter::blocked_for_test(1);

        let diagnostics = reporter.diagnostics();

        assert_eq!(diagnostics.pending_reports, 0);
        assert!(diagnostics.writer_drained);
        assert_eq!(diagnostics.last_write_error, None);
    }
}
