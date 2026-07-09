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
    s3_endpoint: str | None = None,
    s3_access_key_id: str | None = None,
    s3_secret_access_key: str | None = None,
    s3_session_token: str | None = None,
    s3_region: str | None = None,
    s3_path_style: bool | None = None,
    s3_use_ssl: bool | None = None,
) -> Client:
    project_config = _load_project_config(path)
    if data_path is None:
        data_path = _optional_config_string(project_config, "data_path")
    s3_config = _load_s3_config(project_config)
    if _is_s3_data_path(data_path):
        s3_endpoint = _resolve_optional_string(
            s3_endpoint, s3_config, "endpoint"
        )
        s3_access_key_id = _resolve_optional_string(
            s3_access_key_id,
            s3_config,
            "access_key_id",
        )
        s3_secret_access_key = _resolve_optional_string(
            s3_secret_access_key,
            s3_config,
            "secret_access_key",
        )
        s3_session_token = _resolve_optional_string(
            s3_session_token, s3_config, "session_token"
        )
        s3_region = _resolve_optional_string(
            s3_region, s3_config, "region"
        )
        s3_path_style = _resolve_optional_bool(
            s3_path_style, s3_config, "path_style"
        )
        s3_use_ssl = _resolve_optional_bool(
            s3_use_ssl, s3_config, "use_ssl"
        )
    else:
        s3_endpoint = None
        s3_access_key_id = None
        s3_secret_access_key = None
        s3_session_token = None
        s3_region = None
        s3_path_style = None
        s3_use_ssl = None
    return _pulseon.init(
        path,
        data_path=data_path,
        catalog_backend=catalog_backend,
        catalog_path=catalog_path,
        metric_queue_capacity=metric_queue_capacity,
        s3_endpoint=s3_endpoint,
        s3_access_key_id=s3_access_key_id,
        s3_secret_access_key=s3_secret_access_key,
        s3_session_token=s3_session_token,
        s3_region=s3_region,
        s3_path_style=s3_path_style,
        s3_use_ssl=s3_use_ssl,
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


def _load_s3_config(config: Mapping[str, Any]) -> Mapping[str, Any]:
    value = config.get("s3")
    if value is None:
        return {}
    if not isinstance(value, Mapping):
        raise InvalidConfigurationError("config.toml s3 must be a table")
    return value


def _resolve_optional_string(
    explicit_value: str | None,
    config: Mapping[str, Any],
    config_key: str,
) -> str | None:
    if explicit_value is not None:
        return explicit_value
    value = config.get(config_key)
    if value is None:
        return None
    if not isinstance(value, str):
        raise InvalidConfigurationError(
            f"config.toml s3.{config_key} must be a string"
        )
    return value


def _resolve_optional_bool(
    explicit_value: bool | None,
    config: Mapping[str, Any],
    config_key: str,
) -> bool | None:
    if explicit_value is not None:
        return explicit_value
    value = config.get(config_key)
    if value is None:
        return None
    if not isinstance(value, bool):
        raise InvalidConfigurationError(
            f"config.toml s3.{config_key} must be a boolean"
        )
    return value


def _is_s3_data_path(data_path: str | os.PathLike[str] | None) -> bool:
    return data_path is not None and os.fspath(data_path).startswith("s3://")


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
