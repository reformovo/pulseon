import os
from typing import overload

class Diagnostics:
    accepted_reports: int
    dropped_reports: int
    failed_reports: int

class MetricPoint:
    run_id: str
    metric_key: str
    step: int
    timestamp: str
    value_f64: float
    ingested_at: str

class MetricSummary:
    run_id: str
    metric_key: str
    effective_count: int
    last_step: int
    last_value_f64: float
    min_value_f64: float
    max_value_f64: float

class Project:
    project_id: str
    name: str
    created_at: str

class Run:
    run_id: str
    project_id: str
    name: str
    status: str
    created_at: str
    started_at: str
    finished_at: str | None
    @overload
    def log(self, key: str, value: float, /) -> None: ...
    @overload
    def log(self, key: str, step: int, value: float, /) -> None: ...

class Client:
    def create_project(
        self, name: str, project_id: str | None = None
    ) -> Project: ...
    def get_project(self, project_id: str) -> Project: ...
    def create_run(
        self, project_id: str, name: str, run_id: str | None = None
    ) -> Run: ...
    def get_run(self, run_id: str) -> Run: ...
    def resume_run(self, run_id: str) -> Run: ...
    def list_runs(self, project_id: str) -> list[Run]: ...
    def list_orphan_runs(self, project_id: str | None = None) -> list[Run]: ...
    def finish_run(self, run_id: str) -> Run: ...
    def fail_run(self, run_id: str) -> Run: ...
    def shutdown(self) -> bool: ...
    def __enter__(self) -> Client: ...
    def __exit__(
        self, exc_type: object, exc_value: object, traceback: object
    ) -> bool: ...
    def diagnostics(self) -> Diagnostics: ...
    def query_metric(
        self,
        run_id: str,
        metric_key: str,
        start_step: int | None = None,
        end_step: int | None = None,
        max_points: int | None = None,
    ) -> list[MetricPoint]: ...
    def query_metric_summaries(
        self, run_ids: list[str], metric_key: str
    ) -> list[MetricSummary]: ...
    def list_metrics(self, run_id: str) -> list[MetricSummary]: ...

def init(path: str | os.PathLike[str]) -> Client: ...
