use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, SyncSender, TrySendError};
use std::sync::{Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::engine::EngineError;
use crate::engine::time::timestamp_from_millis;
use crate::engine::write::NativeWriteStore;
use crate::model::metric::{MetricKey, Step};
use crate::model::run::RunId;

const DEFAULT_METRIC_BUFFER_CAPACITY: usize = 65_536;
const METRIC_BATCH_MAX_REPORTS: usize = 8_192;
const METRIC_BATCH_MAX_AGE: Duration = Duration::from_millis(10);
const WRITER_MAX_RETRIES: usize = 5;
const WRITER_INITIAL_RETRY_BACKOFF: Duration = Duration::from_millis(50);
const WRITER_MAX_RETRY_BACKOFF: Duration = Duration::from_millis(1_000);
const WRITER_RUNNING: u8 = 0;
const WRITER_RETRYING: u8 = 1;
const WRITER_FAILED: u8 = 2;
const WRITER_CLOSED: u8 = 3;

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
        let Some(sender) = self.sender() else {
            return Err(EngineError::ClientClosed);
        };
        if self.inner.diagnostics.is_writer_failed() {
            return Err(EngineError::MetricWriterFailed {
                message: self
                    .inner
                    .diagnostics
                    .last_write_error()
                    .unwrap_or_else(|| "metric writer failed".to_owned()),
            });
        }
        let report = MetricReport {
            run_id,
            metric_key,
            step,
            value_f64,
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
                Err(EngineError::ClientClosed)
            }
        }
    }

    pub fn diagnostics(&self) -> MetricReporterDiagnostics {
        self.inner.diagnostics.snapshot()
    }

    pub fn drain_for(&self, timeout: Duration) -> bool {
        self.drain(Some(timeout)).is_ok()
    }

    pub fn drain(&self, timeout: Option<Duration>) -> Result<(), EngineError> {
        let deadline = timeout.map(|timeout| Instant::now() + timeout);
        while self.inner.diagnostics.pending_reports() > 0 {
            if self.inner.diagnostics.is_writer_failed() {
                return Err(EngineError::MetricWriterFailed {
                    message: self
                        .inner
                        .diagnostics
                        .last_write_error()
                        .unwrap_or_else(|| "metric writer failed".to_owned()),
                });
            }
            if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
                return Err(EngineError::MetricDrainTimeout);
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        if self.inner.diagnostics.is_writer_failed() {
            return Err(EngineError::MetricWriterFailed {
                message: self
                    .inner
                    .diagnostics
                    .last_write_error()
                    .unwrap_or_else(|| "metric writer failed".to_owned()),
            });
        }
        Ok(())
    }

    pub fn shutdown_for(&self, timeout: Duration) -> bool {
        self.shutdown(Some(timeout)).is_ok()
    }

    pub fn shutdown(&self, timeout: Option<Duration>) -> Result<(), EngineError> {
        let drain_result = self.drain(timeout);
        if matches!(drain_result, Err(EngineError::MetricDrainTimeout)) {
            return drain_result;
        }
        if let Some(sender) = take_mutex_value(&self.inner.sender) {
            drop(sender);
        }
        if let Some(worker) = take_mutex_value(&self.inner.worker) {
            let _ = worker.join();
        }
        if drain_result.is_ok() {
            self.inner.diagnostics.set_writer_closed();
        }
        drain_result
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

    fn increment_persisted_by(&self, count: u64) {
        self.persisted_reports.fetch_add(count, Ordering::Relaxed);
    }

    fn set_writer_running(&self) {
        self.writer_state
            .store(WRITER_RUNNING.into(), Ordering::Relaxed);
    }

    fn set_writer_retrying(&self) {
        self.writer_state
            .store(WRITER_RETRYING.into(), Ordering::Relaxed);
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

    fn last_write_error(&self) -> Option<String> {
        self.last_write_error
            .lock()
            .ok()
            .and_then(|last_write_error| last_write_error.clone())
    }

    fn is_writer_failed(&self) -> bool {
        self.writer_state.load(Ordering::Relaxed) as u8 == WRITER_FAILED
    }

    fn increment_pending(&self) {
        self.pending_reports.fetch_add(1, Ordering::Relaxed);
    }

    fn decrement_pending(&self) {
        self.pending_reports.fetch_sub(1, Ordering::Relaxed);
    }

    fn decrement_pending_by(&self, count: u64) {
        self.pending_reports.fetch_sub(count, Ordering::Relaxed);
    }

    fn pending_reports(&self) -> u64 {
        self.pending_reports.load(Ordering::Relaxed)
    }

    fn snapshot(&self) -> MetricReporterDiagnostics {
        let pending_reports = self.pending_reports();
        let writer_state = match self.writer_state.load(Ordering::Relaxed) as u8 {
            WRITER_RETRYING => "retrying",
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
    let mut next_ingested_at_millis = chrono::Utc::now().timestamp_millis();
    while let Ok(first_report) = receiver.recv() {
        let mut batch = vec![first_report];
        collect_metric_batch(&receiver, &mut batch);
        let batch_len = u64::try_from(batch.len()).unwrap_or(u64::MAX);
        match write_metric_batch_with_retries(
            &connection,
            &batch,
            next_ingested_at_millis,
            &diagnostics,
            std::thread::sleep,
        ) {
            Ok(next_millis) => {
                next_ingested_at_millis = next_millis;
                diagnostics.increment_persisted_by(batch_len);
                diagnostics.decrement_pending_by(batch_len);
            }
            Err(error) => {
                diagnostics.set_last_write_error(error.to_string());
                diagnostics.set_writer_failed();
                return Err(EngineError::MetricWriterFailed {
                    message: error.to_string(),
                });
            }
        }
    }
    Ok(())
}

fn collect_metric_batch(receiver: &mpsc::Receiver<MetricReport>, batch: &mut Vec<MetricReport>) {
    let deadline = Instant::now() + METRIC_BATCH_MAX_AGE;
    while batch.len() < METRIC_BATCH_MAX_REPORTS {
        match receiver.try_recv() {
            Ok(report) => batch.push(report),
            Err(mpsc::TryRecvError::Empty) => {
                let now = Instant::now();
                if now >= deadline {
                    return;
                }
                match receiver.recv_timeout(deadline - now) {
                    Ok(report) => batch.push(report),
                    Err(mpsc::RecvTimeoutError::Timeout | mpsc::RecvTimeoutError::Disconnected) => {
                        return;
                    }
                }
            }
            Err(mpsc::TryRecvError::Disconnected) => return,
        }
    }
}

fn write_metric_batch_with_retries(
    connection: &Arc<Mutex<duckdb::Connection>>,
    batch: &[MetricReport],
    next_ingested_at_millis: i64,
    diagnostics: &MetricReporterDiagnosticsInner,
    sleep: impl FnMut(Duration),
) -> Result<i64, EngineError> {
    retry_metric_batch_write(
        || write_metric_batch(connection, batch, next_ingested_at_millis),
        diagnostics,
        sleep,
    )
}

fn retry_metric_batch_write(
    mut write: impl FnMut() -> Result<i64, EngineError>,
    diagnostics: &MetricReporterDiagnosticsInner,
    mut sleep: impl FnMut(Duration),
) -> Result<i64, EngineError> {
    let mut retry_count = 0;
    let mut backoff = WRITER_INITIAL_RETRY_BACKOFF;
    loop {
        match write() {
            Ok(next_millis) => {
                diagnostics.set_writer_running();
                return Ok(next_millis);
            }
            Err(error) if retry_count < WRITER_MAX_RETRIES => {
                diagnostics.set_last_write_error(error.to_string());
                diagnostics.set_writer_retrying();
                sleep(backoff);
                retry_count += 1;
                backoff = (backoff * 2).min(WRITER_MAX_RETRY_BACKOFF);
            }
            Err(error) => return Err(error),
        }
    }
}

fn write_metric_batch(
    connection: &Arc<Mutex<duckdb::Connection>>,
    batch: &[MetricReport],
    next_ingested_at_millis: i64,
) -> Result<i64, EngineError> {
    let connection = connection
        .lock()
        .map_err(|_| EngineError::ConnectionLockPoisoned)?;
    let store = NativeWriteStore::new(&connection);
    let mut ingested_at_millis = next_ingested_at_millis.max(chrono::Utc::now().timestamp_millis());
    for report in batch {
        match report.step {
            Some(step) => {
                let timestamp = timestamp_from_millis("timestamp", ingested_at_millis)?;
                let ingested_at = timestamp_from_millis("ingested_at", ingested_at_millis)?;
                store.log_metric_at_step_with_timestamps(
                    &report.run_id,
                    &report.metric_key,
                    step,
                    report.value_f64,
                    timestamp,
                    ingested_at,
                )?;
            }
            None => {
                store.log_metric(&report.run_id, &report.metric_key, report.value_f64)?;
            }
        }
        ingested_at_millis += 1;
    }
    Ok(ingested_at_millis)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::bootstrap::open_native_connection;
    use crate::model::types::ProjectId;

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
        let result = reporter.report_metric(
            RunId::from_string("run-1"),
            MetricKey::from_string("train/loss"),
            None,
            0.25,
        );

        assert!(matches!(result, Err(EngineError::ClientClosed)));
        let diagnostics = reporter.diagnostics();
        assert_eq!(diagnostics.pending_reports, 0);
        assert_eq!(diagnostics.writer_state, "closed");
    }

    #[test]
    fn shutdown_timeout_keeps_admission_open() {
        let reporter = MetricReporter::blocked_for_test(2);
        reporter
            .report_metric(
                RunId::from_string("run-1"),
                MetricKey::from_string("train/loss"),
                None,
                0.25,
            )
            .expect("report should enter the queue");

        let shutdown = reporter.shutdown(Duration::from_millis(1).into());

        assert!(matches!(shutdown, Err(EngineError::MetricDrainTimeout)));
        reporter
            .report_metric(
                RunId::from_string("run-1"),
                MetricKey::from_string("train/loss"),
                None,
                0.125,
            )
            .expect("shutdown timeout should leave admission open");

        let diagnostics = reporter.diagnostics();
        assert_eq!(diagnostics.pending_reports, 2);
        assert_eq!(diagnostics.writer_state, "running");
    }

    #[test]
    fn shutdown_after_writer_failure_releases_sender_without_closing_diagnostics() {
        let reporter = MetricReporter::blocked_for_test(1);
        reporter
            .report_metric(
                RunId::from_string("run-1"),
                MetricKey::from_string("train/loss"),
                None,
                0.25,
            )
            .expect("report should enter the queue");
        reporter
            .inner
            .diagnostics
            .set_last_write_error("append metric batch failed".to_owned());
        reporter.inner.diagnostics.set_writer_failed();

        let shutdown = reporter.shutdown(None);
        let repeated_shutdown = reporter.shutdown(None);
        let log_after_shutdown = reporter.report_metric(
            RunId::from_string("run-1"),
            MetricKey::from_string("train/loss"),
            None,
            0.125,
        );

        assert!(matches!(
            shutdown,
            Err(EngineError::MetricWriterFailed { .. })
        ));
        assert!(matches!(
            repeated_shutdown,
            Err(EngineError::MetricWriterFailed { .. })
        ));
        assert!(matches!(log_after_shutdown, Err(EngineError::ClientClosed)));
        let diagnostics = reporter.diagnostics();
        assert_eq!(diagnostics.pending_reports, 1);
        assert_eq!(diagnostics.writer_state, "failed");
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

    #[test]
    fn report_metric_returns_writer_failed_after_failure_latches() {
        let reporter = MetricReporter::blocked_for_test(1);
        reporter
            .inner
            .diagnostics
            .set_last_write_error("append metric batch failed".to_owned());
        reporter.inner.diagnostics.set_writer_failed();

        let result = reporter.report_metric(
            RunId::from_string("run-1"),
            MetricKey::from_string("train/loss"),
            Some(Step::new(0)),
            0.25,
        );

        assert!(
            matches!(result, Err(EngineError::MetricWriterFailed { .. })),
            "expected latched writer failure, got {result:?}",
        );
        assert_eq!(reporter.diagnostics().pending_reports, 0);
    }

    #[test]
    fn retry_metric_batch_write_retries_five_times_before_error() {
        let diagnostics = MetricReporterDiagnosticsInner::default();
        let mut attempts = 0;
        let mut sleeps = Vec::new();

        let result = retry_metric_batch_write(
            || {
                attempts += 1;
                Err(EngineError::InvalidTimestamp {
                    field: "ingested_at",
                    millis: -1,
                })
            },
            &diagnostics,
            |duration| sleeps.push(duration),
        );

        assert!(matches!(result, Err(EngineError::InvalidTimestamp { .. })));
        assert_eq!(attempts, WRITER_MAX_RETRIES + 1);
        assert_eq!(
            sleeps,
            vec![
                Duration::from_millis(50),
                Duration::from_millis(100),
                Duration::from_millis(200),
                Duration::from_millis(400),
                Duration::from_millis(800),
            ],
        );
        assert_eq!(diagnostics.snapshot().writer_state, "retrying");
        assert!(
            diagnostics
                .snapshot()
                .last_write_error
                .as_deref()
                .is_some_and(|message| message.contains("invalid stored timestamp"))
        );
    }

    #[test]
    fn retry_metric_batch_write_recovers_after_transient_errors() {
        let diagnostics = MetricReporterDiagnosticsInner::default();
        let mut attempts = 0;
        let mut sleeps = Vec::new();

        let next_millis = retry_metric_batch_write(
            || {
                attempts += 1;
                if attempts < 3 {
                    return Err(EngineError::InvalidTimestamp {
                        field: "ingested_at",
                        millis: -1,
                    });
                }
                Ok(42)
            },
            &diagnostics,
            |duration| sleeps.push(duration),
        )
        .expect("transient writer errors should recover before retry exhaustion");

        assert_eq!(next_millis, 42);
        assert_eq!(attempts, 3);
        assert_eq!(
            sleeps,
            vec![Duration::from_millis(50), Duration::from_millis(100)]
        );
        assert_eq!(diagnostics.snapshot().writer_state, "drained");
        assert!(
            diagnostics.snapshot().last_write_error.is_some(),
            "last write error should retain the most recent retry error"
        );
    }

    #[test]
    fn collect_metric_batch_stops_at_report_threshold() {
        let (sender, receiver) = mpsc::sync_channel(METRIC_BATCH_MAX_REPORTS + 1);
        for step in 0..=METRIC_BATCH_MAX_REPORTS {
            sender
                .try_send(metric_report(step))
                .expect("test channel should have capacity for queued reports");
        }

        let mut batch = vec![receiver.recv().expect("first report should be queued")];
        collect_metric_batch(&receiver, &mut batch);

        assert_eq!(batch.len(), METRIC_BATCH_MAX_REPORTS);
        assert!(
            receiver.try_recv().is_ok(),
            "one report should remain after the size-capped batch"
        );
    }

    #[test]
    fn write_metric_batch_assigns_ingested_at_in_enqueue_order()
    -> Result<(), Box<dyn std::error::Error>> {
        let root_path =
            std::env::temp_dir().join(format!("pulseon-reporter-{}", uuid::Uuid::new_v4()));
        let connection = Arc::new(Mutex::new(open_native_connection(&root_path)?));
        {
            let connection = connection.lock().expect("test connection lock");
            connection.execute(
                "INSERT INTO dl.pulseon_projects (project_id, name, created_at)
                 VALUES ('project-1', 'local training', now())",
                [],
            )?;
            NativeWriteStore::new(&connection).create_run(
                &ProjectId::from_string("project-1"),
                "baseline",
                Some(RunId::from_string("run-1")),
            )?;
        }
        let batch = vec![metric_report(0), metric_report(1), metric_report(2)];
        let first_ingested_at_millis = 1_700_000_000_000;

        let next_ingested_at_millis =
            write_metric_batch(&connection, &batch, first_ingested_at_millis)?;

        let connection = connection.lock().expect("test connection lock");
        let stored: Vec<(i64, i64)> = connection
            .prepare(
                "SELECT step, epoch_ms(ingested_at)
                 FROM dl.metric_points
                 WHERE run_id = 'run-1'
                 ORDER BY ingested_at",
            )?
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<Result<_, _>>()?;
        assert_eq!(
            stored.iter().map(|(step, _)| *step).collect::<Vec<_>>(),
            vec![0, 1, 2]
        );
        assert_eq!(stored[1].1, stored[0].1 + 1);
        assert_eq!(stored[2].1, stored[1].1 + 1);
        assert_eq!(next_ingested_at_millis, stored[2].1 + 1);
        assert!(stored[0].1 >= first_ingested_at_millis);
        std::fs::remove_dir_all(root_path)?;
        Ok(())
    }

    fn metric_report(step: usize) -> MetricReport {
        MetricReport {
            run_id: RunId::from_string("run-1"),
            metric_key: MetricKey::from_string("train/loss"),
            step: Some(Step::new(
                i64::try_from(step).expect("test step should fit i64"),
            )),
            value_f64: step as f64,
        }
    }
}
