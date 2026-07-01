use std::path::{Path, PathBuf};

use crate::engine::EngineError;
use crate::engine::reporting::{MetricReporter, MetricReporterDiagnostics};
use crate::engine::time::{current_timestamp, timestamp_as_rfc3339};
use crate::engine::write::NativeWriteStore;
use crate::model::metric::{MetricKey, Step};
use crate::model::run::{Run, RunId};
use crate::model::types::{Project, ProjectId};

pub struct NativeClient {
    _root_path: PathBuf,
    reporter: MetricReporter,
    _connection: duckdb::Connection,
}

impl NativeClient {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, EngineError> {
        let root_path = path.as_ref().to_path_buf();
        let catalog_path = root_path.join("catalog.ducklake");
        let data_path = root_path.join("data");
        std::fs::create_dir_all(&data_path)?;

        let connection = duckdb::Connection::open_in_memory()?;
        attach_ducklake(&connection, &catalog_path, &data_path)?;
        create_v1_tables(&connection)?;
        let reporter = MetricReporter::open(catalog_path, data_path)?;

        Ok(Self {
            _root_path: root_path,
            reporter,
            _connection: connection,
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
        self._connection.execute(
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

        NativeWriteStore::new(&self._connection).create_run(project_id, name, run_id)
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

    fn project_exists(&self, project_id: &ProjectId) -> Result<bool, EngineError> {
        let exists = self._connection.query_row(
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
}

pub(crate) fn attach_ducklake(
    connection: &duckdb::Connection,
    catalog_path: &Path,
    data_path: &Path,
) -> Result<(), EngineError> {
    let catalog_path = sql_string_literal(catalog_path.to_string_lossy().as_ref());
    let data_path = sql_string_literal(data_path.to_string_lossy().as_ref());
    connection.execute_batch(&format!(
        "INSTALL ducklake;
         LOAD ducklake;
         ATTACH {catalog_path} AS dl (TYPE ducklake, DATA_PATH {data_path});"
    ))?;
    Ok(())
}

fn create_v1_tables(connection: &duckdb::Connection) -> Result<(), EngineError> {
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS dl.projects (
             project_id VARCHAR NOT NULL,
             name VARCHAR NOT NULL,
             created_at TIMESTAMPTZ NOT NULL
         );
         CREATE TABLE IF NOT EXISTS dl.runs (
             run_id VARCHAR NOT NULL,
             project_id VARCHAR NOT NULL,
             name VARCHAR NOT NULL,
             status VARCHAR NOT NULL,
             created_at TIMESTAMPTZ NOT NULL,
             started_at TIMESTAMPTZ NOT NULL,
             finished_at TIMESTAMPTZ
         );
         CREATE TABLE IF NOT EXISTS dl.metric_points (
             run_id VARCHAR NOT NULL,
             metric_key VARCHAR NOT NULL,
             step BIGINT NOT NULL,
             timestamp TIMESTAMPTZ NOT NULL,
             value_f64 DOUBLE NOT NULL,
             ingested_at TIMESTAMPTZ NOT NULL
         );
         CREATE TABLE IF NOT EXISTS dl.metric_aggregates (
             run_id VARCHAR NOT NULL,
             metric_key VARCHAR NOT NULL,
             effective_count UBIGINT NOT NULL,
             last_step BIGINT NOT NULL,
             last_value_f64 DOUBLE NOT NULL,
             min_value_f64 DOUBLE NOT NULL,
             max_value_f64 DOUBLE NOT NULL
         );",
    )?;
    Ok(())
}

fn sql_string_literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
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
    fn sql_string_literal_escapes_single_quotes() {
        assert_eq!(sql_string_literal("canary's/data"), "'canary''s/data'");
    }
}
