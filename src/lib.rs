// PulseOn native v1 Python extension.
// Architecture ref: docs/v1-native-architecture.md
//
// Module layout:
//   model/  - native v1 domain types
//   engine/ - DuckLake-backed client, reporting, writes, and queries
//   sdk/    - PyO3 bindings

#[cfg(test)]
mod ducklake_test_support;
pub mod engine;
pub mod model;
#[cfg(test)]
mod native_engine_behavior;
mod sdk;

use pyo3::prelude::*;

#[pymodule]
fn _pulseon(m: &Bound<'_, PyModule>) -> PyResult<()> {
    let py = m.py();
    m.add_class::<sdk::arrow::PyArrowTable>()?;
    m.add_class::<sdk::client::PyClient>()?;
    m.add_class::<sdk::client::PyDiagnostics>()?;
    m.add_class::<sdk::client::PyMetricPoint>()?;
    m.add_class::<sdk::client::PyMetricSummary>()?;
    m.add_class::<sdk::client::PyProject>()?;
    m.add_class::<sdk::client::PyRun>()?;
    m.add("PulseOnError", py.get_type::<sdk::client::PulseOnError>())?;
    macro_rules! add_exception {
        ($name:ident) => {
            m.add(stringify!($name), py.get_type::<sdk::client::$name>())?;
        };
    }
    add_exception!(MetricQueueFullError);
    add_exception!(MetricWriterFailedError);
    add_exception!(MetricDrainTimeoutError);
    add_exception!(MetricFlushError);
    add_exception!(MetricFlushTimeoutError);
    add_exception!(RunClosedError);
    add_exception!(ClientClosedError);
    add_exception!(InvalidRunStateError);
    add_exception!(RunAlreadyExistsError);
    add_exception!(RunAlreadyActiveError);
    add_exception!(InvalidConfigurationError);
    add_exception!(StorageError);
    m.add_function(wrap_pyfunction!(sdk::client::init, m)?)?;
    Ok(())
}
