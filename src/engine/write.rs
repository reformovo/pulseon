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
        let now = Utc::now();
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
}

const fn status_as_str(status: RunStatus) -> &'static str {
    match status {
        RunStatus::Running => "running",
        RunStatus::Finished => "finished",
        RunStatus::Failed => "failed",
    }
}

fn timestamp_as_rfc3339(timestamp: DateTime<Utc>) -> String {
    timestamp.to_rfc3339()
}
