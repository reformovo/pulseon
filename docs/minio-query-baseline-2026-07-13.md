# MinIO Metric Query Baseline — 2026-07-13

## Scope

This a5 baseline was produced from commit `d02acff` plus the Phase 5 query
trace instrumentation. It is environment-specific evidence, not a portable
latency or amplification limit.

## Environment

- macOS 26.3 (25D125), arm64
- CPython 3.13.2; installed PulseOn version 0.1.0
- MinIO `RELEASE.2025-09-07T16-13-09Z`, linux/arm64
- `mc RELEASE.2025-08-13T08-35-41Z`, darwin/arm64
- MinIO and the benchmark ran on the same host through port forwarding.

## Command and Scenario

```bash
uv run python scripts/bench_minio_metric_query.py
```

For each catalog backend, the benchmark created three runs and four metric
keys per run: 12 Parquet partitions/files containing 10,000 steps each. It
queried `run-1`, `metric/2`, and the half-open range `[4000, 6000)` five times.
Every query returned 2,000 points.

## Results

| Catalog | Query latency min / median / max | Remote bytes | Parquet GETs | Read amplification | Unrelated reads |
| --- | ---: | ---: | ---: | ---: | ---: |
| DuckDB | 14.903 / 20.922 / 57.938 ms | 221,193 | 7 | 0.9216× | 0 |
| SQLite | 20.256 / 31.459 / 72.049 ms | 221,969 | 7 | 0.9249× | 0 |

MinIO trace ran only around the repeated queries. Remote bytes are the sum of
`callStats.tx`; the observed requests were all Parquet GETs. The benchmark
defines logical result bytes as 24 fixed-width bytes per point (step,
timestamp, and value) per repeat, and read amplification as Parquet response
bytes divided by that logical size. Compression and cache reuse across repeats
can make this workload-level ratio lower than one.

The release gate is structural: any Parquet GET outside the selected `run_id`
or `metric_key_encoded` partition fails the benchmark. Latency, response bytes,
and amplification remain recorded baselines because host, network, cache,
MinIO, DuckDB, and Parquet layout differences affect them.
