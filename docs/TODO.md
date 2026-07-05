# PulseOn Roadmap

> Current source of truth: `docs/catalog-data-boundary.md` and ADRs in
> `docs/adr/`. `docs/v1-native-architecture.md` remains historical context for
> the completed v1 native loop.

## 0.1.0a2 / V2 Roadmap

Source: `docs/catalog-data-boundary.md`, ADR 0005, ADR 0006, and ADR 0007.

V2 turns v1's best-effort in-memory metric buffering into an explicit async
high-throughput reporting model. The target is 100k individual Python
`run.log(...)` calls per second from one Python thread for explicit-step metric
reports. A returned `run.log(...)` call means the report entered the bounded
in-process metric queue; it does not mean the report is durably accepted or
query-visible.

### V2 Implementation Phases

Each phase should be independently reviewable and should keep the public
surface, Rust engine behavior, Python typing, and tests consistent before
moving on. DuckDB is the first storage validation target; SQLite is not complete
until the same DuckLake-backed behavior tests pass against it.

#### Phase 0: Contract Lock

- [ ] Convert the v2 roadmap into testable issue-sized work without changing
  runtime behavior.
- [ ] Add or update test names/fixtures that will own the v2 API, diagnostics,
  queue, writer, storage-layout, locking, finalization, and flush contracts.
- [ ] Keep `docs/catalog-data-boundary.md`, ADR 0005, ADR 0006, ADR 0007,
  `docs/glossary.md`, and this roadmap in sync as terms are sharpened.

Exit gate: no product behavior changes yet; `cargo test`, `uv run pyright`, and
`uv run pytest` still pass on the v1 implementation.

#### Phase 1: Public API Break And Configuration

- [x] Remove auto-step logging from the Python API and require
  `run.log(key, step, value)`.
- [x] Add the v2 error hierarchy and map Rust/PyO3 failures to Python-visible
  exception types.
- [x] Add keyword initialization for `data_path`, `catalog_backend`,
  `catalog_path`, and `metric_queue_capacity`.
- [x] Validate queue capacity, catalog backend names, and unsupported data paths
  before client startup.
- [x] Update `python/pulseon/_pulseon.pyi`, README examples, and Python-facing
  tests with the v2 API shape.

Exit gate: Python users see the intended v2 API and error classes, while storage
and writer internals may still use the existing v1 implementation underneath.

#### Phase 2: Bounded Metric Queue And Diagnostics

- [x] Replace v1 dropped-report semantics with a bounded metric queue and
  non-fatal `MetricQueueFullError`.
- [x] Implement the v2 diagnostics object with read-only fields for pending
  reports, queue-full failures, persisted reports, writer state, last write
  error, and flush status.
- [x] Remove v1 `dropped_reports` terminology from Python diagnostics.
- [x] Keep queries reading persisted DuckLake data only; do not merge queued
  reports into query results.

Exit gate: tests prove queue capacity, queue-full recovery, diagnostics field
shape, post-shutdown diagnostics readability, and absence of silent drops.

#### Phase 3: Batch Writer, Drain, And Shutdown

- [x] Implement 8,192-report or 10 ms batch thresholds with enqueue-order
  preservation and writer-assigned `ingested_at`.
- [x] Retry writer persistence failures five times with bounded exponential
  backoff before entering failed writer state.
- [x] Implement drain barriers for shutdown and writer failure semantics,
  including `MetricDrainTimeoutError`, `MetricWriterFailedError`, and
  context-manager teardown precedence.
- [x] Keep aggregate refresh off the `run.log(...)` hot path.

Exit gate: Rust and Python tests cover writer state transitions, retry
exhaustion, drain timeouts, failed-client shutdown, and context-manager
exception priority.

#### Phase 4: Run Admission, Locks, And Finalization

- [x] Add one active writer client per run using OS advisory run-writer locks.
- [x] Make `create_run(...)` distinct from `resume_run(...)` for existing run
  IDs and terminal runs.
- [ ] Close per-run admission before finalization drain so late `run.log(...)`
  calls raise `RunClosedError`.
- [ ] Keep shutdown as client teardown, not run finalization.

Exit gate: tests prove active-run lock conflicts, crash-leftover lock-file
behavior where practical, terminal-run resume rejection, finalization/logging
races, and lock release after terminal state is written.

