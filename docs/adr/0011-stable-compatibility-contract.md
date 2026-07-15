# ADR 0011: Stable Compatibility Contract

## Status

Accepted.

PulseOn 0.1.0 establishes four independently versioned compatibility surfaces:
the typed Python API, CLI JSON, catalog application schema, and metric-point
Parquet schema. Keeping their versions independent lets a patch release add a
compatible API or migrate catalog implementation state without needlessly
breaking machine consumers or the open Parquet boundary.

## Decision

During the 0.1.x stable line:

- Existing valid calls through the documented, typed Python API keep their
  signatures, return types, exception classes, and documented behavior.
  Additive classes, methods, and optional keyword parameters are compatible.
- CLI JSON carries its own integer `schema_version`. Existing fields keep their
  types and meanings within one JSON schema version; consumers must tolerate
  additive fields. Removing, renaming, or retyping a field, changing a stable
  `kind` or error code, or changing its meaning requires a new JSON schema
  version.
- Every stable store carries a singleton catalog application schema marker.
  PulseOn may add compatible catalog state or provide explicit migrations, but
  ordinary initialization never rewrites an incompatible or unversioned
  existing store. DuckLake's internal metadata is not a PulseOn compatibility
  surface.
- The metric-point Parquet contract remains the open data boundary documented
  in `docs/parquet-schema-contract.md`. Additive nullable columns are
  compatible; removing, renaming, retyping, or changing identity and
  last-write-wins semantics is breaking. The store marker stays in the catalog
  and is never added to `metric_points`.

The package version does not stand in for any of the three persisted or
machine-readable schema versions. A reader must inspect the version belonging
to the surface it consumes.

## Upgrade Boundary

Stores created by 0.1.0a5 have the intended baseline application and Parquet
shape but no store marker. Because 0.1.0a4 and 0.1.0a5 used the same unversioned
storage shape, PulseOn cannot infer their producing release from catalog
contents. The a5 upgrade therefore requires an explicit source-version
assertion; PulseOn validates the baseline shape before adding the marker.
Other unversioned stores are diagnosed as unsupported rather than guessed to be
a5 or silently rewritten. This is a deliberate refusal to promise direct
0.1.0a1-a4 migration.

## Consequences

Compatibility is judged per surface rather than from package versions alone.
Deprecation and support windows must be documented before stable release, and
every future catalog change must state whether it is readable as-is, requires
an explicit migration, or is unsupported.
