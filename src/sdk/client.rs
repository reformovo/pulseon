use std::path::PathBuf;

use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;

use crate::engine::client::NativeClient;

#[pyclass(name = "Client", module = "pulseon._pulseon", unsendable)]
pub struct PyClient {
    _inner: NativeClient,
}

impl PyClient {
    fn new(inner: NativeClient) -> Self {
        Self { _inner: inner }
    }
}

#[pyfunction]
pub fn init(path: PathBuf) -> PyResult<PyClient> {
    NativeClient::open(path)
        .map(PyClient::new)
        .map_err(|error| PyRuntimeError::new_err(error.to_string()))
}
