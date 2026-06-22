// sdk/mod.rs — PyO3 绑定层
// Architecture ref: docs/native-architecture.md §7.3, §8, §6.1

pub mod client;
pub mod run;
pub mod config;
pub mod query;

// Unified error type for PyO3 bindings.
// Merged from sdk/error.rs per oracle review (simplify: extract when it grows).
#[allow(dead_code)]  // used in Phase 5; remove when first referenced
#[derive(Debug, thiserror::Error)]
pub enum SdkError {
    #[error("engine error: {0}")]
    Engine(#[from] crate::engine::EngineError),
    #[error("python error: {0}")]
    Py(#[from] pyo3::PyErr),
}
