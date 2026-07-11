use std::collections::HashMap;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, TryLockError};
use std::time::{Duration, Instant};

use crate::engine::EngineError;
use crate::engine::bootstrap::{
    CatalogBackend, NativeStorageConfig, S3ConnectionConfig, open_native_connection_with_config,
};
use crate::engine::query::NativeQueryStore;
use crate::engine::reporting::{MetricReporter, MetricReporterDiagnostics};
use crate::engine::time::{current_timestamp, timestamp_as_rfc3339};
use crate::engine::write::NativeWriteStore;
use crate::engine::write_rows::status_as_str;
use crate::model::metric::{MetricAggregate, MetricKey, MetricPoint, Step};
use crate::model::run::{Run, RunId, RunStatus};
use crate::model::types::{Project, ProjectId};

pub struct NativeClient {
    root_path: PathBuf,
    reporter: MetricReporter,
    connection: Arc<Mutex<duckdb::Connection>>,
    active_runs: Arc<Mutex<HashMap<RunId, Arc<ActiveRun>>>>,
    flush_lock: Arc<Mutex<()>>,
    is_shutdown: AtomicBool,
}

impl NativeClient {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, EngineError> {
        Self::open_with_metric_queue_capacity(path, 65_536)
    }

    pub fn open_with_metric_queue_capacity(
        path: impl AsRef<Path>,
        metric_queue_capacity: usize,
    ) -> Result<Self, EngineError> {
        Self::open_with_storage_config(path, None, None, metric_queue_capacity)
    }

    pub fn open_with_storage_config(
        path: impl AsRef<Path>,
        catalog_path: Option<PathBuf>,
        data_path: Option<PathBuf>,
        metric_queue_capacity: usize,
    ) -> Result<Self, EngineError> {
        Self::open_with_catalog_backend_storage_config(
            path,
            CatalogBackend::DuckDb,
            catalog_path,
            data_path,
            None,
            metric_queue_capacity,
        )
    }

    pub(crate) fn open_with_catalog_backend_storage_config(
        path: impl AsRef<Path>,
        catalog_backend: CatalogBackend,
        catalog_path: Option<PathBuf>,
        data_path: Option<PathBuf>,
        s3_connection: Option<S3ConnectionConfig>,
        metric_queue_capacity: usize,
    ) -> Result<Self, EngineError> {
        let root_path = path.as_ref().to_path_buf();
        let storage_config = NativeStorageConfig::with_backend_and_s3_config(
            catalog_backend,
            &root_path,
            catalog_path,
            data_path,
            s3_connection,
        );
        let connection = open_native_connection_with_config(storage_config)?;
        let connection = Arc::new(Mutex::new(connection));
        let reporter =
            MetricReporter::open_with_capacity(Arc::clone(&connection), metric_queue_capacity);

        Ok(Self {
            root_path,
            reporter,
            connection,
            active_runs: Arc::new(Mutex::new(HashMap::new())),
            flush_lock: Arc::new(Mutex::new(())),
            is_shutdown: AtomicBool::new(false),
        })
    }

    pub fn create_project(
        &self,
        name: &str,
        project_id: Option<ProjectId>,
    ) -> Result<Project, EngineError> {
        self.ensure_writeable()?;
        let project_id =
            project_id.unwrap_or_else(|| ProjectId::from_string(uuid::Uuid::new_v4().to_string()));
        if self.project_exists(&project_id)? {
            return Err(EngineError::ProjectAlreadyExists {
                project_id: project_id.as_str().to_owned(),
            });
        }

        let created_at = current_timestamp("created_at")?;
        let connection = self.connection()?;
        connection.execute(
            "INSERT INTO pulseon_projects (project_id, name, created_at)
             VALUES (?, ?, ?)",
            (project_id.as_str(), name, timestamp_as_rfc3339(created_at)),
        )?;

        Ok(Project {
            project_id,
            name: name.to_owned(),
            created_at,
        })
    }

    pub fn get_project(&self, project_id: &ProjectId) -> Result<Project, EngineError> {
        let connection = self.connection()?;
        let result = connection.query_row(
            "SELECT project_id, name, epoch_ms(created_at::TIMESTAMPTZ)
             FROM pulseon_projects
             WHERE project_id = ?",
            [project_id.as_str()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            },
        );
        let (project_id, name, created_at_millis) = match result {
            Ok(stored) => stored,
            Err(duckdb::Error::QueryReturnedNoRows) => {
                return Err(EngineError::ProjectNotFound {
                    project_id: project_id.as_str().to_owned(),
                });
            }
            Err(source) => return Err(source.into()),
        };

        Ok(Project {
            project_id: ProjectId::from_string(project_id),
            name,
            created_at: crate::engine::time::timestamp_from_millis(
                "created_at",
                created_at_millis,
            )?,
        })
    }

    pub fn list_projects(&self) -> Result<Vec<Project>, EngineError> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT project_id, name, epoch_ms(created_at::TIMESTAMPTZ)
             FROM pulseon_projects
             ORDER BY created_at, project_id",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })?;

