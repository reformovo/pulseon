"""Pyright coverage for public catalog_backend literals."""

from __future__ import annotations

import pulseon


def accepts_supported_catalog_backend_literals() -> None:
    pulseon.init(catalog_backend="duckdb")
    pulseon.init(catalog_backend="sqlite")


def rejects_unknown_catalog_backend_literals() -> None:
    pulseon.init(catalog_backend="postgres")  # type: ignore[reportArgumentType]
