// Native v1 engine over DuckLake.
// Architecture ref: docs/v1-native-architecture.md

pub(crate) mod bootstrap;
pub mod client;
pub mod query;
pub mod reporting;
mod time;
pub mod write;
mod write_rows;

// Unified error type for engine operations.
// Merged from engine/error.rs per oracle review (simplify: extract to
// error.rs when this grows beyond a handful of variants).
#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("duckdb error: {0}")]
    DuckDb(#[from] duckdb::Error),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("native connection lock was poisoned")]
    ConnectionLockPoisoned,
    #[error("project already exists: {project_id}")]
    ProjectAlreadyExists { project_id: String },
    #[error("project not found: {project_id}")]
    ProjectNotFound { project_id: String },
    #[error("run already exists: {run_id}")]
    RunAlreadyExists { run_id: String },
    #[error("run not found: {run_id}")]
    RunNotFound { run_id: String },
    #[error("invalid run transition for {run_id}: {from} -> {to}")]
    InvalidRunTransition {
        run_id: String,
        from: &'static str,
        to: &'static str,
    },
    #[error("metric query max_points is too large for DuckDB LTTB: {max_points}")]
    MetricQueryMaxPointsTooLarge { max_points: usize },
    #[error("metric queue is full")]
    MetricQueueFull,
    #[error("metric writer failed: {message}")]
    MetricWriterFailed { message: String },
    #[error("DuckDB LTTB extension is unavailable: {message}")]
    LttbExtensionUnavailable { message: String },
    #[error("invalid stored run status: {status}")]
    InvalidRunStatus { status: String },
    #[error("invalid stored timestamp for {field}: {millis}")]
    InvalidTimestamp { field: &'static str, millis: i64 },
}
