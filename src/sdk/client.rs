use std::path::PathBuf;

use pyo3::exceptions::{PyRuntimeError, PyTypeError};
use pyo3::prelude::*;
use pyo3::types::PyTuple;

use crate::engine::client::{NativeClient, NativeRun};
use crate::engine::reporting::MetricReporterDiagnostics;
use crate::model::run::{RunId, RunStatus};
use crate::model::types::{Project, ProjectId};

#[pyclass(name = "Client", module = "pulseon._pulseon", unsendable)]
pub struct PyClient {
    _inner: NativeClient,
}

impl PyClient {
    fn new(inner: NativeClient) -> Self {
        Self { _inner: inner }
    }
}

#[pymethods]
impl PyClient {
    #[pyo3(signature = (name, project_id=None))]
    pub fn create_project(&self, name: &str, project_id: Option<String>) -> PyResult<PyProject> {
        let project_id = project_id.map(ProjectId::from_string);
        self._inner
            .create_project(name, project_id)
            .map(PyProject::from)
            .map_err(runtime_error)
    }

    #[pyo3(signature = (project_id, name, run_id=None))]
    pub fn create_run(
        &self,
        project_id: &str,
        name: &str,
        run_id: Option<String>,
    ) -> PyResult<PyRun> {
        let project_id = ProjectId::from_string(project_id);
        let run_id = run_id.map(RunId::from_string);
        self._inner
            .create_run(&project_id, name, run_id)
            .map(|run| PyRun::from(self._inner.run_handle(run)))
            .map_err(runtime_error)
    }

    pub fn diagnostics(&self) -> PyDiagnostics {
        PyDiagnostics::from(self._inner.diagnostics())
    }
}

#[pyclass(name = "Project", module = "pulseon._pulseon")]
pub struct PyProject {
    #[pyo3(get)]
    project_id: String,
    #[pyo3(get)]
    name: String,
    #[pyo3(get)]
    created_at: String,
}

impl From<Project> for PyProject {
    fn from(project: Project) -> Self {
        Self {
            project_id: project.project_id.as_str().to_owned(),
            name: project.name,
            created_at: project.created_at.to_rfc3339(),
        }
    }
}

#[pyclass(name = "Run", module = "pulseon._pulseon")]
pub struct PyRun {
    inner: NativeRun,
}

#[pymethods]
impl PyRun {
    #[getter]
    pub fn run_id(&self) -> String {
        self.inner.run_id.as_str().to_owned()
    }

    #[getter]
    pub fn project_id(&self) -> String {
        self.inner.project_id.as_str().to_owned()
    }

    #[getter]
    pub fn name(&self) -> &str {
        &self.inner.name
    }

    #[getter]
    pub fn status(&self) -> String {
        status_as_string(self.inner.status)
    }

    #[getter]
    pub fn created_at(&self) -> String {
        self.inner.created_at.to_rfc3339()
    }

    #[getter]
    pub fn started_at(&self) -> String {
        self.inner.started_at.to_rfc3339()
    }

    #[getter]
    pub fn finished_at(&self) -> Option<String> {
        self.inner
            .finished_at
            .map(|timestamp| timestamp.to_rfc3339())
    }

    #[pyo3(signature = (key, *args))]
    pub fn log(&self, key: &str, args: &Bound<'_, PyTuple>) -> PyResult<()> {
        match args.len() {
            1 => {
                let value = args.get_item(0)?.extract::<f64>()?;
                self.inner.log_metric(key, value);
                Ok(())
            }
            2 => {
                let step = args.get_item(0)?.extract::<i64>()?;
                let value = args.get_item(1)?.extract::<f64>()?;
                self.inner.log_metric_at_step(key, step, value);
                Ok(())
            }
            _ => Err(PyTypeError::new_err(
                "log() expects (key, value) or (key, step, value)",
            )),
        }
    }
}

impl From<NativeRun> for PyRun {
    fn from(run: NativeRun) -> Self {
        Self { inner: run }
    }
}

#[pyclass(name = "Diagnostics", module = "pulseon._pulseon")]
pub struct PyDiagnostics {
    #[pyo3(get)]
    accepted_reports: u64,
    #[pyo3(get)]
    dropped_reports: u64,
    #[pyo3(get)]
    failed_reports: u64,
}

impl From<MetricReporterDiagnostics> for PyDiagnostics {
    fn from(diagnostics: MetricReporterDiagnostics) -> Self {
        Self {
            accepted_reports: diagnostics.accepted_reports,
            dropped_reports: diagnostics.dropped_reports,
            failed_reports: diagnostics.failed_reports,
        }
    }
}

#[pyfunction]
pub fn init(path: PathBuf) -> PyResult<PyClient> {
    NativeClient::open(path)
        .map(PyClient::new)
        .map_err(runtime_error)
}

fn runtime_error(error: crate::engine::EngineError) -> PyErr {
    PyRuntimeError::new_err(error.to_string())
}

fn status_as_string(status: RunStatus) -> String {
    match status {
        RunStatus::Running => "running",
        RunStatus::Finished => "finished",
        RunStatus::Failed => "failed",
    }
    .to_owned()
}