#### Phase 5: Catalog/Data Boundary And Parquet Flush

- [ ] Move v2 application tables to catalog-owned `pulseon_projects`,
  `pulseon_runs`, and `pulseon_metric_aggregates`.
- [ ] Support custom local `data_path` and DuckDB catalog defaults first.
- [ ] Add `metric_key_encoded` and partition flushed `metric_points` Parquet by
  `run_id` and `metric_key_encoded`.
- [ ] Implement terminal-run Parquet flush, `flush_run_data(run_id)`, flush
  timeout, idempotent retry, and runtime-only flush diagnostics.
- [ ] Validate SQLite only after the same real DuckLake-backed storage tests
  pass; otherwise explicitly defer it before v2 completion.

Exit gate: tests inspect catalog tables, logical query behavior, partitioned
Parquet layout, custom local data paths, flush failure/timeout diagnostics, and
DuckDB-backed recovery paths.

#### Phase 6: Performance And Release Hardening

- [ ] Add a reproducible local benchmark command for explicit-step
  `run.log(...)` throughput.
- [ ] Measure the actual `MetricReport` memory footprint and update queue
  capacity planning.
- [ ] Prove the 100k calls-per-second one-Python-thread target on a recorded
  local environment.
- [ ] Re-run full Rust/Python verification and update release notes before
  cutting `0.1.0a2`.

Exit gate: correctness gates pass, the benchmark result and environment are
recorded, and any deferred SQLite or S3-compatible storage scope is explicit in
the roadmap.

### V2 Metric Reporting Contract

- [ ] Replace silent drop-on-full behavior with a Python-visible
  `MetricQueueFullError`.
- [ ] Define the v2 error hierarchy: `PulseOnError`,
  `MetricQueueFullError`, `MetricWriterFailedError`,
  `MetricDrainTimeoutError`, `MetricFlushError`, `MetricFlushTimeoutError`,
  `RunClosedError`, `ClientClosedError`, `InvalidRunStateError`,
  `RunAlreadyExistsError`, `RunAlreadyActiveError`,
  `InvalidConfigurationError`, and `StorageError`.
- [ ] Make every v2 Python exception subclass `PulseOnError`; v2 tests should
  assert exception type and useful message text, but should not freeze stable
  exception attribute names or field types.
- [ ] Keep `MetricQueueFullError` non-fatal; later `run.log(...)` calls may
  succeed after the queue drains.
- [ ] Default the metric queue capacity to 65,536 reports.
- [ ] Allow queue capacity to be configured at client initialization.
- [ ] Reject `metric_queue_capacity=0` and values above 1,048,576 with
  `InvalidConfigurationError`.
- [ ] Bound the queue; do not grow memory without limit when DuckLake or
  durable admission lags.
- [ ] Report minimal diagnostics for pending reports, queue-full failures,
  persisted metric points, writer state, and last write errors.
- [ ] Replace v1 `dropped_reports` diagnostics; do not keep dropped-report
  terminology in the v2 Python diagnostics shape.
- [ ] Keep `client.diagnostics()` as a public read-only runtime snapshot for
  users, tests, and operational debugging. It is not a query API, not durable
  history, and not a recovery protocol.
- [ ] Allow `client.diagnostics()` after `shutdown()`; it returns the last
  runtime snapshot, while write paths through that client continue to raise
  `ClientClosedError`.
- [ ] Do not promise cross-field atomic consistency for diagnostics. V2 only
  promises safe reads and documented field types; callers that need drain or
  finalization guarantees must use explicit APIs rather than inferring them
  from multiple diagnostics fields.
- [ ] Keep v2 diagnostics minimal: do not add per-run diagnostics APIs,
  diagnostics event history, telemetry export, stable exception/diagnostic
  attribute ABI, or durable diagnostic tables.
- [ ] Expose diagnostics fields for `pending_reports`, `queue_full_errors`,
  `persisted_reports`, `writer_state`, `last_write_error`,
  `last_flush_run_id`, `last_flush_status`, and `last_flush_error`.
- [ ] Do not expose `queued_reports`, `accepted_reports`, `delayed_reports`,
  `failed_reports`, or `enqueued_reports` in v2 diagnostics.
