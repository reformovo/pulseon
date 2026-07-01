use std::path::PathBuf;

use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;

use crate::engine::client::NativeClient;
use crate::model::run::{Run, RunId, RunStatus};
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
            .map(PyRun::from)
            .map_err(runtime_error)
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
    #[pyo3(get)]
    run_id: String,
    #[pyo3(get)]
    project_id: String,
    #[pyo3(get)]
    name: String,
    #[pyo3(get)]
    status: String,
    #[pyo3(get)]
    created_at: String,
    #[pyo3(get)]
    started_at: String,
    #[pyo3(get)]
    finished_at: Option<String>,
}

impl From<Run> for PyRun {
    fn from(run: Run) -> Self {
        Self {
            run_id: run.run_id.as_str().to_owned(),
            project_id: run.project_id.as_str().to_owned(),
            name: run.name,
            status: status_as_string(run.status),
            created_at: run.created_at.to_rfc3339(),
            started_at: run.started_at.to_rfc3339(),
            finished_at: run.finished_at.map(|timestamp| timestamp.to_rfc3339()),
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
