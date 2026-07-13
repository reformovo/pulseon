"""Dependency-free read-only command-line interface for PulseOn stores."""

from __future__ import annotations

import argparse
from collections.abc import Sequence
import json
import pathlib
import sys

from pulseon import _pulseon


def _non_negative_int(value: str) -> int:
    try:
        parsed = int(value)
    except ValueError as error:
        raise argparse.ArgumentTypeError("expected a non-negative integer") from error
    if parsed < 0:
        raise argparse.ArgumentTypeError("expected a non-negative integer")
    return parsed


def _build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(prog="pulseon")
    parser.add_argument("--path", type=pathlib.Path, default=pathlib.Path("."))
    parser.add_argument("--format", choices=("table", "json"), default="table")
    parser.add_argument("--catalog-backend", choices=("duckdb", "sqlite"))
    parser.add_argument("--catalog-path")
    parser.add_argument("--data-path")
    resources = parser.add_subparsers(dest="resource", required=True)

    projects = resources.add_parser("projects")
    projects.add_subparsers(dest="action", required=True).add_parser("list")

    runs = resources.add_parser("runs")
    runs_list = runs.add_subparsers(dest="action", required=True).add_parser("list")
    runs_list.add_argument("project_id")
    runs_list.add_argument("--status", choices=("running", "finished", "failed"))
    runs_list.add_argument("--limit", type=_non_negative_int)
    runs_list.add_argument("--offset", type=_non_negative_int, default=0)

    metrics = resources.add_parser("metrics")
    metric_actions = metrics.add_subparsers(dest="action", required=True)
    metrics_list = metric_actions.add_parser("list")
    metrics_list.add_argument("run_id")
    metrics_query = metric_actions.add_parser("query")
    metrics_query.add_argument("run_id")
    metrics_query.add_argument("metric_key")
    metrics_query.add_argument("--start-step", type=int)
    metrics_query.add_argument("--end-step", type=int)
    point_limit = metrics_query.add_mutually_exclusive_group()
    point_limit.add_argument("--max-points", type=_non_negative_int, default=200)
    point_limit.add_argument("--all", action="store_true")
    metrics_compare = metric_actions.add_parser("compare")
    metrics_compare.add_argument("metric_key")
    metrics_compare.add_argument("run_ids", nargs="+")
    return parser


def _render_table(headers: Sequence[str], rows: Sequence[Sequence[object]]) -> str:
    text_rows = [[str(value) for value in row] for row in rows]
    widths = [
        max(len(header), *(len(row[index]) for row in text_rows))
        for index, header in enumerate(headers)
    ]

    def render(row: Sequence[str]) -> str:
        return "  ".join(value.ljust(widths[index]) for index, value in enumerate(row)).rstrip()

    divider = ["-" * width for width in widths]
    return "\n".join((render(headers), render(divider), *(render(row) for row in text_rows)))


def _render(
    headers: Sequence[str],
    rows: Sequence[Sequence[object]],
    output_format: str,
) -> str:
    if output_format == "table":
        return _render_table(headers, rows)
    keys = [header.lower() for header in headers]
    data = [dict(zip(keys, row, strict=True)) for row in rows]
    return json.dumps(data, sort_keys=True, separators=(",", ":"))


def _run(client: _pulseon.Client, args: argparse.Namespace) -> str:
    if args.resource == "projects":
        projects = client.list_projects()
        return _render(
            ("PROJECT_ID", "NAME", "CREATED_AT"),
            [(item.project_id, item.name, item.created_at) for item in projects],
            args.format,
        )
    if args.resource == "runs":
        runs = client.list_runs(
            args.project_id,
            status=args.status,
            limit=args.limit,
            offset=args.offset,
        )
        return _render(
            ("RUN_ID", "PROJECT_ID", "NAME", "STATUS", "CREATED_AT"),
            [
                (item.run_id, item.project_id, item.name, item.status, item.created_at)
                for item in runs
            ],
            args.format,
        )
    if args.action == "list":
        metrics = client.list_metrics(args.run_id)
        return _render_summaries(
            metrics, include_metric_key=True, output_format=args.format
        )
    if args.action == "query":
        points = client.query_metric(
            args.run_id,
            args.metric_key,
            start_step=args.start_step,
            end_step=args.end_step,
            max_points=None if args.all else args.max_points,
        )
        return _render(
            ("STEP", "VALUE", "TIMESTAMP"),
            [(item.step, item.value_f64, item.timestamp) for item in points],
            args.format,
        )
    summaries = client.query_metric_summaries(args.run_ids, args.metric_key)
    return _render_summaries(
        summaries, include_metric_key=False, output_format=args.format
    )


def _render_summaries(
    summaries: Sequence[_pulseon.MetricSummary],
    *,
    include_metric_key: bool,
    output_format: str,
) -> str:
    headers = ["RUN_ID"]
    if include_metric_key:
        headers.append("METRIC_KEY")
    headers.extend(("COUNT", "LAST_STEP", "LAST_VALUE", "MIN", "MAX"))
    rows: list[list[object]] = []
    for item in summaries:
        row: list[object] = [item.run_id]
        if include_metric_key:
            row.append(item.metric_key)
        row.extend(
            (
                item.effective_count,
                item.last_step,
                item.last_value_f64,
                item.min_value_f64,
                item.max_value_f64,
            )
        )
        rows.append(row)
    return _render(headers, rows, output_format)


def _resolve_cli_path(
    project_path: pathlib.Path, value: str | None
) -> pathlib.Path | str | None:
    if value is None or "://" in value:
        return value
    path = pathlib.Path(value)
    return path if path.is_absolute() else project_path / path


def main(argv: Sequence[str] | None = None) -> int:
    """Runs the PulseOn CLI and returns its process exit status."""
    args = _build_parser().parse_args(argv)
    project_path = args.path.absolute()
    try:
        with _pulseon.init(
            project_path,
            data_path=_resolve_cli_path(project_path, args.data_path),
            catalog_backend=args.catalog_backend,
            catalog_path=_resolve_cli_path(project_path, args.catalog_path),
            _must_exist=True,
        ) as client:
            print(_run(client, args))
    except _pulseon.PulseOnError as error:
        print(str(error), file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
