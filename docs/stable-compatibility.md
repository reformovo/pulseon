# Stable Compatibility Policy

This policy applies beginning with PulseOn 0.1.0. The package API, CLI JSON,
catalog application schema, and Parquet schema are separate compatibility
surfaces; a version change in one does not imply a version change in another.
ADR 0011 records this decision.

## Typed Python API

Within one stable package minor line, such as 0.1.x, existing documented and
typed calls remain valid. Compatible additive changes include:

- new modules, exported classes, methods, and exception subclasses;
- new optional keyword-only parameters with behavior-preserving defaults;
- new enum or literal values only where callers are already required to handle
  unknown values; and
- documented bug fixes that make behavior conform to the existing contract.

Adding a required argument, removing or renaming a public symbol or parameter,
narrowing accepted input, changing a return type, changing documented query or
lifecycle semantics, or replacing an exception class is breaking. A breaking
change is not shipped in a 0.1.x patch release.

Public API deprecations are documented in release notes and use
`DeprecationWarning` when a runtime call site can be identified. A deprecated
surface remains available for the rest of its stable minor line and is removed
no earlier than the next minor release. Security or data-corruption fixes may
override this notice period, but the release must identify the exception and
provide migration guidance.

## CLI JSON

`schema_version` versions the JSON document, not the PulseOn package. Within one
JSON schema version, additions are optional and consumers must ignore fields
they do not understand. Existing field types and meanings, `kind` values, error
codes, pagination semantics, and exit-status meanings remain stable.

Removing, renaming, or retyping a field, making an optional field required,
changing a stable value's meaning, or changing array ordering where ordering is
documented requires a new JSON schema version. Human-readable table formatting,
object key order, whitespace, and error `message` prose are not machine
contracts.

JSON schema version 1 remains available throughout the 0.1.x and 0.2.x package
lines. A successor may become the default only if the CLI also offers an
explicit way to request every schema still inside this window. Version 1 may be
removed no earlier than 0.3.0, after deprecation in release notes.

## Stable Stores

A stable store has a valid `pulseon_store_metadata` marker. An explicitly
upgraded 0.1.0a5 store becomes a stable schema-version-1 store; other alpha
stores are not stable stores.

Every 0.1.x release supports stores written by every earlier 0.1.x release.
When a write-side schema change is necessary, the current release must provide
an explicit migration and must not rewrite the store during ordinary
initialization. Older binaries are not required to open a store written at a
newer schema version.

The latest 0.1.x store schema remains supported for reading and explicit
migration throughout the 0.2.x line. It may be removed no earlier than 0.3.0,
after deprecation and recovery guidance are published. Backups and retry-safe
migration are required before any destructive catalog transformation.

Unversioned stores are diagnosed without guessing their producer. Because a4
and a5 storage shapes are indistinguishable, the a5 upgrade requires the user
to assert that source version and then validates the baseline shape. There is no
direct a1-a4 migration commitment.

## Parquet Schema

The metric-point Parquet schema remains the durable open-data boundary.
Additive nullable columns are compatible. Readers ignore unknown compatible
columns, and writers preserve existing columns and semantics. Removing,
renaming, or retyping columns, adding required columns, or changing metric
identity, encoding, partitioning, or last-write-wins semantics is breaking and
requires a new explicitly documented data contract rather than an in-place
catalog migration.

## Release Notes

Every stable release that changes one of these surfaces states:

- which surface and version changed;
- whether the change is additive, deprecated, or breaking;
- the first release containing the change;
- the last release or line that will support the old form, when known; and
- any required upgrade, rollback, or recovery action.
