use crate::engine::EngineError;
use crate::engine::write_rows::{StoredMetricAggregate, StoredMetricPoint};
use crate::model::metric::{MetricAggregate, MetricKey, MetricPoint, Step};
use crate::model::run::{RunId, RunStatus};

pub struct NativeQueryStore<'connection> {
    connection: &'connection duckdb::Connection,
}

impl<'connection> NativeQueryStore<'connection> {
    pub const fn new(connection: &'connection duckdb::Connection) -> Self {
        Self { connection }
    }

    pub fn query_metric_effective(
        &self,
        run_id: &RunId,
        metric_key: &MetricKey,
    ) -> Result<Vec<MetricPoint>, EngineError> {
        self.query_metric(run_id, metric_key, None, None, None)
    }

    pub fn query_metric(
        &self,
        run_id: &RunId,
        metric_key: &MetricKey,
        start_step: Option<Step>,
        end_step: Option<Step>,
        max_points: Option<usize>,
    ) -> Result<Vec<MetricPoint>, EngineError> {
        let Some(max_points) = max_points else {
            return self.query_metric_full(run_id, metric_key, start_step, end_step);
        };
        if max_points < 2 {
            return Err(EngineError::MetricQueryMaxPointsTooSmall { max_points });
        }

        let points = self.query_metric_full(run_id, metric_key, start_step, end_step)?;
        Ok(downsample_metric_points(points, max_points))
    }

    pub fn metric_aggregate(
        &self,
        run_id: &RunId,
        metric_key: &MetricKey,
    ) -> Result<MetricAggregate, EngineError> {
        let stored = self.connection.query_row(
            "SELECT run_id, metric_key, effective_count, last_step, last_value_f64,
                    min_value_f64, max_value_f64
             FROM pulseon_metric_aggregates
             WHERE run_id = ?
               AND metric_key = ?",
            (run_id.as_str(), metric_key.as_str()),
            |row| {
                Ok(StoredMetricAggregate {
                    run_id: row.get(0)?,
                    metric_key: row.get(1)?,
                    effective_count: row.get(2)?,
                    last_step: row.get(3)?,
                    last_value_f64: row.get(4)?,
                    min_value_f64: row.get(5)?,
                    max_value_f64: row.get(6)?,
                })
            },
        )?;

        Ok(stored.into_metric_aggregate())
    }

    pub fn query_metric_summaries(
        &self,
        run_ids: &[RunId],
        metric_key: &MetricKey,
    ) -> Result<Vec<MetricAggregate>, EngineError> {
        if run_ids.is_empty() {
            return Ok(Vec::new());
        }

        let requested_rows = (0..run_ids.len())
            .map(|ordinal| format!("(?, {ordinal})"))
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            "WITH requested(run_id, ordinal) AS (VALUES {requested_rows})
             SELECT run_id, metric_key, effective_count, last_step, last_value_f64,
                    min_value_f64, max_value_f64
             FROM (
                 SELECT requested.ordinal, summary.*
                 FROM requested
                 JOIN pulseon_runs AS run USING (run_id)
                 JOIN pulseon_metric_aggregates AS summary USING (run_id)
                 WHERE run.status <> 'running' AND summary.metric_key = ?
                 UNION ALL
                 SELECT requested.ordinal, points.run_id, points.metric_key,
                        count(*) AS effective_count, max(points.step) AS last_step,
                        arg_max(points.value_f64, points.step) AS last_value_f64,
                        min(points.value_f64) AS min_value_f64,
                        max(points.value_f64) AS max_value_f64
                 FROM requested
                 JOIN pulseon_runs AS run USING (run_id)
                 JOIN (
                     SELECT *, row_number() OVER (
                         PARTITION BY run_id, metric_key, step
                         ORDER BY ingested_at DESC, rowid DESC
                     ) AS write_rank
                     FROM dl.metric_points
                     WHERE metric_key = ?
                 ) AS points USING (run_id)
                 WHERE run.status = 'running' AND points.write_rank = 1
                 GROUP BY requested.ordinal, points.run_id, points.metric_key
             )
             ORDER BY ordinal"
        );

        let mut params: Vec<&str> = Vec::with_capacity(run_ids.len() + 2);
        params.extend(run_ids.iter().map(RunId::as_str));
        params.push(metric_key.as_str());
        params.push(metric_key.as_str());
        let mut statement = self.connection.prepare(&sql)?;
        let rows = statement.query_map(
            duckdb::params_from_iter(params),
            stored_metric_aggregate_from_row,
        )?;

