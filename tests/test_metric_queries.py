"""Verify Python SDK metric query, discovery, and downsampling behavior."""

from __future__ import annotations

import pathlib

from tests import helpers


def test_client_queries_metric_points_and_summaries(
    tmp_path: pathlib.Path,
) -> None:
    import pulseon

    client = pulseon.init(tmp_path / "pulseon")
    project = client.create_project("local training", project_id="project-1")
    run = client.create_run(project.project_id, "baseline", run_id="run-1")
    run.log("train/loss", 0, 0.25)
    run.log("train/loss", 1, 0.125)

    points = helpers.wait_for_metric_points(
        client,
        run.run_id,
        "train/loss",
        expected_count=2,
    )
    summaries = client.query_metric_summaries([run.run_id], "train/loss")
    diagnostics = client.diagnostics()

    assert [point.step for point in points] == [0, 1]
    assert [point.value_f64 for point in points] == [0.25, 0.125]
    assert isinstance(points[0], pulseon.MetricPoint)
    assert len(summaries) == 1
    assert isinstance(summaries[0], pulseon.MetricSummary)
    assert summaries[0].effective_count == 2
    assert summaries[0].last_step == 1
    assert summaries[0].last_value_f64 == 0.125
    assert diagnostics.pending_reports == 0
    assert diagnostics.writer_drained
    assert diagnostics.last_write_error is None


def test_client_discovers_metrics_from_aggregate_state(
    tmp_path: pathlib.Path,
) -> None:
    import pulseon

    client = pulseon.init(tmp_path / "pulseon")
    project = client.create_project("local training", project_id="project-1")
    run = client.create_run(project.project_id, "baseline", run_id="run-1")
    run.log("eval/accuracy", 0, 0.8)
    run.log("train/loss", 0, 0.25)
    helpers.wait_for_metric_points(
        client,
        run.run_id,
        "eval/accuracy",
        expected_count=1,
    )
    helpers.wait_for_metric_points(
        client,
        run.run_id,
        "train/loss",
        expected_count=1,
    )

    metrics = client.list_metrics(run.run_id)

    assert [metric.metric_key for metric in metrics] == [
        "eval/accuracy",
        "train/loss",
    ]
    assert [metric.effective_count for metric in metrics] == [1, 1]
    assert isinstance(metrics[0], pulseon.MetricSummary)


def test_client_query_metric_applies_range_filters_and_downsampling(
    tmp_path: pathlib.Path,
) -> None:
    import pulseon

    helpers.configure_lttb_extension()
    client = pulseon.init(tmp_path / "pulseon")
    project = client.create_project("local training", project_id="project-1")
    run = client.create_run(project.project_id, "baseline", run_id="run-1")
    for step in range(100):
        run.log("train/loss", step, float(step))
    helpers.wait_for_metric_points(
        client,
        run.run_id,
        "train/loss",
        expected_count=100,
    )

    ranged_points = client.query_metric(
        run.run_id,
        "train/loss",
        start_step=10,
        end_step=15,
    )
    try:
        downsampled_points = client.query_metric(
            run.run_id,
            "train/loss",
            max_points=10,
        )
    except RuntimeError as error:
        if "DuckDB LTTB extension is unavailable" in str(error):
            import pytest

            pytest.skip(str(error))
        raise

    assert [point.step for point in ranged_points] == [10, 11, 12, 13, 14, 15]
    assert len(downsampled_points) <= 10
    assert downsampled_points[0].step == 0
    assert downsampled_points[-1].step == 99
