"""Shared pytest helpers for Python SDK behavior tests."""

from __future__ import annotations

import os
import pathlib
import time
from typing import TYPE_CHECKING

if TYPE_CHECKING:
    import pulseon


def configure_lttb_extension() -> None:
    if "PULSEON_LTTB_EXTENSION_PATH" in os.environ:
        return

    local_extension = (
        pathlib.Path.home()
        / "projects/duckdb-lttb/build/release/repository/v1.5.4/osx_arm64/"
        "lttb.duckdb_extension"
    )
    if local_extension.is_file():
        os.environ["PULSEON_LTTB_EXTENSION_PATH"] = str(local_extension)
    else:
        os.environ.setdefault("PULSEON_LTTB_AUTO_INSTALL", "1")


def wait_for_metric_points(
    client: pulseon.Client,
    run_id: str,
    metric_key: str,
    expected_count: int,
) -> list[pulseon.MetricPoint]:
    deadline = time.monotonic() + 5.0
    points: list[pulseon.MetricPoint] = []
    while time.monotonic() < deadline:
        points = client.query_metric(run_id, metric_key)
        if len(points) >= expected_count:
            return points
        time.sleep(0.05)
    return points
