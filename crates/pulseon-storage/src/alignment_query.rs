use duckdb::Connection;
use pulseon_model::alignment::{
    AlignedMetricPoint, AlignmentAxis, AlignmentQuery, AlignmentQueryResult, AlignmentReason,
    AlignmentReduction,
};
use pulseon_model::metric::{MetricKey, MetricPoint, Step};
use pulseon_model::run::RunId;

use crate::StorageError;
use crate::metric_query::percent_encode_metric_key;
use crate::time::timestamp_from_millis;

const EXTREMA_PER_BUCKET: usize = 4;

#[derive(Clone, Copy)]
pub(crate) enum AlignmentSource<'a> {
    Project,
    Parquet(&'a str),
}

pub(crate) fn query_aligned_metric(
    connection: &Connection,
    source: AlignmentSource<'_>,
    query: &AlignmentQuery,
    run_start_millis: Option<i64>,
) -> Result<AlignmentQueryResult, StorageError> {
    if query.run_id.as_str().trim().is_empty() || query.metric_key.as_str().trim().is_empty() {
        return Err(StorageError::InvalidIdentity);
    }

    let reasons = query_axis_reasons(connection, source, query, run_start_millis)?;
    let bucket_count = match query.reduction {
        AlignmentReduction::Full => None,
        AlignmentReduction::ScreenBudget { .. } => {
            let max_points = query
                .reduction
                .max_points()
                .expect("screen budgets always have a point limit");
            Some((max_points / EXTREMA_PER_BUCKET).max(1))
        }
    };
    let sql = aligned_points_sql(source, query.axis, bucket_count.is_some());
    let values = query_values(source, query, run_start_millis, bucket_count)?;
    let mut statement = connection.prepare(&sql)?;
    let rows = statement.query_map(
        duckdb::params_from_iter(values.iter().map(|value| value.as_ref())),
        stored_aligned_point_from_row,
    )?;
    let mut points = Vec::new();
    let mut source_row_count = 0;
    for row in rows {
        let (point, count) = row?;
        points.push(point.into_aligned_metric_point()?);
        source_row_count = count;
    }
    Ok(AlignmentQueryResult {
        points,
        source_row_count,
        reasons,
    })
}

fn query_axis_reasons(
    connection: &Connection,
    source: AlignmentSource<'_>,
    query: &AlignmentQuery,
    run_start_millis: Option<i64>,
) -> Result<Vec<AlignmentReason>, StorageError> {
    let sql = format!(
        "{}
         SELECT coalesce(bool_or(axis_value < 0), false),
                coalesce(bool_or(
                    previous_axis_value IS NOT NULL AND axis_value < previous_axis_value
                ), false)
         FROM ordered",
        ordered_ctes(source, query.axis)
    );
    let values = base_values(source, query, run_start_millis);
    let (negative, decreasing): (bool, bool) = connection.query_row(
        &sql,
        duckdb::params_from_iter(values.iter().map(|value| value.as_ref())),
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let mut reasons = Vec::with_capacity(2);
    if negative {
        reasons.push(AlignmentReason::NegativeAxis);
    }
    if decreasing {
        reasons.push(AlignmentReason::DecreasingAxis);
    }
    Ok(reasons)
}

fn aligned_points_sql(
    source: AlignmentSource<'_>,
    axis: AlignmentAxis,
    screen_reduced: bool,
) -> String {
    let selection = if screen_reduced {
        "inside_numbered AS (
             SELECT *, row_number() OVER (ORDER BY step) - 1 AS ordinal,
                    count(*) OVER ()::UBIGINT AS inside_count
             FROM inside
         ),
         inside_bucketed AS (
             SELECT *, floor(
                 ordinal::DOUBLE * ?::DOUBLE / inside_count::DOUBLE
             )::UBIGINT AS bucket
             FROM inside_numbered
         ),
         inside_candidates AS (
             SELECT *,
                    row_number() OVER (PARTITION BY bucket ORDER BY axis_value, step) AS first_rank,
                    row_number() OVER (PARTITION BY bucket ORDER BY axis_value DESC, step DESC) AS last_rank,
                    row_number() OVER (PARTITION BY bucket ORDER BY value_f64, axis_value, step) AS min_rank,
                    row_number() OVER (PARTITION BY bucket ORDER BY value_f64 DESC, axis_value, step) AS max_rank
             FROM inside_bucketed
         ),
         selected AS (
             SELECT * EXCLUDE (ordinal, inside_count, bucket, first_rank, last_rank, min_rank, max_rank)
             FROM inside_candidates
             WHERE first_rank = 1 OR last_rank = 1 OR min_rank = 1 OR max_rank = 1
             UNION ALL SELECT * FROM left_neighbor
             UNION ALL SELECT * FROM right_neighbor
         )"
    } else {
        "selected AS (SELECT * FROM visible_source)"
    };
    format!(
        "{},
         inside AS (
             SELECT * FROM ordered WHERE axis_value >= ? AND axis_value <= ?
         ),
         left_neighbor AS (
             SELECT * FROM ordered WHERE axis_value < ?
             QUALIFY row_number() OVER (ORDER BY axis_value DESC, step DESC) = 1
         ),
         right_neighbor AS (
             SELECT * FROM ordered WHERE axis_value > ?
             QUALIFY row_number() OVER (ORDER BY axis_value, step) = 1
         ),
         visible_source AS (
             SELECT * FROM inside
             UNION ALL SELECT * FROM left_neighbor
             UNION ALL SELECT * FROM right_neighbor
         ),
         {selection}
         SELECT run_id, metric_key, step, epoch_ms(timestamp), value_f64,
                epoch_ms(ingested_at), axis_value,
                (SELECT count(*)::UBIGINT FROM visible_source)
         FROM selected ORDER BY step",
        ordered_ctes(source, axis)
    )
}

