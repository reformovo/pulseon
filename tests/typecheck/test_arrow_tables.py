"""Pyright coverage for Arrow-compatible query results."""

from __future__ import annotations

from typing import assert_type

import pulseon


def accepts_arrow_table_queries(client: pulseon.Client) -> None:
    points = client.query_metric_table(
        "run-1",
        "train/loss",
        start_step=10,
        end_step=20,
        max_points=5,
    )
    summaries = client.query_metric_summaries_table(
        ["run-1", "run-2"], "train/loss"
    )

    assert_type(points, pulseon.ArrowTable)
    assert_type(summaries, pulseon.ArrowTable)
    assert_type(points.row_count, int)
    assert_type(points.source_row_count, int)
    assert_type(points.downsampled, bool)
    assert_type(points.column_names, list[str])
    assert_type(points.__arrow_c_stream__(), object)
