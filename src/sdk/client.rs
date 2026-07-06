use std::path::{Path, PathBuf};
use std::time::Duration;

use pyo3::create_exception;
use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;
use pyo3::types::PyAny;

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

macro_rules! sdk_exception {
    ($name:ident, $message:literal) => {
        create_exception!(pulseon._pulseon, $name, PulseOnError, $message);
    };
}

sdk_exception!(MetricQueueFullError, "The metric queue is full.");
sdk_exception!(MetricWriterFailedError, "The metric writer failed.");
sdk_exception!(MetricDrainTimeoutError, "Metric drain timed out.");
sdk_exception!(MetricFlushError, "Metric flush failed.");
sdk_exception!(MetricFlushTimeoutError, "Metric flush timed out.");
sdk_exception!(RunClosedError, "The run is closed for metric reporting.");
sdk_exception!(ClientClosedError, "The client is closed.");
sdk_exception!(
    InvalidRunStateError,
    "The run state does not allow this operation."
);
sdk_exception!(
    RunAlreadyExistsError,
    "A run with the requested run_id already exists."
);
sdk_exception!(
    RunAlreadyActiveError,
    "The requested run already has an active writer."
);
sdk_exception!(
    InvalidConfigurationError,
    "PulseOn configuration is invalid."
);
sdk_exception!(StorageError, "A storage operation failed.");

#[pyclass(name = "Client", module = "pulseon._pulseon", unsendable)]
pub struct PyClient {
    _inner: NativeClient,
    context_shutdown_timeout: Option<Duration>,
}

