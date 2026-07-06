"""Verify Python SDK metric query, discovery, and max-points behavior."""

from __future__ import annotations

import pathlib

from tests import helpers


def test_client_queries_metric_points_and_terminal_summaries(
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
    client.finish_run(run.run_id)
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
    assert diagnostics.persisted_reports >= 2
    assert diagnostics.writer_state == "drained"
    assert diagnostics.last_write_error is None


def test_active_run_point_queries_do_not_depend_on_aggregate_state(
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

    points = client.query_metric(run.run_id, "train/loss")
    metrics = client.list_metrics(run.run_id)

    assert [point.value_f64 for point in points] == [0.25]
    assert metrics == []


def test_terminal_run_metric_discovery_uses_rebuilt_aggregate_state(
    tmp_path: pathlib.Path,
) -> None:
    import pulseon

    client = pulseon.init(tmp_path / "pulseon")
    project = client.create_project("local training", project_id="project-1")
    run = client.create_run(project.project_id, "baseline", run_id="run-1")
    run.log("eval/accuracy", 0, 0.8)
    run.log("train/loss", 0, 0.25)

    client.finish_run(run.run_id)
    metrics = client.list_metrics(run.run_id)

    assert [metric.metric_key for metric in metrics] == [
        "eval/accuracy",
        "train/loss",
    ]
    assert [metric.effective_count for metric in metrics] == [1, 1]
    assert isinstance(metrics[0], pulseon.MetricSummary)


def test_client_query_metric_applies_range_filters_and_short_max_points(
    tmp_path: pathlib.Path,
) -> None:
    import pulseon

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
    unchanged_points = client.query_metric(
        run.run_id,
        "train/loss",
        max_points=100,
    )

    assert [point.step for point in ranged_points] == [10, 11, 12, 13, 14, 15]
    assert len(unchanged_points) == 100
    assert unchanged_points[0].step == 0
    assert unchanged_points[-1].step == 99
