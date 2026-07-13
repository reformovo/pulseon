"""Verify the opt-in MinIO metric-query benchmark contract."""

from scripts import bench_minio_metric_query


def test_default_scenario_selects_across_runs_keys_files_and_steps() -> None:
    args = bench_minio_metric_query.build_parser().parse_args([])
    assert min(args.run_count, args.metric_key_count, args.repeats) > 1
    assert 0 < args.start_step < args.end_step < args.steps
