"""Verify that the pulseon package can be imported."""

import pathlib
import time


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

    assert [point.step for point in points] == [0, 1]
    assert [point.value_f64 for point in points] == [0.25, 0.125]
    assert isinstance(points[0], pulseon.MetricPoint)
    assert len(summaries) == 1
    assert isinstance(summaries[0], pulseon.MetricSummary)
    assert summaries[0].effective_count == 2
    assert summaries[0].last_step == 1
    assert summaries[0].last_value_f64 == 0.125


def _wait_for_metric_points(
    client: object,
    run_id: str,
    metric_key: str,
    expected_count: int,
) -> list[object]:
    deadline = time.monotonic() + 5.0
    points: list[object] = []
    while time.monotonic() < deadline:
        points = client.query_metric(run_id, metric_key)
        if len(points) >= expected_count:
            return points
        time.sleep(0.05)
    return points
