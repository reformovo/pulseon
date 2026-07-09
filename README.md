# PulseOn

PulseOn is a local-first training metrics tracker backed by Rust, PyO3, DuckDB,
and DuckLake.

Current alpha: 0.1.0a3 / v3 native local storage loop. The v4 development
line adds S3-compatible data paths for the Parquet data area:

- create a project
- start or resume a run
- log explicit-step numeric metrics through a bounded async queue
- query metric series and summaries locally
- support local DuckDB-backed DuckLake catalog storage by default
- support local SQLite-backed DuckLake catalog storage when requested
- support local or S3-compatible Parquet data paths
- keep Parquet as the long-term compatibility boundary

Quickstart:

```python
import pulseon

client = pulseon.init()
project = client.create_project("local training")
run = client.create_run(project.project_id, "baseline")
run.log("train/loss", 0, 0.25)
client.finish_run(run.run_id)
client.shutdown()
```

By default, PulseOn stores local state under `./.pulseon`. Pass an explicit
root path when a project should use a different local store:

```python
client = pulseon.init("runs")
```

The existing storage keywords remain available: `data_path`,
`catalog_backend`, `catalog_path`, and `metric_queue_capacity`. `catalog_path`
must be a local filesystem path. `data_path` may be local, or it may use an
S3-compatible URI such as `s3://bucket/prefix`.

Project-local storage settings can live in `./.pulseon/config.toml`:

```toml
data_path = "s3://example-bucket/pulseon/demo"

[s3]
endpoint = "https://s3.example.com"
region = "us-east-1"
access_key_id = "<access-key-id>"
secret_access_key = "<secret-access-key>"
path_style = true
use_ssl = true
```

Do not commit real S3 credentials. Explicit `pulseon.init(...)` keyword
arguments override values from `config.toml`:

```python
client = pulseon.init(
    data_path="s3://example-bucket/pulseon/demo",
    s3_endpoint="https://s3.example.com",
    s3_access_key_id="<access-key-id>",
    s3_secret_access_key="<secret-access-key>",
    s3_path_style=True,
)
```

For bounded teardown, stop active logging threads before calling
`client.shutdown(timeout=...)`; PulseOn keeps admission open while bounded
shutdown is draining, so concurrent `run.log(...)` calls can prevent that drain
from completing before the timeout.

Architecture entry points:

- [Docs index](docs/README.md)
- [Native storage boundary](docs/native-storage-boundary.md)
- [Glossary](docs/glossary.md)
- [Roadmap](docs/ROADMAP.md)
- [ADRs](docs/adr/)

Runtime extensions:

- DuckLake is installed and loaded by the native engine because it is required
  for native storage.
- DuckDB LTTB is not bundled into the SDK. PulseOn loads an already installed
  `lttb` extension when downsampling is requested with `max_points`. To allow
  PulseOn to download it from the DuckDB community repository at that point,
  set `PULSEON_LTTB_AUTO_INSTALL=1`. To use a local build instead, set
  `PULSEON_LTTB_EXTENSION_PATH=/path/to/lttb.duckdb_extension`.
