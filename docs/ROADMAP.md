# PulseOn Roadmap

> This roadmap tracks current and future work. Shipped release details live in
> `docs/release-notes/`; durable product boundaries live in
> `docs/native-storage-boundary.md` and accepted ADRs in `docs/adr/`.

Pre-1.0 releases do not promise store, API, or machine-output compatibility;
compatibility and migration commitments begin with the future 1.0 release.

## 0.2.x / Desktop Curve Viewer

0.2.x unlocks interactive analysis that 0.1.x's headless read surface cannot
provide, and ships the comparison alignment semantics that the viewer consumes.
See [ADR 0011](adr/0011-desktop-first-curve-viewer.md) for the desktop-first
decision and [ADR 0012](adr/0012-defer-remote-training.md) for the remote
training deferral.

### Phase 1: Workspace Migration and Renderer-Agnostic Chart Core

- [x] One-time workspace migration to a virtual Cargo workspace (root `Cargo.toml`
  holds only `[workspace]`, no `[package]`): move `src/` to
  `crates/pulseon-core/src/`, set `members = ["crates/*"]`, update
  `pyproject.toml`/maturin `manifest-path` and any CI `cargo` invocations. No
  behavior change; `cargo check`, `cargo test`, `uv run maturin develop`,
  `uv run pyright`, and `uv run pytest` must still pass after the move.
- [x] `crates/pulseon-chart-core`: series model, viewport, scales, ticks,
  path projection, path cache, hit testing, selection and zoom state. Must not
  depend on GPUI, egui, Tauri, React, or a browser runtime, and must be unit
  testable without a window.
- [x] `crates/pulseon-data`: Parquet/DuckDB query and PulseOn schema validation,
  viewport-aware query planning, and screen-budgeted point reduction. Reuses the
  existing Parquet schema contract; no schema changes.

### Phase 1.5: Crate Responsibility Realignment

- [x] Extract shared domain and query contracts into `pulseon-model`.
- [x] Split the PyO3 artifact into `pulseon-python`, leaving `pulseon-core` as a
  reusable application library.
- [x] Rename `pulseon-data` to `pulseon-storage` and consolidate native project
  and standalone Parquet reads behind one metric query contract.
- [x] Move DuckDB/DuckLake bootstrap, reads, writes, flush, configuration, and
  storage errors out of Core. Preserve the Python API and Parquet contract.
- [x] Enforce the dependency direction in `docs/crate-boundaries.md` before
  adding the GPUI viewer.

### Phase 2: Comparison Alignment Semantics

Phase 2 is a read-only derived layer over existing Runs and metric facts. It
does not add catalog or Parquet fields, persisted research context or decisions,
source/Git mutation, repetition or significance policy, runtime dependencies,
or a renderer dependency. It may add typed Rust/Python read APIs and replaces
the pre-1.0 CLI JSON envelope with version 2.

#### Phase 2A: Contract and Product Language

- [x] Write `docs/comparison-semantics.md` as the renderer-agnostic 0.2.x
  contract, explicitly marked as changeable before 1.0. Define comparison axis,
  objective metric, comparison evidence, completeness, outcome, and preference
  as general product terms. Candidate and incumbent remain request roles, not
  stored Run identities. Do not create an ADR before 1.0 freezes the contract.
- [x] Lock two axes: raw step and elapsed wall time from `Run.started_at`.
  Elapsed values may repeat but not decrease; negative or non-monotonic axes
  are invalid, and missing Run start metadata is unavailable.
- [x] Define invalid and partial evidence without repairing it. Preserve usable
  numeric evidence from running and failed Runs while keeping their preference
  inconclusive; do not reorder, interpolate, clamp, or replace invalid values.
- [x] Define scalar comparison from the last effective value at the greatest
  step. Raw delta is `candidate - reference`; relative delta divides by the
  absolute reference and is absent for a zero reference. Direction-normalized
  improvement is positive when better. No tolerance, significance, or
  uncertainty claim is made in 0.2.x.

#### Phase 2B: Observation Time and Aligned Query Foundation

- [x] Capture a metric's observation timestamp on the `run.log(...)` enqueue
  path while leaving `ingested_at` on the background writer. Preserve the
  logging signature, queue admission, drain/finalization behavior, and metric
  schema. Document pre-0.2 timestamps as best-effort elapsed evidence because
  their writer-time origin cannot be detected or migrated safely.
- [x] Add shared alignment request/result types and axis-aware storage queries
  for the native project store and standalone Parquet facts. Alignment uses a
  closed viewport plus one neighboring point on each side. Both axes support
  full and screen-budgeted extrema queries; elapsed queries do not use LTTB.
- [x] Keep standalone Parquet fact-only: step alignment remains available,
  while elapsed alignment reports `missing_run_start` rather than treating the
  first point as the Run origin.

#### Phase 2C: Typed Shared Read API

- [x] Expose typed, renderer-independent alignment, objective, comparison,
  ranking, evidence, and result value objects from the shared Rust
  model/Core boundary. Keep storage execution behind the metric-reader
  contract and rendering conversion outside Core.
- [x] Expose matching read-only Python value objects and Client methods for
  aligned metric queries, Run comparison, and ranking; update the type stub and
  Python type-check fixtures. Do not add autoresearch-specific SDK types: the
  SDK language remains Project, Run, metric, comparison, and ranking.

#### Phase 2D: Generic and Autoresearch Comparison Reports

- [x] Upgrade `metrics compare` to require an explicit baseline contained in
  the requested Run set and an explicit `minimize` or `maximize` direction.
  Permit cross-Project comparison. Preserve input order for candidate reports.