- [ ] Treat `last_write_error` as a sanitized human-readable string or `None`;
  do not expose structured error fields for it in v2.
- [ ] Restrict `writer_state` values to `running`, `retrying`, `failed`,
  `drained`, and `closed`. `drained` means no reports are currently pending; it
  does not mean the client is shut down.
- [ ] Move `writer_state` from `drained` back to `running` when a new report is
  enqueued. Enter `closed` only after `shutdown()` drains successfully and
  stops the writer; shutdown timeout must not report `closed`.
- [ ] Treat `pending_reports` as a current value. Treat `queue_full_errors` and
  `persisted_reports` as cumulative counts over the current client lifetime.
- [ ] Define `pending_reports` as reports returned from `run.log(...)` in the
  current client but not yet persisted, including reports still in the queue
  plus reports in the writer's active or retrying batch.
- [ ] Use `pending_reports` as the only exposed pending-work count; do not
  expose queue internals through a separate `queued_reports` field.
- [ ] Keep `persisted_reports` because it is the minimal cumulative signal that
  the async writer is making durable progress.
- [ ] Keep `queue_full_errors` because it is the minimal cumulative signal that
  queue capacity or writer throughput is insufficient.
- [ ] Allow `writer_state=failed` with `pending_reports > 0`; those reports
  have not been durably admitted, and callers must inspect the raised
  `MetricWriterFailedError` and `last_write_error`.
- [ ] Treat all diagnostics report counters and `writer_state` as runtime-only
  current-client state. They are not catalog state, not durable history, and
  are not restored when another process opens the same project.
- [ ] Count a report as persisted once the DuckLake append succeeds, even if
  aggregate refresh later fails or has not run.
- [ ] Accept that runtime diagnostics may undercount if the process crashes
  after a successful DuckLake append but before diagnostics are updated;
  persisted data remains authoritative.
- [ ] Treat `last_flush_run_id`, `last_flush_status`, and `last_flush_error`
  as runtime-only client diagnostics for the current process's most recent
  terminal-run flush attempt, not catalog state. Allowed `last_flush_status`
  values are `none`, `running`, `succeeded`, `failed`, and `timed_out`;
  `last_flush_run_id` and `last_flush_error` are strings or `None`.
- [ ] Initialize `last_write_error` to `None`; initialize flush diagnostics to
  `last_flush_status=none`, `last_flush_run_id=None`, and
  `last_flush_error=None`.
- [ ] Keep `last_flush_run_id` so `last_flush_status` and `last_flush_error`
  have clear ownership in multi-run clients.
- [ ] Keep `last_flush_status`; do not collapse it into `last_flush_error`
  string parsing.
- [ ] Do not clear `last_flush_error` after a later successful flush. A
  successful flush updates `last_flush_status=succeeded` and
  `last_flush_run_id=<run_id>`; `last_flush_error` remains the most recent
  flush error for the current client lifetime.
- [ ] Document `last_flush_error` as the most recent flush error, not the
  current flush state; callers must use `last_flush_status` for current flush
  state.
- [ ] When finalization writes terminal run state but Parquet flush fails,
  raise `MetricFlushError` and update diagnostics to
  `last_flush_run_id=<run_id>` and `last_flush_status=failed`.
- [ ] When `flush_run_data(run_id, timeout=...)` times out, leave the terminal
  run unchanged, update diagnostics to `last_flush_run_id=<run_id>` and
  `last_flush_status=timed_out`, and allow a later retry.
- [ ] Keep ordinary diagnostics, including `last_write_error`, sanitized:
  include failed operation and basename when useful, but do not expose full
  local paths.
- [ ] Do not clear `last_write_error` after a later successful batch; it remains
  the most recent write error for the current client lifetime.
- [ ] While `writer_state=retrying`, update `last_write_error` to the most
  recent retry error.
- [ ] Do not add `clear_diagnostics()` in v2. Users who need a clean diagnostic
  slate must create a new client.
- [ ] Do not expose `debug_diagnostics()`, `verbose=True`, or another public
  debug-dump API in v2 Phase A.
- [ ] Return diagnostics as a typed object with documented attributes, not a
  dict. Keep field names fixed for v2, but do not freeze a broader diagnostic
  attribute ABI.
