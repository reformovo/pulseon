# PulseOn Roadmap

> This roadmap tracks current and future work. Shipped release details live in
> `docs/release-notes/`; durable product boundaries live in
> `docs/native-storage-boundary.md` and accepted ADRs in `docs/adr/`.

## Shipped

- [x] 0.1.0a3 / V3: default `pulseon.init()`, portable catalog adapters,
  DuckDB and SQLite catalog backends, catalog application tables outside
  DuckLake internal aliases, terminal-run Parquet flush, and V3 release notes.
- [x] 0.1.0a4 / V4: S3-compatible data paths for the DuckLake data area,
  MinIO acceptance coverage, local-only catalog paths, narrowed
  catalog-backend type hints, and V4 release notes.

## 0.1.0a4 / V4

V4 adds S3-compatible object-storage support for the DuckLake data area while
keeping the catalog database local. It also narrows the Python type surface for
supported catalog backends. SQLite multi-client lock parity and lock-file
cleanup are deferred unless the V4 scope is explicitly expanded.

### Phase 1: S3 Configuration

- [x] Support `data_path = "s3://bucket/prefix"` through
  `<project>/.pulseon/config.toml` and the existing `data_path` keyword.
- [x] Keep `catalog_path` local-only; `catalog_path = "s3://..."` remains
  `InvalidConfigurationError`.
- [x] Use TOML config for S3 credentials and connection settings. Explicit
  `pulseon.init(...)` keywords override config-file values.
- [x] Move `<project>/.pulseon/config.toml` reading, TOML parsing, and
  config-file/explicit-keyword merge rules into the Rust/PyO3 layer so Python
  remains a thin API facade and native storage configuration has one owner.

### Phase 2: DuckDB HTTPFS Setup

- [x] Configure DuckDB HTTPFS/S3 only for S3 data paths, using connection-local
  secrets that are never persisted into DuckLake or PulseOn catalog tables.

### Phase 3: S3 Storage Behavior

- [x] Preserve local data-path behavior at the PulseOn API level for writes,
  queries, terminal flush, and `flush_run_data(run_id)`.
- [x] Support S3-backed `data_path` with both DuckDB and SQLite catalog
  backends.

### Phase 4: MinIO Acceptance

- [x] Add opt-in MinIO acceptance coverage for DuckDB and SQLite catalog
  backends.
- [x] Cover initialization, metric writes, query visibility, terminal
  finalization, terminal Parquet visibility, and `flush_run_data(run_id)`.
- [x] Keep acceptance credentials out of source control.

### Phase 5: Python Type Surface

- [x] Narrow the Python public type hints for `catalog_backend` from `str` to
  `Literal["duckdb", "sqlite"]` in `python/pulseon/__init__.py` and
  `python/pulseon/_pulseon.pyi`.
- [x] Keep the runtime API string-compatible: unsupported strings still fail at
  runtime through the existing validation path.
- [x] Add or update Python type-check coverage that proves `"duckdb"` and
  `"sqlite"` are accepted by type checkers and unknown literals are rejected.

### Phase 6: Release Gate

- [x] Update README/examples only where needed to show `.pulseon/config.toml`
  and S3 `data_path` usage without exposing secrets.
- [x] Add release notes for 0.1.0a4 covering S3-compatible data paths, MinIO
  acceptance coverage, local-only catalog paths, and narrowed catalog-backend
  type hints.
- [x] Run the full local verification set: `cargo check`, `cargo test`,
  `uv run maturin develop --uv`, `uv run pyright`, and `uv run pytest`.

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
