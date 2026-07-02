"""Verify that the pulseon package can be imported."""

from __future__ import annotations

import os
import pathlib
import time
from typing import TYPE_CHECKING

if TYPE_CHECKING:
    import pytest
    import pulseon


def test_import_pulseon() -> None:
    import pulseon

    assert hasattr(pulseon, "__all__")


def test_init_returns_client(tmp_path: pathlib.Path) -> None:
    import pulseon

    client = pulseon.init(tmp_path / "pulseon")

    assert isinstance(client, pulseon.Client)


def test_client_creates_project_and_run(tmp_path: pathlib.Path) -> None:
    import pulseon

    client = pulseon.init(tmp_path / "pulseon")
    project = client.create_project("local training", project_id="project-1")
    run = client.create_run(project.project_id, "baseline", run_id="run-1")

    assert isinstance(project, pulseon.Project)
    assert project.project_id == "project-1"
    assert project.name == "local training"
    assert isinstance(run, pulseon.Run)
    assert run.run_id == "run-1"
    assert run.project_id == project.project_id
    assert run.name == "baseline"
    assert run.status == "running"


def test_client_raises_actionable_sdk_errors(
    tmp_path: pathlib.Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    import pytest
    import pulseon

    client = pulseon.init(tmp_path / "pulseon")
    project = client.create_project("local training", project_id="project-1")
    run = client.create_run(project.project_id, "baseline", run_id="run-1")

    assert issubclass(pulseon.DuplicateRunError, pulseon.PulseOnError)
    assert issubclass(pulseon.MissingProjectError, pulseon.PulseOnError)
    assert issubclass(pulseon.MissingRunError, pulseon.PulseOnError)
    assert issubclass(pulseon.DuckLakeUnavailableError, pulseon.PulseOnError)
    assert issubclass(pulseon.QueryError, pulseon.PulseOnError)
    with pytest.raises(pulseon.DuplicateRunError):
        client.create_run(project.project_id, "duplicate", run_id=run.run_id)
    with pytest.raises(pulseon.MissingProjectError):
        client.create_run("missing-project", "baseline")
    with pytest.raises(pulseon.MissingRunError):
        client.get_run("missing-run")

    monkeypatch.setenv(
        "PULSEON_LTTB_EXTENSION_PATH",
        str(tmp_path / "missing-lttb.duckdb_extension"),
    )
    query_client = pulseon.init(tmp_path / "query-pulseon")
    query_project = query_client.create_project("query", project_id="query-project")
    query_run = query_client.create_run(
        query_project.project_id,
        "query",
        run_id="query-run",
    )
    for step in range(3):
        query_run.log("train/loss", step, float(step))
    _wait_for_metric_points(
        query_client,
        query_run.run_id,
        "train/loss",
        expected_count=3,
    )
    with pytest.raises(pulseon.QueryError):
        query_client.query_metric(query_run.run_id, "train/loss", max_points=2)


def test_client_selects_existing_project_and_run(tmp_path: pathlib.Path) -> None:
    import pulseon

    root_path = tmp_path / "pulseon"
    client = pulseon.init(root_path)
    project = client.create_project("local training", project_id="project-1")
    run = client.create_run(project.project_id, "baseline", run_id="run-1")
    del client

    reopened_client = pulseon.init(root_path)
    selected_project = reopened_client.get_project(project.project_id)
    selected_run = reopened_client.get_run(run.run_id)

    assert isinstance(selected_project, pulseon.Project)
    assert selected_project.project_id == project.project_id
    assert selected_project.name == "local training"
    assert isinstance(selected_run, pulseon.Run)
    assert selected_run.run_id == run.run_id
    assert selected_run.project_id == selected_project.project_id
    assert selected_run.name == "baseline"
    assert selected_run.status == "running"


def test_client_resumes_existing_run_for_logging(tmp_path: pathlib.Path) -> None:
    import pulseon

    root_path = tmp_path / "pulseon"
    client = pulseon.init(root_path)
    project = client.create_project("local training", project_id="project-1")
    run = client.create_run(project.project_id, "baseline", run_id="run-1")
    del client

    resumed_client = pulseon.init(root_path)
    resumed_run = resumed_client.resume_run(run.run_id)
    resumed_run.log("train/loss", 0, 0.25)
    points = _wait_for_metric_points(
        resumed_client,
        resumed_run.run_id,
        "train/loss",
        expected_count=1,
    )

    assert isinstance(resumed_run, pulseon.Run)
    assert resumed_run.run_id == run.run_id
    assert [point.value_f64 for point in points] == [0.25]


def test_client_lists_project_runs_for_summary_queries(
    tmp_path: pathlib.Path,
) -> None:
    import pulseon

    root_path = tmp_path / "pulseon"
    client = pulseon.init(root_path)
    project = client.create_project("local training", project_id="project-1")
    first_run = client.create_run(project.project_id, "baseline", run_id="run-1")
    second_run = client.create_run(project.project_id, "candidate", run_id="run-2")
    first_run.log("train/loss", 0, 0.25)
    second_run.log("train/loss", 0, 0.125)
    _wait_for_metric_points(client, first_run.run_id, "train/loss", expected_count=1)
    _wait_for_metric_points(client, second_run.run_id, "train/loss", expected_count=1)
    del first_run
    del second_run
    del client

    reopened_client = pulseon.init(root_path)
    runs = reopened_client.list_runs(project.project_id)
    summaries = reopened_client.query_metric_summaries(
        [run.run_id for run in runs],
        "train/loss",
    )

    assert [run.run_id for run in runs] == ["run-1", "run-2"]
    assert [run.name for run in runs] == ["baseline", "candidate"]
    assert [summary.run_id for summary in summaries] == ["run-1", "run-2"]
    assert [summary.last_value_f64 for summary in summaries] == [0.25, 0.125]


def test_client_detects_orphan_running_runs(tmp_path: pathlib.Path) -> None:
    import pulseon

    root_path = tmp_path / "pulseon"
    client = pulseon.init(root_path)
    first_project = client.create_project("local training", project_id="project-1")
    second_project = client.create_project("sweep", project_id="project-2")
    first_run = client.create_run(first_project.project_id, "baseline", run_id="run-1")
    second_run = client.create_run(second_project.project_id, "candidate", run_id="run-2")
    first_run_id = first_run.run_id
    second_run_id = second_run.run_id
    del first_run
    del second_run
    del client

    reopened_client = pulseon.init(root_path)
    all_orphans = reopened_client.list_orphan_runs()
    project_orphans = reopened_client.list_orphan_runs(first_project.project_id)

    assert [run.run_id for run in all_orphans] == [
        first_run_id,
        second_run_id,
    ]
    assert [run.status for run in all_orphans] == ["running", "running"]
    assert [run.run_id for run in project_orphans] == [first_run_id]


def test_client_finalizes_runs_as_finished_or_failed(
    tmp_path: pathlib.Path,
) -> None:
    import pulseon

    client = pulseon.init(tmp_path / "pulseon")
    project = client.create_project("local training", project_id="project-1")
    finished_run = client.create_run(project.project_id, "baseline", run_id="run-1")
    failed_run = client.create_run(project.project_id, "candidate", run_id="run-2")

    finished = client.finish_run(finished_run.run_id)
    failed = client.fail_run(failed_run.run_id)
    orphan_runs = client.list_orphan_runs(project.project_id)

    assert finished.run_id == finished_run.run_id
    assert finished.status == "finished"
    assert finished.finished_at is not None
    assert failed.run_id == failed_run.run_id
    assert failed.status == "failed"
    assert failed.finished_at is not None
    assert orphan_runs == []


def test_client_shutdown_closes_logging_and_context_manager(
    tmp_path: pathlib.Path,
) -> None:
    import pulseon

    root_path = tmp_path / "pulseon"
    client = pulseon.init(root_path)
    project = client.create_project("local training", project_id="project-1")
    run = client.create_run(project.project_id, "baseline", run_id="run-1")

    assert client.shutdown()
    run.log("train/loss", 0, 0.25)
    assert client.diagnostics().failed_reports >= 1

    with pulseon.init(root_path) as context_client:
        selected_project = context_client.get_project(project.project_id)

        assert selected_project.project_id == project.project_id


def test_run_log_accepts_value_and_explicit_step(tmp_path: pathlib.Path) -> None:
    import pulseon

    client = pulseon.init(tmp_path / "pulseon")
    project = client.create_project("local training", project_id="project-1")
    run = client.create_run(project.project_id, "baseline", run_id="run-1")

    run.log("train/loss", 0.25)
    run.log("train/loss", 1, 0.125)
    diagnostics = client.diagnostics()

    assert isinstance(diagnostics, pulseon.Diagnostics)
    assert diagnostics.accepted_reports >= 2
    assert diagnostics.dropped_reports == 0
    assert diagnostics.failed_reports == 0


def test_client_queries_metric_points_and_summaries(tmp_path: pathlib.Path) -> None:
    import pulseon

    client = pulseon.init(tmp_path / "pulseon")
    project = client.create_project("local training", project_id="project-1")
    run = client.create_run(project.project_id, "baseline", run_id="run-1")
    run.log("train/loss", 0, 0.25)
    run.log("train/loss", 1, 0.125)

    points = _wait_for_metric_points(client, run.run_id, "train/loss", expected_count=2)
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
    _wait_for_metric_points(client, run.run_id, "eval/accuracy", expected_count=1)
    _wait_for_metric_points(client, run.run_id, "train/loss", expected_count=1)

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

    _configure_lttb_extension()
    client = pulseon.init(tmp_path / "pulseon")
    project = client.create_project("local training", project_id="project-1")
    run = client.create_run(project.project_id, "baseline", run_id="run-1")
    for step in range(100):
        run.log("train/loss", step, float(step))
    _wait_for_metric_points(client, run.run_id, "train/loss", expected_count=100)

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


def _configure_lttb_extension() -> None:
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


def _wait_for_metric_points(
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
