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
- [ ] Implement `log_metric` with automatic per `(run_id, metric_key)` step.
- [ ] Store metric points with `ingested_at`.
- [ ] Implement logical last-write-wins query semantics.
- [ ] Design materialized-view-like aggregate state for metric discovery and
  summaries: effective count, last, min, max.
- [ ] Allow async repair of stale aggregates after old-step overwrites.

Acceptance: tests cover automatic step, explicit step, duplicate overwrite,
resume conflict, and aggregate values over the effective series.

## Phase 3: Query + Downsampling

- [ ] Implement `query_metric(run_id, metric_key, start_step, end_step,
  max_points)`.
- [ ] Return unchanged series when row count is at or below `max_points`.
- [ ] Enforce strict `max_points` for long series.
- [ ] Preserve endpoints during downsampling.
- [ ] Integrate the DuckDB LTTB plugin rather than implementing downsampling in
  PulseOn.
- [ ] Implement summary query for multi-run comparison.

Acceptance: tests cover range query, last-write-wins query output,
downsampled output length, endpoint preservation, and summary comparison.

## Phase 4: Python SDK

- [ ] Expose `pulseon.init(path)`.
- [ ] Expose project/run creation.
- [ ] Expose `run.log(key, value)` and `run.log(key, step, value)`.
- [ ] Expose metric query and summary query as data-returning APIs only.
- [ ] Update `python/pulseon/_pulseon.pyi`.

Acceptance: pytest covers the native loop from Python without plotting
dependencies.

## Deferred

- Workspace and organization hierarchy.
- Config/tag filtering.
- Deletion, hiding, and archival semantics.
- Public `StorageLayer`.
- Cloud implementations.
- AI Native tables, agent tools, MCP, and auto-research workflows.
