"""PulseOn Python API."""

from __future__ import annotations

import os

from pulseon import _pulseon

Client = _pulseon.Client


def init(path: str | os.PathLike[str]) -> Client:
    return _pulseon.init(path)


__all__ = ["Client", "init"]