- [ ] Make diagnostics object attributes read-only from Python.
- [ ] Require `pending_reports == 0` when `writer_state=closed`; only successful
  shutdown drain may enter `closed`.
- [ ] If `shutdown()` is called after writer failure, do not attempt writer
  recovery, do not change diagnostics to `closed`, release resources, and raise
  `MetricWriterFailedError`.
- [ ] Keep `client.diagnostics()` readable after writer failure and after
  failed shutdown; it returns the final runtime snapshot.
- [ ] Treat `Run` handles owned by a failed writer client as permanently
  unwritable. Later `run.log(...)` raises `MetricWriterFailedError` before
  shutdown and `ClientClosedError` after shutdown releases resources.
- [ ] Repeated `shutdown()` after writer failure must keep raising
  `MetricWriterFailedError`; it is not idempotent success.
- [ ] Preserve `pending_reports` at the failure-time value after
  `writer_state=failed`; do not decrement it as a compensation step.
- [ ] Keep run-writer locks held after writer failure until the user explicitly
  calls `shutdown()` or the process exits; do not auto-release locks on writer
  failure.
- [ ] After failed-client shutdown releases resources, allow a new client to
  `resume_run(run_id)` if the run is still non-terminal and the writer lock can
  be acquired. Reports left in the failed client's `pending_reports` were not
  durably admitted and are lost.
- [ ] Keep full local paths out of ordinary diagnostics; reserve them for a
  future explicit debug/verbose facility or internal chained/source errors.
- [ ] Do not expose `accepted_reports` in v2 Phase A. Because DuckLake is the
  first durable boundary, `persisted_reports` is also the durable-admission
  count for this phase.
- [ ] Treat accepted reports as recoverable durable admission, not in-process
  queue admission. In v2 Phase A, accepted reports are the same reports counted
  as persisted metric points because DuckLake is the first durable boundary.
- [ ] Do not implement a separate admission log in v2 Phase A.
- [ ] Accept that reports returned from `run.log(...)` but not yet durably
  admitted may be lost if the process exits.
- [ ] Keep metric queries reading persisted DuckLake data only; do not merge
  queued in-memory reports into query results.
- [ ] Drain queued reports in background batches using initial thresholds of
  8,192 reports or 10 ms, whichever comes first.
- [ ] Retry background writer failures up to five times with exponential
  backoff from 50 ms to at most 1,000 ms.
- [ ] Enter a non-recoverable failed writer state after retry exhaustion; later
  `run.log(...)` calls should raise `MetricWriterFailedError` until the user
  shuts down and reinitializes the client.
- [ ] Treat background writer persistence failures separately from direct API
  filesystem failures: retry-exhausted writer persistence failures put the
  client into failed writer state and later `run.log(...)` raises
  `MetricWriterFailedError`.
- [ ] Preserve enqueue order within one client process. Multi-threaded calls
  are allowed, but cross-thread ordering is the order reports enter the Rust
  metric queue.
- [ ] Keep the 100k calls-per-second target scoped to one Python thread.
- [ ] Keep aggregate refresh off the `run.log(...)` hot path; update dirty
  aggregate state asynchronously.
- [ ] Make `shutdown()` and context-manager exit wait for drain by default when
  no timeout is supplied; bounded callers must pass an explicit timeout.
- [ ] Make unbounded shutdown return only after drain completes or writer
  failure; writer failure raises `MetricWriterFailedError`.
- [ ] Make bounded shutdown timeout raise `MetricDrainTimeoutError`.
- [ ] Document and test that callers should stop active logging before
  shutdown; v2 does not guarantee bounded shutdown can complete while other
  threads keep admitting new reports.
- [ ] Make context-manager exit follow shutdown resource-release semantics:
  release locks/resources on writer failure and raise `MetricWriterFailedError`
  when no user exception is already active.
- [ ] If a user exception is already active while exiting the context manager,
  do not mask it with `MetricWriterFailedError` or `MetricDrainTimeoutError`;
  user exceptions take priority. Attach the teardown error as exception context
  when practical.
- [ ] Even when a user exception is already active, context-manager exit must
  still perform best-effort resource release. Writer failure during that exit
  releases locks/resources before preserving the user exception.
