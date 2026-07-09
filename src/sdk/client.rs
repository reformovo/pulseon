use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use pyo3::create_exception;
use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;
use pyo3::types::{PyAny, PyDict, PyDictMethods, PyModule};

use crate::engine::bootstrap::{CatalogBackend, S3ConnectionConfig};
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
        match self._inner.shutdown(None) {
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
        path=PathBuf::from("."),
        *,
        data_path=None,
        catalog_backend="duckdb",
        catalog_path=None,
        metric_queue_capacity=65536,
        s3_endpoint=None,
        s3_access_key_id=None,
        s3_secret_access_key=None,
        s3_session_token=None,
        s3_region=None,
        s3_path_style=None,
        s3_use_ssl=None,
    )
)]
#[expect(
    clippy::too_many_arguments,
    reason = "PyO3 exposes init configuration as keyword-only Python API arguments."
)]
pub fn init(
    py: Python<'_>,
    path: PathBuf,
    data_path: Option<PathBuf>,
    catalog_backend: &str,
    catalog_path: Option<PathBuf>,
    metric_queue_capacity: i64,
    s3_endpoint: Option<String>,
    s3_access_key_id: Option<String>,
    s3_secret_access_key: Option<String>,
    s3_session_token: Option<String>,
    s3_region: Option<String>,
    s3_path_style: Option<bool>,
    s3_use_ssl: Option<bool>,
) -> PyResult<PyClient> {
    let project_config = load_project_config(py, &path)?;
    let data_path = resolve_data_path(data_path, project_config.as_ref())?;
    let explicit_s3_params = S3ConnectionParams {
        endpoint: s3_endpoint,
        access_key_id: s3_access_key_id,
        secret_access_key: s3_secret_access_key,
        session_token: s3_session_token,
        region: s3_region,
        path_style: s3_path_style,
        use_ssl: s3_use_ssl,
    };
    let s3_config = if data_path.as_deref().is_some_and(is_s3_data_path) {
        extract_s3_config(project_config.as_ref(), &explicit_s3_params)?
    } else {
        None
    };
    let s3_connection_params = resolve_s3_connection_params(explicit_s3_params, s3_config.as_ref());
    let (catalog_backend, metric_queue_capacity, s3_connection) = validate_init_config(
        data_path.as_deref(),
        catalog_backend,
        catalog_path.as_deref(),
        metric_queue_capacity,
        s3_connection_params,
    )?;
    NativeClient::open_with_catalog_backend_storage_config(
        path,
        catalog_backend,
        catalog_path,
        data_path,
        s3_connection,
        metric_queue_capacity,
    )
    .map(PyClient::new)
    .map_err(runtime_error)
}

#[derive(Default)]
struct S3FileConfig {
    endpoint: Option<String>,
    access_key_id: Option<String>,
    secret_access_key: Option<String>,
    session_token: Option<String>,
    region: Option<String>,
    path_style: Option<bool>,
    use_ssl: Option<bool>,
}

fn load_project_config<'py>(
    py: Python<'py>,
    root_path: &Path,
) -> PyResult<Option<Bound<'py, PyDict>>> {
    let config_path = root_path.join(".pulseon").join("config.toml");
    let content = match fs::read_to_string(&config_path) {
        Ok(content) => content,
        Err(source) if source.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(source) => {
            return Err(InvalidConfigurationError::new_err(format!(
                "failed to read config.toml: {source}"
            )));
        }
    };
    let tomllib = PyModule::import(py, "tomllib")?;
    let config = tomllib
        .call_method1("loads", (content,))
        .map_err(|source| {
            InvalidConfigurationError::new_err(format!("invalid config.toml: {source}"))
        })?;
    let config = config
        .cast_into::<PyDict>()
        .map_err(|_| InvalidConfigurationError::new_err("config.toml must be a table"))?;
    Ok(Some(config))
}

fn resolve_data_path(
    data_path: Option<PathBuf>,
    project_config: Option<&Bound<'_, PyDict>>,
) -> PyResult<Option<PathBuf>> {
    if data_path.is_some() {
        return Ok(data_path);
    }
    optional_config_string(project_config, "data_path", "config.toml data_path")
        .map(|value| value.map(PathBuf::from))
}

