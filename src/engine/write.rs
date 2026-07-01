use std::path::Path;

use crate::engine::EngineError;
use crate::engine::time::{current_timestamp, timestamp_as_rfc3339};
use crate::engine::write_rows::{
    StoredMetricAggregate, StoredMetricPoint, StoredRun, status_as_str,
};
use crate::model::metric::{MetricAggregate, MetricKey, MetricPoint, Step};
use crate::model::run::{Run, RunId, RunStatus};
use crate::model::types::ProjectId;

pub struct NativeWriteStore<'connection> {
    connection: &'connection duckdb::Connection,
}

impl<'connection> NativeWriteStore<'connection> {
    pub const fn new(connection: &'connection duckdb::Connection) -> Self {
        Self { connection }
    }

    pub fn create_run(
        &self,
        project_id: &ProjectId,
        name: &str,
        run_id: Option<RunId>,
    ) -> Result<Run, EngineError> {
        let run_id = run_id.unwrap_or_else(|| RunId::from_string(uuid::Uuid::new_v4().to_string()));
        if self.run_exists(&run_id)? {
            return Err(EngineError::RunAlreadyExists {
                run_id: run_id.as_str().to_owned(),
            });
        }

        let now = current_timestamp("created_at")?;
        self.connection.execute(
            "INSERT INTO dl.runs
                 (run_id, project_id, name, status, created_at, started_at, finished_at)
             VALUES (?, ?, ?, ?, ?, ?, NULL)",
            (
                run_id.as_str(),
                project_id.as_str(),
                name,
                status_as_str(RunStatus::Running),
                timestamp_as_rfc3339(now),
                timestamp_as_rfc3339(now),
            ),
        )?;

        Ok(Run {
            run_id,
            project_id: project_id.clone(),
            name: name.to_owned(),
            status: RunStatus::Running,
            created_at: now,
            started_at: now,
            finished_at: None,
        })
    }

    pub fn resume_run(&self, run_id: &RunId) -> Result<Run, EngineError> {
        let result = self.connection.query_row(
            "SELECT run_id, project_id, name, status, epoch_ms(created_at), epoch_ms(started_at),
                    epoch_ms(finished_at)
             FROM dl.runs
             WHERE run_id = ?",
            [run_id.as_str()],
            |row| {
                Ok(StoredRun {
                    run_id: row.get(0)?,
                    project_id: row.get(1)?,
                    name: row.get(2)?,
                    status: row.get(3)?,
                    created_at_millis: row.get(4)?,
                    started_at_millis: row.get(5)?,
                    finished_at_millis: row.get(6)?,
                })
            },
        );
        let stored = match result {
            Ok(stored) => stored,
            Err(duckdb::Error::QueryReturnedNoRows) => {
                return Err(EngineError::RunNotFound {
                    run_id: run_id.as_str().to_owned(),
                });
            }
            Err(source) => return Err(source.into()),
        };

        stored.into_run()
    }

    pub fn log_metric(
        &self,
        run_id: &RunId,
        metric_key: &MetricKey,
        value_f64: f64,
    ) -> Result<MetricPoint, EngineError> {
        let step = self.next_metric_step(run_id, metric_key)?;
        self.log_metric_at_step(run_id, metric_key, step, value_f64)
    }

