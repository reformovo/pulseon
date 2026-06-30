use chrono::{DateTime, Utc};

use crate::engine::EngineError;
use crate::model::run::{Run, RunId, RunStatus};
use crate::model::types::ProjectId;

pub struct NativeWriteStore<'connection> {
    connection: &'connection duckdb::Connection,
}

impl<'connection> NativeWriteStore<'connection> {
    pub const fn new(connection: &'connection duckdb::Connection) -> Self {
        Self { connection }
    }

    pub fn create_run(
        &self,
        project_id: &ProjectId,
        name: &str,
        run_id: Option<RunId>,
    ) -> Result<Run, EngineError> {
        let run_id = run_id.unwrap_or_else(|| RunId::from_string(uuid::Uuid::new_v4().to_string()));
        if self.run_exists(&run_id)? {
            return Err(EngineError::RunAlreadyExists {
                run_id: run_id.as_str().to_owned(),
            });
        }

        let timestamp_millis = Utc::now().timestamp_millis();
        let now = timestamp_from_millis("created_at", timestamp_millis)?;
        self.connection.execute(
            "INSERT INTO dl.runs
                 (run_id, project_id, name, status, created_at, started_at, finished_at)
             VALUES (?, ?, ?, ?, ?, ?, NULL)",
            (
                run_id.as_str(),
                project_id.as_str(),
                name,
                status_as_str(RunStatus::Running),
                timestamp_as_rfc3339(now),
                timestamp_as_rfc3339(now),
            ),
        )?;

        Ok(Run {
            run_id,
            project_id: project_id.clone(),
            name: name.to_owned(),
            status: RunStatus::Running,
            created_at: now,
            started_at: now,
            finished_at: None,
        })
    }

    pub fn resume_run(&self, run_id: &RunId) -> Result<Run, EngineError> {
        let result = self.connection.query_row(
            "SELECT run_id, project_id, name, status, epoch_ms(created_at), epoch_ms(started_at),
                    epoch_ms(finished_at)
             FROM dl.runs
             WHERE run_id = ?",
            [run_id.as_str()],
            |row| {
                Ok(StoredRun {
                    run_id: row.get(0)?,
                    project_id: row.get(1)?,
                    name: row.get(2)?,
                    status: row.get(3)?,
                    created_at_millis: row.get(4)?,
                    started_at_millis: row.get(5)?,
                    finished_at_millis: row.get(6)?,
                })
            },
        );
        let stored = match result {
            Ok(stored) => stored,
            Err(duckdb::Error::QueryReturnedNoRows) => {
                return Err(EngineError::RunNotFound {
                    run_id: run_id.as_str().to_owned(),
                });
            }
            Err(source) => return Err(source.into()),
        };

        stored.into_run()
    }

    fn run_exists(&self, run_id: &RunId) -> Result<bool, EngineError> {
        let count: i64 = self.connection.query_row(
            "SELECT count(*) FROM dl.runs WHERE run_id = ?",
            [run_id.as_str()],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }
}

struct StoredRun {
    run_id: String,
    project_id: String,
    name: String,
    status: String,
    created_at_millis: i64,
    started_at_millis: i64,
    finished_at_millis: Option<i64>,
}

impl StoredRun {
    fn into_run(self) -> Result<Run, EngineError> {
        Ok(Run {
            run_id: RunId::from_string(self.run_id),
            project_id: ProjectId::from_string(self.project_id),
            name: self.name,
            status: run_status_from_str(&self.status)?,
            created_at: timestamp_from_millis("created_at", self.created_at_millis)?,
            started_at: timestamp_from_millis("started_at", self.started_at_millis)?,
            finished_at: self
                .finished_at_millis
                .map(|millis| timestamp_from_millis("finished_at", millis))
                .transpose()?,
        })
    }
}

const fn status_as_str(status: RunStatus) -> &'static str {
    match status {
        RunStatus::Running => "running",
        RunStatus::Finished => "finished",
        RunStatus::Failed => "failed",
    }
}

fn run_status_from_str(status: &str) -> Result<RunStatus, EngineError> {
    match status {
        "running" => Ok(RunStatus::Running),
        "finished" => Ok(RunStatus::Finished),
        "failed" => Ok(RunStatus::Failed),
        _ => Err(EngineError::InvalidRunStatus {
            status: status.to_owned(),
        }),
    }
}

fn timestamp_as_rfc3339(timestamp: DateTime<Utc>) -> String {
    timestamp.to_rfc3339()
}

fn timestamp_from_millis(field: &'static str, millis: i64) -> Result<DateTime<Utc>, EngineError> {
    DateTime::from_timestamp_millis(millis).ok_or(EngineError::InvalidTimestamp { field, millis })
}
