# ADR 0004: Non-Blocking Metric Reporting

## Status
Accepted.

## Context
`run.log(...)` is called from the training hot path. Slow storage, aggregate
repair, downsampling preparation, or future export work must not make the
training process wait for PulseOn.

Native v1 still depends on DuckLake, but storage durability is not allowed to
turn metric reporting into training-loop backpressure.

## Decision
V1 metric reporting is training-loop non-blocking.

Ordinary metric logging must enqueue or otherwise hand off a metric point and
return without waiting for durable storage flush, aggregate repair, query index
maintenance, downsampling work, or future upload/export work.

V1 should use bounded in-process buffering for native mode. If the reporting
path cannot accept more metrics immediately, PulseOn must prefer losing or
delaying metric data over blocking the training step. Reporting failures and
dropped points must be observable through diagnostics, but ordinary hot-path
logging must not raise by default for transient storage or backpressure
failures.

Run finalization may perform a bounded drain or flush, but it must not hang
indefinitely. Query results are eventually consistent with accepted metric
reports until the writer has drained.

## Consequences
- Metric reporting has weaker durability than the training process itself.
- Tests need to cover slow or blocked storage paths without allowing
  `run.log(...)` to stall indefinitely.
- Diagnostics for dropped or failed metric reports are required before v1 can
  claim reliable local tracking.
- Aggregate freshness remains best-effort; stale aggregate repair must stay off
  the training hot path.
