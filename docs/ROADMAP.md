# PulseOn Roadmap

> This roadmap tracks current and future work. Shipped release details live in
> `docs/release-notes/`; durable product boundaries live in
> `docs/native-storage-boundary.md` and accepted ADRs in `docs/adr/`.

The completed alpha plan is in the
[`0.1.0a5` release notes](release-notes/0.1.0a5.md). Pre-1.0 releases do not
promise store, API, or machine-output compatibility; compatibility and
migration commitments begin with the future 1.x line.

## 0.1.0rc1 to 0.1.0 / V6

V6 prepares the native metric loop and headless read surface for the 0.1.0
release. It prioritizes fresh-install behavior and automated gates.
Autoresearch, viewers, MCP, shared cloud coordination, workspace hierarchy,
and pre-1.0 compatibility or migration are not release blockers.

### Deferred to 1.x: Stable Contract

These items are intentionally not part of the 0.1.0 release.

- [ ] Accept an ADR defining 1.x compatibility for the typed Python API,
  versioned CLI JSON, catalog application schema, and Parquet schema.
- [ ] Add an explicit store schema/version marker without changing the metric
  point Parquet compatibility boundary.
- [ ] Support an explicit `0.1.0a5` store upgrade; diagnose older unversioned
  stores without promising direct a1-a4 migration.
- [ ] Document additive changes, deprecation, breaking changes, and the support
  window for stable stores and machine-readable output.

### Deferred to 1.x: Store Doctor and Migration

These items are intentionally not part of the 0.1.0 release.

- [ ] Add read-only `pulseon doctor` output for configured and detected catalog
  backends, schema compatibility, conflicting artifacts, and recovery advice.
- [ ] Keep ordinary diagnostics sanitized. Expose full local paths only through
  an explicit verbose diagnostic mode.
- [ ] Add an explicit, retry-safe migration command that backs up catalog state
  and never rewrites a store during ordinary initialization.
- [ ] Cover DuckDB and SQLite stores, backend/config mismatches, mixed legacy
  artifacts, interrupted migration, retry, and backup recovery.

### Phase 3: Fresh-install Downsampling

- [x] Keep the CLI's 200-point default and automatically run the official
  `INSTALL lttb FROM community; LOAD lttb;` flow when first required.
- [x] Keep Python SDK downloads opt-in so library queries do not introduce
  implicit network access.
- [x] Preserve offline paths through `--all` and `PULSEON_LTTB_EXTENSION_PATH`,
  with structured guidance when installation or loading fails.
- [x] Delegate signed extension compatibility to DuckDB and the community
  extension repository instead of duplicating their platform matrix in
  PulseOn's generated CI; platforms without an upstream build retain `--all`.

### Phase 4: Automated Release Gates

- [x] Make Rust formatting, Clippy, Rust tests, Pyright, and pytest required CI
  jobs rather than release-note-only manual evidence.
- [ ] Run the MinIO/S3 acceptance and read-amplification gates automatically on
  the Linux release path for both catalog backends.
- [ ] Install every wheel and run import plus minimal init, log, finish, and
  query smoke tests on primary Linux, macOS, and Windows targets.
- [ ] Verify sdist installation and keep tag publication blocked on all test,
  acceptance, wheel-smoke, and artifact-build jobs.

### Phase 5: Release Candidate Validation

- [ ] Publish `0.1.0rc1` with release-candidate package metadata, classifiers,
  README, release notes, and known limits.
- [ ] Validate fresh stores with DuckDB and SQLite catalogs, local and
  S3-compatible data paths, and both online and offline LTTB paths.
- [ ] Run sustained training, concurrent reads, failure, restart, and terminal
  flush scenarios against packaged artifacts.
- [ ] Freeze the RC surface and accept only release blockers until
  promotion.

### Phase 6: 0.1.0 Promotion

- [ ] Resolve every release blocker found during the RC window and rerun
  the complete automated gate on the final commit.
- [ ] Publish `0.1.0`, replace alpha metadata, document behavior and known
  limits, and verify wheel and sdist installation from published artifacts.

## Later Backlog

### Local Coordination

- [ ] Define and validate multi-client SQLite run-writer coordination before
  expanding the current single-writer native contract.

### Credentials and Remote Training

- [ ] Add environment-variable or AWS credential-chain discovery for S3
  credentials when explicit config-file credentials are insufficient.
- [ ] Evaluate the [remote control-service boundary](drafts/remote-training-architecture-notes.md)
  before adding remote writers or shared catalog coordination.
- [ ] Consider PostgreSQL catalog support only when remote service scale or
  availability requires it.

### Analysis and Agent Workflows

- [ ] Evaluate the [native curve viewer](drafts/gpui-curve-viewer-spike.md)
  without adding plotting dependencies to the Python/Rust SDK.
- [ ] Evaluate the [research driver](drafts/autoresearch-control-loop-notes.md)
  without moving source or Git mutation into PulseOn Core.
- [ ] Design workspace hierarchy, config/tag filtering, export, Web UI, MCP,
  and other agent-facing surfaces as independently reviewable roadmap phases.
