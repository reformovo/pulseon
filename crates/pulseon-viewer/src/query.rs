use crate::source::ReadSession;
use pulseon_chart_core::{DataPoint, Series, SeriesId};
use pulseon_core::engine::EngineError;
use pulseon_core::engine::query::NativeQueryStore;
use pulseon_model::alignment::{
    AlignedMetricResult, AlignmentAxis, AlignmentQuery, AlignmentQueryError, AlignmentReduction,
    AlignmentViewport,
};
use pulseon_model::comparison::EvidenceCompleteness;
use pulseon_model::metric::MetricKey;
use pulseon_model::run::{Run, RunId};
use pulseon_storage::StorageError;

/// Shared series selection for overview and detail queries.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CurveSelection {
    pub run_ids: Vec<RunId>,
    pub metric_key: MetricKey,
    pub axis: AlignmentAxis,
}

/// Full non-negative-axis overview query.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OverviewRequest {
    pub selection: CurveSelection,
    pub physical_width: u32,
}

/// Closed-viewport detail query.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DetailRequest {
    pub selection: CurveSelection,
    pub viewport: AlignmentViewport,
    pub physical_width: u32,
}

/// One Run's evidence and optional drawable chart series.
#[derive(Clone, Debug, PartialEq)]
pub struct CurveSeriesSnapshot {
    pub run: Run,
    pub evidence: AlignedMetricResult,
    pub chart_series: Option<Series>,
}

/// Immutable reduced curves returned for one requested viewport.
#[derive(Clone, Debug, PartialEq)]
pub struct CurveSnapshot {
    pub viewport: AlignmentViewport,
    pub point_budget: u32,
    pub real_range: Option<AlignmentViewport>,
    pub series: Vec<CurveSeriesSnapshot>,
}

/// Failures while planning or executing a viewer curve query.
#[derive(Debug, thiserror::Error)]
pub enum QueryError {
    #[error(transparent)]
    Alignment(#[from] AlignmentQueryError),
    #[error(transparent)]
    Chart(#[from] pulseon_chart_core::ChartError),
    #[error(transparent)]
    Core(#[from] EngineError),
    #[error(transparent)]
    Storage(#[from] StorageError),
}

impl ReadSession {
    /// Queries the full non-negative axis at one point per physical pixel.
    ///
    /// # Errors
    ///
    /// Returns [`QueryError`] when the request cannot be planned or executed.
    pub fn query_overview(&self, request: &OverviewRequest) -> Result<CurveSnapshot, QueryError> {
        let viewport = AlignmentViewport::new(0, i64::MAX)?;
        query_curves(
            self,
            &request.selection,
            viewport,
            overview_budget(request.physical_width),
        )
    }

    /// Queries a closed detail viewport at two points per physical pixel.
    ///
    /// # Errors
    ///
    /// Returns [`QueryError`] when the request cannot be planned or executed.
    pub fn query_detail(&self, request: &DetailRequest) -> Result<CurveSnapshot, QueryError> {
        query_curves(
            self,
            &request.selection,
            request.viewport,
            detail_budget(request.physical_width),
        )
    }
}

fn overview_budget(physical_width: u32) -> u32 {
    physical_width.clamp(500, 2_000)
}

fn detail_budget(physical_width: u32) -> u32 {
    physical_width.saturating_mul(2).clamp(2_000, 10_000)
}

fn query_curves(
    session: &ReadSession,
    selection: &CurveSelection,
    viewport: AlignmentViewport,
    point_budget: u32,
) -> Result<CurveSnapshot, QueryError> {
    let runs = session.connection().get_runs(&selection.run_ids)?;
    let store = NativeQueryStore::new(session.connection());
    let reduction = AlignmentReduction::screen_budget(point_budget, 1)?;
    let mut real_bounds: Option<(i64, i64)> = None;
    let mut series = Vec::with_capacity(runs.len());
    for run in runs {
        let evidence = store.query_aligned_metric(
            &AlignmentQuery {
                run_id: run.run_id.clone(),
                metric_key: selection.metric_key.clone(),
                axis: selection.axis,
                viewport,
                reduction,
            },
            run.status,
        )?;
        let drawable = matches!(
            evidence.completeness,
            EvidenceCompleteness::Complete | EvidenceCompleteness::Partial
        );
        let chart_series = drawable
            .then(|| {
                Series::new(
                    SeriesId::new(run.run_id.as_str())?,
                    evidence
                        .points
                        .iter()
                        .map(|point| DataPoint::new(point.axis_value as f64, point.point.value_f64))
                        .collect(),
                )
            })
            .transpose()?;
        if drawable {
            for axis_value in evidence
                .points
                .iter()
                .map(|point| point.axis_value)
                .filter(|value| *value >= viewport.start() && *value <= viewport.end())
            {
                real_bounds = Some(match real_bounds {
                    Some((start, end)) => (start.min(axis_value), end.max(axis_value)),
                    None => (axis_value, axis_value),
                });
            }
        }
        series.push(CurveSeriesSnapshot {
            run,
            evidence,
            chart_series,
        });
    }
    let real_range = real_bounds
        .map(|(start, end)| AlignmentViewport::new(start, end))
        .transpose()?;
    Ok(CurveSnapshot {
        viewport,
        point_budget,
        real_range,
        series,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screen_budgets_clamp_density_and_overflow() {
        assert_eq!(
            [
                overview_budget(1),
                overview_budget(900),
                overview_budget(9_000),
                detail_budget(1),
                detail_budget(2_500),
                detail_budget(u32::MAX)
            ],
            [500, 900, 2_000, 2_000, 5_000, 10_000]
        );
    }
}
