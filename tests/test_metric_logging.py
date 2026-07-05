"""Verify Python SDK metric logging behavior."""

from __future__ import annotations

import pathlib

import pytest


def test_run_log_requires_explicit_step(tmp_path: pathlib.Path) -> None:
    import pulseon

    client = pulseon.init(tmp_path / "pulseon")
    project = client.create_project("local training", project_id="project-1")
    run = client.create_run(project.project_id, "baseline", run_id="run-1")

    run.log("train/loss", 0, 0.25)
    with pytest.raises(TypeError):
        run.log("train/loss", 0.125)  # type: ignore[call-arg]
    diagnostics = client.diagnostics()

    assert isinstance(diagnostics, pulseon.Diagnostics)
    assert diagnostics.accepted_reports >= 1
    assert diagnostics.dropped_reports == 0
    assert diagnostics.failed_reports == 0
