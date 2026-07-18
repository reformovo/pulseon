use duckdb::Connection;

use crate::{ColumnSchema, DataError, SchemaReport, validate_metric_point_schema};

const MAX_QUERY_POINTS: usize = 1_000_000;
const EXTREMA_PER_BUCKET: usize = 4;

const METRIC_QUERY_SQL: &str = "WITH ranked AS (
         SELECT run_id, metric_key, step, timestamp, value_f64, ingested_at,
                row_number() OVER (
                    PARTITION BY run_id, metric_key, step
                    ORDER BY ingested_at DESC, filename DESC, file_row_number DESC
                ) AS write_rank
         FROM read_parquet(
             ?, hive_partitioning = true, union_by_name = true,
             filename = true, file_row_number = true
         )
         WHERE run_id = ? AND metric_key = ? AND metric_key_encoded = ?
           AND (? IS NULL OR step >= ?)
           AND (? IS NULL OR step < ?)
     ),
     effective AS (
         SELECT run_id, metric_key, step, timestamp, value_f64, ingested_at
         FROM ranked WHERE write_rank = 1
     ),
     numbered AS (
         SELECT *, row_number() OVER (ORDER BY step) - 1 AS ordinal,
                count(*) OVER ()::UBIGINT AS source_row_count
         FROM effective
     ),
     bucketed AS (
         SELECT *, floor(
             ordinal::DOUBLE * ?::DOUBLE / source_row_count::DOUBLE
         )::UBIGINT AS bucket
         FROM numbered
     ),
     candidates AS (
         SELECT *,
                row_number() OVER (PARTITION BY bucket ORDER BY step) AS first_rank,
                row_number() OVER (PARTITION BY bucket ORDER BY step DESC) AS last_rank,
                row_number() OVER (PARTITION BY bucket ORDER BY value_f64, step) AS min_rank,
                row_number() OVER (PARTITION BY bucket ORDER BY value_f64 DESC, step) AS max_rank
         FROM bucketed
     )
     SELECT run_id, metric_key, step, epoch_ms(timestamp), value_f64,
            epoch_ms(ingested_at), source_row_count
     FROM candidates
     WHERE first_rank = 1 OR last_rank = 1 OR min_rank = 1 OR max_rank = 1
     ORDER BY step";

/// A DuckDB-readable Parquet file, glob, or object-store URI.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParquetSource(String);

impl ParquetSource {
    /// Creates a non-empty DuckDB Parquet location.
    ///
    /// # Errors
    ///
    /// Returns [`DataError::EmptySource`] for a blank location.
    pub fn new(location: impl Into<String>) -> Result<Self, DataError> {
        let location = location.into();
        if location.trim().is_empty() {
            return Err(DataError::EmptySource);
        }
        Ok(Self(location))
    }

    pub fn location(&self) -> &str {
        &self.0
    }
}

/// Half-open step bounds applied before effective-series downsampling.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct StepViewport {
    pub start: Option<i64>,
    pub end: Option<i64>,
}

impl StepViewport {
    /// Creates optional `[start, end)` bounds.
    ///
    /// # Errors
    ///
    /// Returns [`DataError::InvalidViewport`] when both bounds are not increasing.
    pub fn new(start: Option<i64>, end: Option<i64>) -> Result<Self, DataError> {
        if let (Some(start), Some(end)) = (start, end)
            && start >= end
        {
            return Err(DataError::InvalidViewport);
        }
        Ok(Self { start, end })
    }
}

/// Screen-derived upper bound for points returned by one query.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct QueryBudget {
    pixel_width: u32,
    points_per_pixel: u16,
}

impl QueryBudget {
    /// Creates a positive viewport query budget.
    ///
    /// # Errors
    ///
    /// Returns [`DataError::InvalidQueryBudget`] when either value is zero.
    pub fn new(pixel_width: u32, points_per_pixel: u16) -> Result<Self, DataError> {
        if pixel_width == 0 || points_per_pixel == 0 {
            return Err(DataError::InvalidQueryBudget);
        }
        Ok(Self {
            pixel_width,
            points_per_pixel,
        })
    }

    pub fn max_points(self) -> usize {
        (self.pixel_width as usize)
            .saturating_mul(self.points_per_pixel as usize)
            .clamp(EXTREMA_PER_BUCKET, MAX_QUERY_POINTS)
    }
}

/// Validated inputs for one metric-series viewport query.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MetricQuery {
    run_id: String,
    metric_key: String,
    viewport: StepViewport,
    budget: QueryBudget,
}

impl MetricQuery {
    /// Creates a query for one effective metric series.
    ///
    /// # Errors
    ///
    /// Returns [`DataError::InvalidIdentity`] for blank identities.
    pub fn new(
        run_id: impl Into<String>,
        metric_key: impl Into<String>,
        viewport: StepViewport,
        budget: QueryBudget,
    ) -> Result<Self, DataError> {
        let run_id = run_id.into();
        let metric_key = metric_key.into();
        if run_id.trim().is_empty() || metric_key.trim().is_empty() {
            return Err(DataError::InvalidIdentity);
        }
        Ok(Self {
            run_id,
            metric_key,
            viewport,
            budget,
        })
    }

