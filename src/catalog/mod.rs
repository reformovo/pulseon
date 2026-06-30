// catalog/mod.rs — CatalogLayer trait + 实现
// Architecture ref: docs/native-architecture.md §2.2, §6.1

pub mod cloud_impl;
pub mod ducklake_impl;
pub mod trait_def;
pub mod types;

// Unified error type for catalog operations.
// Merged from engine/error.rs per oracle review (simplify: 10-line enum
// doesn't need its own file yet — extract when it grows).
#[allow(dead_code)] // used in Phase 2; remove when first referenced
#[derive(Debug, thiserror::Error)]
pub enum CatalogError {
    #[error("duckdb error: {0}")]
    DuckDb(#[from] duckdb::Error),
    #[error("invalid input: {0}")]
    InvalidInput(String),
    #[error("not found: {0}")]
    NotFound(String),
}
