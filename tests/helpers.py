"""Shared pytest helpers for Python SDK behavior tests."""

from __future__ import annotations

import time
from typing import TYPE_CHECKING

if TYPE_CHECKING:
    import pulseon


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
        f"pending_reports={diagnostics.pending_reports}, "
        f"queue_full_errors={diagnostics.queue_full_errors}, "
        f"persisted_reports={diagnostics.persisted_reports}, "
        f"writer_state={diagnostics.writer_state!r}, "
        f"last_write_error={diagnostics.last_write_error!r}, "
        f"last_flush_run_id={diagnostics.last_flush_run_id!r}, "
        f"last_flush_status={diagnostics.last_flush_status!r}, "
        f"last_flush_error={diagnostics.last_flush_error!r}"
        "}"
    )