    pub fn plan(&self) -> MetricQueryPlan {
        let max_points = self.budget.max_points();
        MetricQueryPlan {
            viewport: self.viewport,
            max_points,
            bucket_count: (max_points / EXTREMA_PER_BUCKET).max(1),
        }
    }
}

/// Inspectable execution choices derived from a viewport and screen budget.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MetricQueryPlan {
    pub viewport: StepViewport,
    pub max_points: usize,
    pub bucket_count: usize,
}

/// One effective metric point returned from Parquet.
#[derive(Clone, Debug, PartialEq)]
pub struct MetricPointRow {
    pub run_id: String,
    pub metric_key: String,
    pub step: i64,
    pub timestamp_ms: i64,
    pub value_f64: f64,
    pub ingested_at_ms: i64,
}

/// Viewport result plus its pre-downsampling effective row count.
#[derive(Clone, Debug, PartialEq)]
pub struct MetricQueryResult {
    pub points: Vec<MetricPointRow>,
    pub source_row_count: u64,
    pub plan: MetricQueryPlan,
}

/// A validated Parquet metric source queried through an existing DuckDB connection.
pub struct DuckDbMetricStore<'connection> {
    connection: &'connection Connection,
    source: ParquetSource,
    schema: SchemaReport,
}

impl<'connection> DuckDbMetricStore<'connection> {
    /// Inspects and validates a Parquet source before it becomes queryable.
    ///
    /// # Errors
    ///
    /// Returns an error for DuckDB failures or an incompatible schema.
    pub fn open(
        connection: &'connection Connection,
        source: ParquetSource,
    ) -> Result<Self, DataError> {
        let columns = inspect_schema(connection, &source)?;
        let schema = validate_metric_point_schema(columns)?;
        Ok(Self {
            connection,
            source,
            schema,
        })
    }

    pub const fn schema(&self) -> &SchemaReport {
        &self.schema
    }

    /// Queries one half-open viewport and applies a screen-derived point budget.
    ///
    /// # Errors
    ///
    /// Returns [`DataError::DuckDb`] when query preparation or execution fails.
    pub fn query_metric(&self, query: &MetricQuery) -> Result<MetricQueryResult, DataError> {
        let plan = query.plan();
        let expected_encoded_key = percent_encode_metric_key(&query.metric_key);
        let mut statement = self.connection.prepare(METRIC_QUERY_SQL)?;
        let rows = statement.query_map(
            duckdb::params![
                self.source.location(),
                query.run_id.as_str(),
                query.metric_key.as_str(),
                expected_encoded_key.as_str(),
                plan.viewport.start,
                plan.viewport.start,
                plan.viewport.end,
                plan.viewport.end,
                i64::try_from(plan.bucket_count).map_err(|_| DataError::InvalidQueryBudget)?,
            ],
            |row| {
                Ok((
                    MetricPointRow {
                        run_id: row.get(0)?,
                        metric_key: row.get(1)?,
                        step: row.get(2)?,
                        timestamp_ms: row.get(3)?,
                        value_f64: row.get(4)?,
                        ingested_at_ms: row.get(5)?,
                    },
                    row.get::<_, u64>(6)?,
                ))
            },
        )?;

        let mut points = Vec::new();
        let mut source_row_count = 0;
        for row in rows {
            let (point, row_count) = row?;
            points.push(point);
            source_row_count = row_count;
        }
        Ok(MetricQueryResult {
            points,
            source_row_count,
            plan,
        })
    }
}

fn percent_encode_metric_key(value: &str) -> String {
    let mut encoded = String::new();
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'.' | b'_' | b'~' | b'-' => {
                encoded.push(char::from(byte));
            }
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }
    encoded
}

