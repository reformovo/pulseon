"""Verify Python SDK metric logging behavior."""

from __future__ import annotations

import pathlib


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
