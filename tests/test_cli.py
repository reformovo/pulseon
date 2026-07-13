"""Verify the dependency-free read-only PulseOn CLI."""

from __future__ import annotations

import json
import math
import pathlib

import pytest

from pulseon import cli
from tests import helpers


def test_cli_discovers_running_metric_points_through_all_read_commands(
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
    run.log("train/loss", 1, 0.25)
    helpers.wait_for_metric_points(
        client, run.run_id, "train/loss", expected_count=2
    )
    client.shutdown()
    monkeypatch.chdir(root_path)

    commands = (
        (["projects", "list"], "project-1"),
        (
            [
                "runs",
                "list",
                "project-1",
                "--status",
                "running",
                "--limit",
                "1",
            ],
            "run-1",
        ),
        (["metrics", "list", "run-1"], "train/loss"),
        (
            [
                "metrics",
                "query",
                "run-1",
                "train/loss",
                "--start-step",
                "0",
                "--end-step",
                "1",
                "--all",
            ],
            "0.5",
        ),
        (["metrics", "compare", "train/loss", "run-1"], "run-1"),
    )
    for argv, expected in commands:
        assert cli.main(argv) == 0
        assert expected in capsys.readouterr().out

    kinds = ("projects", "runs", "metrics", "metric_points", "metric_summaries")
    for (argv, _), kind in zip(commands, kinds, strict=True):
        assert cli.main(["--format", "json", *argv]) == 0
        document = json.loads(capsys.readouterr().out)
        assert document["schema_version"] == 1
        assert document["kind"] == kind
        assert isinstance(document["data"], list)
        assert "page" in document
        assert "meta" in document
        if kind == "runs":
            assert document["page"] == {
                "offset": 0,
                "limit": 1,
                "returned": 1,
                "has_more": False,
            }
        if kind == "metric_points":
            assert [point["step"] for point in document["data"]] == [0]


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


def test_cli_json_operation_errors_are_structured(
    tmp_path: pathlib.Path,
    capsys: pytest.CaptureFixture[str],
) -> None:
    root_path = tmp_path / "missing"
    root_path.mkdir()

    status = cli.main(
        ["--path", str(root_path), "--format", "json", "projects", "list"]
    )

    assert status == 1
    captured = capsys.readouterr()
    assert captured.out == ""
    assert json.loads(captured.err) == {
        "schema_version": 1,
        "error": {
            "code": "storage_error",
            "message": "catalog not found: catalog.ducklake",
        },
    }


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
    assert json.loads(capsys.readouterr().out) == {
        "schema_version": 1,
        "kind": "projects",
        "data": [
            {
                "created_at": project.created_at,
                "name": "training",
                "project_id": "project-1",
            }
        ],
        "page": None,
        "meta": {},
    }


def test_cli_json_includes_pagination_and_metric_query_metadata(
    tmp_path: pathlib.Path,
    capsys: pytest.CaptureFixture[str],
) -> None:
    import pulseon

    root_path = tmp_path / "project"
    client = pulseon.init(root_path)
    project = client.create_project("training", project_id="project-1")
    first_run = client.create_run(project.project_id, "first", run_id="run-1")
    first_run.log("loss", 0, 0.5)
    client.finish_run(first_run.run_id)
    second_run = client.create_run(project.project_id, "second", run_id="run-2")
    second_run.log("loss", 0, 0.25)
    client.finish_run(second_run.run_id)
    client.shutdown()

    global_args = ["--path", str(root_path), "--format", "json"]
    assert cli.main(
        [*global_args, "runs", "list", "project-1", "--limit", "1"]
    ) == 0
    runs_document = json.loads(capsys.readouterr().out)
    assert runs_document["schema_version"] == 1
    assert runs_document["kind"] == "runs"
    assert len(runs_document["data"]) == 1
    assert runs_document["page"] == {
        "offset": 0,
        "limit": 1,
        "returned": 1,
        "has_more": True,
    }
    assert runs_document["meta"] == {}

    assert cli.main(
        [*global_args, "metrics", "query", "run-1", "loss"]
    ) == 0
    query_document = json.loads(capsys.readouterr().out)
    assert query_document["schema_version"] == 1
    assert query_document["kind"] == "metric_points"
    assert query_document["page"] is None
    assert query_document["meta"] == {
        "source_row_count": 1,
        "returned_row_count": 1,
        "downsampled": False,
    }
    assert query_document["data"][0]["step"] == 0


def test_cli_json_normalizes_non_finite_metric_values(
    tmp_path: pathlib.Path,
    capsys: pytest.CaptureFixture[str],
) -> None:
    import pulseon

    root_path = tmp_path / "project"
    client = pulseon.init(root_path)
    project = client.create_project("training", project_id="project-1")
    run = client.create_run(project.project_id, "non-finite", run_id="run-1")
    for step, value in enumerate((math.nan, math.inf, -math.inf)):
        run.log("loss", step, value)
    client.finish_run(run.run_id)
    client.shutdown()

    global_args = ["--path", str(root_path), "--format", "json"]
    assert cli.main(
        [*global_args, "metrics", "query", "run-1", "loss", "--all"]
    ) == 0
    query = json.loads(capsys.readouterr().out)
    assert [row["value"] for row in query["data"]] == [
        "NaN",
        "Infinity",
        "-Infinity",
    ]

    assert cli.main([*global_args, "metrics", "list", "run-1"]) == 0
    summary = json.loads(capsys.readouterr().out)
    assert all(
        not isinstance(value, float) or math.isfinite(value)
        for value in summary["data"][0].values()
    )


def test_cli_preserves_symlinked_project_path(
    tmp_path: pathlib.Path,
    capsys: pytest.CaptureFixture[str],
) -> None:
    import pulseon

    real_path = tmp_path / "real-project"
    real_path.mkdir()
    linked_path = tmp_path / "linked-project"
    try:
        linked_path.symlink_to(real_path, target_is_directory=True)
    except OSError as error:
        pytest.skip(f"directory symlinks are unavailable: {error}")
    client = pulseon.init(linked_path)
    client.create_project("linked", project_id="project-1")
    client.shutdown()

    status = cli.main(
        ["--path", str(linked_path), "projects", "list"]
    )

    assert status == 0
    captured = capsys.readouterr()
    assert "project-1" in captured.out
    assert captured.err == ""


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


@pytest.mark.parametrize(
    "argv",
    (
        ["runs", "list", "project-1", "--limit", "-1"],
        ["runs", "list", "project-1", "--offset", "-1"],
        ["metrics", "query", "run-1", "loss", "--max-points", "-1"],
    ),
    ids=("limit", "offset", "max-points"),
)
def test_cli_rejects_negative_unsigned_arguments(
    argv: list[str], capsys: pytest.CaptureFixture[str]
) -> None:
    with pytest.raises(SystemExit) as error_info:
        cli.main(argv)

    assert error_info.value.code == 2
    captured = capsys.readouterr()
    assert captured.out == ""
    assert "expected a non-negative integer" in captured.err
    assert "Traceback" not in captured.err


def test_cli_json_usage_errors_are_structured(
    capsys: pytest.CaptureFixture[str],
) -> None:
    with pytest.raises(SystemExit) as error_info:
        cli.main(
            [
                "--format",
                "json",
                "metrics",
                "query",
                "run-1",
                "loss",
                "--max-points",
                "-1",
            ]
        )

    assert error_info.value.code == 2
    captured = capsys.readouterr()
    assert captured.out == ""
    document = json.loads(captured.err)
    assert document["schema_version"] == 1
    assert document["error"]["code"] == "cli_usage_error"
    assert "expected a non-negative integer" in document["error"]["message"]
    assert "usage:" not in captured.err


def test_cli_table_output_is_deterministic_and_uncolored() -> None:
    first = cli._render_table(("STEP", "VALUE"), ((0, 0.5), (10, 0.25)))
    second = cli._render_table(("STEP", "VALUE"), ((0, 0.5), (10, 0.25)))

    assert first == "STEP  VALUE\n----  -----\n0     0.5\n10    0.25"
    assert second == first
    assert "\x1b" not in first


def test_cli_keeps_s3_credentials_out_of_arguments() -> None:
    help_text = cli._build_parser().format_help()

    assert "secret" not in help_text.lower()
    assert "access-key" not in help_text.lower()
    assert "session-token" not in help_text.lower()


def test_cli_sanitizes_config_credentials_and_catalog_paths(
    tmp_path: pathlib.Path,
    capsys: pytest.CaptureFixture[str],
) -> None:
    root_path = tmp_path / "project"
    config_path = root_path / ".pulseon" / "config.toml"
    config_path.parent.mkdir(parents=True)
    secret = "credential-must-not-leak"
    config_path.write_text(
        'data_path = "s3://private-bucket/metrics"\n'
        "[s3]\n"
        'endpoint = "127.0.0.1:9000"\n'
        'access_key_id = "private-access-key"\n'
        f'secret_access_key = "{secret}"\n',
        encoding="utf-8",
    )
    private_catalog = tmp_path / "private" / "tenant" / "catalog.ducklake"

    status = cli.main(
        [
            "--path",
            str(root_path),
            "--format",
            "json",
            "--catalog-path",
            str(private_catalog),
            "projects",
            "list",
        ]
    )

    assert status == 1
    error = capsys.readouterr().err
    document = json.loads(error)
    assert document["error"] == {
        "code": "storage_error",
        "message": "catalog not found: catalog.ducklake",
    }
    assert secret not in error
    assert str(private_catalog.parent) not in error


def test_cli_json_sanitizes_lttb_extension_path(
    tmp_path: pathlib.Path,
    monkeypatch: pytest.MonkeyPatch,
    capsys: pytest.CaptureFixture[str],
) -> None:
    import pulseon

    root_path = tmp_path / "project"
    client = pulseon.init(root_path)
    project = client.create_project("training", project_id="project-1")
    run = client.create_run(project.project_id, "long", run_id="run-1")
    for step in range(201):
        run.log("loss", step, float(step))
    client.finish_run(run.run_id)
    client.shutdown()
    private_extension = (
        tmp_path / "private" / "tenant" / "missing-lttb.duckdb_extension"
    )
    monkeypatch.setenv("PULSEON_LTTB_EXTENSION_PATH", str(private_extension))

    status = cli.main(
        [
            "--path",
            str(root_path),
            "--format",
            "json",
            "metrics",
            "query",
            "run-1",
            "loss",
        ]
    )

    assert status == 1
    captured = capsys.readouterr()
    assert captured.out == ""
    error = json.loads(captured.err)["error"]
    assert error["code"] == "storage_error"
    assert private_extension.name in error["message"]
    assert str(private_extension.parent) not in captured.err