fn ordered_ctes(source: AlignmentSource<'_>, axis: AlignmentAxis) -> String {
    let (relation, tie_breaker) = match source {
        AlignmentSource::Project => ("dl.metric_points", "rowid DESC"),
        AlignmentSource::Parquet(_) => (
            "read_parquet(?, hive_partitioning = true, union_by_name = true, \
             filename = true, file_row_number = true)",
            "filename DESC, file_row_number DESC",
        ),
    };
    let axis_expression = match axis {
        AlignmentAxis::Step => "step",
        AlignmentAxis::ElapsedTime => "epoch_ms(timestamp) - ?",
    };
    format!(
        "WITH ranked AS (
             SELECT run_id, metric_key, step, timestamp, value_f64, ingested_at,
                    row_number() OVER (
                        PARTITION BY run_id, metric_key, step
                        ORDER BY ingested_at DESC, {tie_breaker}
                    ) AS write_rank
             FROM {relation}
             WHERE run_id = ? AND metric_key = ? AND metric_key_encoded = ?
         ),
         effective AS (
             SELECT run_id, metric_key, step, timestamp, value_f64, ingested_at
             FROM ranked WHERE write_rank = 1
         ),
         derived AS (
             SELECT *, {axis_expression} AS axis_value FROM effective
         ),
         ordered AS (
             SELECT *, lag(axis_value) OVER (ORDER BY step) AS previous_axis_value
             FROM derived
         )"
    )
}

fn base_values(
    source: AlignmentSource<'_>,
    query: &AlignmentQuery,
    run_start_millis: Option<i64>,
) -> Vec<Box<dyn duckdb::ToSql>> {
    let mut values: Vec<Box<dyn duckdb::ToSql>> = Vec::with_capacity(5);
    if let AlignmentSource::Parquet(location) = source {
        values.push(Box::new(location.to_owned()));
    }
    values.extend([
        Box::new(query.run_id.as_str().to_owned()) as Box<dyn duckdb::ToSql>,
        Box::new(query.metric_key.as_str().to_owned()),
        Box::new(percent_encode_metric_key(query.metric_key.as_str())),
    ]);
    if matches!(query.axis, AlignmentAxis::ElapsedTime) {
        values.push(Box::new(run_start_millis));
    }
    values
}

