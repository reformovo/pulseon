"""PulseOn Python API."""

from __future__ import annotations

import os
import pathlib
import tomllib
from collections.abc import Mapping
from typing import Any

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
    project_config = _load_project_config(path)
    if data_path is None:
        data_path = _optional_config_string(project_config, "data_path")
    return _pulseon.init(
        path,
        data_path=data_path,
        catalog_backend=catalog_backend,
        catalog_path=catalog_path,
        metric_queue_capacity=metric_queue_capacity,
    )


def _load_project_config(path: str | os.PathLike[str]) -> Mapping[str, Any]:
    config_path = pathlib.Path(os.fspath(path)) / ".pulseon" / "config.toml"
    if not config_path.exists():
        return {}
    try:
        with config_path.open("rb") as config_file:
            return tomllib.load(config_file)
    except tomllib.TOMLDecodeError as exc:
        raise InvalidConfigurationError(f"invalid config.toml: {exc}") from exc


def _optional_config_string(
    config: Mapping[str, Any], key: str
) -> str | None:
    value = config.get(key)
    if value is None:
        return None
    if not isinstance(value, str):
        raise InvalidConfigurationError(f"config.toml {key} must be a string")
    return value


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
