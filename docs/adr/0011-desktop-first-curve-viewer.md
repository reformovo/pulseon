# ADR 0011: Desktop-First Curve Viewer

## Status
Accepted.

The crate responsibility and dependency details in this ADR are superseded by
[ADR 0013](0013-separate-model-storage-core-and-python.md). The desktop-first
decision and renderer boundary remain accepted.

## Context
PulseOn 0.1.x ships only a headless read surface (Python SDK + read-only CLI with
versioned JSON). Trainers and agents can query metric series, but there is no
interactive way to view, pan, zoom, and compare multi-run training curves. The
Parquet schema is the long-term product boundary, and Core intentionally avoids
plotting dependencies.

`docs/drafts/gpui-curve-viewer-spike.md` evaluated GPUI, egui, and a browser/Tauri
route. GPUI is the stronger candidate for a polished native desktop analysis
workbench; egui is simpler for prototypes; a browser/Tauri core is the safer route
only if cross-platform browser delivery is a first-release requirement.

## Decision
PulseOn's interactive analysis experience is desktop-first. The next major
version (0.2.x) delivers a usable native desktop curve viewer built with GPUI,
living in a separate workspace under the existing repository. Browser/WASM remains
a possible future path, not the current one.

The repository becomes a virtual Cargo workspace (root `Cargo.toml` holds only
`[workspace]`, no `[package]`), matching the layout used by Ruff, Polars, and
Zed. The existing `_pulseon` cdylib moves to `crates/pulseon-core/` and sits
alongside the new viewer crates so all crates are equal workspace members:

- `crates/pulseon-core` — the existing `_pulseon` cdylib (Python extension),
  moved from the root. maturin points at it via `manifest-path`.
- `crates/pulseon-chart-core` — series model, viewport, scales, path projection,
  path cache, hit testing; no GPUI/egui/Tauri/browser dependency.
- `crates/pulseon-data` — Parquet/DuckDB query and PulseOn schema validation.
- `crates/pulseon-viewer` — GPUI desktop shell, layout, rendering adapter.

The viewer does not change the Python/Rust SDK surface or the Parquet schema.

## Considered Options
- **Browser/Tauri first.** Rejected: a refined panel-heavy experiment analysis
  workbench is better served by a native desktop shell, and the product priority
  is local analysis, not cross-platform browser delivery.
- **egui first.** Rejected as primary: simpler for prototypes, but a polished
  product UI would require more custom component work than GPUI. egui remains
  useful for quick Rust-native prototypes and possible WASM demos.
- **Add plotting to the Python/Rust SDK.** Rejected: violates the product boundary
  (ADR 0001, ADR 0010) and couples the SDK to a rendering stack.
- **Root crate stays the cdylib, viewer crates nested under `crates/`.**
  Rejected: asymmetric (root crate is special), no mainstream precedent found
  for maturin cdylib-at-root plus sibling `crates/` bins. Ruff, Polars, and Zed
  all use a virtual workspace with every crate under `crates/`. The one-time
  migration cost is paid now, alongside the viewer's first phase, so the
  workspace is in its long-term shape before new crates accumulate.

## Consequences
- 0.2.x Phase 1 begins with a one-time workspace migration: move `src/` to
  `crates/pulseon-core/src/`, make root `Cargo.toml` a virtual workspace
  (`members = ["crates/*"]`), and update `pyproject.toml`/maturin
  (`manifest-path = "crates/pulseon-core/Cargo.toml"`) plus any CI `cargo`
  invocations. No behavior change; existing tests and `maturin develop` must
  still pass after the move.
- 0.2.x ships two artifacts: the existing Python wheel (`maturin build`) and a
  native desktop binary (`cargo build -p pulseon-viewer --release`).
- The viewer stays in the same repository while the project is single-owner and
  lightweight. Split to a separate repository when the viewer gains an
  independent release cadence, independent owner, or an independent CI matrix
  (GPUI's macOS/Windows graphics testing differs from the Python wheel's Linux
  matrix).
- `pulseon-chart-core` must remain unit-testable without a GPUI window so the
  data and chart model can evolve without a renderer.
- `pulseon-data` owns viewport filtering and screen-budgeted point reduction;
  `pulseon-chart-core` projects every point delivered across the typed series
  boundary, avoiding a second, non-equivalent downsampling pass.
- Replacing GPUI later would require a renderer adapter rewrite, not a data
  model rewrite — this is the boundary the split protects.
- Supported comparison alignment semantics are defined by the versioned Core
  contract and consumed by the viewer, not invented inside the renderer.
