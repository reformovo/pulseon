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

const REPORTER_DRAIN_TIMEOUT: Duration = Duration::from_millis(500);

pub struct NativeClient {
    _root_path: PathBuf,
    reporter: MetricReporter,
    connection: Arc<Mutex<duckdb::Connection>>,
}

impl NativeClient {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, EngineError> {
        let root_path = path.as_ref().to_path_buf();
        let connection = open_native_connection(&root_path)?;
        let connection = Arc::new(Mutex::new(connection));
        let reporter = MetricReporter::open(Arc::clone(&connection));

        Ok(Self {
            _root_path: root_path,
            reporter,
            connection,
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

        let connection = self.connection()?;
        NativeWriteStore::new(&connection).create_run(project_id, name, run_id)
    }

    pub fn get_run(&self, run_id: &RunId) -> Result<Run, EngineError> {
        let connection = self.connection()?;
        NativeWriteStore::new(&connection).resume_run(run_id)
    }

    pub fn resume_run(&self, run_id: &RunId) -> Result<Run, EngineError> {
        self.get_run(run_id)
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

    pub fn shutdown(&self) -> bool {
        self.reporter.shutdown_for(REPORTER_DRAIN_TIMEOUT)
    }

    pub fn run_handle(&self, run: Run) -> NativeRun {
        NativeRun {
            run_id: run.run_id,
            project_id: run.project_id,
            name: run.name,
            status: run.status,
            created_at: run.created_at,
            started_at: run.started_at,
            finished_at: run.finished_at,
            reporter: self.reporter.clone(),
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
        let _drained = self.reporter.drain_for(REPORTER_DRAIN_TIMEOUT);
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
            return self.get_run(run_id);
        }

        let run = self.get_run(run_id)?;
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
}

impl NativeRun {
    pub fn log_metric(&self, metric_key: &str, value_f64: f64) {
        self.reporter.report_metric(
            self.run_id.clone(),
            MetricKey::from_string(metric_key),
            None,
            value_f64,
        );
    }

    pub fn log_metric_at_step(&self, metric_key: &str, step: i64, value_f64: f64) {
        self.reporter.report_metric(
            self.run_id.clone(),
            MetricKey::from_string(metric_key),
            Some(Step::new(step)),
            value_f64,
        );
    }
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
}
