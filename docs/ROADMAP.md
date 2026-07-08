# PulseOn Roadmap

> This roadmap tracks current and future work. Shipped release details live in
> `docs/release-notes/`; durable product boundaries live in
> `docs/native-storage-boundary.md` and accepted ADRs in `docs/adr/`.

## 0.1.0a3 / V3 Backlog

V3/a3 should continue from the current native contract. Each item should remain
a small, independently reviewable change with tests and the smallest relevant
verification gate.

- [ ] Make `pulseon.init()` default to the current working directory,
  equivalent to `pulseon.init(".")`, so the default local store remains
  `./.pulseon` without requiring an explicit path argument.
- [ ] Update `python/pulseon/__init__.py`, `python/pulseon/_pulseon.pyi`,
  README/examples, and Python-facing tests to cover both no-argument
  initialization and explicit custom-root initialization.
- [ ] Set DuckLake table-level options for `metric_points` to reduce tiny
  Parquet files from short runs or small writer appends. Start by raising
  `data_inlining_row_limit` above DuckLake's default 10 rows, with an initial
  target near the current writer batch size (`8192` rows), so tens/hundreds of
  rows stay inline until terminal-run flush. Evaluate whether
  `target_file_size` also needs a PulseOn default after measuring local
  examples.
- [ ] Move PulseOn-owned catalog tables out of DuckLake's internal metadata
  namespace while keeping the layout portable across DuckDB, SQLite, and future
  PostgreSQL catalog backends. Do not rely on schemas as the portable baseline:
  SQLite does not support PostgreSQL/DuckDB-style schemas inside one database
  file. Use an explicit PulseOn-owned namespace strategy, such as stable
  `pulseon_*` table names or a backend-specific schema adapter, and include
  full SQLite support and parity tests in the a3 acceptance gate.
- [ ] Clean up run-writer lock files after releasing the OS advisory lock when
  it is safe to do so, so successful terminal runs and shutdowns do not leave
  confusing empty `.pulseon/locks/runs/*.lock` files behind. Preserve the v2
  safety contract: process crashes may still leave lock files on disk,
  leftover files without a held OS lock must not block resume, and cleanup must
  not delete a lock file that another client has recreated or currently holds.

## Post-V3 Backlog

- [ ] Add S3-compatible `data_path` support, including local MinIO. The design
  must cover credentials, DuckDB HTTPFS configuration, path-style vs
  virtual-hosted-style addressing, secret-safe tests, and a MinIO acceptance
  test.
- [ ] Add an explicit debug dump or verbose diagnostics facility for local
  troubleshooting, including full path details when the caller opts in.
- [ ] Revisit cloud, workspace hierarchy, config/tag filtering, built-in
  plotting, and AI Native features after the local native metric loop is stable.
