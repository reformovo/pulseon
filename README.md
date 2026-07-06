# PulseOn

PulseOn is a local-first training metrics tracker backed by Rust, PyO3, DuckDB,
and DuckLake.

Current focus: plan and implement the 0.1.0a2 native loop:

- create a project
- start or resume a run
- log explicit-step numeric metrics through a bounded async queue
- query metric series and summaries locally
- support local DuckDB DuckLake catalog storage
- defer SQLite catalog support until real DuckLake-backed parity tests pass
- keep the v2 public data path local-filesystem only
- keep Parquet as the long-term compatibility boundary

Python API shape:

```python
import pulseon

client = pulseon.init(
    "runs",
    data_path=None,
    catalog_backend="duckdb",
    catalog_path=None,
    metric_queue_capacity=65536,
)
project = client.create_project("local training")
run = client.create_run(project.project_id, "baseline")
run.log("train/loss", 0, 0.25)
```

For bounded teardown, stop active logging threads before calling
`client.shutdown(timeout=...)`; PulseOn keeps admission open while bounded
shutdown is draining, so concurrent `run.log(...)` calls can prevent that drain
from completing before the timeout.

Architecture entry points:

- [Catalog/data boundary](docs/catalog-data-boundary.md)
- [V1 native architecture](docs/v1-native-architecture.md)
- [Glossary](docs/glossary.md)
- [Roadmap](docs/TODO.md)
- [ADRs](docs/adr/)

Runtime extensions:

- DuckLake is installed and loaded by the native engine because it is required
  for native storage.
- DuckDB LTTB is not bundled into the SDK. PulseOn loads an already installed
  `lttb` extension when downsampling is requested with `max_points`. To allow
  PulseOn to download it from the DuckDB community repository at that point,
  set `PULSEON_LTTB_AUTO_INSTALL=1`. To use a local build instead, set
  `PULSEON_LTTB_EXTENSION_PATH=/path/to/lttb.duckdb_extension`.
