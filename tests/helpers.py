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
    *,
    timeout_seconds: float = 5.0,
    sleep_seconds: float = 0.05,
) -> list[pulseon.MetricPoint]:
    deadline = time.monotonic() + timeout_seconds
    points: list[pulseon.MetricPoint] = []
    while time.monotonic() <= deadline:
        points = client.query_metric(run_id, metric_key)
        if len(points) >= expected_count:
            return points
        time.sleep(sleep_seconds)

    actual_count = len(points)
    diagnostics = client.diagnostics()
    raise AssertionError(
        "Timed out waiting for metric points: "
        f"expected_count={expected_count}, "
        f"actual_count={actual_count}, "
        f"run_id={run_id!r}, "
        f"metric_key={metric_key!r}, "
        f"diagnostics={_format_diagnostics(diagnostics)}"
    )


def _format_diagnostics(diagnostics: pulseon.Diagnostics) -> str:
    return (
        "{"
        f"accepted_reports={diagnostics.accepted_reports}, "
        f"dropped_reports={diagnostics.dropped_reports}, "
        f"failed_reports={diagnostics.failed_reports}, "
        f"pending_reports={diagnostics.pending_reports}, "
        f"writer_drained={diagnostics.writer_drained}, "
        f"last_write_error={diagnostics.last_write_error!r}"
        "}"
    )
