"""Dependency-free read-only command-line interface for PulseOn stores."""

from __future__ import annotations

import argparse
from collections.abc import Sequence
import contextlib
import io
import json
import math
import pathlib
import sys

from pulseon import _pulseon

_JSON_SCHEMA_VERSION = 1
_OPERATION_ERROR_CODES = {
    "ClientClosedError": "client_closed",
    "InvalidConfigurationError": "invalid_configuration",
    "InvalidRunStateError": "invalid_run_state",
    "MetricDrainTimeoutError": "metric_drain_timeout",
    "MetricFlushError": "metric_flush_failed",
    "MetricFlushTimeoutError": "metric_flush_timeout",
    "MetricQueueFullError": "metric_queue_full",
    "MetricWriterFailedError": "metric_writer_failed",
    "RunAlreadyActiveError": "run_already_active",
    "RunAlreadyExistsError": "run_already_exists",
    "RunClosedError": "run_closed",
    "StorageError": "storage_error",
}


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


def _parse_args(argv: Sequence[str] | None) -> argparse.Namespace:
    error_output = io.StringIO()
    try:
        with contextlib.redirect_stderr(error_output):
            return _build_parser().parse_args(argv)
    except SystemExit as error:
        if error.code == 2 and _json_requested(argv):
            print(
                _render_error("cli_usage_error", _usage_message(error_output.getvalue())),
                file=sys.stderr,
            )
        else:
            print(error_output.getvalue(), end="", file=sys.stderr)
        raise


def _json_requested(argv: Sequence[str] | None) -> bool:
    arguments = tuple(sys.argv[1:] if argv is None else argv)
    for index, value in enumerate(arguments):
        if value == "--format=json":
            return True
        if value == "--format" and arguments[index + 1 : index + 2] == ("json",):
            return True
    return False


def _usage_message(output: str) -> str:
    marker = ": error: "
    return output.rsplit(marker, maxsplit=1)[-1].strip()


def _render_error(code: str, message: str) -> str:
    document = {
        "schema_version": _JSON_SCHEMA_VERSION,
        "error": {"code": code, "message": message},
    }
    return _dump_json(document)


def _dump_json(document: object) -> str:
    return json.dumps(
        document,
        allow_nan=False,
        sort_keys=True,
        separators=(",", ":"),
    )


def _json_scalar(value: object) -> object:
    """Returns a standard-JSON representation for a scalar value."""
    if not isinstance(value, float) or math.isfinite(value):
        return value
    if math.isnan(value):
        return "NaN"
    return "Infinity" if value > 0 else "-Infinity"


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
    *,
    kind: str,
    page: dict[str, object] | None = None,
    meta: dict[str, object] | None = None,
) -> str:
    if output_format == "table":
        return _render_table(headers, rows)
    keys = [header.lower() for header in headers]
    data = [
        {
            key: _json_scalar(value)
            for key, value in zip(keys, row, strict=True)
        }
        for row in rows
    ]
    document = {
        "schema_version": _JSON_SCHEMA_VERSION,
        "kind": kind,
        "data": data,
        "page": page,
        "meta": {} if meta is None else meta,
    }
    return _dump_json(document)


def _run(client: _pulseon.Client, args: argparse.Namespace) -> str:
    if args.resource == "projects":
        projects = client.list_projects()
        return _render(
            ("PROJECT_ID", "NAME", "CREATED_AT"),
            [(item.project_id, item.name, item.created_at) for item in projects],
            args.format,
            kind="projects",
        )
    if args.resource == "runs":
        query_limit = None if args.limit is None else args.limit + 1
        runs = client.list_runs(
            args.project_id,
            status=args.status,
            limit=query_limit,
            offset=args.offset,
        )
        has_more = args.limit is not None and len(runs) > args.limit
        if has_more:
            runs = runs[: args.limit]
        return _render(
            ("RUN_ID", "PROJECT_ID", "NAME", "STATUS", "CREATED_AT"),
            [
                (item.run_id, item.project_id, item.name, item.status, item.created_at)
                for item in runs
            ],
            args.format,
            kind="runs",
            page={
                "offset": args.offset,
                "limit": args.limit,
                "returned": len(runs),
                "has_more": has_more,
            },
        )
    if args.action == "list":
        metrics = client.list_metrics(args.run_id)
        return _render_summaries(
            metrics,
            include_metric_key=True,
            output_format=args.format,
            kind="metrics",
        )
    if args.action == "query":
        max_points = None if args.all else args.max_points
        meta = None
        if args.format == "json":
            points, source_row_count, downsampled = (
                client._query_metric_with_metadata(
                    args.run_id,
                    args.metric_key,
                    start_step=args.start_step,
                    end_step=args.end_step,
                    max_points=max_points,
                )
            )
            meta = {
                "source_row_count": source_row_count,
                "returned_row_count": len(points),
                "downsampled": downsampled,
            }
        else:
            points = client.query_metric(
                args.run_id,
                args.metric_key,
                start_step=args.start_step,
                end_step=args.end_step,
                max_points=max_points,
            )
        return _render(
            ("STEP", "VALUE", "TIMESTAMP"),
            [(item.step, item.value_f64, item.timestamp) for item in points],
            args.format,
            kind="metric_points",
            meta=meta,
        )
    summaries = client.query_metric_summaries(args.run_ids, args.metric_key)
    return _render_summaries(
        summaries,
        include_metric_key=False,
        output_format=args.format,
        kind="metric_summaries",
    )


def _render_summaries(
    summaries: Sequence[_pulseon.MetricSummary],
    *,
    include_metric_key: bool,
    output_format: str,
    kind: str,
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
    return _render(headers, rows, output_format, kind=kind)


def _resolve_cli_path(
    project_path: pathlib.Path, value: str | None
) -> pathlib.Path | str | None:
    if value is None or "://" in value:
        return value
    path = pathlib.Path(value)
    return path if path.is_absolute() else project_path / path


def main(argv: Sequence[str] | None = None) -> int:
    """Runs the PulseOn CLI and returns its process exit status."""
    args = _parse_args(argv)
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
        message = str(error)
        if args.format == "json":
            code = _OPERATION_ERROR_CODES.get(type(error).__name__, "operation_failed")
            message = _render_error(code, message)
        print(message, file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
