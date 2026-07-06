"""Verify Python SDK client, project, and run lifecycle behavior."""

from __future__ import annotations

import pathlib

import pytest

from tests import helpers


def test_init_returns_client(tmp_path: pathlib.Path) -> None:
    import pulseon

    client = pulseon.init(tmp_path / "pulseon")

    assert isinstance(client, pulseon.Client)


def test_init_accepts_v2_configuration_keywords(tmp_path: pathlib.Path) -> None:
    import pulseon

    data_path = tmp_path / "custom-data"
    catalog_path = tmp_path / "catalog" / "catalog.ducklake"
    client = pulseon.init(
        tmp_path / "pulseon",
        data_path=data_path,
        catalog_backend="duckdb",
        catalog_path=catalog_path,
        metric_queue_capacity=1024,
    )
    project = client.create_project("local training", project_id="project-1")

    assert isinstance(client, pulseon.Client)
    assert project.project_id == "project-1"
    assert data_path.is_dir()
    assert catalog_path.is_file()


def test_init_uses_duckdb_catalog_and_data_defaults(tmp_path: pathlib.Path) -> None:
    import pulseon

    root_path = tmp_path / "pulseon"
    client = pulseon.init(root_path)
    project = client.create_project("local training", project_id="project-1")

    assert project.project_id == "project-1"
    assert (root_path / ".pulseon" / "catalog.ducklake").is_file()
    assert (root_path / ".pulseon" / "data").is_dir()


def test_init_rejects_invalid_v2_configuration(tmp_path: pathlib.Path) -> None:
    import pulseon

    invalid_kwargs = [
        {"metric_queue_capacity": -1},
        {"metric_queue_capacity": 0},
        {"metric_queue_capacity": 1_048_577},
        {"catalog_backend": "postgres"},
        {"catalog_backend": "sqlite"},
        {"data_path": "s3://bucket/pulseon"},
        {"catalog_path": "s3://bucket/catalog.ducklake"},
    ]
    for index, kwargs in enumerate(invalid_kwargs):
        root_path = tmp_path / f"pulseon-{index}"
        with pytest.raises(pulseon.InvalidConfigurationError):
            pulseon.init(root_path, **kwargs)
        assert not root_path.exists()


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
    run_id = run.run_id
    del run
    del client

    resumed_client = pulseon.init(root_path)
    resumed_run = resumed_client.resume_run(run_id)
    resumed_run.log("train/loss", 0, 0.25)
    points = helpers.wait_for_metric_points(
        resumed_client,
        resumed_run.run_id,
        "train/loss",
        expected_count=1,
    )

    assert isinstance(resumed_run, pulseon.Run)
    assert resumed_run.run_id == run_id
    assert [point.value_f64 for point in points] == [0.25]


def test_active_run_lock_conflict_and_release_after_shutdown(
    tmp_path: pathlib.Path,
) -> None:
    import pulseon

    root_path = tmp_path / "pulseon"
    first_client = pulseon.init(root_path)
    project = first_client.create_project("local training", project_id="project-1")
    run = first_client.create_run(project.project_id, "baseline", run_id="run-1")
    second_client = pulseon.init(root_path)

    with pytest.raises(pulseon.RunAlreadyActiveError, match="run-1"):
        second_client.resume_run(run.run_id)

    first_client.shutdown()
    resumed = second_client.resume_run(run.run_id)

    assert resumed.run_id == run.run_id


def test_create_run_existing_id_requires_explicit_resume(
    tmp_path: pathlib.Path,
) -> None:
    import pulseon

    root_path = tmp_path / "pulseon"
    first_client = pulseon.init(root_path)
    project = first_client.create_project("local training", project_id="project-1")
    first_client.create_run(project.project_id, "baseline", run_id="run-1")
    second_client = pulseon.init(root_path)

    with pytest.raises(pulseon.RunAlreadyExistsError, match="run-1"):
        second_client.create_run(project.project_id, "duplicate", run_id="run-1")


def test_resume_run_rejects_terminal_runs(tmp_path: pathlib.Path) -> None:
    import pulseon

    root_path = tmp_path / "pulseon"
    client = pulseon.init(root_path)
    project = client.create_project("local training", project_id="project-1")
    run = client.create_run(project.project_id, "baseline", run_id="run-1")
    client.finish_run(run.run_id)

    with pytest.raises(pulseon.InvalidRunStateError, match="finished -> running"):
        client.resume_run(run.run_id)