        rows.map(|row| {
            let (project_id, name, created_at_millis) = row?;
            Ok(Project {
                project_id: ProjectId::from_string(project_id),
                name,
                created_at: crate::engine::time::timestamp_from_millis(
                    "created_at",
                    created_at_millis,
                )?,
            })
        })
        .collect()
    }

    pub fn create_run(
        &self,
        project_id: &ProjectId,
        name: &str,
        run_id: Option<RunId>,
    ) -> Result<Run, EngineError> {
        self.ensure_writeable()?;
        if !self.project_exists(project_id)? {
            return Err(EngineError::ProjectNotFound {
                project_id: project_id.as_str().to_owned(),
            });
        }

        let run_id = run_id.unwrap_or_else(|| RunId::from_string(uuid::Uuid::new_v4().to_string()));
        match self.get_run(&run_id) {
            Ok(_) => {
                return Err(EngineError::RunAlreadyExists {
                    run_id: run_id.as_str().to_owned(),
                });
            }
            Err(EngineError::RunNotFound { .. }) => {}
            Err(error) => return Err(error),
        }
        let active_run = self.acquire_run_writer(&run_id)?;
        let connection = self.connection()?;
        let created =
            NativeWriteStore::new(&connection).create_run(project_id, name, Some(run_id.clone()));
        drop(connection);
        if created.is_err() {
            self.release_run_writer(&run_id);
        }
        created.inspect_err(|_| active_run.release_lock())
    }

    pub fn get_run(&self, run_id: &RunId) -> Result<Run, EngineError> {
        let connection = self.connection()?;
        NativeWriteStore::new(&connection).resume_run(run_id)
    }

    pub fn resume_run(&self, run_id: &RunId) -> Result<Run, EngineError> {
        self.ensure_writeable()?;
        let run = self.get_run(run_id)?;
        if run.status != RunStatus::Running {
            return Err(EngineError::InvalidRunTransition {
                run_id: run_id.as_str().to_owned(),
                from: run_status_value(run.status),
                to: run_status_value(RunStatus::Running),
            });
        }
        self.acquire_run_writer(run_id)?;
        Ok(run)
    }

    pub fn list_runs(&self, project_id: &ProjectId) -> Result<Vec<Run>, EngineError> {
        self.list_runs_filtered(project_id, None, None, 0)
    }

    pub fn list_runs_filtered(
        &self,
        project_id: &ProjectId,
        status: Option<RunStatus>,
        limit: Option<usize>,
        offset: usize,
    ) -> Result<Vec<Run>, EngineError> {
        if !self.project_exists(project_id)? {
            return Err(EngineError::ProjectNotFound {
                project_id: project_id.as_str().to_owned(),
            });
        }

        let run_ids = {
            let connection = self.connection()?;
            let mut sql = String::from(
                "SELECT run_id
                 FROM pulseon_runs
                 WHERE project_id = ?
                   AND status = COALESCE(?, status)
                 ORDER BY created_at, run_id",
            );
            if let Some(limit) = limit {
                sql.push_str(&format!(" LIMIT {limit}"));
            } else if offset > 0 {
                sql.push_str(" LIMIT ALL");
            }
            if offset > 0 {
                sql.push_str(&format!(" OFFSET {offset}"));
            }
            let mut statement = connection.prepare(&sql)?;
            let rows = statement
                .query_map((project_id.as_str(), status.map(status_as_str)), |row| {
                    Ok(RunId::from_string(row.get::<_, String>(0)?))
                })?;
            rows.collect::<Result<Vec<_>, _>>()?
        };

        run_ids
            .iter()
            .map(|run_id| self.get_run(run_id))
            .collect::<Result<Vec<_>, _>>()
    }

    pub fn list_orphan_runs(
        &self,
        project_id: Option<&ProjectId>,
    ) -> Result<Vec<Run>, EngineError> {
        if let Some(project_id) = project_id
            && !self.project_exists(project_id)?
        {
            return Err(EngineError::ProjectNotFound {
                project_id: project_id.as_str().to_owned(),
            });
        }

        let run_ids = {
            let connection = self.connection()?;
            match project_id {
                Some(project_id) => {
                    let mut statement = connection.prepare(
                        "SELECT run_id
                         FROM pulseon_runs
                         WHERE project_id = ?
                           AND status = 'running'
                         ORDER BY created_at, run_id",
                    )?;
                    let rows = statement.query_map([project_id.as_str()], |row| {
                        Ok(RunId::from_string(row.get::<_, String>(0)?))
                    })?;
                    rows.collect::<Result<Vec<_>, _>>()?
                }
                None => {
                    let mut statement = connection.prepare(
                        "SELECT run_id
                         FROM pulseon_runs
                         WHERE status = 'running'
                         ORDER BY created_at, run_id",
                    )?;
                    let rows = statement
                        .query_map([], |row| Ok(RunId::from_string(row.get::<_, String>(0)?)))?;
                    rows.collect::<Result<Vec<_>, _>>()?
                }
            }
        };

        run_ids
            .iter()
            .map(|run_id| self.get_run(run_id))
            .collect::<Result<Vec<_>, _>>()
    }

    pub fn finish_run(&self, run_id: &RunId) -> Result<Run, EngineError> {
        self.finish_run_with_timeout(run_id, None)
    }

    pub fn finish_run_with_timeout(
        &self,
        run_id: &RunId,
        timeout: Option<Duration>,
    ) -> Result<Run, EngineError> {
        self.finalize_run(run_id, RunStatus::Finished, timeout)
    }

    pub fn fail_run(&self, run_id: &RunId) -> Result<Run, EngineError> {
        self.fail_run_with_timeout(run_id, None)
    }

    pub fn fail_run_with_timeout(
        &self,
        run_id: &RunId,
        timeout: Option<Duration>,
    ) -> Result<Run, EngineError> {
        self.finalize_run(run_id, RunStatus::Failed, timeout)
    }

    pub fn flush_run_data(
        &self,
        run_id: &RunId,
        timeout: Option<Duration>,
    ) -> Result<(), EngineError> {
        self.ensure_writeable()?;
        let run = self.get_run(run_id)?;
        if run.status == RunStatus::Running {
            return Err(EngineError::InvalidRunTransition {
                run_id: run_id.as_str().to_owned(),
                from: run_status_value(run.status),
                to: "flushed",
            });
        }

        let deadline = timeout.map(|timeout| Instant::now() + timeout);
        self.reporter.set_flush_running(run_id);
        let _flush_guard = match self.acquire_flush_lock(deadline) {
            Ok(guard) => guard,
            Err(error) => {
                self.reporter.set_flush_timed_out(run_id);
                return Err(error);
            }
        };
        if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
            self.reporter.set_flush_timed_out(run_id);
            return Err(EngineError::MetricFlushTimeout);
        }

        let connection = self.connection()?;
        let flush_interrupt = install_flush_interrupt(&connection, deadline);
        let result = connection.execute_batch(
            "CALL ducklake_flush_inlined_data('dl', table_name => 'metric_points');",
        );
        if let Some(flush_interrupt) = &flush_interrupt {
            flush_interrupt.mark_completed();
        }
        drop(connection);
        if deadline.is_some_and(|deadline| Instant::now() >= deadline)
            || flush_interrupt
                .as_ref()
                .is_some_and(FlushInterrupt::timed_out)
        {
            self.reporter.set_flush_timed_out(run_id);
            return Err(EngineError::MetricFlushTimeout);
        }
        match result {
            Ok(()) => {
                self.reporter.set_flush_succeeded(run_id);
                Ok(())
            }
            Err(_source) => {
                let message = "flush metric_points failed".to_owned();
                self.reporter.set_flush_failed(run_id, message.clone());
                Err(EngineError::MetricFlush { message })
            }
        }
    }

    pub fn shutdown(&self, timeout: Option<Duration>) -> Result<(), EngineError> {
        let result = self.reporter.shutdown(timeout);
        if !matches!(result, Err(EngineError::MetricDrainTimeout)) {
            self.release_all_run_writers();
        }
        if result.is_ok() {
            self.is_shutdown.store(true, Ordering::Release);
        }
        result
    }

    pub fn run_handle(&self, run: Run) -> NativeRun {
        let active_run = self
            .active_runs
            .lock()
            .ok()
            .and_then(|active_runs| active_runs.get(&run.run_id).cloned())
            .unwrap_or_else(ActiveRun::closed);
        NativeRun {
            run_id: run.run_id,
            project_id: run.project_id,
            name: run.name,
            status: run.status,
            created_at: run.created_at,
            started_at: run.started_at,
            finished_at: run.finished_at,
            reporter: self.reporter.clone(),
            active_run,
        }
    }

    pub fn diagnostics(&self) -> MetricReporterDiagnostics {
        self.reporter.diagnostics()
    }

    pub fn query_metric(
        &self,
        run_id: &RunId,
        metric_key: &MetricKey,
        start_step: Option<Step>,
        end_step: Option<Step>,
        max_points: Option<usize>,
    ) -> Result<Vec<MetricPoint>, EngineError> {
        let connection = self.connection()?;
        NativeQueryStore::new(&connection)
            .query_metric(run_id, metric_key, start_step, end_step, max_points)
    }

    pub fn query_metric_summaries(
        &self,
        run_ids: &[RunId],
        metric_key: &MetricKey,
    ) -> Result<Vec<MetricAggregate>, EngineError> {
        let connection = self.connection()?;
        NativeQueryStore::new(&connection).query_metric_summaries(run_ids, metric_key)
    }

    pub fn list_metrics(&self, run_id: &RunId) -> Result<Vec<MetricAggregate>, EngineError> {
        self.get_run(run_id)?;
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT run_id, metric_key, effective_count, last_step, last_value_f64,
                    min_value_f64, max_value_f64
             FROM pulseon_metric_aggregates
             WHERE run_id = ?
             ORDER BY metric_key",
        )?;
        let rows = statement.query_map([run_id.as_str()], |row| {
            Ok(MetricAggregate {
                run_id: RunId::from_string(row.get::<_, String>(0)?),
                metric_key: MetricKey::from_string(row.get::<_, String>(1)?),
                effective_count: row.get(2)?,
                last_step: Step::new(row.get(3)?),
                last_value_f64: row.get(4)?,
                min_value_f64: row.get(5)?,
                max_value_f64: row.get(6)?,
            })
        })?;

        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    fn project_exists(&self, project_id: &ProjectId) -> Result<bool, EngineError> {
        let connection = self.connection()?;
        let exists = connection.query_row(
            "SELECT EXISTS (
                 SELECT 1
                 FROM pulseon_projects
                 WHERE project_id = ?
             )",
            [project_id.as_str()],
            |row| row.get(0),
        )?;
        Ok(exists)
    }

    fn finalize_run(
        &self,
        run_id: &RunId,
        target_status: RunStatus,
        timeout: Option<Duration>,
    ) -> Result<Run, EngineError> {
        self.ensure_writeable()?;
        let run = self.get_run(run_id)?;
        if run.status != RunStatus::Running {
            return Err(EngineError::InvalidRunTransition {
                run_id: run_id.as_str().to_owned(),
                from: run_status_value(run.status),
                to: run_status_value(target_status),
            });
        }
        let active_run = self.acquire_run_writer(run_id)?;
        active_run.close_admission()?;
        if let Err(error) = self.reporter.drain(timeout) {
            active_run.open_admission()?;
            return Err(error);
        }
        {
            let connection = self.connection()?;
            if let Err(error) =
                NativeWriteStore::new(&connection).rebuild_metric_aggregates_for_run(run_id)
            {
                drop(connection);
                active_run.open_admission()?;
                return Err(error);
            }
        }
        let finished_at = current_timestamp("finished_at")?;
        let connection = self.connection()?;
        let updated = connection.execute(
            "UPDATE pulseon_runs
             SET status = ?,
                 finished_at = ?
             WHERE run_id = ?
               AND status = 'running'",
            (
                run_status_value(target_status),
                timestamp_as_rfc3339(finished_at),
                run_id.as_str(),
            ),
        )?;
        drop(connection);

        if updated > 0 {
            active_run.mark_terminal()?;
            active_run.release_lock();
            self.release_run_writer(run_id);
            self.flush_run_data(run_id, None)?;
            return self.get_run(run_id);
        }

        let run = self.get_run(run_id)?;
        active_run.open_admission()?;
        Err(EngineError::InvalidRunTransition {
            run_id: run_id.as_str().to_owned(),
            from: run_status_value(run.status),
            to: run_status_value(target_status),
        })
    }

    fn connection(&self) -> Result<MutexGuard<'_, duckdb::Connection>, EngineError> {
        self.connection
            .lock()
            .map_err(|_| EngineError::ConnectionLockPoisoned)
    }

    fn ensure_writeable(&self) -> Result<(), EngineError> {
        if self.is_shutdown.load(Ordering::Acquire) {
            return Err(EngineError::ClientClosed);
        }
        Ok(())
    }

    fn acquire_flush_lock(
        &self,
        deadline: Option<Instant>,
    ) -> Result<MutexGuard<'_, ()>, EngineError> {
        let Some(deadline) = deadline else {
            return self
                .flush_lock
                .lock()
                .map_err(|_| EngineError::ConnectionLockPoisoned);
        };
        loop {
            match self.flush_lock.try_lock() {
                Ok(guard) => return Ok(guard),
                Err(TryLockError::Poisoned(_)) => return Err(EngineError::ConnectionLockPoisoned),
                Err(TryLockError::WouldBlock) => {
                    if Instant::now() >= deadline {
                        return Err(EngineError::MetricFlushTimeout);
                    }
                    std::thread::sleep(Duration::from_millis(1));
                }
            }
        }
    }

    fn acquire_run_writer(&self, run_id: &RunId) -> Result<Arc<ActiveRun>, EngineError> {
        let mut active_runs = self
            .active_runs
            .lock()
            .map_err(|_| EngineError::ConnectionLockPoisoned)?;
        if let Some(active_run) = active_runs.get(run_id) {
            return Ok(Arc::clone(active_run));
        }

        let lock_dir = self.root_path.join(".pulseon").join("locks").join("runs");
        std::fs::create_dir_all(&lock_dir).map_err(|source| EngineError::Storage {
            operation: "creating run lock directory",
            name: path_basename(&lock_dir),
            source,
        })?;
        let lock_path = lock_dir.join(format!(
            "{}.lock",
            percent_encode_path_segment(run_id.as_str())
        ));
        let lock_file = File::options()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock_path)
            .map_err(|source| EngineError::Storage {
                operation: "opening run lock file",
                name: path_basename(&lock_path),
                source,
            })?;
        match lock_file.try_lock() {
            Ok(()) => {
                let active_run = Arc::new(ActiveRun::open(lock_file));
                active_runs.insert(run_id.clone(), Arc::clone(&active_run));
                Ok(active_run)
            }
            Err(std::fs::TryLockError::WouldBlock) => Err(EngineError::RunAlreadyActive {
                run_id: run_id.as_str().to_owned(),
            }),
            Err(source) => Err(EngineError::Storage {
                operation: "locking run lock file",
                name: path_basename(&lock_path),
                source: source.into(),
            }),
        }
    }

    fn release_run_writer(&self, run_id: &RunId) {
        if let Ok(mut active_runs) = self.active_runs.lock()
            && let Some(active_run) = active_runs.remove(run_id)
        {
            active_run.release_lock();
        }
    }

    fn release_all_run_writers(&self) {
        if let Ok(mut active_runs) = self.active_runs.lock() {
            for (_, active_run) in active_runs.drain() {
                active_run.release_lock();
            }
        }
    }
}

