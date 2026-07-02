# PulseOn V1 Roadmap

> Source of truth: `docs/v1-native-architecture.md` and ADRs in `docs/adr/`.
> Older Cloud, workspace, StorageLayer, and AI Native plans are intentionally out
> of the v1 roadmap.

## Phase 0: Scaffold Done

- [x] Rust/PyO3 package scaffold with maturin.
- [x] Python import test.
- [x] Initial module layout.

## Phase 1: Native Data Model + DuckLake Probe

- [x] Define minimal Rust types: `Project`, `Run`, `RunStatus`,
  `MetricPoint`, `MetricAggregate`, query parameter structs, and IDs.
- [x] Attach DuckLake in tests and create the minimal v1 tables.
- [x] Verify the model against real insert/query behavior instead of pure types
  only.
- [x] Do not add workspace, config, tags, artifacts, events, Cloud skeletons, or
  public storage abstractions.

Acceptance: Rust tests can initialize a temporary DuckLake dataset, create a
project/run, insert metric points, and read them back.

## Phase 2: Write Path + Aggregates

- [x] Implement run creation with generated or user-supplied `run_id`.
- [x] Require explicit resume when `run_id` already exists.
- [x] Implement `log_metric` with automatic per `(run_id, metric_key)` step.
- [x] Store metric points with `ingested_at`.
- [x] Implement logical last-write-wins query semantics.
- [x] Design materialized-view-like aggregate state for metric discovery and
  summaries: effective count, last, min, max.
- [x] Allow async repair of stale aggregates after old-step overwrites.

Acceptance: tests cover automatic step, explicit step, duplicate overwrite,
resume conflict, and aggregate values over the effective series.

## Phase 3: Query + Downsampling

- [x] Implement `query_metric(run_id, metric_key, start_step, end_step,
  max_points)`.
- [x] Return unchanged series when row count is at or below `max_points`.
- [x] Enforce strict `max_points` for long series.
- [x] Preserve endpoints during downsampling.
- [x] Integrate the DuckDB LTTB plugin rather than implementing downsampling in
  PulseOn.
- [x] Implement summary query for multi-run comparison.

Acceptance: tests cover range query, last-write-wins query output,
downsampled output length, endpoint preservation, and summary comparison.

## Phase 4: Python SDK

- [x] Expose `pulseon.init(path)`.
- [x] Expose project/run creation.
- [x] Expose `run.log(key, value)` and `run.log(key, step, value)`.
- [x] Keep ordinary `run.log(...)` calls training-loop non-blocking with
  bounded native buffering.
- [x] Surface dropped or failed metric reports through diagnostics without
  raising by default from hot-path logging.
- [x] Expose metric query and summary query as data-returning APIs only.
- [x] Update `python/pulseon/_pulseon.pyi`.

Acceptance: pytest covers the native loop from Python without plotting
dependencies. Rust or Python tests simulate a slow or blocked writer and verify
ordinary `run.log(...)` calls do not wait indefinitely.

## Phase 5: V1 Closure Backlog

- [x] Expose project/run selection APIs for existing local data.
- [x] Expose explicit run resume in Python instead of requiring callers to
  recreate run handles from saved IDs.
- [x] Expose run listing by project so multi-run comparison does not require
  callers to persist run IDs outside PulseOn.
- [x] Make orphan `running` runs detectable from Python.
- [x] Implement run finalization APIs for `finished` and `failed` lifecycle
  transitions.
- [x] Define bounded drain behavior for run finalization and client shutdown.
- [x] Expose an explicit client shutdown path, preferably context-manager
  friendly.
- [x] Expose metric discovery as data-returning APIs over aggregate/index state.
- [x] Clarify diagnostics semantics: accepted reports mean accepted into the
  native buffer, not durably stored metric points.
- [x] Expand diagnostics with writer freshness state such as backlog/drained
  status and last write error.
- [x] Document the stable PulseOn Parquet schema contract and compatibility
  rules.
- [x] Keep charting outside PulseOn v1: return chart-ready metric data only and
  do not add built-in plotting dependencies or rendering APIs.
- [x] Add Python-facing tests for query range filters, downsampling limits,
  endpoint preservation, and summary comparison.
- [x] Define Python-facing error classes for actionable SDK failures such as
  duplicate run, missing project/run, DuckLake unavailable, and query failure.

Acceptance: a restarted Python process can select existing local data, detect
orphan running runs, resume or finalize runs explicitly, discover metric keys,
query chart-ready data without plotting dependencies, and inspect diagnostics
that distinguish queued, persisted, dropped, delayed, and failed metric reports.

## Structural Cleanup Backlog

Source: 2026-07-02 code-organization audit against
`docs/v1-native-architecture.md`. These items are not new product features; they
remove obsolete scaffold shape and make the existing native v1 behavior easier
to maintain.

- [x] Split query behavior out of `src/engine/write.rs`. `NativeWriteStore`
  currently owns run creation, metric writes, effective-series queries,
  downsampling, LTTB extension loading, aggregate lookup, summary queries, and
  aggregate repair. Move read/query/downsampling code to a query-focused engine
  module and keep the write path responsible for writes and write-side aggregate
  maintenance.
