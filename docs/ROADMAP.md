# PulseOn Roadmap

> This roadmap tracks current and future work. Shipped release details live in
> `docs/release-notes/`; durable product boundaries live in
> `docs/native-storage-boundary.md` and accepted ADRs in `docs/adr/`.

## 0.1.0a3 / V3 Shipped

V3/a3 hardened the native local loop before adding object storage or shared
catalogs. Each shipped item remained a small, independently reviewable change
with tests and the smallest relevant verification gate. Existing 0.1.0a2
development stores do not need migration support.

### Phase 1: Public API Defaults

- [x] Make `pulseon.init()` default to the current working directory,
  equivalent to `pulseon.init(".")`, so the default local store remains
  `./.pulseon` without requiring an explicit path argument.
- [x] Keep the rest of the public Python API shape unchanged for V3.
- [x] Update `python/pulseon/__init__.py`, `python/pulseon/_pulseon.pyi`,
  README/examples, and Python-facing tests to cover no-argument initialization,
  explicit custom-root initialization, and unchanged custom `catalog_path` /
  `data_path` behavior.
- [x] Verification gate: `uv run maturin develop --uv`, `uv run pyright`, and
  the Python init/lifecycle tests that cover default and explicit roots.

### Phase 2: Catalog Adapter on DuckDB

- [x] Add a backend-aware catalog adapter while keeping only
  `catalog_backend="duckdb"` enabled. The adapter must provide the DuckLake
  attach statement, default catalog filename, catalog application table
  qualification, and any backend setup needed before table creation.
- [x] Keep backend-specific default catalog filenames, but do not enforce file
  suffixes when the user passes an explicit `catalog_path`.
- [x] Move PulseOn-owned catalog application tables out of DuckLake's internal
  metadata alias for the DuckDB backend. Code must stop hard-coding
  `__ducklake_metadata_dl.pulseon_projects`,
  `__ducklake_metadata_dl.pulseon_runs`, and
  `__ducklake_metadata_dl.pulseon_metric_aggregates`.
- [x] Keep stable PulseOn-owned table names:
  `pulseon_projects`, `pulseon_runs`, and `pulseon_metric_aggregates`. Do not
  rely on schema-qualified names as the portable baseline because SQLite does
  not support PostgreSQL/DuckDB-style schemas inside one database file.
- [x] Store PulseOn catalog application tables in the same catalog database file
  as DuckLake metadata for the DuckDB backend.
- [x] Preserve existing DuckDB behavior for project/run lifecycle, metric writes,
  active and terminal queries, aggregate refresh, terminal flush, custom local
  `data_path`, explicit custom `catalog_path`, and invalid backend/configuration
  errors.
- [x] Verification gate: `cargo check`, `cargo test`, `uv run maturin develop
  --uv`, and the existing Python lifecycle/query/diagnostics tests.

### Phase 3: SQLite Backend Parity

- [x] Promote `catalog_backend="sqlite"` from deferred to supported in V3.
  DuckDB remains the default backend; PostgreSQL remains post-V3.
- [x] Add the SQLite catalog adapter path with a default
  `<project>/.pulseon/catalog.sqlite` catalog file.
- [x] Store PulseOn catalog application tables in the same SQLite catalog
  database file as DuckLake metadata, without addressing them through DuckLake's
  internal metadata alias.
- [x] Add real DuckLake-backed parity tests for DuckDB and SQLite covering:
  client initialization, project/run lifecycle, metric writes, active and
  terminal queries, aggregate refresh, terminal flush, custom local `data_path`,
  explicit custom `catalog_path`, and invalid backend/configuration errors.
- [x] Verify that SQLite stores DuckLake metadata, PulseOn catalog application
  tables, inline metric data, and Parquet data-file references without requiring
  DuckLake internal aliases in PulseOn SQL.
- [x] Verification gate: `cargo check`, `cargo test`, `uv run maturin develop
  --uv`, `uv run pyright`, and `uv run pytest`.

### Phase 4: Metric Data Layout

- [x] Set the DuckLake table-level option
  `data_inlining_row_limit=8192` for `metric_points` to reduce tiny Parquet
  files from short runs and small writer appends.
- [x] Keep `target_file_size` unset in V3 unless local measurements prove the
  default causes a concrete problem. Record any measurement in the relevant test
  or follow-up note rather than adding a public configuration knob.
- [x] Preserve the terminal-run flush contract: `finish_run(...)` and
  `fail_run(...)` drain accepted reports, write terminal lifecycle state, and
  flush inline `metric_points` to Parquet; `flush_run_data(run_id)` remains the
  retry API.
- [x] Add DuckDB and SQLite coverage proving short runs stay inline before
  terminal flush and become Parquet-visible after terminal flush.
- [x] Verification gate: `cargo test`, `uv run maturin develop --uv`, and
  Python lifecycle tests that inspect terminal Parquet visibility.

### Phase 5: Release Gate

- [x] Update README and examples so `pulseon.init()` is the default quickstart
  path and explicit-root initialization remains documented.
- [x] Add release notes that V3 may require a fresh local store instead of
  migrating 0.1.0a2 development stores.
- [x] Run the full local verification set: `cargo check`, `cargo test`,
  `uv run maturin develop --uv`, `uv run pyright`, and `uv run pytest`.

## V4 Backlog

- [ ] Narrow the Python public type hints for `catalog_backend` from `str` to
  `Literal["duckdb", "sqlite"]` in `python/pulseon/__init__.py` and
  `python/pulseon/_pulseon.pyi`, while keeping the runtime API string-compatible.
- [ ] Clean up run-writer lock files only when the release path can prove it is
  deleting the original lock file for the released writer. If that cannot be
  proven safely, leave the file on disk.
- [ ] Preserve the v2 safety contract: process crashes may leave lock files on
  disk; leftover files without a held OS lock must not block resume; cleanup
  must not delete a lock file that another client has recreated or currently
  holds.
- [ ] Add Rust and Python-facing tests for successful terminal finalization,
  shutdown release, leftover stale lock files, and the race-safe "leave it on
  disk when unsure" path.
- [ ] Expand SQLite parity to cover multi-client run-writer lock behavior.

## Post-V3 Backlog

- [ ] Add S3-compatible `data_path` support, including local MinIO. The design
  must cover credentials, DuckDB HTTPFS configuration, path-style vs
  virtual-hosted-style addressing, secret-safe tests, and a MinIO acceptance
  test.
- [ ] Add an explicit debug dump or verbose diagnostics facility for local
  troubleshooting, including full path details when the caller opts in.
- [ ] Revisit cloud, workspace hierarchy, config/tag filtering, built-in
  plotting, and AI Native features after the local native metric loop is stable.