struct FlushInterrupt {
    completed: Arc<AtomicBool>,
    timed_out: Arc<AtomicBool>,
}

impl FlushInterrupt {
    fn mark_completed(&self) {
        self.completed.store(true, Ordering::Relaxed);
    }

    fn timed_out(&self) -> bool {
        self.timed_out.load(Ordering::Relaxed)
    }
}

fn install_flush_interrupt(
    connection: &duckdb::Connection,
    deadline: Option<Instant>,
) -> Option<FlushInterrupt> {
    let deadline = deadline?;
    let remaining = deadline.saturating_duration_since(Instant::now());
    let completed = Arc::new(AtomicBool::new(false));
    let timed_out = Arc::new(AtomicBool::new(false));
    let timer_completed = Arc::clone(&completed);
    let timer_timed_out = Arc::clone(&timed_out);
    let interrupt = connection.interrupt_handle();
    std::thread::spawn(move || {
        std::thread::sleep(remaining);
        if !timer_completed.load(Ordering::Relaxed) {
            timer_timed_out.store(true, Ordering::Relaxed);
            interrupt.interrupt();
        }
    });
    Some(FlushInterrupt {
        completed,
        timed_out,
    })
}

const fn run_status_value(status: RunStatus) -> &'static str {
    match status {
        RunStatus::Running => "running",
        RunStatus::Finished => "finished",
        RunStatus::Failed => "failed",
    }
}

