# PulseOn

PulseOn is a local-first training metrics tracker backed by Rust, PyO3, DuckDB,
and DuckLake.

Current focus: validate the v1 native loop for individual trainers:

- create a project
- start or resume a run
- log numeric metrics
- query metric series and summaries locally
- keep Parquet as the long-term compatibility boundary

Architecture entry points:

- [V1 native architecture](docs/v1-native-architecture.md)
- [Glossary](docs/glossary.md)
- [Roadmap](docs/TODO.md)
- [ADRs](docs/adr/)

Runtime extensions:

- DuckLake is installed and loaded by the native engine because it is required
  for v1 storage.
- DuckDB LTTB is not bundled into the SDK. PulseOn loads an already installed
  `lttb` extension when downsampling is requested with `max_points`. To allow
  PulseOn to download it from the DuckDB community repository at that point,
  set `PULSEON_LTTB_AUTO_INSTALL=1`. To use a local build instead, set
  `PULSEON_LTTB_EXTENSION_PATH=/path/to/lttb.duckdb_extension`.
