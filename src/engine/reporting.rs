use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, SyncSender, TrySendError};
use std::sync::{Mutex, MutexGuard};
use std::thread::JoinHandle;

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
        let worker = std::thread::spawn(move || {
            let result = metric_worker(connection, receiver);
            if result.is_err() {
                worker_diagnostics.increment_failed();
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
        match sender.try_send(report) {
            Ok(()) => self.inner.diagnostics.increment_accepted(),
            Err(TrySendError::Full(_)) => self.inner.diagnostics.increment_dropped(),
            Err(TrySendError::Disconnected(_)) => self.inner.diagnostics.increment_failed(),
        }
    }

    pub fn diagnostics(&self) -> MetricReporterDiagnostics {
        self.inner.diagnostics.snapshot()
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

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct MetricReporterDiagnostics {
    pub accepted_reports: u64,
    pub dropped_reports: u64,
    pub failed_reports: u64,
}

#[derive(Default)]
struct MetricReporterDiagnosticsInner {
    accepted_reports: AtomicU64,
    dropped_reports: AtomicU64,
    failed_reports: AtomicU64,
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

    fn snapshot(&self) -> MetricReporterDiagnostics {
        MetricReporterDiagnostics {
            accepted_reports: self.accepted_reports.load(Ordering::Relaxed),
            dropped_reports: self.dropped_reports.load(Ordering::Relaxed),
            failed_reports: self.failed_reports.load(Ordering::Relaxed),
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
) -> Result<(), EngineError> {
    for report in receiver {
        let connection = connection
            .lock()
            .map_err(|_| EngineError::ConnectionLockPoisoned)?;
        let store = NativeWriteStore::new(&connection);
        let result = match report.step {
            Some(step) => {
                store.log_metric_at_step(&report.run_id, &report.metric_key, step, report.value_f64)
            }
            None => store.log_metric(&report.run_id, &report.metric_key, report.value_f64),
        };
        result?;
    }
    Ok(())
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
    }
}
