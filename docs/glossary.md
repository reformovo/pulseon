# PulseOn Glossary

This glossary defines the native product language. Version-specific
architecture documents define which terms are contractually required for each
release.

## Product Terms
- **Project**: Lightweight namespace for related runs; metadata stays minimal.
- **Run**: One training execution with user-supplied or generated `run_id`.
- **Metric key**: User-facing metric name; path escaping is storage detail.
- **Metric series**: All points for one `(run_id, metric_key)` pair.
- **Metric point**: One numeric observation in a metric series.
- **Metric reporting**: The hot-path handoff from training code to PulseOn.
  Reporting must not block training progress.
- **Queued report**: A metric report received by the hot-path API but not yet
  durably admitted. Queued reports may be lost if the process exits before
  admission.
- **Metric queue**: The bounded in-process handoff used by the hot-path metric
  reporting API. A full metric queue is an admission failure, not a silent drop.
- **Accepted report**: A metric report that PulseOn has durably admitted and
  can recover after process restart. Admission to an in-process buffer alone is
  not acceptance.
- **Persisted metric point**: A metric point that has been written to the
  native storage engine and is visible to PulseOn queries under effective-series
  semantics.
- **Run summary**: Derived per-run values for run lists and comparisons.
- **Metric aggregate**: Materialized-view-like index/state from metric writes.
- **Reporting diagnostics**: Minimal observable state for pending reports,
  queue-full failures, persisted reports, writer state, and last errors.
  Diagnostics are a runtime snapshot, not durable history, telemetry, or query
  results.
- **Flush diagnostics**: Runtime-only client diagnostics for the current
  process's latest terminal-run data flush attempt. They are not catalog state
  and are not recoverable after process restart.

## Storage Terms
- **PulseOn logical schema**: Product-owned project, run, metric, point, and
  summary schema.
- **Parquet schema**: The open compatibility boundary for native metric data.
- **Catalog backend**: The database engine DuckLake uses for metadata in native
  mode, such as DuckDB or SQLite.
- **Data path**: The local filesystem location where DuckLake writes Parquet
  data files.
- **Catalog path**: The local path or connection target used for DuckLake
  catalog metadata.
- **DuckLake**: The required native storage engine for v2. It is a core
  dependency during validation, not a public product protocol.
- **Native mode**: Local-first operation without a separate service process.

## Out Of Current Native Scope
- Workspace and organization hierarchy.
- Cloud implementation and Cloud source files.
- Run or metric deletion.
- Agent memory tables such as hypothesis, insight, decision, and report.
- MCP server and tool-calling APIs.