- [ ] Do not add a new logging dependency for teardown errors. Prefer Python
  exception context; if chaining is too complex for v2, leave diagnostics as
  the observable teardown state.
- [ ] Treat drain timeout during context-manager exit as incomplete shutdown:
  client-wide admission remains open, the run-writer lock stays held, and the
  client remains usable if the caller still has a reference.
- [ ] If context-manager exit times out while draining and no user exception is
  active, raise `MetricDrainTimeoutError`.
- [ ] If context-manager exit times out while draining and a user exception is
  active, keep the user exception primary and expose the timeout through
  diagnostics or exception context when practical.
- [ ] After a context-manager drain timeout, allow the caller to continue using
  existing `Run` handles and retry `shutdown(timeout=None)`,
  `finish_run(...)`, or `fail_run(...)`.
- [ ] Keep `shutdown()` as client teardown, not run finalization. It must not
  change run lifecycle state.
- [ ] Make bounded shutdown drain first while client-wide metric admission
  remains open. If it times out, shutdown did not complete and the client
  remains usable.
- [ ] After shutdown drain succeeds, atomically close client-wide metric
  admission, stop the writer, release locks/resources, and mark the client
  shut down.
- [ ] Make `run.log(...)` through a shut-down client raise
  `ClientClosedError`, even if the underlying run is still running and can be
  resumed from another client.
- [ ] Treat `Run` handles from a shut-down client as permanently unusable;
  users must create a new client and explicitly resume the run to get a new
  handle.
- [ ] Allow context-manager exit to leave running runs as running/orphaned;
  users may later resume, finish, or fail those runs explicitly.
- [ ] Make `resume_run(run_id)` reject terminal runs with
  `InvalidRunStateError`; terminal runs are queryable through client query APIs
  but do not produce writable `Run` handles.
- [ ] Keep `create_run(user_supplied_run_id)` distinct from resume: if the run
  already exists, raise `RunAlreadyExistsError` and require explicit
  `resume_run(...)` even when no writer lock is active.
- [ ] Include the conflicting `run_id` in the `RunAlreadyExistsError` message.
- [ ] Support only one active writer client per run in v2.
- [ ] Acquire the local run-writer lock for both `create_run(...)` and
  `resume_run(...)` before returning a writable `Run` handle.
- [ ] Implement the local run-writer lock with an OS advisory file lock; do not
  build a lock table, lease protocol, heartbeat, or stale-lock cleanup system
  for v2.
- [ ] Store run-writer lock files under
  `<project>/.pulseon/locks/runs/<percent-encoded-run-id>.lock`; lock files are
  local runtime state and are not catalog tables.
- [ ] Raise `StorageError`, not `InvalidConfigurationError`, for local
  filesystem I/O failures affecting catalog paths, data paths, lock
  directories, or lock files.
- [ ] Keep ordinary `StorageError` messages focused on the failed operation and
  basename; do not expose full local paths by default.
- [ ] Allow initialization `StorageError` debugging through chained/source
  errors or a future explicit debug/verbose facility, not through ordinary
  messages.
- [ ] Encode lock-file `run_id` path segments with the same RFC 3986
  percent-encoding rule used by `metric_key_encoded`.
- [ ] Treat an existing lock file without a held OS lock as not active; process
  crash should release the OS lock even if the file remains on disk.
- [ ] Acquire the local run-writer lock with try-lock semantics; do not block
  waiting for another active writer.
- [ ] If another client already holds the writer lock, `create_run(...)` or
  `resume_run(...)` immediately raises non-fatal `RunAlreadyActiveError`;
  callers may retry later.
- [ ] Include the conflicting `run_id` in the `RunAlreadyActiveError` message.
- [ ] Do not expose the local run-writer lock path in `RunAlreadyActiveError`
  by default.
- [ ] Make `finish_run(...)` and `fail_run(...)` drain queued reports before
  writing terminal run state.
- [ ] Make `finish_run(...)` and `fail_run(...)` reject already terminal runs
  with `InvalidRunStateError`, including repeated calls with the same target
  terminal state.
- [ ] Close metric admission for the run before finalization drain; later
  `run.log(...)` calls for that run must raise `RunClosedError`.
