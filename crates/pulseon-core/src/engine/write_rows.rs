use crate::engine::EngineError;
use crate::engine::time::timestamp_from_millis;
use crate::model::metric::{MetricAggregate, MetricKey, MetricPoint, Step};
use crate::model::run::{Run, RunId, RunStatus};
use crate::model::types::ProjectId;

pub struct StoredRun {
    pub run_id: String,
    pub project_id: String,
    pub name: String,
    pub status: String,
    pub created_at_millis: i64,
    pub started_at_millis: i64,
    pub finished_at_millis: Option<i64>,
}

impl StoredRun {
    pub fn into_run(self) -> Result<Run, EngineError> {
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

pub struct StoredMetricPoint {
    pub run_id: String,
    pub metric_key: String,
    pub step: i64,
    pub timestamp_millis: i64,
    pub value_f64: f64,
    pub ingested_at_millis: i64,
}

pub struct StoredMetricAggregate {
    pub run_id: String,
    pub metric_key: String,
    pub effective_count: u64,
    pub last_step: i64,
    pub last_value_f64: f64,
    pub min_value_f64: f64,
    pub max_value_f64: f64,
}

impl StoredMetricAggregate {
    pub fn into_metric_aggregate(self) -> MetricAggregate {
        MetricAggregate {
            run_id: RunId::from_string(self.run_id),
            metric_key: MetricKey::from_string(self.metric_key),
            effective_count: self.effective_count,
            last_step: Step::new(self.last_step),
            last_value_f64: self.last_value_f64,
            min_value_f64: self.min_value_f64,
            max_value_f64: self.max_value_f64,
        }
    }
}

impl StoredMetricPoint {
    pub fn into_metric_point(self) -> Result<MetricPoint, EngineError> {
        Ok(MetricPoint {
            run_id: RunId::from_string(self.run_id),
            metric_key: MetricKey::from_string(self.metric_key),
            step: Step::new(self.step),
            timestamp: timestamp_from_millis("timestamp", self.timestamp_millis)?,
            value_f64: self.value_f64,
            ingested_at: timestamp_from_millis("ingested_at", self.ingested_at_millis)?,
        })
    }
}

pub const fn status_as_str(status: RunStatus) -> &'static str {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_status_round_trips_storage_values() {
        let statuses = [
            (RunStatus::Running, "running"),
            (RunStatus::Finished, "finished"),
            (RunStatus::Failed, "failed"),
        ];

        for (status, raw) in statuses {
            assert_eq!(status_as_str(status), raw);
            assert_eq!(run_status_from_str(raw).unwrap(), status);
        }
    }

    #[test]
    fn run_status_from_str_rejects_unknown_storage_value() {
        let err = run_status_from_str("paused").unwrap_err();

        assert!(
            matches!(
                err,
                EngineError::InvalidRunStatus { ref status } if status == "paused"
            ),
            "expected invalid run status error, got {err:?}",
        );
    }
}
