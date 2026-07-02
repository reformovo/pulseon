"""Verify Python SDK diagnostics and shutdown behavior."""

from __future__ import annotations

import pathlib


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