fn inspect_schema(
    connection: &Connection,
    source: &ParquetSource,
) -> Result<Vec<ColumnSchema>, DataError> {
    let mut statement = connection.prepare(
        "DESCRIBE SELECT * FROM read_parquet(
             ?, hive_partitioning = true, union_by_name = true
         )",
    )?;
    let rows = statement.query_map([source.location()], |row| {
        let nullable: String = row.get(2)?;
        Ok(ColumnSchema::new(
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            nullable.eq_ignore_ascii_case("YES"),
        ))
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(DataError::from)
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;

    static NEXT_TEST_ID: AtomicU64 = AtomicU64::new(0);

    struct TestParquet {
        directory: PathBuf,
        location: String,
    }

    impl TestParquet {
        fn create(connection: &Connection) -> Result<Self, Box<dyn Error>> {
            let id = NEXT_TEST_ID.fetch_add(1, Ordering::Relaxed);
            let directory =
                std::env::temp_dir().join(format!("pulseon-data-{}-{id}", std::process::id()));
            fs::create_dir_all(&directory)?;
            let path = directory.join("metric_points.parquet");
            connection.execute_batch(&format!(
                "CREATE TABLE metric_points (
                     run_id VARCHAR NOT NULL,
                     metric_key VARCHAR NOT NULL,
                     metric_key_encoded VARCHAR NOT NULL,
                     step BIGINT NOT NULL,
                     timestamp TIMESTAMPTZ NOT NULL,
                     value_f64 DOUBLE NOT NULL,
                     ingested_at TIMESTAMPTZ NOT NULL
                 );
                 INSERT INTO metric_points VALUES
                     ('run-1', 'train/loss', 'train%2Floss', 1, '2026-01-01 00:00:01+00', 1.0, '2026-01-01 00:00:01+00'),
                     ('run-1', 'train/loss', 'train%2Floss', 2, '2026-01-01 00:00:02+00', 2.0, '2026-01-01 00:00:02+00'),
                     ('run-1', 'train/loss', 'train%2Floss', 2, '2026-01-01 00:00:02+00', 0.2, '2026-01-01 00:00:03+00'),
                     ('run-1', 'train/loss', 'train%2Floss', 3, '2026-01-01 00:00:03+00', 0.5, '2026-01-01 00:00:04+00'),
                     ('run-1', 'train/loss', 'train%2Floss', 4, '2026-01-01 00:00:04+00', 0.4, '2026-01-01 00:00:05+00'),
                     ('run-1', 'train/loss', 'train%2Floss', 5, '2026-01-01 00:00:05+00', 0.3, '2026-01-01 00:00:06+00'),
                     ('run-1', 'bad/key', 'bad-key', 1, '2026-01-01 00:00:01+00', 1.0, '2026-01-01 00:00:01+00');
                 INSERT INTO metric_points
                 SELECT 'run-1', 'train/loss', 'train%2Floss', step,
                        to_timestamp((1767225600 + step)::DOUBLE),
                        1.0 / step,
                        to_timestamp((1767225610 + step)::DOUBLE)
                 FROM range(6, 26) AS generated(step);
                 COPY metric_points TO '{}' (FORMAT PARQUET);",
                path.display()
            ))?;
            Ok(Self {
                directory,
                location: path.to_string_lossy().into_owned(),
            })
        }
    }

    impl Drop for TestParquet {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.directory);
        }
    }

    #[test]
    fn query_plan_uses_half_open_viewport_and_screen_budget() {
        let viewport = StepViewport::new(Some(10), Some(20)).expect("viewport should be valid");
        let budget = QueryBudget::new(200, 2).expect("budget should be valid");
        let query =
            MetricQuery::new("run-1", "loss", viewport, budget).expect("query should be valid");

        assert_eq!(
            query.plan(),
            MetricQueryPlan {
                viewport,
                max_points: 400,
                bucket_count: 100,
            }
        );
    }

    #[test]
    fn store_validates_schema_and_queries_effective_viewport() -> Result<(), Box<dyn Error>> {
        let connection = Connection::open_in_memory()?;
        let parquet = TestParquet::create(&connection)?;
        let source = ParquetSource::new(parquet.location.clone())?;
        let store = DuckDbMetricStore::open(&connection, source)?;
        let query = MetricQuery::new(
            "run-1",
            "train/loss",
            StepViewport::new(Some(2), Some(5))?,
            QueryBudget::new(2, 2)?,
        )?;

        let result = store.query_metric(&query)?;

        assert!(store.schema().additive_columns.is_empty());
        assert_eq!(result.source_row_count, 3);
        assert_eq!(
            result
                .points
                .iter()
                .map(|point| (point.step, point.value_f64))
                .collect::<Vec<_>>(),
            vec![(2, 0.2), (3, 0.5), (4, 0.4)]
        );

        let dense_query = MetricQuery::new(
            "run-1",
            "train/loss",
            StepViewport::default(),
            QueryBudget::new(1, 1)?,
        )?;
        let dense_result = store.query_metric(&dense_query)?;
        assert_eq!(dense_result.source_row_count, 25);
        assert!(dense_result.points.len() <= dense_result.plan.max_points);
        assert_eq!(dense_result.points.first().map(|point| point.step), Some(1));
        assert_eq!(dense_result.points.last().map(|point| point.step), Some(25));

        let mismatched_partition_query = MetricQuery::new(
            "run-1",
            "bad/key",
            StepViewport::default(),
            QueryBudget::new(1, 1)?,
        )?;
        let mismatched_result = store.query_metric(&mismatched_partition_query)?;
        assert!(mismatched_result.points.is_empty());
        assert_eq!(mismatched_result.source_row_count, 0);
        Ok(())
    }

    #[test]
    fn percent_encoding_matches_the_parquet_partition_contract() {
        assert_eq!(
            percent_encode_metric_key("train/loss 零"),
            "train%2Floss%20%E9%9B%B6"
        );
    }

    #[test]
    fn metric_query_filters_the_public_partition_columns() {
        assert!(METRIC_QUERY_SQL.contains("run_id = ?"));
        assert!(METRIC_QUERY_SQL.contains("metric_key_encoded = ?"));
    }
}
