// engine/mod.rs — 编排层：持有三层引用，实现写入/查询路径
// Architecture ref: docs/native-architecture.md §4, §5, §6.1

pub mod client;
pub mod flush;
mod time;
pub mod write;
mod write_rows;

// Unified error type for engine operations.
// Merged from engine/error.rs per oracle review (simplify: extract to
// error.rs when this grows beyond a handful of variants).
// Note: both CatalogError and EngineError wrap duckdb::Error via #[from].
// If engine calls catalog: DuckDB → CatalogError → EngineError::Catalog.
// If engine calls DuckDB directly: DuckDB → EngineError::DuckDb.
#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("catalog error: {0}")]
    Catalog(#[from] crate::catalog::CatalogError),
    #[error("duckdb error: {0}")]
    DuckDb(#[from] duckdb::Error),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("run already exists: {run_id}")]
    RunAlreadyExists { run_id: String },
    #[error("run not found: {run_id}")]
    RunNotFound { run_id: String },
    #[error("metric query max_points is too large for DuckDB LTTB: {max_points}")]
    MetricQueryMaxPointsTooLarge { max_points: usize },
    #[error("DuckDB LTTB extension is unavailable: {message}")]
    LttbExtensionUnavailable { message: String },
    #[error("invalid stored run status: {status}")]
    InvalidRunStatus { status: String },
    #[error("invalid stored timestamp for {field}: {millis}")]
    InvalidTimestamp { field: &'static str, millis: i64 },
}
