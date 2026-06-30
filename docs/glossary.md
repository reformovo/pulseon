# PulseOn Glossary

This glossary defines the v1 product language. Terms not listed here are not
part of the v1 architecture contract.

## Product Terms
- **Project**: Lightweight namespace for related runs; metadata stays minimal.
- **Run**: One training execution with user-supplied or generated `run_id`.
- **Metric key**: User-facing metric name; path escaping is implementation detail.
- **Metric series**: All points for one `(run_id, metric_key)` pair.
- **Metric point**: One numeric observation in a metric series.
- **Run summary**: Derived per-run values for run lists and comparisons.
- **Metric aggregate**: Materialized-view-like index/state from metric writes.

## Storage Terms
- **PulseOn logical schema**: Product-owned project, run, metric, point, and
  summary schema.
- **Parquet schema**: The long-term compatibility boundary for v1 data.
- **DuckLake**: The required native storage engine for v1. It is a core
  dependency during validation, not a public product protocol.
- **Native mode**: Local-first operation without a separate service process.

## Out Of V1 Scope
- Workspace and organization hierarchy.
- Cloud implementation and Cloud source files.
- Run or metric deletion.
- Agent memory tables such as hypothesis, insight, decision, and report.
- MCP server and tool-calling APIs.