    pub fn log_metric_at_step(
        &self,
        run_id: &RunId,
        metric_key: &MetricKey,
        step: Step,
        value_f64: f64,
    ) -> Result<MetricPoint, EngineError> {
        let timestamp = current_timestamp("timestamp")?;
        let ingested_at = current_timestamp("ingested_at")?;
        self.connection.execute(
            "INSERT INTO dl.metric_points
                 (run_id, metric_key, step, timestamp, value_f64, ingested_at)
             VALUES (?, ?, ?, ?, ?, ?)",
            (
                run_id.as_str(),
                metric_key.as_str(),
                step.value(),
                timestamp_as_rfc3339(timestamp),
                value_f64,
                timestamp_as_rfc3339(ingested_at),
            ),
        )?;
        self.refresh_metric_aggregate(run_id, metric_key)?;

        Ok(MetricPoint {
            run_id: run_id.clone(),
            metric_key: metric_key.clone(),
            step,
            timestamp,
            value_f64,
            ingested_at,
        })
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

    pub fn metric_aggregate(
        &self,
        run_id: &RunId,
        metric_key: &MetricKey,
    ) -> Result<MetricAggregate, EngineError> {
        let stored = self.connection.query_row(
            "SELECT run_id, metric_key, effective_count, last_step, last_value_f64,
                    min_value_f64, max_value_f64
             FROM dl.metric_aggregates
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
             SELECT summary.run_id, summary.metric_key, summary.effective_count,
                    summary.last_step, summary.last_value_f64, summary.min_value_f64,
                    summary.max_value_f64
             FROM dl.metric_aggregates AS summary
             JOIN requested ON summary.run_id = requested.run_id
             WHERE summary.metric_key = ?
             ORDER BY requested.ordinal"
        );

        let mut params: Vec<&str> = Vec::with_capacity(run_ids.len() + 1);
        params.extend(run_ids.iter().map(RunId::as_str));
        params.push(metric_key.as_str());
        let mut statement = self.connection.prepare(&sql)?;
        let rows = statement.query_map(duckdb::params_from_iter(params), |row| {
            Ok(StoredMetricAggregate {
                run_id: row.get(0)?,
                metric_key: row.get(1)?,
                effective_count: row.get(2)?,
                last_step: row.get(3)?,
                last_value_f64: row.get(4)?,
                min_value_f64: row.get(5)?,
                max_value_f64: row.get(6)?,
            })
        })?;

        rows.map(|row| Ok(row?.into_metric_aggregate())).collect()
    }

    pub fn repair_metric_aggregate(
        &self,
        run_id: &RunId,
        metric_key: &MetricKey,
    ) -> Result<(), EngineError> {
        self.refresh_metric_aggregate(run_id, metric_key)
    }

    fn ensure_lttb_extension_loaded(&self) -> Result<(), EngineError> {
        if self.lttb_function_available() {
            return Ok(());
        }

        match self.connection.execute_batch("LOAD lttb;") {
            Ok(()) if self.lttb_function_available() => Ok(()),
            Ok(()) | Err(_) => self.install_and_load_lttb_extension(),
        }
    }

    fn install_and_load_lttb_extension(&self) -> Result<(), EngineError> {
        if let Some(path) = std::env::var_os("PULSEON_LTTB_EXTENSION_PATH") {
            return self.load_lttb_extension_from_path(Path::new(&path));
        }

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

    fn run_exists(&self, run_id: &RunId) -> Result<bool, EngineError> {
        let count: i64 = self.connection.query_row(
            "SELECT count(*) FROM dl.runs WHERE run_id = ?",
            [run_id.as_str()],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }

    fn next_metric_step(
        &self,
        run_id: &RunId,
        metric_key: &MetricKey,
    ) -> Result<Step, EngineError> {
        let next: i64 = self.connection.query_row(
            "SELECT coalesce(max(step) + 1, 0)
             FROM dl.metric_points
             WHERE run_id = ?
               AND metric_key = ?",
            (run_id.as_str(), metric_key.as_str()),
            |row| row.get(0),
        )?;
        Ok(Step::new(next))
    }

    fn refresh_metric_aggregate(
        &self,
        run_id: &RunId,
        metric_key: &MetricKey,
    ) -> Result<(), EngineError> {
        self.connection.execute(
            "DELETE FROM dl.metric_aggregates
             WHERE run_id = ?
               AND metric_key = ?",
            (run_id.as_str(), metric_key.as_str()),
        )?;
        self.connection.execute(
            "INSERT INTO dl.metric_aggregates
                 (run_id, metric_key, effective_count, last_step, last_value_f64,
                  min_value_f64, max_value_f64)
             SELECT run_id, metric_key, count(*), arg_max(step, step), arg_max(value_f64, step),
                    min(value_f64), max(value_f64)
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
             GROUP BY run_id, metric_key",
            (run_id.as_str(), metric_key.as_str()),
        )?;
        Ok(())
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

fn sql_string_literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}
