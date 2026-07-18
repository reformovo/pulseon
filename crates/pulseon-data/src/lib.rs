//! Viewport-aware access to PulseOn metric-point Parquet data.

#![forbid(unsafe_code)]

mod query;
mod schema;

use std::error::Error;
use std::fmt;

pub use query::{
    DuckDbMetricStore, MetricPointRow, MetricQuery, MetricQueryPlan, MetricQueryResult,
    ParquetSource, QueryBudget, StepViewport,
};
pub use schema::{ColumnSchema, SchemaReport, validate_metric_point_schema};

/// Failures returned by PulseOn data inspection and querying.
#[derive(Debug)]
pub enum DataError {
    EmptySource,
    InvalidIdentity,
    InvalidViewport,
    InvalidQueryBudget,
    MissingColumn {
        name: &'static str,
    },
    DuplicateColumn {
        name: String,
    },
    IncompatibleColumnType {
        name: &'static str,
        expected: &'static str,
        actual: String,
    },
    IncompatibleAdditiveColumn {
        name: String,
    },
    DuckDb(duckdb::Error),
}

impl fmt::Display for DataError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptySource => formatter.write_str("Parquet source must not be empty"),
            Self::InvalidIdentity => formatter.write_str("run id and metric key must not be empty"),
            Self::InvalidViewport => {
                formatter.write_str("step viewport bounds must form a non-empty half-open range")
            }
            Self::InvalidQueryBudget => {
                formatter.write_str("query pixel width and point density must be positive")
            }
            Self::MissingColumn { name } => {
                write!(
                    formatter,
                    "metric_points is missing required column `{name}`"
                )
            }
            Self::DuplicateColumn { name } => {
                write!(
                    formatter,
                    "metric_points contains duplicate column `{name}`"
                )
            }
            Self::IncompatibleColumnType {
                name,
                expected,
                actual,
            } => write!(
                formatter,
                "metric_points column `{name}` must be {expected}, found {actual}"
            ),
            Self::IncompatibleAdditiveColumn { name } => write!(
                formatter,
                "additive metric_points column `{name}` must be nullable"
            ),
            Self::DuckDb(source) => write!(formatter, "DuckDB data query failed: {source}"),
        }
    }
}

impl Error for DataError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::DuckDb(source) => Some(source),
            _ => None,
        }
    }
}

impl From<duckdb::Error> for DataError {
    fn from(source: duckdb::Error) -> Self {
        Self::DuckDb(source)
    }
}
