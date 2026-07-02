"""Verify shared pytest helper behavior."""

from __future__ import annotations

import dataclasses
from typing import TYPE_CHECKING, cast

import pytest

from tests import helpers

if TYPE_CHECKING:
    import pulseon


@dataclasses.dataclass(frozen=True)
class _FakeDiagnostics:
    accepted_reports: int = 1
    dropped_reports: int = 2
    failed_reports: int = 3
    pending_reports: int = 4
    writer_drained: bool = False
    last_write_error: str | None = "write failed"


class _FakeClient:
    def query_metric(
        self,
        run_id: str,
        metric_key: str,
    ) -> list[pulseon.MetricPoint]:
        del run_id, metric_key
        return []

    def diagnostics(self) -> _FakeDiagnostics:
        return _FakeDiagnostics()


def test_wait_for_metric_points_timeout_includes_context() -> None:
    client = cast("pulseon.Client", _FakeClient())

    with pytest.raises(AssertionError) as error:
        helpers.wait_for_metric_points(
            client,
            "run-1",
            "train/loss",
            expected_count=2,
            timeout_seconds=0.0,
            sleep_seconds=0.0,
        )

    message = str(error.value)
    assert "expected_count=2" in message
    assert "actual_count=0" in message
    assert "run_id='run-1'" in message
    assert "metric_key='train/loss'" in message
    assert "accepted_reports=1" in message
    assert "pending_reports=4" in message