impl PyClient {
    fn new(inner: NativeClient, context_shutdown_timeout: Option<Duration>) -> Self {
        Self {
            _inner: inner,
            context_shutdown_timeout,
        }
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

    #[pyo3(signature = (run_id, timeout=None))]
    pub fn finish_run(&self, run_id: &str, timeout: Option<f64>) -> PyResult<PyRun> {
        let run_id = RunId::from_string(run_id);
        let timeout = timeout
            .map(|seconds| duration_from_seconds("finish_run timeout", seconds))
            .transpose()?;
        self._inner
            .finish_run_with_timeout(&run_id, timeout)
            .map(|run| PyRun::from(self._inner.run_handle(run)))
            .map_err(runtime_error)
    }

    #[pyo3(signature = (run_id, timeout=None))]
    pub fn fail_run(&self, run_id: &str, timeout: Option<f64>) -> PyResult<PyRun> {
        let run_id = RunId::from_string(run_id);
        let timeout = timeout
            .map(|seconds| duration_from_seconds("fail_run timeout", seconds))
            .transpose()?;
        self._inner
            .fail_run_with_timeout(&run_id, timeout)
            .map(|run| PyRun::from(self._inner.run_handle(run)))
            .map_err(runtime_error)
    }

    #[pyo3(signature = (run_id, timeout=None))]
    pub fn flush_run_data(&self, run_id: &str, timeout: Option<f64>) -> PyResult<()> {
        let run_id = RunId::from_string(run_id);
        let timeout = timeout
            .map(|seconds| duration_from_seconds("flush timeout", seconds))
            .transpose()?;
        self._inner
            .flush_run_data(&run_id, timeout)
            .map_err(runtime_error)
    }

    #[pyo3(signature = (timeout=None))]
    pub fn shutdown(&self, timeout: Option<f64>) -> PyResult<()> {
        let timeout = timeout
            .map(|seconds| duration_from_seconds("shutdown timeout", seconds))
            .transpose()?;
        self._inner.shutdown(timeout).map_err(runtime_error)
    }

    pub fn __enter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    pub fn __exit__(
        &self,
        exc_type: &Bound<'_, PyAny>,
        exc_value: &Bound<'_, PyAny>,
        _traceback: &Bound<'_, PyAny>,
    ) -> PyResult<bool> {
        let user_exception_active = !exc_type.is_none();
        match self._inner.shutdown(self.context_shutdown_timeout) {
            Ok(()) => Ok(false),
            Err(error) if user_exception_active => {
                attach_exception_context(exc_value, runtime_error(error));
                Ok(false)
            }
            Err(error) => Err(runtime_error(error)),
        }
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

    pub fn log(&self, key: &str, step: i64, value: f64) -> PyResult<()> {
        self.inner
            .log_metric_at_step(key, step, value)
            .map_err(runtime_error)
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
    pending_reports: u64,
    #[pyo3(get)]
    queue_full_errors: u64,
    #[pyo3(get)]
    persisted_reports: u64,
    #[pyo3(get)]
    writer_state: &'static str,
    #[pyo3(get)]
    last_write_error: Option<String>,
    #[pyo3(get)]
    last_flush_run_id: Option<String>,
    #[pyo3(get)]
    last_flush_status: &'static str,
    #[pyo3(get)]
    last_flush_error: Option<String>,
}

impl From<MetricReporterDiagnostics> for PyDiagnostics {
    fn from(diagnostics: MetricReporterDiagnostics) -> Self {
        Self {
            pending_reports: diagnostics.pending_reports,
            queue_full_errors: diagnostics.queue_full_errors,
            persisted_reports: diagnostics.persisted_reports,
            writer_state: diagnostics.writer_state,
            last_write_error: diagnostics.last_write_error,
            last_flush_run_id: diagnostics.last_flush_run_id,
            last_flush_status: diagnostics.last_flush_status,
            last_flush_error: diagnostics.last_flush_error,
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
#[pyo3(
    signature = (
        path,
        *,
        data_path=None,
        catalog_backend="duckdb",
        catalog_path=None,
        metric_queue_capacity=65536,
        context_shutdown_timeout=None
    )
)]
pub fn init(
    path: PathBuf,
    data_path: Option<PathBuf>,
    catalog_backend: &str,
    catalog_path: Option<PathBuf>,
    metric_queue_capacity: i64,
    context_shutdown_timeout: Option<f64>,
) -> PyResult<PyClient> {
    let context_shutdown_timeout = context_shutdown_timeout
        .map(|seconds| duration_from_seconds("context_shutdown_timeout", seconds))
        .transpose()?;
    let metric_queue_capacity = validate_init_config(
        data_path.as_deref(),
        catalog_backend,
        catalog_path.as_deref(),
        metric_queue_capacity,
    )?;
    NativeClient::open_with_storage_config(path, catalog_path, data_path, metric_queue_capacity)
        .map(|client| PyClient::new(client, context_shutdown_timeout))
        .map_err(runtime_error)
}

fn validate_init_config(
    data_path: Option<&Path>,
    catalog_backend: &str,
    catalog_path: Option<&Path>,
    metric_queue_capacity: i64,
) -> PyResult<usize> {
    if !(1..=1_048_576).contains(&metric_queue_capacity) {
        return Err(InvalidConfigurationError::new_err(
            "metric_queue_capacity must be between 1 and 1048576",
        ));
    }
    if catalog_backend == "sqlite" {
        return Err(InvalidConfigurationError::new_err(
            "catalog_backend='sqlite' is deferred until real DuckLake-backed SQLite tests pass",
        ));
    }
    if catalog_backend != "duckdb" {
        return Err(InvalidConfigurationError::new_err(format!(
            "unsupported catalog_backend: {catalog_backend}"
        )));
    }
    if data_path.is_some_and(is_uri_path) {
        return Err(InvalidConfigurationError::new_err(
            "data_path must be a local filesystem path",
        ));
    }
    if catalog_path.is_some_and(is_uri_path) {
        return Err(InvalidConfigurationError::new_err(
            "catalog_path must be a local filesystem path",
        ));
    }
    usize::try_from(metric_queue_capacity).map_err(|_| {
        InvalidConfigurationError::new_err("metric_queue_capacity must be between 1 and 1048576")
    })
}

fn is_uri_path(path: &Path) -> bool {
    path.to_string_lossy().contains("://")
}

fn duration_from_seconds(name: &str, seconds: f64) -> PyResult<Duration> {
    if seconds < 0.0 || !seconds.is_finite() {
        return Err(InvalidConfigurationError::new_err(format!(
            "{name} must be a finite non-negative number"
        )));
    }
    Ok(Duration::from_secs_f64(seconds))
}

fn attach_exception_context(exc_value: &Bound<'_, PyAny>, teardown_error: PyErr) {
    if exc_value.is_none() {
        return;
    }
    let py = exc_value.py();
    let teardown_value = teardown_error.value(py);
    let _ = exc_value.setattr("__context__", teardown_value);
}

fn runtime_error(error: crate::engine::EngineError) -> PyErr {
    let message = error.to_string();
    match error {
        crate::engine::EngineError::RunAlreadyExists { .. } => {
            RunAlreadyExistsError::new_err(message)
        }
        crate::engine::EngineError::RunAlreadyActive { .. } => {
            RunAlreadyActiveError::new_err(message)
        }
        crate::engine::EngineError::RunClosed { .. } => RunClosedError::new_err(message),
        crate::engine::EngineError::InvalidRunTransition { .. } => {
            InvalidRunStateError::new_err(message)
        }
        crate::engine::EngineError::DuckDb(_)
        | crate::engine::EngineError::Io(_)
        | crate::engine::EngineError::ProjectAlreadyExists { .. }
        | crate::engine::EngineError::ProjectNotFound { .. }
        | crate::engine::EngineError::RunNotFound { .. }
        | crate::engine::EngineError::LttbExtensionUnavailable { .. }
        | crate::engine::EngineError::Storage { .. }
        | crate::engine::EngineError::StorageDuckDb { .. } => StorageError::new_err(message),
        crate::engine::EngineError::MetricQueryMaxPointsTooLarge { .. } => {
            PulseOnError::new_err(message)
        }
        crate::engine::EngineError::MetricQueueFull => MetricQueueFullError::new_err(message),
        crate::engine::EngineError::MetricWriterFailed { .. } => {
            MetricWriterFailedError::new_err(message)
        }
        crate::engine::EngineError::MetricDrainTimeout => MetricDrainTimeoutError::new_err(message),
        crate::engine::EngineError::MetricFlush { .. } => MetricFlushError::new_err(message),
        crate::engine::EngineError::MetricFlushTimeout => MetricFlushTimeoutError::new_err(message),
        crate::engine::EngineError::ClientClosed => ClientClosedError::new_err(message),
        _ => PulseOnError::new_err(message),
    }
}

fn status_as_string(status: RunStatus) -> String {
    match status {
        RunStatus::Running => "running",
        RunStatus::Finished => "finished",
        RunStatus::Failed => "failed",
    }
    .to_owned()
}
