// PyO3 bindings for the native v1 Python API.
// Architecture ref: docs/v1-native-architecture.md

pub mod client;

// Unified error type for PyO3 bindings.
// Merged from sdk/error.rs per oracle review (simplify: extract when it grows).
#[allow(dead_code)] // kept for future SDK boundary conversions
#[derive(Debug, thiserror::Error)]
pub enum SdkError {
    #[error("engine error: {0}")]
    Engine(#[from] crate::engine::EngineError),
    #[error("python error: {0}")]
    Py(#[from] pyo3::PyErr),
}