- [ ] Resolve finalization/logging races by Rust admission gate order, not
  Python call start time. Reports admitted before the close barrier drain;
  reports reaching admission after the barrier raise `RunClosedError`.
- [ ] Use an internal global enqueue sequence for drain barriers and writer
  ordering; do not expose it in the public schema.
- [ ] Guarantee that normal finalization starts Parquet flush only after all
  reports queued before the close barrier are persisted.
- [ ] Make unbounded finalization wait until drain completes or writer failure.
- [ ] If the writer is already failed, make `finish_run(...)` and
  `fail_run(...)` raise `MetricWriterFailedError`; do not write terminal run
  state and do not flush.
- [ ] Make bounded finalization timeout raise `MetricDrainTimeoutError` without
  writing terminal run state.
- [ ] Keep the run-writer lock after finalization drain timeout because the run
  remains writable by the current client.
- [ ] Release the run-writer lock once terminal lifecycle state is written,
  even if the later Parquet flush raises `MetricFlushError`.

### V2 Step Semantics

- [ ] Remove automatic step assignment from the public v2 API.
- [ ] Require explicit `step` for the 100k calls-per-second benchmark.
- [ ] Delete `run.log(key, value)` instead of keeping a compatibility slow path.
- [ ] Keep only `run.log(key: str, step: int, value: float) -> None`.
- [ ] Generate `ingested_at` in the batch writer rather than the Python hot
  path.
- [ ] Make writer-assigned `ingested_at` strictly increasing in enqueue order,
  including reports in the same batch, so last-write-wins does not need a
  separate public `ingest_seq` column.
- [ ] Update Python typing and docs so explicit step is required.
- [ ] Do not require migration compatibility with earlier development datasets.
- [ ] Do not add a schema/version marker solely for v2 planning.

### V2 Storage Boundary

- [ ] Support custom Parquet `data_path` independently from the catalog path.
- [ ] Publicly support only local filesystem `data_path` values in v2.
- [ ] Reject S3-compatible `data_path` values in v2 with
  `InvalidConfigurationError`.
- [ ] Support DuckDB and SQLite as configurable native DuckLake catalog
  backends.
- [ ] Treat DuckDB as the first validation target. SQLite is complete only if
  real DuckLake-backed tests prove the same schema, transaction, locking, and
  multi-client behavior.
- [ ] If SQLite cannot satisfy the v2 storage contract without special
  compatibility behavior, explicitly block v2 or defer SQLite support rather
  than presenting fake backend compatibility.
- [ ] Store project, run, and aggregate application state in catalog tables
  named `pulseon_projects`, `pulseon_runs`, and `pulseon_metric_aggregates`
  rather than DuckLake logical tables.
- [ ] Select `catalog_backend` explicitly instead of inferring it from
  `catalog_path` suffix.
- [ ] Accept only case-sensitive `catalog_backend` values `"duckdb"` and
  `"sqlite"`; unknown values raise `InvalidConfigurationError`.
- [ ] Keep PostgreSQL catalog support out of the v2 public surface.
- [ ] Add a keyword-based init shape covering `data_path`, `catalog_backend`,
  `catalog_path`, and `metric_queue_capacity`.
- [ ] Use default paths of `<project>/.pulseon/catalog.ducklake` for DuckDB,
  `<project>/.pulseon/catalog.sqlite` for SQLite, and
  `<project>/.pulseon/data` for Parquet data.
- [ ] Partition flushed `metric_points` Parquet by `run_id` and
  `metric_key_encoded`.
- [ ] Add `metric_key_encoded` to `metric_points`; keep raw user-facing
  `metric_key` unencoded.
- [ ] Treat `metric_key_encoded` as a public Parquet schema and partition
  column.
- [ ] Use RFC 3986 percent-encoding for `metric_key_encoded`: preserve
  `[A-Za-z0-9._~-]` and encode all other UTF-8 bytes as uppercase `%XX`.
- [ ] Do not denormalize `project_id` into the v2 metric point fact table.
- [ ] Do not store project metadata in the data path; project-scoped query and
  export logic must use catalog `pulseon_runs(project_id)` metadata.
- [ ] Do not partition metric point Parquet by `step`; allow DuckLake to create
  multiple files within the same run/key partition.