fn query_values(
    source: AlignmentSource<'_>,
    query: &AlignmentQuery,
    run_start_millis: Option<i64>,
    bucket_count: Option<usize>,
) -> Result<Vec<Box<dyn duckdb::ToSql>>, StorageError> {
    let mut values = base_values(source, query, run_start_millis);
    values.extend([
        Box::new(query.viewport.start()) as Box<dyn duckdb::ToSql>,
        Box::new(query.viewport.end()),
        Box::new(query.viewport.start()),
        Box::new(query.viewport.end()),
    ]);
    if let Some(bucket_count) = bucket_count {
        values.push(Box::new(i64::try_from(bucket_count).map_err(|_| {
            StorageError::QueryMaxPointsTooLarge {
                max_points: bucket_count.saturating_mul(EXTREMA_PER_BUCKET),
            }
        })?));
    }
    Ok(values)
}

struct StoredAlignedPoint {
    run_id: String,
    metric_key: String,
    step: i64,
    timestamp_millis: i64,
    value_f64: f64,
    ingested_at_millis: i64,
    axis_value: i64,
}

impl StoredAlignedPoint {
    fn into_aligned_metric_point(self) -> Result<AlignedMetricPoint, StorageError> {
        Ok(AlignedMetricPoint {
            point: MetricPoint {
                run_id: RunId::from_string(self.run_id),
                metric_key: MetricKey::from_string(self.metric_key),
                step: Step::new(self.step),
                timestamp: timestamp_from_millis("timestamp", self.timestamp_millis)?,
                value_f64: self.value_f64,
                ingested_at: timestamp_from_millis("ingested_at", self.ingested_at_millis)?,
            },
            axis_value: self.axis_value,
        })
    }
}

