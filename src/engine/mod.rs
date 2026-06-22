// engine/mod.rs — 编排层：持有三层引用，实现写入/查询路径
// Architecture ref: docs/native-architecture.md §4, §5, §6.1

pub mod client;
pub mod write;
pub mod flush;

// Unified error type for engine operations.
// Merged from engine/error.rs per oracle review (simplify: extract to
// error.rs when this grows beyond a handful of variants).
// Note: both CatalogError and EngineError wrap duckdb::Error via #[from].
// If engine calls catalog: DuckDB → CatalogError → EngineError::Catalog.
// If engine calls DuckDB directly: DuckDB → EngineError::DuckDb.
#[allow(dead_code)]  // used in Phase 4; remove when first referenced
#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("catalog error: {0}")]
    Catalog(#[from] crate::catalog::CatalogError),
    #[error("duckdb error: {0}")]
    DuckDb(#[from] duckdb::Error),
    #[error("invalid input: {0}")]
    InvalidInput(String),
    #[error("not found: {0}")]
    NotFound(String),
}
