"""PulseOn Python API."""

from __future__ import annotations

import os

from pulseon import _pulseon

Client = _pulseon.Client
ClientClosedError = _pulseon.ClientClosedError
Diagnostics = _pulseon.Diagnostics
InvalidConfigurationError = _pulseon.InvalidConfigurationError
InvalidRunStateError = _pulseon.InvalidRunStateError
MetricDrainTimeoutError = _pulseon.MetricDrainTimeoutError
MetricFlushError = _pulseon.MetricFlushError
MetricFlushTimeoutError = _pulseon.MetricFlushTimeoutError
MetricPoint = _pulseon.MetricPoint
MetricQueueFullError = _pulseon.MetricQueueFullError
MetricSummary = _pulseon.MetricSummary
MetricWriterFailedError = _pulseon.MetricWriterFailedError
PulseOnError = _pulseon.PulseOnError
Project = _pulseon.Project
Run = _pulseon.Run
RunAlreadyActiveError = _pulseon.RunAlreadyActiveError
RunAlreadyExistsError = _pulseon.RunAlreadyExistsError
RunClosedError = _pulseon.RunClosedError
StorageError = _pulseon.StorageError


def init(
    path: str | os.PathLike[str] = ".",
    *,
    data_path: str | os.PathLike[str] | None = None,
    catalog_backend: str = "duckdb",
    catalog_path: str | os.PathLike[str] | None = None,
    metric_queue_capacity: int = 65536,
) -> Client:
    return _pulseon.init(
        path,
        data_path=data_path,
        catalog_backend=catalog_backend,
        catalog_path=catalog_path,
        metric_queue_capacity=metric_queue_capacity,
    )


__all__ = [
    "Client",
    "ClientClosedError",
    "Diagnostics",
    "InvalidConfigurationError",
    "InvalidRunStateError",
    "MetricDrainTimeoutError",
    "MetricFlushError",
    "MetricFlushTimeoutError",
    "MetricPoint",
    "MetricQueueFullError",
    "MetricSummary",
    "MetricWriterFailedError",
    "PulseOnError",
    "Project",
    "Run",
    "RunAlreadyActiveError",
    "RunAlreadyExistsError",
    "RunClosedError",
    "StorageError",
    "init",
]
