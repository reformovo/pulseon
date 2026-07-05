use std::collections::HashMap;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use crate::engine::EngineError;
use crate::engine::bootstrap::open_native_connection;
use crate::engine::query::NativeQueryStore;
use crate::engine::reporting::{MetricReporter, MetricReporterDiagnostics};
use crate::engine::time::{current_timestamp, timestamp_as_rfc3339};
use crate::engine::write::NativeWriteStore;
use crate::model::metric::{MetricAggregate, MetricKey, MetricPoint, Step};
use crate::model::run::{Run, RunId, RunStatus};
use crate::model::types::{Project, ProjectId};

pub struct NativeClient {
    root_path: PathBuf,
    reporter: MetricReporter,
    connection: Arc<Mutex<duckdb::Connection>>,
    active_runs: Arc<Mutex<HashMap<RunId, Arc<ActiveRun>>>>,
}

impl NativeClient {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, EngineError> {
        Self::open_with_metric_queue_capacity(path, 65_536)
    }

    pub fn open_with_metric_queue_capacity(
        path: impl AsRef<Path>,
        metric_queue_capacity: usize,
    ) -> Result<Self, EngineError> {
        let root_path = path.as_ref().to_path_buf();
        let connection = open_native_connection(&root_path)?;
        let connection = Arc::new(Mutex::new(connection));
        let reporter =
            MetricReporter::open_with_capacity(Arc::clone(&connection), metric_queue_capacity);

        Ok(Self {
            root_path,
            reporter,
            connection,
            active_runs: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    pub fn create_project(
        &self,
        name: &str,
        project_id: Option<ProjectId>,
    ) -> Result<Project, EngineError> {
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
            "INSERT INTO dl.projects (project_id, name, created_at)
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
            "SELECT project_id, name, epoch_ms(created_at)
             FROM dl.projects
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

    pub fn create_run(
        &self,
        project_id: &ProjectId,
        name: &str,
        run_id: Option<RunId>,
    ) -> Result<Run, EngineError> {
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
        if !self.project_exists(project_id)? {
            return Err(EngineError::ProjectNotFound {
                project_id: project_id.as_str().to_owned(),
            });
        }

        let run_ids = {
            let connection = self.connection()?;
            let mut statement = connection.prepare(
                "SELECT run_id
                 FROM dl.runs
                 WHERE project_id = ?
                 ORDER BY created_at, run_id",
            )?;
            let rows = statement.query_map([project_id.as_str()], |row| {
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
                         FROM dl.runs
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
                         FROM dl.runs
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
        self.finalize_run(run_id, RunStatus::Finished)
    }

    pub fn fail_run(&self, run_id: &RunId) -> Result<Run, EngineError> {
        self.finalize_run(run_id, RunStatus::Failed)
    }

    pub fn shutdown(&self, timeout: Option<Duration>) -> Result<(), EngineError> {
        let result = self.reporter.shutdown(timeout);
        if !matches!(result, Err(EngineError::MetricDrainTimeout)) {
            self.release_all_run_writers();
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
             FROM dl.metric_aggregates
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
                 FROM dl.projects
                 WHERE project_id = ?
             )",
            [project_id.as_str()],
            |row| row.get(0),
        )?;
        Ok(exists)
    }

    fn finalize_run(&self, run_id: &RunId, target_status: RunStatus) -> Result<Run, EngineError> {
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
        if let Err(error) = self.reporter.drain(None) {
            active_run.open_admission()?;
            return Err(error);
        }
        let finished_at = current_timestamp("finished_at")?;
        let connection = self.connection()?;
        let updated = connection.execute(
            "UPDATE dl.runs
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
    pub fn log_metric(&self, metric_key: &str, value_f64: f64) -> Result<(), EngineError> {
        self.reporter.report_metric(
            self.run_id.clone(),
            MetricKey::from_string(metric_key),
            None,
            value_f64,
        )
    }

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
                Some(Step::new(step)),
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

    #[test]
    fn open_initializes_ducklake_dataset() -> Result<(), Box<dyn std::error::Error>> {
        let root_path =
            std::env::temp_dir().join(format!("pulseon-client-{}", uuid::Uuid::new_v4()));

        let _client = NativeClient::open(&root_path)?;

        assert!(root_path.join("data").is_dir());
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
