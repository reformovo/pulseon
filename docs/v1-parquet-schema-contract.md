# PulseOn V1 Parquet Schema Contract

PulseOn v1 treats the product-owned Parquet data shape as the compatibility
boundary. DuckLake catalog metadata, temporary files, indexes, and extension
state are implementation details.

## Compatibility Rules

- Additive nullable columns are compatible.
- Removing columns, renaming columns, changing column types, or changing primary
  identity semantics is incompatible.
- Readers must ignore unknown compatible columns.
- Writers must preserve the long metric-point table shape for numeric metrics.
- Metric query semantics remain last-write-wins by `(run_id, metric_key, step)`
  using `ingested_at` as the ordering timestamp.

## Tables

### `projects`

| Column | Type | Required | Contract |
| --- | --- | --- | --- |
| `project_id` | string | yes | Stable project identity. |
| `name` | string | yes | User-facing project name. |
| `created_at` | timestamp | yes | Project creation timestamp. |

### `runs`

| Column | Type | Required | Contract |
| --- | --- | --- | --- |
| `run_id` | string | yes | Stable run identity. |
| `project_id` | string | yes | Owning project identity. |
| `name` | string | yes | User-facing run name. |
| `status` | string | yes | One of `running`, `finished`, or `failed`. |
| `created_at` | timestamp | yes | Run record creation timestamp. |
| `started_at` | timestamp | yes | Training start timestamp. |
| `finished_at` | timestamp | no | Terminal lifecycle timestamp. |

### `metric_points`

| Column | Type | Required | Contract |
| --- | --- | --- | --- |
| `run_id` | string | yes | Owning run identity. |
| `metric_key` | string | yes | User-facing metric key. |
| `step` | int64 | yes | Metric step. |
| `timestamp` | timestamp | yes | User metric timestamp. |
| `value_f64` | float64 | yes | Numeric metric value. |
| `ingested_at` | timestamp | yes | PulseOn ingestion timestamp. |

### `metric_aggregates`

`metric_aggregates` is materialized-view-like state over effective metric
series. It is query/index state, but v1 readers may use it for metric discovery
and summary data when present.

| Column | Type | Required | Contract |
| --- | --- | --- | --- |
| `run_id` | string | yes | Owning run identity. |
| `metric_key` | string | yes | User-facing metric key. |
| `effective_count` | uint64 | yes | Count after last-write-wins compaction. |
| `last_step` | int64 | yes | Highest effective step. |
| `last_value_f64` | float64 | yes | Value at `last_step`. |
| `min_value_f64` | float64 | yes | Minimum effective value. |
| `max_value_f64` | float64 | yes | Maximum effective value. |