- [ ] Build or refresh `metric_aggregates` after run finalization instead of
  maintaining aggregate freshness during active metric reporting.
- [ ] Keep active-run metric discovery/query freshness based on persisted
  DuckLake `metric_points`, not queued in-memory reports.
- [ ] Flush inline `metric_points` data to Parquet when runs reach a terminal
  state, with failed flush operations surfaced in runtime-only diagnostics.
- [ ] If Parquet flush fails after terminal lifecycle state is written, raise
  `MetricFlushError` and do not roll back the terminal run state.
- [ ] Keep ordinary `MetricFlushError` messages focused on the failed operation
  and basename; do not expose full local paths by default.
- [ ] Add `flush_run_data(run_id)` to retry Parquet flush for a terminal run
  without changing run lifecycle state.
- [ ] Make `flush_run_data(run_id)` idempotent when terminal-run data is
  already Parquet-visible.
- [ ] Use idempotent `flush_run_data(run_id)` as restart-time recovery if the
  process crashes after Parquet flush succeeds but before the caller observes
  success; do not add durable Parquet visibility state for this case.
- [ ] Make `flush_run_data(run_id)` return `None` on success; failure and
  timeout are reported only through exceptions.
- [ ] Support an optional timeout for `flush_run_data(run_id)`; without a
  timeout, wait until flush succeeds or fails.
- [ ] Make `flush_run_data(run_id)` timeout raise `MetricFlushTimeoutError`.
- [ ] Make `flush_run_data(run_id)` reject non-terminal runs with
  `InvalidRunStateError`.
- [ ] Serialize terminal-run flush work inside one client with a client-wide
  flush mutex. Concurrent `flush_run_data(...)` calls wait for that mutex; if a
  timeout is supplied, waiting for the mutex counts against the timeout.
- [ ] Do not use repeated `finish_run(...)` or `fail_run(...)` calls as Parquet
  flush retry; closed or terminal runs should use `flush_run_data(run_id)`.
- [ ] Do not add a durable `run_storage_state` or Parquet visibility catalog
  table in v2.
- [ ] Ensure normal finalization drains queued reports for the run before
  flushing inline metric data; do not flush if drain fails or times out.
- [ ] Accept that shutdown can leave running-run metric data persisted in
  DuckLake but not forced Parquet-visible; only terminal runs require Parquet
  visibility in v2.
- [ ] Keep Parquet as the v2 open compatibility boundary even if future
  ClickHouse support augments or replaces the serving/query backend.
- [ ] Do not enforce physical uniqueness for `metric_points`; keep logical
  last-write-wins semantics for duplicate `(run_id, metric_key, step)` rows.

### Queue Capacity Planning

These estimates are planning guardrails. The implementation must measure the
actual `MetricReport` footprint and update the table before declaring the v2
performance target met.

| Queue capacity | Approx burst at 100k/s | Estimated memory range |
| --- | ---: | ---: |
| 16,384 reports | 164 ms | 3-8 MiB |
| 65,536 reports | 655 ms | 12-32 MiB |
| 262,144 reports | 2.6 s | 48-128 MiB |
| 1,048,576 reports | 10.5 s | 192-512 MiB |

Acceptance: v2 exposes explicit queue-full failures instead of silent loss,
keeps `run.log(...)` hot-path work bounded, proves the explicit-step 100k/s
single-thread Python benchmark, and preserves the catalog/data boundary without
making queued reports look durably accepted.

### V2 Benchmark Gate

The explicit-step 100k calls-per-second target is a local benchmark gate, not a
CI gate. The implementation must add a reproducible benchmark command and
record the benchmark environment and result before v2 is considered complete.
The benchmark command should live under `scripts/`, for example
`uv run python scripts/bench_log_throughput.py`. CI should keep correctness
tests for queue behavior, writer state, storage layout, and API errors, but it
should not fail based on timing-sensitive throughput numbers.

## Post-V2 Backlog

- [ ] Add S3-compatible `data_path` support, including local MinIO. The design
  must cover credentials, DuckDB HTTPFS configuration, path-style vs
  virtual-hosted-style addressing, secret-safe tests, and a MinIO acceptance
  test.
- [ ] Add an explicit debug dump or verbose diagnostics facility for local
  troubleshooting, including full path details when the caller opts in.
