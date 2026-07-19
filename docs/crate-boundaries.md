# Crate Boundaries

PulseOn workspace crates follow one dependency direction:

```text
pulseon-model <- pulseon-storage <- pulseon-core <- pulseon-python
       ^                 ^                ^
       +---------- pulseon-viewer --------+----> pulseon-chart-core
```

`pulseon-viewer` is the future composition root and may depend directly on the
model, storage, core, and chart crates. All other reverse dependencies and
cycles are forbidden.

## Responsibilities

- **`pulseon-model`** owns projects, runs, metrics, typed identities, query
  inputs, reduction policies, and query results. It has no storage, Python, or
  rendering dependencies.
- **`pulseon-storage`** owns project configuration, DuckDB/DuckLake catalogs,
  schema bootstrap and validation, encoding, reads, writes, aggregate repair,
  flush, S3 setup, and storage errors. It exposes a narrow metric-reader
  interface implemented by the native project store and Parquet dataset reader.
- **`pulseon-core`** owns client and run lifecycle, report admission, the
  background queue, drain and finalization orchestration, shutdown, diagnostics,
  and comparison use cases. It contains no SQL or Python bindings.
- **`pulseon-python`** owns the PyO3 extension, Python classes and exceptions,
  Arrow capsules, argument conversion, and error mapping. It contains no
  product or storage policy.
- **`pulseon-chart-core`** owns renderer-independent chart series, viewports,
  scales, projected paths, hit testing, and interaction state. Its generic
  chart points intentionally remain distinct from metric points.
- **`pulseon-viewer`** owns source selection, background query scheduling,
  conversion from metric points to chart points, GPUI state, and rendering.

## Shared Query Contract

Both metric readers apply half-open step filtering and last-write-wins effective
series semantics before optional reduction. The storage crate owns full-series,
LTTB, and screen-budgeted extrema query strategies; chart code projects every
point returned by storage and does not downsample again.

Aligned queries are separate from ordinary half-open step queries. They derive
raw-step or elapsed-time coordinates after last-write-wins, use a closed
viewport plus one strict neighbor on each side, and expose full or
screen-budgeted extrema results through the shared metric-reader interface.
The native reader resolves elapsed origin from Run metadata. The standalone
Parquet reader remains fact-only: it supports step alignment and reports
`missing_run_start` for elapsed alignment.

The native project store is the authoritative read source for catalog discovery
and inline plus Parquet-backed facts. A standalone Parquet dataset is a
compatibility source for flushed facts only and does not imply project or run
discovery.
