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


def init(path: str | os.PathLike[str]) -> Client:
    return _pulseon.init(path)


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
