# PulseOn Roadmap

> This roadmap tracks current and future work. Shipped release details live in
> `docs/release-notes/`; durable product boundaries live in
> `docs/native-storage-boundary.md` and accepted ADRs in `docs/adr/`.

## 0.1.0a5 / V5

V5 completes the 0.1.0 alpha line with a shared headless read surface for
trainers and agents. It makes persisted PulseOn data discoverable from projects
through metric points, adds Arrow-compatible Python results and a read-only CLI,
and keeps Web UI, built-in plotting, MCP, arbitrary SQL, and file export out of
the release.

### Phase 1: Read Contract and Discovery

- [x] Add `Client.list_projects()` so callers can begin data discovery without
  a known project identifier.
- [x] Extend run discovery with optional lifecycle-status filtering, stable
  created-time ordering, and `limit`/`offset` pagination while preserving the
  existing `list_runs(project_id)` default behavior.
- [x] Define the read contract as catalog project/run metadata plus persisted
  effective metric points. Queued reports remain outside query visibility.
- [x] Preserve last-write-wins metric semantics and a4 catalog and Parquet
  compatibility for both DuckDB and SQLite catalog backends.

### Phase 2: Fresh Queries and Built-in Downsampling

- [x] Make metric discovery and summaries reflect persisted points for running
  runs, while retaining aggregate-backed terminal-run fast paths.
- [x] Support mixed running and terminal runs in summary comparisons without
  changing requested run ordering.
- [ ] Change metric-query `end_step` semantics from inclusive to exclusive so
  ranges consistently use `[start_step, end_step)`. Update native predicates,
  Python documentation and types, behavioral tests, and migration notes
  together as an explicit compatibility change.
- [ ] Replace the optional runtime-downloaded LTTB extension path with built-in,
  deterministic downsampling. Require `max_points >= 2`, preserve endpoints,
  keep short series unchanged, and apply range and last-write-wins semantics
  before downsampling.

### Phase 3: Arrow-compatible Python Results

- [ ] Preserve the existing Python object-list query APIs and add
  `query_metric_table(...)` and `query_metric_summaries_table(...)` returning an
  Arrow PyCapsule-compatible `ArrowTable`.
- [ ] Expose table row counts, source row counts, downsampling state, column
  names, and `__arrow_c_stream__` without requiring pyarrow, pandas, or Polars
  as runtime dependencies.
- [ ] Keep metric-point columns aligned with the public query model and Parquet
  contract without exposing storage-only `metric_key_encoded`; expose timestamps
  as UTC millisecond Arrow timestamps.
- [ ] Update `python/pulseon/_pulseon.pyi`, package exports, and type-check tests
  for every new public Python class and method.

### Phase 4: Existing-store Configuration and Read-only CLI

- [ ] Extend `<project>/.pulseon/config.toml` with optional `catalog_backend`
  and local-only `catalog_path`. Explicit SDK and CLI values override config,
  and absent values retain the DuckDB defaults.
- [ ] Let `pulseon.init(..., catalog_backend=None)` select the configured
  backend or fall back to DuckDB while preserving no-argument behavior.
- [ ] Resolve relative `data_path` and `catalog_path` values from config against
  the project root, and document the relative-data-path compatibility change.
- [ ] Add a native existing-store open path for the CLI so a missing catalog is
  an error and never creates an empty store.
- [ ] Add a dependency-free `pulseon` console command with `projects list`,
  `runs list`, `metrics list`, `metrics query`, and `metrics compare`.
- [ ] Support global `--path`, `--format table|json`, and explicit non-secret
  backend/path overrides; resolve relative CLI paths against `--path`.
- [ ] Default CLI point queries to 200 points, expose mutually exclusive
  `--max-points` and `--all`, and keep table output deterministic and uncolored.
- [ ] Keep S3 credentials in project config rather than command-line arguments,
  and preserve existing path and credential sanitization in errors.

### Phase 5: Versioned Machine Output and S3 Query Gate

- [ ] Define JSON success output with `schema_version`, `kind`, `data`, `page`,
  and `meta`; include pagination state and metric-query source/downsampling
  metadata where applicable.
- [ ] Write JSON errors to stderr with stable error codes and sanitized messages.
  Reserve exit status 1 for operation failures and 2 for CLI usage failures.
- [ ] Add an opt-in MinIO/S3 metric-query benchmark covering realistic run,
  metric-key, file-count, and step-range selections for both catalog backends.
- [ ] Measure repeated query latency, remote response bytes, and read
  amplification. Treat reads from unrelated `run_id` or
  `metric_key_encoded` partitions as a gate failure; record environment-specific
  latency and amplification as the a5 baseline rather than absolute limits.

### Phase 6: Release Gate

- [ ] Add SDK, CLI, Arrow, local-backend, and opt-in MinIO coverage for the full
  project-to-point discovery path, including running-run freshness, half-open
  ranges, pagination, empty Arrow schemas, structured errors, and missing-store
  behavior.
- [ ] Update README examples, public type documentation, and 0.1.0a5 release
  notes. Include migration notes for half-open ranges, config-relative paths,
  and `max_points < 2`.
- [ ] Remove the public LTTB download/configuration guidance after built-in
  downsampling is verified.
- [ ] Run `cargo fmt --all --check`,
  `cargo clippy --workspace --all-targets --all-features -- -D warnings`,
  `cargo check`, `cargo test`, `uv run maturin develop --uv`,
  `uv run pyright`, `uv run pytest`, the opt-in MinIO gates, and the release
  wheel build.

## Later Backlog

- [ ] Expand SQLite parity to cover multi-client run-writer lock behavior.
- [ ] Consider race-safe run-writer lock-file deletion only if the release path
  can prove it is deleting the original lock file for the released writer.
- [ ] Add environment-variable or AWS credential-chain discovery for S3
  credentials if explicit config-file credentials become too limiting.
- [ ] Add an explicit debug dump or verbose diagnostics facility for local
  troubleshooting, including full path details when the caller opts in.
- [ ] Revisit cloud, workspace hierarchy, config/tag filtering, built-in
  plotting, and AI Native features after the local native metric loop is stable.