- [x] Add `autoresearch compare` as a role-oriented view over the same Core
  report. Its incumbent is either explicit or the direction-aware best eligible
  Run from an explicit comparator pool; it is never inferred from project
  history. No eligible incumbent yields insufficient evidence, not mutation.
- [x] Report primary and secondary last values, raw and relative deltas,
  normalized improvement, structured completeness/reasons, numeric outcome,
  and compute-only preference. Secondary metrics never affect outcome,
  preference, ranking, or tie-breaking in 0.2.x.
- [x] Allow running and failed Runs to expose available numeric evidence but
  mark their report partial and preference inconclusive. Unknown or duplicate
  Run identities are request errors; missing metrics and non-finite values are
  per-item unavailable/invalid evidence.

#### Phase 2E: Ranking and Machine Output

- [ ] Add `autoresearch leaderboard` and `autoresearch best` over a required
  Project with an optional explicit Run subset. Only finished Runs with a
  finite primary objective are eligible; other Runs remain visible with a null
  rank and structured reason.
- [ ] Use direction-aware competition ranking (`1, 1, 3`). Exact ties share a
  rank; selecting one best/incumbent prefers the earlier `created_at`, then
  lexical `run_id`. An empty eligible set is a successful `best = null` result.
- [ ] Default leaderboard output to 50 entries with limit/offset pagination and
  an explicit all-results option. Compute ranks over the full eligible set
  before pagination.
- [ ] Bump every CLI success and error JSON envelope to schema version 2. Keep
  deterministic kinds, ordering, reason codes, pagination metadata, standard
  JSON encoding for non-finite metric values, and null relative deltas when the
  reference is zero. CLI comparison output stays bounded evidence; aligned
  curve points remain on the typed Rust/Python read surface.

#### Phase 2 Validation Gates

- [ ] Preserve the native storage and crate dependency boundaries, effective
  last-write-wins series, half-open ordinary step queries, Parquet schema, and
  non-blocking metric-reporting contract.
- [ ] Cover native/Parquet alignment parity, negative and decreasing elapsed
  axes, missing Run starts, viewport neighbors, screen budgets, both objective
  directions, zero/non-finite values, partial Runs, cross-Project pairs,
  incumbent pools, ranking ties, empty best, pagination, typed Python use, and
  deterministic JSON.
- [ ] Pass `cargo check`, `cargo test`, `uv run maturin develop --uv`,
  `uv run pyright`, and `uv run pytest`; run the logging throughput benchmark
  after moving timestamp capture and document any measurable regression.

### Phase 3: GPUI Desktop Viewer

- [ ] `crates/pulseon-viewer`: GPUI desktop shell, layout, file/directory
  picking, panels and commands, rendering adapter. Consumes the Phase 2
  comparison contract for multi-axis comparison support.
- [ ] Spike validation gates from `docs/drafts/gpui-curve-viewer-spike.md`:
  render 10 visible series smoothly after viewport downsampling; pan, zoom, and
  hover remain responsive with million-point source series; chart-core is unit
  testable without a GPUI window; replacing GPUI would require a renderer adapter
  rewrite, not a data model rewrite.
- [ ] Release artifacts: Python wheel (`maturin build`, unchanged) plus a native
  desktop binary (`cargo build -p pulseon-viewer --release`, new).

### Out of 0.2.x Scope

- Cumulative-token and normalized-budget comparison axes.
- Stable Contract / compatibility ADR / schema version marker / deprecation
  policy (deferred to 1.0).
- Retry-safe migration command (mutates state; deferred to 1.0).
- Persisted research decisions / durable research context / lineage / decisions
  in catalog state (ADR-gated, later).
- Research driver with Git/source mutation (ADR-gated, later).
- Remote training service delivery (deferred per ADR 0012).
- Repetition / significance / uncertainty policies (after deterministic
  policies are validated).

## Later Backlog

### Local Coordination

- [ ] Define and validate multi-client SQLite run-writer coordination before
  expanding the current single-writer native contract.

### Credentials and Remote Training

- [ ] Add environment-variable or AWS credential-chain discovery for S3
  credentials when explicit config-file credentials are insufficient.
- [ ] Revisit the [remote control-service boundary](drafts/remote-training-architecture-notes.md)
  when local training is complete and a real rented-GPU workflow exists, per
  [ADR 0012](adr/0012-defer-remote-training.md). Produce a remote training ADR
  before adding remote writers or shared catalog coordination.
- [ ] Consider PostgreSQL catalog support only when remote service scale or
  availability requires it.

### Analysis and Agent Workflows

- [ ] Evaluate the [research driver](drafts/autoresearch-control-loop-notes.md)
  without moving source or Git mutation into PulseOn Core.
- [ ] Design workspace hierarchy, config/tag filtering, export, Web UI, MCP,
  and other agent-facing surfaces as independently reviewable roadmap phases.

## 1.0 / Stable Contract

1.0 freezes the surfaces proven by 0.2.x. It is the first release with a
compatibility commitment; pre-1.0 releases make none.

- [ ] Accept an ADR defining 1.0 compatibility for the typed Python API,
  versioned CLI JSON, catalog application schema, and Parquet schema.
- [ ] Add an explicit store schema/version marker without changing the metric
  point Parquet compatibility boundary.
- [ ] Support an explicit `0.1.0a5` store upgrade; diagnose older unversioned
  stores without promising direct a1-a4 migration.
- [ ] Document additive changes, deprecation, breaking changes, and the support
  window for stable stores and machine-readable output.
- [ ] Add an explicit, retry-safe migration command that backs up catalog state
  and never rewrites a store during ordinary initialization. Cover DuckDB and
  SQLite stores, backend/config mismatches, mixed legacy artifacts, interrupted
  migration, retry, and backup recovery.
