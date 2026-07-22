# Viewer Performance Validation

## Measurement Record

- Date: 2026-07-22
- Base commit: `ec89fab`, plus the validation changes in the working tree
- Platform: macOS 26.3 (25D125), arm64, Apple M4 Pro
- Rust: 1.97.1
- Xcode: 26.6 (17F113)
- Metal compiler: Xcode Metal Toolchain 17.6.109.0
- Display target: built-in Liquid Retina XDR with ProMotion enabled

The automated scale, CPU, build, type, and test gates pass. The interactive
120 Hz Metal trace remains pending because the current automation session
cannot synthesize and record the required continuous gestures, and macOS denied
screen capture. The Roadmap frame-rate item remains open until that trace is
recorded.

## Scale Fixture and Query Contract

The opt-in release validation generated both DuckDB and SQLite native Projects.
Each Project contained 10 finished Runs with 1,000,000 effective `loss` points
per Run. Database-side `range` generation avoided constructing million-point
Rust vectors. The retained local fixtures occupied 143 MiB for DuckDB and
136 MiB for SQLite; they are not repository artifacts.

All assertions passed:

- overview returned at most 2,000 points plus two neighbors per Run and at most
  20,020 points overall;
- detail returned at most 10,000 points plus two neighbors per Run and at most
  100,020 points overall;
- narrowing the detail viewport kept the 10,000-point budget, reduced each
  Run's source rows from 1,000,000 to 100,003, and preserved the requested
  viewport; and
- every viewer chart point matched its storage evidence point, proving that the
  viewer did not crop or resample the reduced result.

Query timings measure request submission through immutable worker snapshot
delivery and exclude rendering. Warm values are five samples.

| Backend | Query | Cold | Warm min / median / max |
| --- | --- | ---: | ---: |
| DuckDB | Overview | 1733.905 ms | 1627.727 / 1672.603 / 1718.678 ms |
| DuckDB | Full detail | 1773.152 ms | 1653.415 / 1666.751 / 1700.821 ms |
| DuckDB | Narrow detail | 686.589 ms | 674.201 / 696.890 / 721.489 ms |
| SQLite | Overview | 1767.398 ms | 1603.606 / 1639.230 / 1762.119 ms |
| SQLite | Full detail | 1714.943 ms | 1620.128 / 1680.154 / 1744.267 ms |
| SQLite | Narrow detail | 778.690 ms | 702.445 / 709.406 / 722.522 ms |

## CPU Budget

The release-only test used 10 series with 10,002 renderer-owned points each.
Every scenario passed p95 <= 8.33 ms and maximum <= 16.7 ms.

| Scenario | Samples | p50 | p95 | Maximum |
| --- | ---: | ---: | ---: | ---: |
| Brush resize | 1,000 | 0.000 ms | 0.000 ms | 0.000 ms |
| Brush pan | 1,000 | 0.000 ms | 0.000 ms | 0.000 ms |
| Brush zoom | 1,000 | 0.000 ms | 0.000 ms | 0.000 ms |
| Cached path preparation | 200 | 0.324 ms | 0.405 ms | 1.180 ms |
| Uncached path preparation | 200 | 7.621 ms | 7.892 ms | 8.075 ms |
| Hit testing | 200 | 0.196 ms | 0.208 ms | 0.255 ms |

## 120 Hz Product Check

The release viewer opened the retained 10-million-point DuckDB Project with
`MTL_HUD_ENABLED=1`, and Metal HUD initialized frame interval, present delay,
FPS, and logical FPS metrics. This confirms the release binary and HUD can run
against the scale fixture, but it is not the required interaction evidence.

To close the remaining gate, capture a Metal System Trace on the built-in
ProMotion display after initial detail load and a five-second warm-up. Exercise
brush handle resize, selected-window drag, main-chart pan, wheel or pinch zoom,
and hover continuously for ten seconds each. The HUD must sustain 120 FPS and
the trace must show no viewer-caused presentation spanning two 120 Hz refresh
periods. Record the trace conclusion here before closing the Roadmap item.

## Verification

Passed:

- `cargo fmt --all --check`
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- `cargo check`
- `cargo test`
- `cargo test -p pulseon-viewer --features test-support`
- `cargo build -p pulseon-viewer --release`
- `uv run maturin develop --uv`
- `uv run pyright` (zero errors)
- `uv run pytest` (106 passed, 2 opt-in MinIO tests skipped)
- `uv run maturin build --out dist`

The Rust build emitted existing future-incompatibility warnings for `block`
0.1.6 and `proc-macro-error2` 2.0.1; warnings were not produced by PulseOn code
and did not bypass the strict Clippy gate.