        rows.map(|row| Ok(row?.into_metric_aggregate())).collect()
    }

    pub fn list_metrics(
        &self,
        run_id: &RunId,
        run_status: RunStatus,
    ) -> Result<Vec<MetricAggregate>, EngineError> {
        let sql = match run_status {
            RunStatus::Running => {
                "WITH effective AS (
                     SELECT *, row_number() OVER (
                         PARTITION BY run_id, metric_key, step
                         ORDER BY ingested_at DESC, rowid DESC
                     ) AS write_rank
                     FROM dl.metric_points WHERE run_id = ?
                 )
                 SELECT run_id, metric_key, count(*) AS effective_count,
                        max(step) AS last_step,
                        arg_max(value_f64, step) AS last_value_f64,
                        min(value_f64) AS min_value_f64,
                        max(value_f64) AS max_value_f64
                 FROM effective WHERE write_rank = 1
                 GROUP BY run_id, metric_key ORDER BY metric_key"
            }
            RunStatus::Finished | RunStatus::Failed => {
                "SELECT run_id, metric_key, effective_count, last_step, last_value_f64,
                        min_value_f64, max_value_f64
                 FROM pulseon_metric_aggregates
                 WHERE run_id = ? ORDER BY metric_key"
            }
        };
        let mut statement = self.connection.prepare(sql)?;
        let rows = statement.query_map([run_id.as_str()], stored_metric_aggregate_from_row)?;

        rows.map(|row| Ok(row?.into_metric_aggregate())).collect()
    }

    fn query_metric_full(
        &self,
        run_id: &RunId,
        metric_key: &MetricKey,
        start_step: Option<Step>,
        end_step: Option<Step>,
    ) -> Result<Vec<MetricPoint>, EngineError> {
        let start_step = start_step.map(Step::value);
        let end_step = end_step.map(Step::value);
        let mut statement = self.connection.prepare(
            "SELECT run_id, metric_key, step, epoch_ms(timestamp), value_f64, epoch_ms(ingested_at)
             FROM (
                 SELECT *,
                        row_number() OVER (
                            PARTITION BY run_id, metric_key, step
                            ORDER BY ingested_at DESC, rowid DESC
                        ) AS write_rank
                 FROM dl.metric_points
                 WHERE run_id = ?
                   AND metric_key = ?
             )
             WHERE write_rank = 1
               AND (? IS NULL OR step >= ?)
               AND (? IS NULL OR step < ?)
             ORDER BY step",
        )?;
        let rows = statement.query_map(
            (
                run_id.as_str(),
                metric_key.as_str(),
                start_step,
                start_step,
                end_step,
                end_step,
            ),
            stored_metric_point_from_row,
        )?;
        let points: Vec<MetricPoint> = rows
            .map(|row| row?.into_metric_point())
            .collect::<Result<_, _>>()?;

        Ok(points)
    }
}

fn downsample_metric_points(points: Vec<MetricPoint>, max_points: usize) -> Vec<MetricPoint> {
    if points.len() <= max_points {
        return points;
    }
    if max_points == 2 {
        let last = points.len() - 1;
        return take_metric_points(points, [0, last]);
    }

    let bucket_width = (points.len() - 2) as f64 / (max_points - 2) as f64;
    let mut selected = Vec::with_capacity(max_points);
    selected.push(0);
    let mut anchor = 0;
    for bucket in 0..(max_points - 2) {
        let average_start = ((bucket + 1) as f64 * bucket_width).floor() as usize + 1;
        let average_end =
            (((bucket + 2) as f64 * bucket_width).floor() as usize + 1).min(points.len());
        let average_len = (average_end - average_start) as f64;
        let average_step = points[average_start..average_end]
            .iter()
            .map(|point| point.step.value() as f64)
            .sum::<f64>()
            / average_len;
        let average_value = points[average_start..average_end]
            .iter()
            .map(|point| point.value_f64)
            .sum::<f64>()
            / average_len;
        let range_start = (bucket as f64 * bucket_width).floor() as usize + 1;
        let range_end =
            (((bucket + 1) as f64 * bucket_width).floor() as usize + 1).min(points.len() - 1);
        let anchor_step = points[anchor].step.value() as f64;
        let anchor_value = points[anchor].value_f64;
        let mut largest_area = -1.0;
        let mut next_anchor = range_start;
        for (index, point) in points[range_start..range_end].iter().enumerate() {
            let point_step = point.step.value() as f64;
            let area = ((anchor_step - average_step) * (point.value_f64 - anchor_value)
                - (anchor_step - point_step) * (average_value - anchor_value))
                .abs();
            if area > largest_area {
                largest_area = area;
                next_anchor = range_start + index;
            }
        }
        selected.push(next_anchor);
        anchor = next_anchor;
    }
    selected.push(points.len() - 1);
    take_metric_points(points, selected)
}

fn take_metric_points(
    points: Vec<MetricPoint>,
    selected: impl IntoIterator<Item = usize>,
) -> Vec<MetricPoint> {
    let mut selected = selected.into_iter();
    let mut next = selected.next();
    points
        .into_iter()
        .enumerate()
        .filter_map(|(index, point)| {
            if next == Some(index) {
                next = selected.next();
                Some(point)
            } else {
                None
            }
        })
        .collect()
}

fn stored_metric_point_from_row(row: &duckdb::Row<'_>) -> duckdb::Result<StoredMetricPoint> {
    Ok(StoredMetricPoint {
        run_id: row.get(0)?,
        metric_key: row.get(1)?,
        step: row.get(2)?,
        timestamp_millis: row.get(3)?,
        value_f64: row.get(4)?,
        ingested_at_millis: row.get(5)?,
    })
}

fn stored_metric_aggregate_from_row(
    row: &duckdb::Row<'_>,
) -> duckdb::Result<StoredMetricAggregate> {
    Ok(StoredMetricAggregate {
        run_id: row.get(0)?,
        metric_key: row.get(1)?,
        effective_count: row.get(2)?,
        last_step: row.get(3)?,
        last_value_f64: row.get(4)?,
        min_value_f64: row.get(5)?,
        max_value_f64: row.get(6)?,
    })
}
