# PulseOn Benchmark Report — 2026-07-10

## Scope

This report records the admission-throughput benchmark and the repeated local
versus configured S3/OSS persistence benchmark. Results were produced from the
current worktree based on commit `7c44fbc`.

## Environment

- Platform: macOS 26.3, arm64
- Python: CPython 3.13.2
- PulseOn version reported by the package: `v0.1.0a4`
- Working directory: `/Users/kaikai/projects/pulseon`

## Commands

```bash
uv run python scripts/bench_log_throughput.py --reports 100000

uv run python scripts/bench_log_persistence.py \
  --reports 1000 \
  --repeats 3 \
  --object-storage-config .pulseon/config.toml
```

The persistence command used the default 120-second drain timeout. The report
does not include the object-storage URI, endpoint, or credentials.

## Admission Throughput

| Reports | Elapsed | Throughput | Queue-full errors |
| ---: | ---: | ---: | ---: |
| 100,000 | 0.052489 s | 1,905,160 calls/s | 0 |

Diagnostics immediately after the timed loop reported 100,000 pending reports,
zero persisted reports, no writer error, and a running writer. This is expected:
the benchmark measures only `run.log(...)` admission and deliberately exits
without waiting for persistence.

## Persistence Results

Each storage target ran three independent repeats with 1,000 reports per
repeat. Values below show median and the observed min–max range.

| Phase | Local | S3/OSS | S3/OSS ÷ local median |
| --- | ---: | ---: | ---: |
| Setup | 97.938 ms (97.116–98.955) | 109.816 ms (108.061–114.138) | 1.12× |
| Admission | 0.784 ms (0.783–0.931) | 0.821 ms (0.804–0.961) | 1.05× |
| Admission throughput | 1,275,714 calls/s | 1,218,708 calls/s | 0.96× |
| Drain | 60.845 ms (49.711–61.567) | 48.322 ms (46.892–48.430) | 0.79× |
| Finalization | 52.214 ms (37.390–53.199) | 176.167 ms (167.758–288.003) | 3.37× |
| Shutdown | 0.051 ms (0.047–0.066) | 0.048 ms (0.046–0.049) | 0.93× |

All six repeats completed with:

- `pending_reports = 0` after drain;
- `persisted_reports = 1000` after drain;
- `last_flush_status = "succeeded"` after finalization;
- no finalization flush error;
- `writer_state = "closed"` after shutdown.

## Interpretation

- Admission performance is effectively storage-independent, as expected from
  the bounded in-process queue design.
- S3/OSS setup was about 12% slower at the median.
- S3/OSS finalization was 3.37 times the local median because this phase includes
  the terminal Parquet flush to the configured object store.
- The lower S3/OSS drain median should not be interpreted as faster remote
  persistence. Drain timing includes asynchronous scheduling and polling, while
  the object-store Parquet write is reflected primarily in finalization.
- Shutdown overhead was negligible for both targets after successful
  finalization.
