"""Verify the dependency-free read-only PulseOn CLI."""

from __future__ import annotations

import json
import pathlib

import pytest

from pulseon import cli


def test_cli_exposes_all_read_commands(
    tmp_path: pathlib.Path,
    monkeypatch: pytest.MonkeyPatch,
    capsys: pytest.CaptureFixture[str],
) -> None:
    import pulseon

    root_path = tmp_path / "project"
    client = pulseon.init(root_path)
    project = client.create_project("training", project_id="project-1")
    run = client.create_run(project.project_id, "baseline", run_id="run-1")
    run.log("train/loss", 0, 0.5)
    client.finish_run(run.run_id)
    client.shutdown()
    monkeypatch.chdir(root_path)

    commands = (
        (["projects", "list"], "project-1"),
        (["runs", "list", "project-1"], "run-1"),
        (["metrics", "list", "run-1"], "train/loss"),
        (["metrics", "query", "run-1", "train/loss"], "0.5"),
        (["metrics", "compare", "train/loss", "run-1"], "run-1"),
    )
    for argv, expected in commands:
        assert cli.main(argv) == 0
        assert expected in capsys.readouterr().out


def test_cli_missing_store_fails_without_creating_it(
    tmp_path: pathlib.Path,
    monkeypatch: pytest.MonkeyPatch,
    capsys: pytest.CaptureFixture[str],
) -> None:
    root_path = tmp_path / "missing"
    root_path.mkdir()
    monkeypatch.chdir(root_path)

    assert cli.main(["projects", "list"]) == 1

    captured = capsys.readouterr()
    assert captured.out == ""
    assert captured.err == "catalog not found: catalog.ducklake\n"
    assert not (root_path / ".pulseon").exists()


def test_cli_resolves_global_path_overrides_against_project(
    tmp_path: pathlib.Path,
    monkeypatch: pytest.MonkeyPatch,
    capsys: pytest.CaptureFixture[str],
) -> None:
    import pulseon

    root_path = tmp_path / "workspace" / "project"
    client = pulseon.init(
        root_path,
        catalog_backend="sqlite",
        catalog_path=root_path / "storage" / "catalog.sqlite",
        data_path=root_path / "storage" / "data",
    )
    project = client.create_project("training", project_id="project-1")
    client.shutdown()
    monkeypatch.chdir(tmp_path)

    status = cli.main(
        [
            "--path",
            "workspace/project",
            "--format",
            "json",
            "--catalog-backend",
            "sqlite",
            "--catalog-path",
            "storage/catalog.sqlite",
            "--data-path",
            "storage/data",
            "projects",
            "list",
        ]
    )

    assert status == 0
    assert json.loads(capsys.readouterr().out) == [
        {
            "created_at": project.created_at,
            "name": "training",
            "project_id": "project-1",
        }
    ]


def test_cli_metric_query_point_limits_are_mutually_exclusive() -> None:
    parser = cli._build_parser()

    defaults = parser.parse_args(["metrics", "query", "run-1", "loss"])
    all_points = parser.parse_args(
        ["metrics", "query", "run-1", "loss", "--all"]
    )

    assert defaults.max_points == 200
    assert defaults.all is False
    assert all_points.all is True
    with pytest.raises(SystemExit) as error_info:
        parser.parse_args(
            [
                "metrics",
                "query",
                "run-1",
                "loss",
                "--max-points",
                "20",
                "--all",
            ]
        )
    assert error_info.value.code == 2


def test_cli_table_output_is_deterministic_and_uncolored() -> None:
    first = cli._render_table(("STEP", "VALUE"), ((0, 0.5), (10, 0.25)))
    second = cli._render_table(("STEP", "VALUE"), ((0, 0.5), (10, 0.25)))

    assert first == "STEP  VALUE\n----  -----\n0     0.5\n10    0.25"
    assert second == first
    assert "\x1b" not in first