pub struct NativeRun {
    pub run_id: RunId,
    pub project_id: ProjectId,
    pub name: String,
    pub status: crate::model::run::RunStatus,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub started_at: chrono::DateTime<chrono::Utc>,
    pub finished_at: Option<chrono::DateTime<chrono::Utc>>,
    reporter: MetricReporter,
    active_run: Arc<ActiveRun>,
}

impl NativeRun {
    pub fn log_metric_at_step(
        &self,
        metric_key: &str,
        step: i64,
        value_f64: f64,
    ) -> Result<(), EngineError> {
        self.active_run.with_open_admission(&self.run_id, || {
            self.reporter.report_metric(
                self.run_id.clone(),
                MetricKey::from_string(metric_key),
                Step::new(step),
                value_f64,
            )
        })
    }
}

struct ActiveRun {
    lock_file: Mutex<Option<File>>,
    admission: Mutex<RunAdmission>,
}

impl ActiveRun {
    fn open(lock_file: File) -> Self {
        Self {
            lock_file: Mutex::new(Some(lock_file)),
            admission: Mutex::new(RunAdmission::Open),
        }
    }

    fn closed() -> Arc<Self> {
        Arc::new(Self {
            lock_file: Mutex::new(None),
            admission: Mutex::new(RunAdmission::Terminal),
        })
    }

    fn with_open_admission(
        &self,
        run_id: &RunId,
        report: impl FnOnce() -> Result<(), EngineError>,
    ) -> Result<(), EngineError> {
        let admission = self
            .admission
            .lock()
            .map_err(|_| EngineError::ConnectionLockPoisoned)?;
        match *admission {
            RunAdmission::Open => report(),
            RunAdmission::Closing | RunAdmission::Terminal => Err(EngineError::RunClosed {
                run_id: run_id.as_str().to_owned(),
            }),
        }
    }

    fn close_admission(&self) -> Result<(), EngineError> {
        let mut admission = self
            .admission
            .lock()
            .map_err(|_| EngineError::ConnectionLockPoisoned)?;
        *admission = RunAdmission::Closing;
        Ok(())
    }

    fn open_admission(&self) -> Result<(), EngineError> {
        let mut admission = self
            .admission
            .lock()
            .map_err(|_| EngineError::ConnectionLockPoisoned)?;
        *admission = RunAdmission::Open;
        Ok(())
    }

    fn mark_terminal(&self) -> Result<(), EngineError> {
        let mut admission = self
            .admission
            .lock()
            .map_err(|_| EngineError::ConnectionLockPoisoned)?;
        *admission = RunAdmission::Terminal;
        Ok(())
    }

