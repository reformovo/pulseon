"""Verify actionable Python SDK error mapping."""

from __future__ import annotations

import pathlib

import pytest

from tests import helpers


def test_client_raises_actionable_sdk_errors(
    tmp_path: pathlib.Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
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
    helpers.wait_for_metric_points(
        query_client,
        query_run.run_id,
        "train/loss",
        expected_count=3,
    )
    with pytest.raises(pulseon.QueryError):
        query_client.query_metric(query_run.run_id, "train/loss", max_points=2)