fn resolve_s3_connection_params(
    explicit: S3ConnectionParams,
    s3_config: Option<&S3FileConfig>,
) -> S3ConnectionParams {
    S3ConnectionParams {
        endpoint: resolve_config_string(
            explicit.endpoint,
            s3_config.and_then(|config| config.endpoint.as_deref()),
        ),
        access_key_id: resolve_config_string(
            explicit.access_key_id,
            s3_config.and_then(|config| config.access_key_id.as_deref()),
        ),
        secret_access_key: resolve_config_string(
            explicit.secret_access_key,
            s3_config.and_then(|config| config.secret_access_key.as_deref()),
        ),
        session_token: resolve_config_string(
            explicit.session_token,
            s3_config.and_then(|config| config.session_token.as_deref()),
        ),
        region: resolve_config_string(
            explicit.region,
            s3_config.and_then(|config| config.region.as_deref()),
        ),
        path_style: resolve_config_bool(
            explicit.path_style,
            s3_config.and_then(|config| config.path_style),
        ),
        use_ssl: resolve_config_bool(
            explicit.use_ssl,
            s3_config.and_then(|config| config.use_ssl),
        ),
    }
}

fn extract_s3_config(
    config: Option<&Bound<'_, PyDict>>,
    explicit: &S3ConnectionParams,
) -> PyResult<Option<S3FileConfig>> {
    let Some(config) = config else {
        return Ok(None);
    };
    let Some(value) = config.get_item("s3")? else {
        return Ok(None);
    };
    let config = value
        .cast_into::<PyDict>()
        .map_err(|_| InvalidConfigurationError::new_err("config.toml s3 must be a table"))?;
    Ok(Some(S3FileConfig {
        endpoint: optional_config_string_unless_explicit(
            &config,
            explicit.endpoint.is_some(),
            "endpoint",
            "config.toml s3.endpoint",
        )?,
        access_key_id: optional_config_string_unless_explicit(
            &config,
            explicit.access_key_id.is_some(),
            "access_key_id",
            "config.toml s3.access_key_id",
        )?,
        secret_access_key: optional_config_string_unless_explicit(
            &config,
            explicit.secret_access_key.is_some(),
            "secret_access_key",
            "config.toml s3.secret_access_key",
        )?,
        session_token: optional_config_string_unless_explicit(
            &config,
            explicit.session_token.is_some(),
            "session_token",
            "config.toml s3.session_token",
        )?,
        region: optional_config_string_unless_explicit(
            &config,
            explicit.region.is_some(),
            "region",
            "config.toml s3.region",
        )?,
        path_style: optional_config_bool_unless_explicit(
            &config,
            explicit.path_style.is_some(),
            "path_style",
            "config.toml s3.path_style",
        )?,
        use_ssl: optional_config_bool_unless_explicit(
            &config,
            explicit.use_ssl.is_some(),
            "use_ssl",
            "config.toml s3.use_ssl",
        )?,
    }))
}

fn optional_config_string_unless_explicit(
    config: &Bound<'_, PyDict>,
    is_explicit: bool,
    key: &str,
    label: &str,
) -> PyResult<Option<String>> {
    if is_explicit {
        return Ok(None);
    }
    optional_config_string(Some(config), key, label)
}

fn optional_config_string(
    config: Option<&Bound<'_, PyDict>>,
    key: &str,
    label: &str,
) -> PyResult<Option<String>> {
    let Some(config) = config else {
        return Ok(None);
    };
    let Some(value) = config.get_item(key)? else {
        return Ok(None);
    };
    value
        .extract::<String>()
        .map(Some)
        .map_err(|_| InvalidConfigurationError::new_err(format!("{label} must be a string")))
}

fn optional_config_bool_unless_explicit(
    config: &Bound<'_, PyDict>,
    is_explicit: bool,
    key: &str,
    label: &str,
) -> PyResult<Option<bool>> {
    if is_explicit {
        return Ok(None);
    }
    optional_config_bool(config, key, label)
}

fn optional_config_bool(
    config: &Bound<'_, PyDict>,
    key: &str,
    label: &str,
) -> PyResult<Option<bool>> {
    let Some(value) = config.get_item(key)? else {
        return Ok(None);
    };
    value
        .extract::<bool>()
        .map(Some)
        .map_err(|_| InvalidConfigurationError::new_err(format!("{label} must be a boolean")))
}

fn resolve_config_string(explicit: Option<String>, config: Option<&str>) -> Option<String> {
    explicit.or_else(|| config.map(ToOwned::to_owned))
}

fn resolve_config_bool(explicit: Option<bool>, config: Option<bool>) -> Option<bool> {
    explicit.or(config)
}

