"""PulseOn Python API."""

from __future__ import annotations

import os

from pulseon import _pulseon

Client = _pulseon.Client
Diagnostics = _pulseon.Diagnostics
MetricPoint = _pulseon.MetricPoint
MetricSummary = _pulseon.MetricSummary
Project = _pulseon.Project
Run = _pulseon.Run


def init(path: str | os.PathLike[str]) -> Client:
    return _pulseon.init(path)


__all__ = [
    "Client",
    "Diagnostics",
    "MetricPoint",
    "MetricSummary",
    "Project",
    "Run",
    "init",
]
