"""Verify that the pulseon package can be imported."""

import pathlib


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
