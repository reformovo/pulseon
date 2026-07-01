use std::path::PathBuf;

use pyo3::create_exception;
use pyo3::exceptions::{PyRuntimeError, PyTypeError};
use pyo3::prelude::*;
use pyo3::types::{PyAny, PyTuple};

use crate::engine::client::{NativeClient, NativeRun};
use crate::engine::reporting::MetricReporterDiagnostics;
use crate::model::metric::{MetricAggregate, MetricKey, MetricPoint, Step};
use crate::model::run::{RunId, RunStatus};
use crate::model::types::{Project, ProjectId};

create_exception!(
    pulseon._pulseon,
    PulseOnError,
    PyRuntimeError,
    "Base class for PulseOn SDK errors."
);
create_exception!(
    pulseon._pulseon,
    DuplicateRunError,
    PulseOnError,
    "A run with the requested run_id already exists."
);
create_exception!(
    pulseon._pulseon,
    MissingProjectError,
    PulseOnError,
    "The requested project does not exist."
);
create_exception!(
    pulseon._pulseon,
    MissingRunError,
    PulseOnError,
    "The requested run does not exist."
);
create_exception!(
    pulseon._pulseon,
    DuckLakeUnavailableError,
    PulseOnError,
    "DuckLake could not be loaded or used."
);
create_exception!(
    pulseon._pulseon,
    QueryError,
    PulseOnError,
    "A metric query failed."
);

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

    pub fn get_project(&self, project_id: &str) -> PyResult<PyProject> {
        let project_id = ProjectId::from_string(project_id);
        self._inner
            .get_project(&project_id)
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

    pub fn get_run(&self, run_id: &str) -> PyResult<PyRun> {
        let run_id = RunId::from_string(run_id);
        self._inner
            .get_run(&run_id)
            .map(|run| PyRun::from(self._inner.run_handle(run)))
            .map_err(runtime_error)
    }

    pub fn resume_run(&self, run_id: &str) -> PyResult<PyRun> {
        let run_id = RunId::from_string(run_id);
        self._inner
            .resume_run(&run_id)
            .map(|run| PyRun::from(self._inner.run_handle(run)))
            .map_err(runtime_error)
    }

    pub fn list_runs(&self, project_id: &str) -> PyResult<Vec<PyRun>> {
        let project_id = ProjectId::from_string(project_id);
        self._inner
            .list_runs(&project_id)
            .map(|runs| {
                runs.into_iter()
                    .map(|run| PyRun::from(self._inner.run_handle(run)))
                    .collect()
            })
            .map_err(runtime_error)
    }

    #[pyo3(signature = (project_id=None))]
    pub fn list_orphan_runs(&self, project_id: Option<String>) -> PyResult<Vec<PyRun>> {
        let project_id = project_id.map(ProjectId::from_string);
        self._inner
            .list_orphan_runs(project_id.as_ref())
            .map(|runs| {
                runs.into_iter()
                    .map(|run| PyRun::from(self._inner.run_handle(run)))
                    .collect()
            })
            .map_err(runtime_error)
    }

    pub fn finish_run(&self, run_id: &str) -> PyResult<PyRun> {
        let run_id = RunId::from_string(run_id);
        self._inner
            .finish_run(&run_id)
            .map(|run| PyRun::from(self._inner.run_handle(run)))
            .map_err(runtime_error)
    }

    pub fn fail_run(&self, run_id: &str) -> PyResult<PyRun> {
        let run_id = RunId::from_string(run_id);
        self._inner
            .fail_run(&run_id)
            .map(|run| PyRun::from(self._inner.run_handle(run)))
            .map_err(runtime_error)
    }

    pub fn shutdown(&self) -> bool {
        self._inner.shutdown()
    }

    pub fn __enter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    pub fn __exit__(
        &self,
        _exc_type: &Bound<'_, PyAny>,
        _exc_value: &Bound<'_, PyAny>,
        _traceback: &Bound<'_, PyAny>,
    ) -> bool {
        self.shutdown();
        false
    }

    pub fn diagnostics(&self) -> PyDiagnostics {
        PyDiagnostics::from(self._inner.diagnostics())
    }

    #[pyo3(signature = (run_id, metric_key, start_step=None, end_step=None, max_points=None))]
    pub fn query_metric(
        &self,
        run_id: &str,
        metric_key: &str,
        start_step: Option<i64>,
        end_step: Option<i64>,
        max_points: Option<usize>,
    ) -> PyResult<Vec<PyMetricPoint>> {
        let run_id = RunId::from_string(run_id);
        let metric_key = MetricKey::from_string(metric_key);
        self._inner
            .query_metric(
                &run_id,
                &metric_key,
                start_step.map(Step::new),
                end_step.map(Step::new),
                max_points,
            )
            .map(|points| points.into_iter().map(PyMetricPoint::from).collect())
            .map_err(runtime_error)
    }

    pub fn query_metric_summaries(
        &self,
        run_ids: Vec<String>,
        metric_key: &str,
    ) -> PyResult<Vec<PyMetricSummary>> {
        let run_ids: Vec<RunId> = run_ids.into_iter().map(RunId::from_string).collect();
        let metric_key = MetricKey::from_string(metric_key);
        self._inner
            .query_metric_summaries(&run_ids, &metric_key)
            .map(|summaries| summaries.into_iter().map(PyMetricSummary::from).collect())
            .map_err(runtime_error)
    }

    pub fn list_metrics(&self, run_id: &str) -> PyResult<Vec<PyMetricSummary>> {
        let run_id = RunId::from_string(run_id);
        self._inner
            .list_metrics(&run_id)
            .map(|metrics| metrics.into_iter().map(PyMetricSummary::from).collect())
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
    #[pyo3(get)]
    pending_reports: u64,
    #[pyo3(get)]
    writer_drained: bool,
    #[pyo3(get)]
    last_write_error: Option<String>,
}

impl From<MetricReporterDiagnostics> for PyDiagnostics {
    fn from(diagnostics: MetricReporterDiagnostics) -> Self {
        Self {
            accepted_reports: diagnostics.accepted_reports,
            dropped_reports: diagnostics.dropped_reports,
            failed_reports: diagnostics.failed_reports,
            pending_reports: diagnostics.pending_reports,
            writer_drained: diagnostics.writer_drained,
            last_write_error: diagnostics.last_write_error,
        }
    }
}

#[pyclass(name = "MetricPoint", module = "pulseon._pulseon")]
pub struct PyMetricPoint {
    #[pyo3(get)]
    run_id: String,
    #[pyo3(get)]
    metric_key: String,
    #[pyo3(get)]
    step: i64,
    #[pyo3(get)]
    timestamp: String,
    #[pyo3(get)]
    value_f64: f64,
    #[pyo3(get)]
    ingested_at: String,
}

impl From<MetricPoint> for PyMetricPoint {
    fn from(point: MetricPoint) -> Self {
        Self {
            run_id: point.run_id.as_str().to_owned(),
            metric_key: point.metric_key.as_str().to_owned(),
            step: point.step.value(),
            timestamp: point.timestamp.to_rfc3339(),
            value_f64: point.value_f64,
            ingested_at: point.ingested_at.to_rfc3339(),
        }
    }
}

#[pyclass(name = "MetricSummary", module = "pulseon._pulseon")]
pub struct PyMetricSummary {
    #[pyo3(get)]
    run_id: String,
    #[pyo3(get)]
    metric_key: String,
    #[pyo3(get)]
    effective_count: u64,
    #[pyo3(get)]
    last_step: i64,
    #[pyo3(get)]
    last_value_f64: f64,
    #[pyo3(get)]
    min_value_f64: f64,
    #[pyo3(get)]
    max_value_f64: f64,
}

impl From<MetricAggregate> for PyMetricSummary {
    fn from(summary: MetricAggregate) -> Self {
        Self {
            run_id: summary.run_id.as_str().to_owned(),
            metric_key: summary.metric_key.as_str().to_owned(),
            effective_count: summary.effective_count,
            last_step: summary.last_step.value(),
            last_value_f64: summary.last_value_f64,
            min_value_f64: summary.min_value_f64,
            max_value_f64: summary.max_value_f64,
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
    let message = error.to_string();
    match error {
        crate::engine::EngineError::RunAlreadyExists { .. } => DuplicateRunError::new_err(message),
        crate::engine::EngineError::ProjectNotFound { .. } => MissingProjectError::new_err(message),
        crate::engine::EngineError::RunNotFound { .. } => MissingRunError::new_err(message),
        crate::engine::EngineError::LttbExtensionUnavailable { .. }
        | crate::engine::EngineError::MetricQueryMaxPointsTooLarge { .. } => {
            QueryError::new_err(message)
        }
        crate::engine::EngineError::DuckDb(source) if is_ducklake_error(&source) => {
            DuckLakeUnavailableError::new_err(message)
        }
        crate::engine::EngineError::DuckDb(_) => QueryError::new_err(message),
        _ => PulseOnError::new_err(message),
    }
}

fn is_ducklake_error(error: &duckdb::Error) -> bool {
    error.to_string().to_lowercase().contains("ducklake")
}

fn status_as_string(status: RunStatus) -> String {
    match status {
        RunStatus::Running => "running",
        RunStatus::Finished => "finished",
        RunStatus::Failed => "failed",
    }
    .to_owned()
}
