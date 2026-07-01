// PulseOn — training metrics tracking with AI Native auto-research support.
// Architecture ref: docs/native-architecture.md
//
// Module layout (§6.1):
//   model/    — logical data model (pure types, no I/O)
//   catalog/  — CatalogLayer trait + DuckLake/SQLite impl
//   storage/  — StorageLayer trait + local/S3 impls
//   compute/  — ComputeLayer trait + QueryInterface + DuckDB impl
//   engine/   — orchestration: write path, flush, client lifecycle
//   sdk/      — PyO3 bindings (pyo3 dependency isolated here)

mod catalog;
mod compute;
#[cfg(test)]
mod ducklake_probe;
#[cfg(test)]
mod ducklake_test_support;
pub mod engine;
pub mod model;
mod sdk;
mod storage;

use pyo3::prelude::*;

#[pymodule]
fn _pulseon(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<sdk::client::PyClient>()?;
    m.add_function(wrap_pyfunction!(sdk::client::init, m)?)?;
    Ok(())
}