def test_leftover_lock_file_does_not_block_resume(
    tmp_path: pathlib.Path,
) -> None:
    import pulseon

    root_path = tmp_path / "pulseon"
    first_client = pulseon.init(root_path)
    project = first_client.create_project("local training", project_id="project-1")
    run = first_client.create_run(
        project.project_id,
        "baseline",
        run_id="run/leftover lock",
    )
    first_client.shutdown()
    lock_file = (
        root_path
        / ".pulseon"
        / "locks"
        / "runs"
        / "run%2Fleftover%20lock.lock"
    )

    resumed = pulseon.init(root_path).resume_run(run.run_id)

    assert lock_file.is_file()
    assert resumed.run_id == run.run_id


def test_client_lists_project_runs_for_terminal_summary_queries(
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
    helpers.wait_for_metric_points(
        client,
        first_run.run_id,
        "train/loss",
        expected_count=1,
    )
    helpers.wait_for_metric_points(
        client,
        second_run.run_id,
        "train/loss",
        expected_count=1,
    )
    client.finish_run(first_run.run_id)
    client.finish_run(second_run.run_id)
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


def test_finalization_closes_run_for_late_logging(
    tmp_path: pathlib.Path,
) -> None:
    import pulseon

    client = pulseon.init(tmp_path / "pulseon")
    project = client.create_project("local training", project_id="project-1")
    run = client.create_run(project.project_id, "baseline", run_id="run-1")
    run.log("train/loss", 0, 0.25)

    finished = client.finish_run(run.run_id)

    with pytest.raises(pulseon.RunClosedError, match="run-1"):
        run.log("train/loss", 1, 0.125)
    points = client.query_metric(run.run_id, "train/loss")
    assert finished.status == "finished"
    assert [point.value_f64 for point in points] == [0.25]


def test_finish_run_flushes_partitioned_parquet_and_updates_diagnostics(
    tmp_path: pathlib.Path,
) -> None:
    import pulseon

    root_path = tmp_path / "pulseon"
    client = pulseon.init(root_path)
    project = client.create_project("local training", project_id="project-1")
    run = client.create_run(project.project_id, "baseline", run_id="run-1")
    run.log("train/loss", 0, 0.25)

    finished = client.finish_run(run.run_id)
    diagnostics = client.diagnostics()
    partition_path = (
        root_path
        / ".pulseon"
        / "data"
        / "main"
        / "metric_points"
        / "run_id=run-1"
        / "metric_key_encoded=train%252Floss"
    )

    assert finished.status == "finished"
    assert diagnostics.last_flush_run_id == "run-1"
    assert diagnostics.last_flush_status == "succeeded"
    assert any(partition_path.glob("*.parquet"))


def test_flush_run_data_retries_terminal_run_visibility(
    tmp_path: pathlib.Path,
) -> None:
    import pulseon

    client = pulseon.init(tmp_path / "pulseon")
    project = client.create_project("local training", project_id="project-1")
    run = client.create_run(project.project_id, "baseline", run_id="run-1")
    client.finish_run(run.run_id)

    client.flush_run_data(run.run_id)
    diagnostics = client.diagnostics()

    assert diagnostics.last_flush_run_id == "run-1"
    assert diagnostics.last_flush_status == "succeeded"


def test_flush_run_data_rejects_running_runs(tmp_path: pathlib.Path) -> None:
    import pulseon

    client = pulseon.init(tmp_path / "pulseon")
    project = client.create_project("local training", project_id="project-1")
    run = client.create_run(project.project_id, "baseline", run_id="run-1")

    with pytest.raises(pulseon.InvalidRunStateError, match="running -> flushed"):
        client.flush_run_data(run.run_id)


def test_shutdown_does_not_finalize_running_runs(
    tmp_path: pathlib.Path,
) -> None:
    import pulseon

    root_path = tmp_path / "pulseon"
    client = pulseon.init(root_path)
    project = client.create_project("local training", project_id="project-1")
    run = client.create_run(project.project_id, "baseline", run_id="run-1")

    client.shutdown()

    with pytest.raises(pulseon.ClientClosedError):
        run.log("train/loss", 0, 0.25)
    reopened_client = pulseon.init(root_path)
    running_run = reopened_client.get_run(run.run_id)
    resumed_run = reopened_client.resume_run(run.run_id)
    assert running_run.status == "running"
    assert running_run.finished_at is None
    assert resumed_run.run_id == run.run_id