fn stored_aligned_point_from_row(
    row: &duckdb::Row<'_>,
) -> duckdb::Result<(StoredAlignedPoint, u64)> {
    Ok((
        StoredAlignedPoint {
            run_id: row.get(0)?,
            metric_key: row.get(1)?,
            step: row.get(2)?,
            timestamp_millis: row.get(3)?,
            value_f64: row.get(4)?,
            ingested_at_millis: row.get(5)?,
            axis_value: row.get(6)?,
        },
        row.get(7)?,
    ))
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use pulseon_model::alignment::{AlignmentReduction, AlignmentViewport};

    use super::*;
    use crate::ProjectMetricReader;

    fn connection() -> Result<Connection, Box<dyn Error>> {
        let connection = Connection::open_in_memory()?;
        connection.execute_batch(
            "CREATE SCHEMA dl;
             CREATE TABLE dl.metric_points (
                 run_id VARCHAR NOT NULL,
                 metric_key VARCHAR NOT NULL,
                 metric_key_encoded VARCHAR NOT NULL,
                 step BIGINT NOT NULL,
                 timestamp TIMESTAMPTZ NOT NULL,
                 value_f64 DOUBLE NOT NULL,
                 ingested_at TIMESTAMPTZ NOT NULL
             );
             CREATE TABLE pulseon_runs (run_id VARCHAR, started_at TIMESTAMPTZ);
             INSERT INTO pulseon_runs VALUES ('run-1', epoch_ms(1000));
             INSERT INTO dl.metric_points
             SELECT 'run-1', 'loss', 'loss', step, epoch_ms(1000 + step * 10),
                    step::DOUBLE, epoch_ms(2000 + step)
             FROM range(0, 7) AS generated(step);
             INSERT INTO dl.metric_points VALUES
                 ('run-1', 'loss', 'loss', 3, epoch_ms(1030), -1.0, epoch_ms(3000));",
        )?;
        Ok(connection)
    }

    fn query(axis: AlignmentAxis, reduction: AlignmentReduction) -> AlignmentQuery {
        AlignmentQuery {
            run_id: RunId::from_string("run-1"),
            metric_key: MetricKey::from_string("loss"),
            axis,
            viewport: AlignmentViewport::new(2, 4).expect("test viewport should be valid"),
            reduction,
        }
    }

    #[test]
    fn full_step_query_keeps_closed_viewport_neighbors_and_effective_points()
    -> Result<(), Box<dyn Error>> {
        let connection = connection()?;
        let result = ProjectMetricReader::new(&connection)
            .query_aligned_metric(&query(AlignmentAxis::Step, AlignmentReduction::Full))?;

        assert_eq!(
            result
                .points
                .iter()
                .map(|point| (point.axis_value, point.point.value_f64))
                .collect::<Vec<_>>(),
            vec![(1, 1.0), (2, 2.0), (3, -1.0), (4, 4.0), (5, 5.0)]
        );
        assert_eq!(result.source_row_count, 5);
        assert!(result.reasons.is_empty());
        Ok(())
    }

    #[test]
    fn elapsed_query_retains_equal_axes_and_reports_decreases() -> Result<(), Box<dyn Error>> {
        let connection = connection()?;
        connection.execute_batch(
            "UPDATE dl.metric_points SET timestamp = epoch_ms(1020) WHERE step = 3;
             UPDATE dl.metric_points SET timestamp = epoch_ms(1010) WHERE step = 4;",
        )?;
        let mut elapsed = query(AlignmentAxis::ElapsedTime, AlignmentReduction::Full);
        elapsed.viewport = AlignmentViewport::new(10, 20)?;
        let result = ProjectMetricReader::new(&connection).query_aligned_metric(&elapsed)?;

        assert_eq!(result.reasons, vec![AlignmentReason::DecreasingAxis]);
        assert_eq!(
            result
                .points
                .iter()
                .map(|point| point.axis_value)
                .collect::<Vec<_>>(),
            vec![0, 10, 20, 20, 10, 50]
        );
        Ok(())
    }

    #[test]
    fn elapsed_screen_query_uses_extrema_without_lttb() -> Result<(), Box<dyn Error>> {
        let connection = connection()?;
        let mut elapsed = query(
            AlignmentAxis::ElapsedTime,
            AlignmentReduction::screen_budget(1, 1)?,
        );
        elapsed.viewport = AlignmentViewport::new(10, 50)?;
        let result = ProjectMetricReader::new(&connection).query_aligned_metric(&elapsed)?;

        assert_eq!(result.source_row_count, 7);
        assert!(result.downsampled());
        assert!(result.reasons.is_empty());
        assert_eq!(result.points.first().map(|point| point.axis_value), Some(0));
        assert_eq!(result.points.last().map(|point| point.axis_value), Some(60));
        Ok(())
    }

    #[test]
    fn screen_query_keeps_neighbors_and_bucket_extrema() -> Result<(), Box<dyn Error>> {
        let connection = connection()?;
        let mut screen = query(
            AlignmentAxis::Step,
            AlignmentReduction::screen_budget(1, 1)?,
        );
        screen.viewport = AlignmentViewport::new(1, 5)?;
        let result = query_aligned_metric(&connection, AlignmentSource::Project, &screen, None)?;

        assert_eq!(result.source_row_count, 7);
        assert!(result.downsampled());
        assert!(result.points.iter().any(|point| point.axis_value == 0));
        assert!(result.points.iter().any(|point| point.axis_value == 6));
        assert!(
            result
                .points
                .iter()
                .any(|point| point.point.value_f64 == -1.0)
        );
        Ok(())
    }

    #[test]
    fn step_query_reports_negative_axis_without_repairing_points() -> Result<(), Box<dyn Error>> {
        let connection = connection()?;
        connection.execute_batch(
            "INSERT INTO dl.metric_points VALUES
                 ('run-1', 'loss', 'loss', -1, epoch_ms(990), 9.0, epoch_ms(1999));",
        )?;
        let mut negative = query(AlignmentAxis::Step, AlignmentReduction::Full);
        negative.viewport = AlignmentViewport::new(-1, 0)?;
        let result = ProjectMetricReader::new(&connection).query_aligned_metric(&negative)?;

        assert_eq!(result.reasons, vec![AlignmentReason::NegativeAxis]);
        assert!(result.points.iter().any(|point| point.axis_value == -1));
        Ok(())
    }
}
