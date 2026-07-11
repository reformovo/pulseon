use std::path::Path;

use crate::engine::EngineError;
use crate::engine::write_rows::{StoredMetricAggregate, StoredMetricPoint};
use crate::model::metric::{MetricAggregate, MetricKey, MetricPoint, Step};
use crate::model::run::{RunId, RunStatus};

const LTTB_AUTO_INSTALL_ENV: &str = "PULSEON_LTTB_AUTO_INSTALL";
const LTTB_EXTENSION_PATH_ENV: &str = "PULSEON_LTTB_EXTENSION_PATH";

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

        let row_count = self.count_metric_effective(run_id, metric_key, start_step, end_step)?;
        if row_count <= max_points as u64 {
            return self.query_metric_full(run_id, metric_key, start_step, end_step);
        }

        let max_points_i64 = i64::try_from(max_points)
            .map_err(|_| EngineError::MetricQueryMaxPointsTooLarge { max_points })?;
        self.ensure_lttb_extension_loaded()?;
        self.query_metric_downsampled(run_id, metric_key, start_step, end_step, max_points_i64)
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
               AND (? IS NULL OR step <= ?)
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

    fn query_metric_downsampled(
        &self,
        run_id: &RunId,
        metric_key: &MetricKey,
        start_step: Option<Step>,
        end_step: Option<Step>,
        max_points: i64,
    ) -> Result<Vec<MetricPoint>, EngineError> {
        let start_step = start_step.map(Step::value);
        let end_step = end_step.map(Step::value);
        let mut statement = self.connection.prepare(
            "WITH effective AS (
                 SELECT run_id, metric_key, step, timestamp, value_f64, ingested_at
                 FROM (
                     SELECT *, row_number() OVER
                                (PARTITION BY run_id, metric_key, step
                                 ORDER BY ingested_at DESC, rowid DESC) AS write_rank
                     FROM dl.metric_points
                     WHERE run_id = ? AND metric_key = ?
                 )
                 WHERE write_rank = 1
                   AND (? IS NULL OR step >= ?)
                   AND (? IS NULL OR step <= ?)
             ),
             sampled AS (
                 SELECT unnest(lttb(step, value_f64, ?)) AS point
                 FROM effective
             )
             SELECT effective.run_id, effective.metric_key, effective.step,
                    epoch_ms(effective.timestamp), effective.value_f64,
                    epoch_ms(effective.ingested_at)
             FROM sampled
             JOIN effective ON effective.step = sampled.point.x ORDER BY effective.step",
        )?;
        let rows = statement.query_map(
            (
                run_id.as_str(),
                metric_key.as_str(),
                start_step,
                start_step,
                end_step,
                end_step,
                max_points,
            ),
            stored_metric_point_from_row,
        )?;

        rows.map(|row| row?.into_metric_point()).collect()
    }

    fn count_metric_effective(
        &self,
        run_id: &RunId,
        metric_key: &MetricKey,
        start_step: Option<Step>,
        end_step: Option<Step>,
    ) -> Result<u64, EngineError> {
        let start_step = start_step.map(Step::value);
        let end_step = end_step.map(Step::value);
        let count = self.connection.query_row(
            "SELECT count(DISTINCT step)
             FROM dl.metric_points
             WHERE run_id = ? AND metric_key = ?
               AND (? IS NULL OR step >= ?)
               AND (? IS NULL OR step <= ?)",
            (
                run_id.as_str(),
                metric_key.as_str(),
                start_step,
                start_step,
                end_step,
                end_step,
            ),
            |row| row.get(0),
        )?;
        Ok(count)
    }

    fn ensure_lttb_extension_loaded(&self) -> Result<(), EngineError> {
        if self.lttb_function_available() {
            return Ok(());
        }

        let load_error = match self.connection.execute_batch("LOAD lttb;") {
            Ok(()) if self.lttb_function_available() => return Ok(()),
            Ok(()) => None,
            Err(source) => Some(source.to_string()),
        };

        if let Some(path) = std::env::var_os(LTTB_EXTENSION_PATH_ENV) {
            return self.load_lttb_extension_from_path(Path::new(&path));
        }

        if lttb_auto_install_allowed(std::env::var_os(LTTB_AUTO_INSTALL_ENV).as_deref()) {
            return self.install_and_load_lttb_extension();
        }

        let mut message = format!(
            "lttb is not loaded and PulseOn will not download it automatically; \
             set {LTTB_AUTO_INSTALL_ENV}=1 to allow INSTALL lttb FROM community, \
             or set {LTTB_EXTENSION_PATH_ENV} to a local lttb.duckdb_extension"
        );
        if let Some(load_error) = load_error {
            message.push_str("; LOAD lttb failed: ");
            message.push_str(&load_error);
        }
        Err(EngineError::LttbExtensionUnavailable { message })
    }

    fn install_and_load_lttb_extension(&self) -> Result<(), EngineError> {
        match self
            .connection
            .execute_batch("INSTALL lttb FROM community; LOAD lttb;")
        {
            Ok(()) if self.lttb_function_available() => Ok(()),
            Ok(()) => Err(EngineError::LttbExtensionUnavailable {
                message: "INSTALL/LOAD lttb did not register lttb".to_owned(),
            }),
            Err(source) => Err(EngineError::LttbExtensionUnavailable {
                message: source.to_string(),
            }),
        }
    }

    fn load_lttb_extension_from_path(&self, path: &Path) -> Result<(), EngineError> {
        let path = sql_string_literal(path.to_string_lossy().as_ref());
        match self.connection.execute_batch(&format!("LOAD {path};")) {
            Ok(()) if self.lttb_function_available() => Ok(()),
            Ok(()) => Err(EngineError::LttbExtensionUnavailable {
                message: "LOAD lttb from PULSEON_LTTB_EXTENSION_PATH did not register lttb"
                    .to_owned(),
            }),
            Err(source) => Err(EngineError::LttbExtensionUnavailable {
                message: source.to_string(),
            }),
        }
    }

    fn lttb_function_available(&self) -> bool {
        self.connection
            .query_row(
                "SELECT count(*) FROM (SELECT lttb(1::BIGINT, 1::DOUBLE, 1::BIGINT))",
                [],
                |row| row.get::<_, i64>(0),
            )
            .is_ok()
    }
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

fn sql_string_literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn lttb_auto_install_allowed(value: Option<&std::ffi::OsStr>) -> bool {
    value
        .and_then(std::ffi::OsStr::to_str)
        .is_some_and(|value| matches!(value, "1" | "true" | "TRUE" | "yes" | "YES" | "on" | "ON"))
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;

    use super::*;

    fn allows(value: &str) -> bool {
        lttb_auto_install_allowed(Some(OsStr::new(value)))
    }

    #[test]
    fn lttb_auto_install_is_opt_in() {
        assert!(!lttb_auto_install_allowed(None));
        assert!(!allows("0"));
        assert!(allows("1"));
        assert!(allows("true"));
        assert!(allows("yes"));
        assert!(allows("on"));
    }
}
