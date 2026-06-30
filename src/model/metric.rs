use chrono::{DateTime, Utc};

use crate::model::run::RunId;

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct MetricKey(String);

impl MetricKey {
    pub fn from_string(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct Step(i64);

impl Step {
    pub const fn new(value: i64) -> Self {
        Self(value)
    }

    pub const fn value(self) -> i64 {
        self.0
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct MetricPoint {
    pub run_id: RunId,
    pub metric_key: MetricKey,
    pub step: Step,
    pub timestamp: DateTime<Utc>,
    pub value_f64: f64,
    pub ingested_at: DateTime<Utc>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MetricAggregate {
    pub run_id: RunId,
    pub metric_key: MetricKey,
    pub effective_count: u64,
    pub last_step: Step,
    pub last_value_f64: f64,
    pub min_value_f64: f64,
    pub max_value_f64: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MetricQuery {
    pub run_id: RunId,
    pub metric_key: MetricKey,
    pub start_step: Option<Step>,
    pub end_step: Option<Step>,
    pub max_points: Option<usize>,
}
