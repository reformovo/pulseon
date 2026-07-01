"""PulseOn Python API."""

from __future__ import annotations

import os

from pulseon import _pulseon

Client = _pulseon.Client
Diagnostics = _pulseon.Diagnostics
DuckLakeUnavailableError = _pulseon.DuckLakeUnavailableError
DuplicateRunError = _pulseon.DuplicateRunError
MetricPoint = _pulseon.MetricPoint
MetricSummary = _pulseon.MetricSummary
MissingProjectError = _pulseon.MissingProjectError
MissingRunError = _pulseon.MissingRunError
PulseOnError = _pulseon.PulseOnError
Project = _pulseon.Project
QueryError = _pulseon.QueryError
Run = _pulseon.Run


def init(path: str | os.PathLike[str]) -> Client:
    return _pulseon.init(path)


__all__ = [
    "Client",
    "Diagnostics",
    "DuckLakeUnavailableError",
    "DuplicateRunError",
    "MetricPoint",
    "MetricSummary",
    "MissingProjectError",
    "MissingRunError",
    "PulseOnError",
    "Project",
    "QueryError",
    "Run",
    "init",
]