fn validate_init_config(
    data_path: Option<&Path>,
    catalog_backend: &str,
    catalog_path: Option<&Path>,
    metric_queue_capacity: i64,
    s3_connection: S3ConnectionParams,
) -> PyResult<(CatalogBackend, usize, Option<S3ConnectionConfig>)> {
    if !(1..=1_048_576).contains(&metric_queue_capacity) {
        return Err(InvalidConfigurationError::new_err(
            "metric_queue_capacity must be between 1 and 1048576",
        ));
    }
    let catalog_backend = CatalogBackend::from_name(catalog_backend).ok_or_else(|| {
        InvalidConfigurationError::new_err(format!(
            "unsupported catalog_backend: {catalog_backend}"
        ))
    })?;
    if data_path.is_some_and(is_unsupported_data_uri_path) {
        return Err(InvalidConfigurationError::new_err(
            "data_path must be a local filesystem path or s3:// URI",
        ));
    }
    if catalog_path.is_some_and(is_uri_path) {
        return Err(InvalidConfigurationError::new_err(
            "catalog_path must be a local filesystem path",
        ));
    }
    let metric_queue_capacity = usize::try_from(metric_queue_capacity).map_err(|_| {
        InvalidConfigurationError::new_err("metric_queue_capacity must be between 1 and 1048576")
    })?;
    let s3_connection = validate_s3_connection_config(data_path, s3_connection)?;
    Ok((catalog_backend, metric_queue_capacity, s3_connection))
}

fn is_uri_path(path: &Path) -> bool {
    path.to_string_lossy().contains("://")
}

fn is_unsupported_data_uri_path(path: &Path) -> bool {
    let path = path.to_string_lossy();
    path.contains("://") && !path.starts_with("s3://")
}

struct S3ConnectionParams {
    endpoint: Option<String>,
    access_key_id: Option<String>,
    secret_access_key: Option<String>,
    session_token: Option<String>,
    region: Option<String>,
    path_style: Option<bool>,
    use_ssl: Option<bool>,
}

fn validate_s3_connection_config(
    data_path: Option<&Path>,
    s3_connection: S3ConnectionParams,
) -> PyResult<Option<S3ConnectionConfig>> {
    if !data_path.is_some_and(is_s3_data_path) {
        return Ok(None);
    }
    Ok(Some(S3ConnectionConfig::new(
        required_s3_string(s3_connection.endpoint, "s3_endpoint")?,
        required_s3_string(s3_connection.access_key_id, "s3_access_key_id")?,
        required_s3_string(s3_connection.secret_access_key, "s3_secret_access_key")?,
        optional_s3_string(s3_connection.session_token, "s3_session_token")?,
        optional_s3_string(s3_connection.region, "s3_region")?,
        s3_connection.path_style,
        s3_connection.use_ssl,
    )))
}

fn is_s3_data_path(path: &Path) -> bool {
    path.to_string_lossy().starts_with("s3://")
}

fn required_s3_string(value: Option<String>, name: &str) -> PyResult<String> {
    let value = optional_s3_string(value, name)?;
    value.ok_or_else(|| {
        InvalidConfigurationError::new_err(format!("{name} is required when data_path is s3://"))
    })
}

fn optional_s3_string(value: Option<String>, name: &str) -> PyResult<Option<String>> {
    let Some(value) = value else {
        return Ok(None);
    };
    if value.trim().is_empty() {
        return Err(InvalidConfigurationError::new_err(format!(
            "{name} must not be empty"
        )));
    }
    Ok(Some(value))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn s3_config_merges_explicit_keywords_over_file_values() {
        let config = S3FileConfig {
            endpoint: Some("from-config:9000".to_owned()),
            access_key_id: Some("from-config".to_owned()),
            secret_access_key: Some("from-config-secret".to_owned()),
            path_style: Some(false),
            use_ssl: Some(true),
            ..S3FileConfig::default()
        };

        let merged = resolve_s3_connection_params(
            S3ConnectionParams {
                endpoint: Some("override:9000".to_owned()),
                access_key_id: None,
                secret_access_key: None,
                session_token: None,
                region: None,
                path_style: Some(true),
                use_ssl: None,
            },
            Some(&config),
        );

        assert_eq!(merged.endpoint.as_deref(), Some("override:9000"));
        assert_eq!(merged.access_key_id.as_deref(), Some("from-config"));
        assert_eq!(
            merged.secret_access_key.as_deref(),
            Some("from-config-secret")
        );
        assert_eq!(merged.path_style, Some(true));
        assert_eq!(merged.use_ssl, Some(true));
    }
}
