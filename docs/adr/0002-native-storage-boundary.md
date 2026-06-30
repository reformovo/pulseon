# ADR 0002: Native Storage Boundary

## Status
Accepted.

## Context
PulseOn needs local data ownership and a durable format. V1 validates DuckLake
for metric writes before building custom staging, flush, and compaction.

## Decision
DuckLake is a required v1 dependency in native mode. If the DuckLake extension
cannot load, v1 may fail fast.

The long-term compatibility boundary is the PulseOn-owned Parquet schema, not
DuckLake metadata. DuckLake may be replaced later if validation shows the need
for a custom implementation, but v1 does not implement that replacement.

Native v1 does not expose or require a separate `StorageLayer`. Data writes go
through DuckLake-managed tables and flush into Parquet.

## Consequences
- DuckLake is allowed to shape v1 implementation details.
- Parquet file schema must be documented and treated as stable.
- DuckLake catalog internals are not product API.
- Cloud storage and object-store abstractions are deferred.
