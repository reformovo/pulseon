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
