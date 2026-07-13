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

    error_types = [
        pulseon.MetricQueueFullError,
        pulseon.MetricWriterFailedError,
        pulseon.MetricDrainTimeoutError,
        pulseon.MetricFlushError,
        pulseon.MetricFlushTimeoutError,
        pulseon.RunClosedError,
        pulseon.ClientClosedError,
        pulseon.InvalidRunStateError,
        pulseon.RunAlreadyExistsError,
        pulseon.RunAlreadyActiveError,
        pulseon.InvalidConfigurationError,
        pulseon.StorageError,
    ]
    assert all(issubclass(error_type, pulseon.PulseOnError) for error_type in error_types)
    with pytest.raises(pulseon.RunAlreadyExistsError):
        client.create_run(project.project_id, "duplicate", run_id=run.run_id)
    with pytest.raises(pulseon.StorageError):
        client.create_run("missing-project", "baseline")
    with pytest.raises(pulseon.StorageError):
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
    with pytest.raises(pulseon.PulseOnError, match="max_points must be at least 2"):
        query_client.query_metric(query_run.run_id, "train/loss", max_points=1)
    with pytest.raises(pulseon.StorageError, match="LTTB extension is unavailable"):
        query_client.query_metric(query_run.run_id, "train/loss", max_points=2)


def test_ducklake_attach_storage_error_is_sanitized(tmp_path: pathlib.Path) -> None:
    import pulseon

    root_path = tmp_path / "pulseon"
    catalog_path = tmp_path / "private" / "catalog.ducklake"
    data_path = tmp_path / "secret-data"
    catalog_path.mkdir(parents=True)

    with pytest.raises(pulseon.StorageError) as error_info:
        pulseon.init(root_path, catalog_path=catalog_path, data_path=data_path)

    message = str(error_info.value)
    assert "attaching DuckLake catalog" in message
    assert "catalog.ducklake" in message
    assert "secret-data" in message
    assert str(tmp_path) not in message