- [x] Split storage bootstrap/schema code out of `src/engine/client.rs`.
  `NativeClient` currently mixes DuckLake attach, schema creation, project/run
  selection, run lifecycle, metric discovery, metric queries, diagnostics, and
  `NativeRun`. Introduce focused native modules for bootstrap/schema and
  project/run/query operations, while preserving the current Python API.
- [x] Deduplicate v1 DuckLake schema and attach helpers. The schema exists in
  both `src/engine/client.rs` and `src/ducklake_test_support.rs`; tests should
  use the same schema/bootstrap path as the engine so future Parquet contract
  changes do not drift.
- [x] Remove obsolete scaffold modules that contradict the v1 native boundary:
  `src/catalog/`, `src/storage/`, and `src/compute/`. Their leaf files are
  one-line TODO placeholders for `CatalogLayer`, `StorageLayer`,
  `ComputeLayer`, Cloud, S3, and future query abstractions, while v1 explicitly
  has no public `StorageLayer` or Cloud skeletons. Relocate any still-needed
  error type before removing `mod catalog`, `mod storage`, and `mod compute`
  from `src/lib.rs`.
- [x] Remove invalid model placeholder files from `src/model/`: `artifact.rs`,
  `config.rs`, `event.rs`, `summary.rs`, and `tag.rs`. Configs, tags,
  artifacts, and events are deferred outside v1, and the implemented v1 summary
  state already lives in `metric.rs` as `MetricAggregate`.
- [x] Remove or replace stale SDK placeholder files: `src/sdk/config.rs`,
  `src/sdk/query.rs`, and `src/sdk/run.rs`. The implemented Python-facing
  classes currently live in `src/sdk/client.rs`; either split those classes into
  real modules or delete the empty Phase 5 placeholders.
- [x] Remove `src/engine/flush.rs` or turn it into a real drain/finalization
  module. It is a Phase 4 one-line placeholder, while bounded drain behavior is
  already implemented in `src/engine/reporting.rs` and used by
  `src/engine/client.rs`.
- [x] Update stale module comments after removing placeholders. `src/lib.rs`
  and several `mod.rs` headers still describe the removed broad architecture
  (`docs/native-architecture.md`, AI Native, catalog/storage/compute layers).
  Point maintainers at `docs/v1-native-architecture.md` and the v1 native
  module boundaries instead.
- [x] Keep valid empty marker files. `python/pulseon/py.typed` and
  `tests/__init__.py` are intentional marker/package files, not invalid
  placeholders.

Acceptance: placeholder-only Rust files and obsolete module declarations are
gone, v1 behavior is unchanged, Rust and Python public APIs stay compatible, and
the relevant gates (`cargo check`, `cargo test`, `uv run pyright`, and
`uv run pytest`) pass.

## Testing Optimization Backlog

Source: 2026-07-02 test-organization audit against
`docs/v1-native-architecture.md`, Python testing conventions, and Rust testing
best practices. These items do not add product behavior; they make the existing
v1 coverage easier to maintain and easier to diagnose when it fails.

- [x] Split `tests/test_init.py` by Python-facing behavior. Keep import smoke
  coverage separate from SDK lifecycle, metric logging, query/downsampling,
  diagnostics, and actionable error tests so each file name explains the
  behavior it protects.
- [x] Make Python async-write wait helpers fail explicitly on timeout. Replace
  partial-result returns with assertion failures that include the expected
  count, actual count, run id, metric key, and current diagnostics.
- [ ] Rename or reorganize `src/ducklake_probe.rs` as native engine behavior
  tests. The file now protects core v1 write/query/aggregate semantics rather
  than a temporary DuckLake probe, so its name and grouping should reflect the
  durable responsibility.
- [ ] Reduce repeated Rust DuckLake setup in behavior tests without hiding the
  storage boundary. Keep tests backed by real temporary DuckLake datasets, but
  move boilerplate project/run/metric setup into small test helpers with
  behavior-focused names.
- [ ] Keep low-level Rust unit tests near implementation details and keep
  public Python tests at the SDK boundary. Unit tests should cover private
  parsing, status, reporter, and environment-option edge cases; pytest should
  verify user-visible behavior only.
- [ ] Stabilize downsampling coverage. Rust tests should keep a deterministic
  LTTB path for strict length and endpoint behavior; Python tests should cover
  SDK-level success/error mapping without depending on a developer-local
  extension path.
- [ ] Add or maintain a v1 coverage matrix in this roadmap. The matrix should
  map run lifecycle, explicit resume, non-blocking logging, last-write-wins,
  aggregate discovery, range query, downsampling, summary comparison, and
  diagnostics to their Rust and Python test owners.

Acceptance: test files are named by protected behavior, helper failures are
diagnostic, native engine semantics still exercise real DuckLake storage,
Python tests stay focused on public SDK behavior, and the relevant gates
(`cargo check`, `cargo test`, `uv run pyright`, and `uv run pytest`) pass.

## Deferred

- Workspace and organization hierarchy.
- Config/tag filtering.
- Deletion, hiding, and archival semantics.
- Public `StorageLayer`.
- Cloud implementations.
- AI Native tables, agent tools, MCP, and auto-research workflows.
