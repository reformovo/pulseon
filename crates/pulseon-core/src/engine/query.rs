use pulseon_storage::{ProjectConnection, ProjectMetricReader};

use crate::engine::EngineError;
use crate::model::metric::{
    MetricAggregate, MetricKey, MetricPoint, MetricQuery, ReductionPolicy, Step,
};
use crate::model::run::{RunId, RunStatus};

pub struct NativeQueryStore<'connection> {
    source: QuerySource<'connection>,
}

enum QuerySource<'connection> {
    Project(&'connection ProjectConnection),
    #[cfg(test)]
    DuckDb(&'connection duckdb::Connection),
}

pub type MetricQueryResult = pulseon_model::metric::MetricQueryResult;

impl<'connection> NativeQueryStore<'connection> {
    pub const fn new(connection: &'connection ProjectConnection) -> Self {
        Self {
            source: QuerySource::Project(connection),
        }
    }

    #[cfg(test)]
    pub const fn from_duckdb(connection: &'connection duckdb::Connection) -> Self {
        Self {
            source: QuerySource::DuckDb(connection),
        }
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
        self.query_metric_with_metadata(run_id, metric_key, start_step, end_step, max_points)
            .map(|result| result.points)
    }

    pub fn query_metric_with_metadata(
        &self,
        run_id: &RunId,
        metric_key: &MetricKey,
        start_step: Option<Step>,
        end_step: Option<Step>,
        max_points: Option<usize>,
    ) -> Result<MetricQueryResult, EngineError> {
        let reduction = match max_points {
            None => ReductionPolicy::Full,
            Some(max_points) => ReductionPolicy::lttb(max_points)
                .map_err(|_| EngineError::MetricQueryMaxPointsTooSmall { max_points })?,
        };
        // Preserve the shipped Python behavior for empty or reversed ranges.
        let query = MetricQuery {
            run_id: run_id.clone(),
            metric_key: metric_key.clone(),
            start_step,
            end_step,
            reduction,
        };
        Ok(self.reader().query_metric(&query)?)
    }

    pub fn metric_aggregate(
        &self,
        run_id: &RunId,
        metric_key: &MetricKey,
    ) -> Result<MetricAggregate, EngineError> {
        Ok(self.reader().metric_aggregate(run_id, metric_key)?)
    }

    pub fn query_metric_summaries(
        &self,
        run_ids: &[RunId],
        metric_key: &MetricKey,
    ) -> Result<Vec<MetricAggregate>, EngineError> {
        Ok(self.reader().query_metric_summaries(run_ids, metric_key)?)
    }

    pub fn list_metrics(
        &self,
        run_id: &RunId,
        run_status: RunStatus,
    ) -> Result<Vec<MetricAggregate>, EngineError> {
        Ok(self.reader().list_metrics(run_id, run_status)?)
    }

    fn reader(&self) -> ProjectMetricReader<'_> {
        match self.source {
            QuerySource::Project(connection) => ProjectMetricReader::new(connection),
            #[cfg(test)]
            QuerySource::DuckDb(connection) => ProjectMetricReader::new(connection),
        }
    }
}
