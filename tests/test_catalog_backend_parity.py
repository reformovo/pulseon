"""Verify DuckLake catalog backend parity for DuckDB and SQLite."""

from __future__ import annotations

import pathlib
import sqlite3

import pytest

from tests import helpers

_CATALOG_BACKENDS = ("duckdb", "sqlite")


@pytest.mark.parametrize("catalog_backend", _CATALOG_BACKENDS)
def test_catalog_backend_round_trips_native_storage_workflow(
    tmp_path: pathlib.Path,
    catalog_backend: str,
) -> None:
    import pulseon

    root_path = tmp_path / catalog_backend / "pulseon"
    data_path = tmp_path / catalog_backend / "custom-data"
    catalog_path = tmp_path / catalog_backend / "catalog" / "custom-catalog.db"
    client = pulseon.init(
        root_path,
        data_path=data_path,
        catalog_backend=catalog_backend,
        catalog_path=catalog_path,
    )
    project = client.create_project("local training", project_id="project-1")
    run = client.create_run(project.project_id, "baseline", run_id="run-1")
    run.log("train/loss", 0, 0.25)
    run.log("train/loss", 1, 0.125)
    run.log("eval/accuracy", 0, 0.8)

    active_points = helpers.wait_for_metric_points(
        client,
        run.run_id,
        "train/loss",
        expected_count=2,
    )
    active_metrics = client.list_metrics(run.run_id)
    finished = client.finish_run(run.run_id)
    terminal_points = client.query_metric(run.run_id, "train/loss")
    summaries = client.query_metric_summaries([run.run_id], "train/loss")
    metrics = client.list_metrics(run.run_id)
    diagnostics = client.diagnostics()
    del run
    del client

    reopened = pulseon.init(
        root_path,
        data_path=data_path,
        catalog_backend=catalog_backend,
        catalog_path=catalog_path,
    )
    reopened_project = reopened.get_project(project.project_id)
    reopened_run = reopened.get_run(finished.run_id)
    reopened_runs = reopened.list_runs(project.project_id)

    assert [point.value_f64 for point in active_points] == [0.25, 0.125]
    assert active_metrics == []
    assert finished.status == "finished"
    assert [point.step for point in terminal_points] == [0, 1]
    assert [summary.effective_count for summary in summaries] == [2]
    assert [summary.last_value_f64 for summary in summaries] == [0.125]
    assert [metric.metric_key for metric in metrics] == [
        "eval/accuracy",
        "train/loss",
    ]
    assert diagnostics.last_flush_status == "succeeded"
    assert any(
        (
            data_path
            / "main"
            / "metric_points"
            / "run_id=run-1"
            / "metric_key_encoded=train%252Floss"
        ).glob("*.parquet")
    )
    assert catalog_path.is_file()
    assert data_path.is_dir()
    assert reopened_project.name == "local training"
    assert reopened_run.status == "finished"
    assert [stored_run.run_id for stored_run in reopened_runs] == ["run-1"]


@pytest.mark.parametrize("catalog_backend", _CATALOG_BACKENDS)
def test_catalog_backend_rejects_invalid_local_storage_configuration(
    tmp_path: pathlib.Path,
    catalog_backend: str,
) -> None:
    import pulseon

    invalid_kwargs = [
        {"data_path": "s3://bucket/pulseon"},
        {"catalog_path": "s3://bucket/catalog.ducklake"},
    ]
    for index, kwargs in enumerate(invalid_kwargs):
        with pytest.raises(pulseon.InvalidConfigurationError):
            pulseon.init(
                tmp_path / catalog_backend / f"pulseon-{index}",
                catalog_backend=catalog_backend,
                **kwargs,
            )


def test_sqlite_catalog_file_contains_ducklake_and_pulseon_state(
    tmp_path: pathlib.Path,
) -> None:
    import pulseon

    root_path = tmp_path / "pulseon"
    catalog_path = root_path / ".pulseon" / "catalog.sqlite"
    client = pulseon.init(root_path, catalog_backend="sqlite")
    project = client.create_project("local training", project_id="project-1")
    run = client.create_run(project.project_id, "baseline", run_id="run-1")
    run.log("train/loss", 0, 0.25)
    helpers.wait_for_metric_points(
        client,
        run.run_id,
        "train/loss",
        expected_count=1,
    )

    tables_before_flush = _sqlite_table_names(catalog_path)
    inline_tables = [
        table
        for table in tables_before_flush
        if table.startswith("ducklake_inlined_data_")
    ]
    assert "ducklake_metadata" in tables_before_flush
    assert "ducklake_table" in tables_before_flush
    assert "pulseon_projects" in tables_before_flush
    assert "pulseon_runs" in tables_before_flush
    assert "pulseon_metric_aggregates" in tables_before_flush
    assert inline_tables
    assert _sqlite_table_count(catalog_path, "pulseon_projects") == 1
    assert _sqlite_table_count(catalog_path, "pulseon_runs") == 1
    assert sum(_sqlite_table_count(catalog_path, table) for table in inline_tables) >= 1

    client.finish_run(run.run_id)

    assert _sqlite_table_count(catalog_path, "pulseon_metric_aggregates") == 1
    assert _sqlite_table_count(catalog_path, "ducklake_data_file") >= 1


def test_unknown_catalog_backend_is_rejected(tmp_path: pathlib.Path) -> None:
    import pulseon

    with pytest.raises(pulseon.InvalidConfigurationError, match="postgres"):
        pulseon.init(tmp_path / "pulseon", catalog_backend="postgres")


def _sqlite_table_names(catalog_path: pathlib.Path) -> set[str]:
    with sqlite3.connect(catalog_path) as connection:
        rows = connection.execute(
            "SELECT name FROM sqlite_master WHERE type = 'table'"
        ).fetchall()
    return {str(row[0]) for row in rows}


def _sqlite_table_count(catalog_path: pathlib.Path, table_name: str) -> int:
    if not table_name.replace("_", "").isalnum():
        raise ValueError(f"invalid SQLite table name: {table_name}")
    with sqlite3.connect(catalog_path) as connection:
        count = connection.execute(f'SELECT count(*) FROM "{table_name}"').fetchone()
    if count is None:
        raise AssertionError(f"missing SQLite count for {table_name}")
    return int(count[0])
