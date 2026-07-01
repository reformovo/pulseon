"""PulseOn Python API."""

from __future__ import annotations

import os

from pulseon import _pulseon

Client = _pulseon.Client
Diagnostics = _pulseon.Diagnostics
Project = _pulseon.Project
Run = _pulseon.Run


def init(path: str | os.PathLike[str]) -> Client:
    return _pulseon.init(path)


__all__ = ["Client", "Diagnostics", "Project", "Run", "init"]
