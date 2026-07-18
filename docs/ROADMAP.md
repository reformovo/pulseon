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
- [ ] `crates/pulseon-chart-core`: series model, viewport, scales, ticks,
  downsampling, path cache, hit testing, selection and zoom state. Must not
  depend on GPUI, egui, Tauri, React, or a browser runtime, and must be unit
  testable without a window.
- [ ] `crates/pulseon-data`: Parquet/DuckDB query and PulseOn schema validation,
  viewport-aware query planning. Reuses the existing Parquet schema contract; no
  schema changes.

### Phase 2: Comparison Alignment Semantics

- [ ] Define the comparison contract (step, elapsed wall time, cumulative
  tokens, normalized budget progress [0,1]) as a shareable, renderer-agnostic
  surface. Document in a `docs/comparison-semantics.md`-style spec marked as a
  0.2.x contract that may change before 1.0; no ADR until 1.0 freezes it.
- [ ] `metrics compare` / `autoresearch compare` with objective direction,
  incumbent derivation, absolute and relative delta, secondary metrics, evidence
  completeness, and versioned JSON envelope. Comparison output is evidence
  (deltas, compute-only advice), never persisted research decisions or catalog
  state.
- [ ] `autoresearch leaderboard` / `autoresearch best` building on pairwise
  comparison: direction-aware ranking across many runs.

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
