"""Verify Python SDK diagnostics and shutdown behavior."""

from __future__ import annotations

import pathlib

import pytest


def test_client_shutdown_closes_logging_and_context_manager(
    tmp_path: pathlib.Path,
) -> None:
    import pulseon

    root_path = tmp_path / "pulseon"
    client = pulseon.init(root_path)
    project = client.create_project("local training", project_id="project-1")
    run = client.create_run(project.project_id, "baseline", run_id="run-1")

    assert client.shutdown()
    diagnostics = client.diagnostics()
    assert diagnostics.pending_reports == 0
    assert diagnostics.writer_state == "closed"
    assert diagnostics.last_write_error is None

    run.log("train/loss", 0, 0.25)
    assert client.diagnostics().writer_state == "closed"

    with pulseon.init(root_path) as context_client:
        selected_project = context_client.get_project(project.project_id)

        assert selected_project.project_id == project.project_id


def test_client_diagnostics_fields_are_read_only(tmp_path: pathlib.Path) -> None:
    import pulseon

    client = pulseon.init(tmp_path / "pulseon")
    diagnostics = client.diagnostics()

    assert diagnostics.pending_reports == 0
    assert diagnostics.queue_full_errors == 0
    assert diagnostics.persisted_reports == 0
    assert diagnostics.writer_state == "drained"
    assert diagnostics.last_write_error is None
    assert diagnostics.last_flush_run_id is None
    assert diagnostics.last_flush_status == "none"
    assert diagnostics.last_flush_error is None
    for removed_field in (
        "accepted_reports",
        "dropped_reports",
        "failed_reports",
        "writer_drained",
    ):
        assert not hasattr(diagnostics, removed_field)
    with pytest.raises(AttributeError):
        setattr(diagnostics, "pending_reports", 1)
