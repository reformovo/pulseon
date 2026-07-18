# ADR 0013: Separate Model, Storage, Core, and Python Boundaries

## Status
Accepted.

PulseOn separates shared product types, native persistence, application
orchestration, and the Python extension into `pulseon-model`,
`pulseon-storage`, `pulseon-core`, and `pulseon-python`. The split lets the
Python SDK and desktop viewer reuse one authoritative query contract without
making PyO3, DuckDB, or rendering types leak across layers; only the narrow
metric-read interface has multiple implementations because native project
stores and standalone Parquet datasets are both current product inputs.

`pulseon-core` uses concrete storage types for other operations instead of
introducing speculative repository traits. `pulseon-chart-core` stays
domain-independent, and downsampling belongs to the storage query pipeline so
renderers never apply a second, non-equivalent reduction pass.