    fn release_lock(&self) {
        if let Ok(mut lock_file) = self.lock_file.lock()
            && let Some(lock_file) = lock_file.take()
        {
            let _ = lock_file.unlock();
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RunAdmission {
    Open,
    Closing,
    Terminal,
}

fn percent_encode_path_segment(value: &str) -> String {
    let mut encoded = String::new();
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'.' | b'_' | b'~' | b'-' => {
                encoded.push(char::from(byte));
            }
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }
    encoded
}

fn path_basename(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("storage path")
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::bootstrap::{
        attach_ducklake, open_native_connection, setup_duckdb_catalog_adapter,
    };

    fn partition_contains_parquet(path: &Path) -> std::io::Result<bool> {
        if !path.is_dir() {
            return Ok(false);
        }
        for entry in std::fs::read_dir(path)? {
            let entry = entry?;
            if entry
                .path()
                .extension()
                .is_some_and(|extension| extension == "parquet")
            {
                return Ok(true);
            }
        }
        Ok(false)
    }

    #[test]
    fn open_initializes_ducklake_dataset() -> Result<(), Box<dyn std::error::Error>> {
        let root_path =
            std::env::temp_dir().join(format!("pulseon-client-{}", uuid::Uuid::new_v4()));

        let _client = NativeClient::open(&root_path)?;

        assert!(root_path.join(".pulseon").join("data").is_dir());
        std::fs::remove_dir_all(root_path)?;
        Ok(())
    }

    #[test]
    fn open_uses_custom_catalog_without_requiring_ducklake_suffix()
    -> Result<(), Box<dyn std::error::Error>> {
        let root_path =
            std::env::temp_dir().join(format!("pulseon-client-{}", uuid::Uuid::new_v4()));
        let custom_root =
            std::env::temp_dir().join(format!("pulseon-storage-{}", uuid::Uuid::new_v4()));
        let catalog_path = custom_root.join("catalog").join("catalog.db");
        let data_path = custom_root.join("parquet-data");

        let client = NativeClient::open_with_storage_config(
            &root_path,
            Some(catalog_path.clone()),
            Some(data_path.clone()),
            65_536,
        )?;
        let project = client.create_project(
            "local training",
            Some(ProjectId::from_string("project-custom-paths")),
        )?;

        assert_eq!(project.project_id.as_str(), "project-custom-paths");
        assert!(catalog_path.is_file());
        assert!(data_path.is_dir());
        assert!(!root_path.join(".pulseon").join("data").exists());
        let _ = std::fs::remove_dir_all(root_path);
        std::fs::remove_dir_all(custom_root)?;
        Ok(())
    }

    #[test]
    fn open_stores_application_tables_in_catalog_file() -> Result<(), Box<dyn std::error::Error>> {
        let root_path =
            std::env::temp_dir().join(format!("pulseon-client-{}", uuid::Uuid::new_v4()));
        let catalog_path = root_path.join(".pulseon").join("catalog.ducklake");
        let data_path = root_path.join(".pulseon").join("data");
        {
            let client = NativeClient::open(&root_path)?;
            let project = client.create_project(
                "local training",
                Some(ProjectId::from_string("project-catalog-file")),
            )?;
            client.create_run(
                &project.project_id,
                "baseline",
                Some(RunId::from_string("run-catalog-file")),
            )?;
        }

        let connection = duckdb::Connection::open_in_memory()?;
        attach_ducklake(&connection, &catalog_path, &data_path)?;
        setup_duckdb_catalog_adapter(&connection, &catalog_path)?;
        let stored: (i64, i64) = connection.query_row(
            "SELECT
                 (SELECT count(*) FROM pulseon_projects),
                 (SELECT count(*) FROM pulseon_runs)",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;

        assert_eq!(stored, (1, 1));
        std::fs::remove_dir_all(root_path)?;
        Ok(())
    }

    #[test]
    fn create_project_and_run_persist_v1_records() -> Result<(), Box<dyn std::error::Error>> {
        let root_path =
            std::env::temp_dir().join(format!("pulseon-client-{}", uuid::Uuid::new_v4()));
        let client = NativeClient::open(&root_path)?;

        let project = client.create_project(
            "local training",
            Some(ProjectId::from_string("project-python")),
        )?;
        let run = client.create_run(
            &project.project_id,
            "baseline",
            Some(RunId::from_string("run-python")),
        )?;
        let run_handle = client.run_handle(run);

        assert_eq!(project.project_id.as_str(), "project-python");
        assert_eq!(run_handle.run_id.as_str(), "run-python");
        assert_eq!(run_handle.project_id, project.project_id);
        std::fs::remove_dir_all(root_path)?;
        Ok(())
    }

    #[test]
    fn get_project_and_run_select_existing_records() -> Result<(), Box<dyn std::error::Error>> {
        let root_path =
            std::env::temp_dir().join(format!("pulseon-client-{}", uuid::Uuid::new_v4()));
        let client = NativeClient::open(&root_path)?;

        let created_project = client.create_project(
            "local training",
            Some(ProjectId::from_string("project-existing")),
        )?;
        let created_run = client.create_run(
            &created_project.project_id,
            "baseline",
            Some(RunId::from_string("run-existing")),
        )?;

        let selected_project = client.get_project(&created_project.project_id)?;
        let selected_run = client.get_run(&created_run.run_id)?;

        assert_eq!(selected_project, created_project);
        assert_eq!(selected_run, created_run);
        std::fs::remove_dir_all(root_path)?;
        Ok(())
    }

    #[test]
    fn list_projects_returns_projects_in_stable_catalog_order()
    -> Result<(), Box<dyn std::error::Error>> {
        let root_path =
            std::env::temp_dir().join(format!("pulseon-client-{}", uuid::Uuid::new_v4()));
        let client = NativeClient::open(&root_path)?;

        assert_eq!(client.list_projects()?, []);

        let first =
            client.create_project("local training", Some(ProjectId::from_string("project-1")))?;
        let second = client.create_project("sweep", Some(ProjectId::from_string("project-2")))?;

        assert_eq!(client.list_projects()?, [first, second]);
        std::fs::remove_dir_all(root_path)?;
        Ok(())
    }

    #[test]
    fn list_runs_filters_status_and_paginates_stable_order()
    -> Result<(), Box<dyn std::error::Error>> {
        let root_path =
            std::env::temp_dir().join(format!("pulseon-client-{}", uuid::Uuid::new_v4()));
        let client = NativeClient::open(&root_path)?;
        let project = client.create_project(
            "local training",
            Some(ProjectId::from_string("project-runs")),
        )?;
        let first = client.create_run(
            &project.project_id,
            "first",
            Some(RunId::from_string("run-1")),
        )?;
        let second = client.create_run(
            &project.project_id,
            "second",
            Some(RunId::from_string("run-2")),
        )?;
        let third = client.create_run(
            &project.project_id,
            "third",
            Some(RunId::from_string("run-3")),
        )?;
        client.finish_run(&first.run_id)?;
        client.fail_run(&third.run_id)?;

        let page = client.list_runs_filtered(&project.project_id, None, Some(1), 1)?;
        let finished =
            client.list_runs_filtered(&project.project_id, Some(RunStatus::Finished), None, 0)?;

        assert_eq!(page, [second]);
        assert_eq!([finished[0].run_id.as_str()], [first.run_id.as_str()]);
        std::fs::remove_dir_all(root_path)?;
        Ok(())
    }

    #[test]
    fn query_metric_excludes_queued_reports_until_they_are_persisted()
    -> Result<(), Box<dyn std::error::Error>> {
        let root_path =
            std::env::temp_dir().join(format!("pulseon-client-{}", uuid::Uuid::new_v4()));
        let connection = Arc::new(Mutex::new(open_native_connection(&root_path)?));
        let client = NativeClient {
            root_path: root_path.clone(),
            reporter: MetricReporter::blocked_for_test(1),
            connection,
            active_runs: Arc::new(Mutex::new(HashMap::new())),
            flush_lock: Arc::new(Mutex::new(())),
            is_shutdown: AtomicBool::new(false),
        };
        let project = client.create_project(
            "local training",
            Some(ProjectId::from_string("project-queued")),
        )?;
        let run = client.create_run(
            &project.project_id,
            "baseline",
            Some(RunId::from_string("run-queued")),
        )?;
        let run_handle = client.run_handle(run);

        run_handle.log_metric_at_step("train/loss", 0, 0.25)?;
        let points = client.query_metric(
            &run_handle.run_id,
            &MetricKey::from_string("train/loss"),
            None,
            None,
            None,
        )?;

        assert_eq!(points, []);
        assert_eq!(client.diagnostics().pending_reports, 1);
        std::fs::remove_dir_all(root_path)?;
        Ok(())
    }

    #[test]
    fn finish_run_updates_status_and_rejects_second_transition()
    -> Result<(), Box<dyn std::error::Error>> {
        let root_path =
            std::env::temp_dir().join(format!("pulseon-client-{}", uuid::Uuid::new_v4()));
        let client = NativeClient::open(&root_path)?;
        let project = client.create_project(
            "local training",
            Some(ProjectId::from_string("project-finalize")),
        )?;
        let run = client.create_run(
            &project.project_id,
            "baseline",
            Some(RunId::from_string("run-finalize")),
        )?;

        let finished = client.finish_run(&run.run_id)?;
        let second_transition = client.fail_run(&run.run_id).unwrap_err();

        assert_eq!(finished.status, RunStatus::Finished);
        assert!(finished.finished_at.is_some());
        assert!(
            matches!(
                second_transition,
                EngineError::InvalidRunTransition {
                    from: "finished",
                    to: "failed",
                    ..
                }
            ),
            "expected invalid finished -> failed transition, got {second_transition:?}",
        );
        std::fs::remove_dir_all(root_path)?;
        Ok(())
    }

    #[test]
    fn finish_run_drains_reports_and_closes_existing_run_handle()
    -> Result<(), Box<dyn std::error::Error>> {
        let root_path =
            std::env::temp_dir().join(format!("pulseon-client-{}", uuid::Uuid::new_v4()));
        let client = NativeClient::open(&root_path)?;
        let project = client.create_project(
            "local training",
            Some(ProjectId::from_string("project-close-admission")),
        )?;
        let run = client.create_run(
            &project.project_id,
            "baseline",
            Some(RunId::from_string("run-close-admission")),
        )?;
        let run_handle = client.run_handle(run.clone());
        run_handle.log_metric_at_step("train/loss", 0, 0.25)?;

        let finished = client.finish_run(&run.run_id)?;
        let late_log = run_handle.log_metric_at_step("train/loss", 1, 0.125);
        let points = client.query_metric(
            &run.run_id,
            &MetricKey::from_string("train/loss"),
            None,
            None,
            None,
        )?;

        assert_eq!(finished.status, RunStatus::Finished);
        assert_eq!(points.len(), 1);
        assert!(
            matches!(late_log, Err(EngineError::RunClosed { .. })),
            "expected late log to raise RunClosed, got {late_log:?}",
        );
        std::fs::remove_dir_all(root_path)?;
        Ok(())
    }

    #[test]
    fn finish_run_rebuilds_metric_aggregates_after_drain() -> Result<(), Box<dyn std::error::Error>>
    {
        let root_path =
            std::env::temp_dir().join(format!("pulseon-client-{}", uuid::Uuid::new_v4()));
        let client = NativeClient::open(&root_path)?;
        let project = client.create_project(
            "local training",
            Some(ProjectId::from_string("project-final-aggregates")),
        )?;
        let run = client.create_run(
            &project.project_id,
            "baseline",
            Some(RunId::from_string("run-final-aggregates")),
        )?;
        let run_handle = client.run_handle(run.clone());
        run_handle.log_metric_at_step("train/loss", 0, 0.25)?;
        run_handle.log_metric_at_step("train/loss", 0, 0.125)?;
        run_handle.log_metric_at_step("eval/accuracy", 0, 0.8)?;

        client.finish_run(&run.run_id)?;
        let summaries = client.list_metrics(&run.run_id)?;

        assert_eq!(
            summaries
                .iter()
                .map(|summary| (
                    summary.metric_key.as_str(),
                    summary.effective_count,
                    summary.last_value_f64
                ))
                .collect::<Vec<_>>(),
            vec![("eval/accuracy", 1, 0.8), ("train/loss", 1, 0.125)],
        );
        std::fs::remove_dir_all(root_path)?;
        Ok(())
    }

    #[test]
    fn finish_run_flushes_metric_points_to_partitioned_parquet()
    -> Result<(), Box<dyn std::error::Error>> {
        let root_path =
            std::env::temp_dir().join(format!("pulseon-client-{}", uuid::Uuid::new_v4()));
        let client = NativeClient::open(&root_path)?;
        let project = client.create_project(
            "local training",
            Some(ProjectId::from_string("project-flush")),
        )?;
        let run = client.create_run(
            &project.project_id,
            "baseline",
            Some(RunId::from_string("run-flush")),
        )?;
        let run_handle = client.run_handle(run.clone());
        run_handle.log_metric_at_step("train/loss", 0, 0.25)?;

        let finished = client.finish_run(&run.run_id)?;
        let diagnostics = client.diagnostics();
        let partition_path = root_path
            .join(".pulseon")
            .join("data")
            .join("main")
            .join("metric_points")
            .join("run_id=run-flush")
            .join("metric_key_encoded=train%252Floss");
        let data_main_path = root_path.join(".pulseon").join("data").join("main");

        assert_eq!(finished.status, RunStatus::Finished);
        assert_eq!(diagnostics.last_flush_run_id.as_deref(), Some("run-flush"));
        assert_eq!(diagnostics.last_flush_status, "succeeded");
        assert!(
            partition_contains_parquet(&partition_path)?,
            "expected partitioned parquet under {}",
            partition_path.display(),
        );
        assert!(
            !data_main_path.join("pulseon_projects").exists()
                && !data_main_path.join("pulseon_runs").exists()
                && !data_main_path.join("pulseon_metric_aggregates").exists(),
            "application tables must stay catalog-owned, not in the data path",
        );
        std::fs::remove_dir_all(root_path)?;
        Ok(())
    }

    #[test]
    fn flush_run_data_is_idempotent_for_terminal_runs() -> Result<(), Box<dyn std::error::Error>> {
        let root_path =
            std::env::temp_dir().join(format!("pulseon-client-{}", uuid::Uuid::new_v4()));
        let client = NativeClient::open(&root_path)?;
        let project = client.create_project(
            "local training",
            Some(ProjectId::from_string("project-flush-idempotent")),
        )?;
        let run = client.create_run(
            &project.project_id,
            "baseline",
            Some(RunId::from_string("run-flush-idempotent")),
        )?;

        client.finish_run(&run.run_id)?;
        client.flush_run_data(&run.run_id, None)?;
        let diagnostics = client.diagnostics();

        assert_eq!(
            diagnostics.last_flush_run_id.as_deref(),
            Some("run-flush-idempotent"),
        );
        assert_eq!(diagnostics.last_flush_status, "succeeded");
        std::fs::remove_dir_all(root_path)?;
        Ok(())
    }

    #[test]
    fn flush_run_data_rejects_running_runs() -> Result<(), Box<dyn std::error::Error>> {
        let root_path =
            std::env::temp_dir().join(format!("pulseon-client-{}", uuid::Uuid::new_v4()));
        let client = NativeClient::open(&root_path)?;
        let project = client.create_project(
            "local training",
            Some(ProjectId::from_string("project-flush-running")),
        )?;
        let run = client.create_run(
            &project.project_id,
            "baseline",
            Some(RunId::from_string("run-flush-running")),
        )?;

        let flush = client.flush_run_data(&run.run_id, None);

        assert!(
            matches!(
                flush,
                Err(EngineError::InvalidRunTransition {
                    from: "running",
                    to: "flushed",
                    ..
                })
            ),
            "expected running-run flush to raise invalid transition, got {flush:?}",
        );
        std::fs::remove_dir_all(root_path)?;
        Ok(())
    }

    #[test]
    fn flush_run_data_timeout_updates_runtime_diagnostics() -> Result<(), Box<dyn std::error::Error>>
    {
        let root_path =
            std::env::temp_dir().join(format!("pulseon-client-{}", uuid::Uuid::new_v4()));
        let client = NativeClient::open(&root_path)?;
        let project = client.create_project(
            "local training",
            Some(ProjectId::from_string("project-flush-timeout")),
        )?;
        let run = client.create_run(
            &project.project_id,
            "baseline",
            Some(RunId::from_string("run-flush-timeout")),
        )?;
        client.finish_run(&run.run_id)?;
        let _held_flush_lock = client.flush_lock.lock().expect("test flush lock");

        let flush = client.flush_run_data(&run.run_id, Some(Duration::from_millis(1)));
        let diagnostics = client.diagnostics();

        assert!(matches!(flush, Err(EngineError::MetricFlushTimeout)));
        assert_eq!(
            diagnostics.last_flush_run_id.as_deref(),
            Some("run-flush-timeout"),
        );
        assert_eq!(diagnostics.last_flush_status, "timed_out");
        std::fs::remove_dir_all(root_path)?;
        Ok(())
    }

    #[test]
    fn finalization_flush_failure_keeps_terminal_state_and_updates_diagnostics()
    -> Result<(), Box<dyn std::error::Error>> {
        let root_path =
            std::env::temp_dir().join(format!("pulseon-client-{}", uuid::Uuid::new_v4()));
        let client = NativeClient::open(&root_path)?;
        let project = client.create_project(
            "local training",
            Some(ProjectId::from_string("project-flush-failure")),
        )?;
        let run = client.create_run(
            &project.project_id,
            "baseline",
            Some(RunId::from_string("run-flush-failure")),
        )?;
        let run_handle = client.run_handle(run.clone());
        run_handle.log_metric_at_step("train/loss", 0, 0.25)?;
        client.reporter.drain(None)?;
        let metric_points_path = root_path
            .join(".pulseon")
            .join("data")
            .join("main")
            .join("metric_points");
        std::fs::create_dir_all(metric_points_path.parent().expect("metric_points parent"))?;
        if metric_points_path.is_dir() {
            std::fs::remove_dir_all(&metric_points_path)?;
        }
        std::fs::write(&metric_points_path, b"not a directory")?;

        let finish = client.finish_run(&run.run_id);
        let stored = client.get_run(&run.run_id)?;
        let diagnostics = client.diagnostics();

        assert!(
            matches!(finish, Err(EngineError::MetricFlush { .. })),
            "expected finalization to surface MetricFlush, got {finish:?}",
        );
        assert_eq!(stored.status, RunStatus::Finished);
        assert_eq!(
            diagnostics.last_flush_run_id.as_deref(),
            Some("run-flush-failure"),
        );
        assert_eq!(diagnostics.last_flush_status, "failed");
        assert_eq!(
            diagnostics.last_flush_error.as_deref(),
            Some("flush metric_points failed"),
        );
        std::fs::remove_dir_all(root_path)?;
        Ok(())
    }

    #[test]
    fn finish_run_with_timeout_does_not_write_terminal_state()
    -> Result<(), Box<dyn std::error::Error>> {
        let root_path =
            std::env::temp_dir().join(format!("pulseon-client-{}", uuid::Uuid::new_v4()));
        let connection = Arc::new(Mutex::new(open_native_connection(&root_path)?));
        let client = NativeClient {
            root_path: root_path.clone(),
            reporter: MetricReporter::blocked_for_test(2),
            connection,
            active_runs: Arc::new(Mutex::new(HashMap::new())),
            flush_lock: Arc::new(Mutex::new(())),
            is_shutdown: AtomicBool::new(false),
        };
        let project = client.create_project(
            "local training",
            Some(ProjectId::from_string("project-finalize-timeout")),
        )?;
        let run = client.create_run(
            &project.project_id,
            "baseline",
            Some(RunId::from_string("run-finalize-timeout")),
        )?;
        let run_handle = client.run_handle(run.clone());
        run_handle.log_metric_at_step("train/loss", 0, 0.25)?;

        let finish = client.finish_run_with_timeout(&run.run_id, Some(Duration::from_millis(1)));
        let stored = client.get_run(&run.run_id)?;
        let second_client = NativeClient::open(&root_path)?;
        let resumed_by_second_client = second_client.resume_run(&run.run_id);

        assert!(matches!(finish, Err(EngineError::MetricDrainTimeout)));
        assert_eq!(stored.status, RunStatus::Running);
        assert!(stored.finished_at.is_none());
        assert!(
            matches!(
                resumed_by_second_client,
                Err(EngineError::RunAlreadyActive { .. })
            ),
            "expected finalization timeout to keep writer lock held, got {resumed_by_second_client:?}",
        );
        std::fs::remove_dir_all(root_path)?;
        Ok(())
    }

    #[test]
    fn admission_close_waits_for_in_flight_log_gate_before_rejecting_late_logs()
    -> Result<(), Box<dyn std::error::Error>> {
        let lock_path = std::env::temp_dir().join(format!("pulseon-lock-{}", uuid::Uuid::new_v4()));
        let lock_file = File::options()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&lock_path)?;
        let active_run = Arc::new(ActiveRun::open(lock_file));
        let run_id = RunId::from_string("run-admission-race");
        let (entered_sender, entered_receiver) = std::sync::mpsc::channel();
        let (release_sender, release_receiver) = std::sync::mpsc::channel();
        let (logged_sender, logged_receiver) = std::sync::mpsc::channel();
        let in_flight = Arc::clone(&active_run);
        let in_flight_run_id = run_id.clone();
        let log_thread = std::thread::spawn(move || {
            let result = in_flight.with_open_admission(&in_flight_run_id, || {
                entered_sender
                    .send(())
                    .expect("test should observe in-flight log gate");
                release_receiver
                    .recv()
                    .expect("test should release in-flight log gate");
                Ok(())
            });
            logged_sender
                .send(result)
                .expect("test should observe log result");
        });
        entered_receiver.recv()?;
        let barrier = Arc::clone(&active_run);
        let (closed_sender, closed_receiver) = std::sync::mpsc::channel();
        let close_thread = std::thread::spawn(move || {
            barrier
                .close_admission()
                .expect("close barrier should acquire admission gate");
            closed_sender
                .send(())
                .expect("test should observe close barrier");
        });

        assert!(
            closed_receiver
                .recv_timeout(Duration::from_millis(10))
                .is_err(),
            "close barrier should wait for the in-flight admission gate",
        );
        release_sender.send(())?;
        let log_result = logged_receiver.recv()?;
        close_thread
            .join()
            .expect("close barrier thread should join");
        log_thread.join().expect("log thread should join");
        let late_log = active_run.with_open_admission(&run_id, || Ok(()));

        assert!(log_result.is_ok());
        assert!(
            matches!(late_log, Err(EngineError::RunClosed { .. })),
            "expected log after close barrier to raise RunClosed, got {late_log:?}",
        );
        std::fs::remove_file(lock_path)?;
        Ok(())
    }

    #[test]
    fn finish_run_releases_writer_lock_after_terminal_state()
    -> Result<(), Box<dyn std::error::Error>> {
        let root_path =
            std::env::temp_dir().join(format!("pulseon-client-{}", uuid::Uuid::new_v4()));
        let client = NativeClient::open(&root_path)?;
        let project = client.create_project(
            "local training",
            Some(ProjectId::from_string("project-terminal-lock")),
        )?;
        let run = client.create_run(
            &project.project_id,
            "baseline",
            Some(RunId::from_string("run-terminal-lock")),
        )?;
        let lock_path = root_path
            .join(".pulseon")
            .join("locks")
            .join("runs")
            .join("run-terminal-lock.lock");

        client.finish_run(&run.run_id)?;
        let lock_file = File::options().read(true).write(true).open(lock_path)?;
        let lock_result = lock_file.try_lock();

        assert!(
            lock_result.is_ok(),
            "expected terminal finalization to release writer lock, got {lock_result:?}",
        );
        std::fs::remove_dir_all(root_path)?;
        Ok(())
    }

    #[test]
    fn shutdown_tears_down_client_without_finalizing_running_run()
    -> Result<(), Box<dyn std::error::Error>> {
        let root_path =
            std::env::temp_dir().join(format!("pulseon-client-{}", uuid::Uuid::new_v4()));
        let client = NativeClient::open(&root_path)?;
        let project = client.create_project(
            "local training",
            Some(ProjectId::from_string("project-shutdown-running")),
        )?;
        let run = client.create_run(
            &project.project_id,
            "baseline",
            Some(RunId::from_string("run-shutdown-running")),
        )?;
        let run_handle = client.run_handle(run.clone());

        client.shutdown(None)?;
        let log_after_shutdown = run_handle.log_metric_at_step("train/loss", 0, 0.25);
        let create_project_after_shutdown =
            client.create_project("late project", Some(ProjectId::from_string("late-project")));
        let create_run_after_shutdown = client.create_run(
            &project.project_id,
            "late run",
            Some(RunId::from_string("late-run")),
        );
        let resume_after_shutdown = client.resume_run(&run.run_id);
        let finish_after_shutdown = client.finish_run(&run.run_id);
        let fail_after_shutdown = client.fail_run(&run.run_id);
        let flush_after_shutdown = client.flush_run_data(&run.run_id, None);
        let reopened_client = NativeClient::open(&root_path)?;
        let reopened_run = reopened_client.get_run(&run.run_id)?;
        let resumed_run = reopened_client.resume_run(&run.run_id)?;

        assert!(
            matches!(log_after_shutdown, Err(EngineError::ClientClosed)),
            "expected closed client error after shutdown, got {log_after_shutdown:?}",
        );
        assert!(
            matches!(
                create_project_after_shutdown,
                Err(EngineError::ClientClosed)
            ),
            "expected closed client error after shutdown, got {create_project_after_shutdown:?}",
        );
        assert!(
            matches!(create_run_after_shutdown, Err(EngineError::ClientClosed)),
            "expected closed client error after shutdown, got {create_run_after_shutdown:?}",
        );
        assert!(
            matches!(resume_after_shutdown, Err(EngineError::ClientClosed)),
            "expected closed client error after shutdown, got {resume_after_shutdown:?}",
        );
        assert!(
            matches!(finish_after_shutdown, Err(EngineError::ClientClosed)),
            "expected closed client error after shutdown, got {finish_after_shutdown:?}",
        );
        assert!(
            matches!(fail_after_shutdown, Err(EngineError::ClientClosed)),
            "expected closed client error after shutdown, got {fail_after_shutdown:?}",
        );
        assert!(
            matches!(flush_after_shutdown, Err(EngineError::ClientClosed)),
            "expected closed client error after shutdown, got {flush_after_shutdown:?}",
        );
        assert_eq!(reopened_run.status, RunStatus::Running);
        assert!(reopened_run.finished_at.is_none());
        assert_eq!(resumed_run.run_id, run.run_id);
        reopened_client.shutdown(None)?;
        std::fs::remove_dir_all(root_path)?;
        Ok(())
    }

    #[test]
    fn run_writer_lock_rejects_second_active_client() -> Result<(), Box<dyn std::error::Error>> {
        let root_path =
            std::env::temp_dir().join(format!("pulseon-client-{}", uuid::Uuid::new_v4()));
        let first_client = NativeClient::open(&root_path)?;
        let project = first_client.create_project(
            "local training",
            Some(ProjectId::from_string("project-lock")),
        )?;
        let run = first_client.create_run(
            &project.project_id,
            "baseline",
            Some(RunId::from_string("run-lock")),
        )?;
        let second_client = NativeClient::open(&root_path)?;

        let resume = second_client.resume_run(&run.run_id);

        assert!(
            matches!(resume, Err(EngineError::RunAlreadyActive { .. })),
            "expected active writer conflict, got {resume:?}",
        );
        first_client.shutdown(None)?;
        let resumed_after_shutdown = second_client.resume_run(&run.run_id)?;
        assert_eq!(resumed_after_shutdown.run_id, run.run_id);
        std::fs::remove_dir_all(root_path)?;
        Ok(())
    }

    #[test]
    fn create_run_existing_id_reports_exists_before_active_lock()
    -> Result<(), Box<dyn std::error::Error>> {
        let root_path =
            std::env::temp_dir().join(format!("pulseon-client-{}", uuid::Uuid::new_v4()));
        let first_client = NativeClient::open(&root_path)?;
        let project = first_client.create_project(
            "local training",
            Some(ProjectId::from_string("project-existing-active")),
        )?;
        first_client.create_run(
            &project.project_id,
            "baseline",
            Some(RunId::from_string("run-existing-active")),
        )?;
        let second_client = NativeClient::open(&root_path)?;

        let duplicate = second_client.create_run(
            &project.project_id,
            "duplicate",
            Some(RunId::from_string("run-existing-active")),
        );

        assert!(
            matches!(duplicate, Err(EngineError::RunAlreadyExists { .. })),
            "expected duplicate run id error, got {duplicate:?}",
        );
        first_client.shutdown(None)?;
        std::fs::remove_dir_all(root_path)?;
        Ok(())
    }

    #[test]
    fn resume_run_rejects_terminal_runs() -> Result<(), Box<dyn std::error::Error>> {
        let root_path =
            std::env::temp_dir().join(format!("pulseon-client-{}", uuid::Uuid::new_v4()));
        let client = NativeClient::open(&root_path)?;
        let project = client.create_project(
            "local training",
            Some(ProjectId::from_string("project-terminal-resume")),
        )?;
        let run = client.create_run(
            &project.project_id,
            "baseline",
            Some(RunId::from_string("run-terminal-resume")),
        )?;
        client.finish_run(&run.run_id)?;

        let resumed = client.resume_run(&run.run_id);

        assert!(
            matches!(
                resumed,
                Err(EngineError::InvalidRunTransition {
                    from: "finished",
                    to: "running",
                    ..
                })
            ),
            "expected terminal resume rejection, got {resumed:?}",
        );
        std::fs::remove_dir_all(root_path)?;
        Ok(())
    }

    #[test]
    fn leftover_lock_file_without_os_lock_does_not_block_resume()
    -> Result<(), Box<dyn std::error::Error>> {
        let root_path =
            std::env::temp_dir().join(format!("pulseon-client-{}", uuid::Uuid::new_v4()));
        let first_client = NativeClient::open(&root_path)?;
        let project = first_client.create_project(
            "local training",
            Some(ProjectId::from_string("project-leftover-lock")),
        )?;
        let run = first_client.create_run(
            &project.project_id,
            "baseline",
            Some(RunId::from_string("run/leftover lock")),
        )?;
        first_client.shutdown(None)?;
        let lock_path = root_path
            .join(".pulseon")
            .join("locks")
            .join("runs")
            .join("run%2Fleftover%20lock.lock");
        assert!(lock_path.is_file());

        let second_client = NativeClient::open(&root_path)?;
        let resumed = second_client.resume_run(&run.run_id)?;

        assert_eq!(resumed.run_id, run.run_id);
        second_client.shutdown(None)?;
        std::fs::remove_dir_all(root_path)?;
        Ok(())
    }

    #[test]
    fn percent_encode_path_segment_uses_rfc3986_unreserved_bytes() {
        assert_eq!(
            percent_encode_path_segment("run/space ü._~-"),
            "run%2Fspace%20%C3%BC._~-",
        );
    }
}
