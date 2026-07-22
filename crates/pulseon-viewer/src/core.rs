use pulseon_chart_core::{AxisRange, BrushState};
use pulseon_model::alignment::{AlignmentAxis, AlignmentViewport};
use pulseon_model::metric::MetricKey;
use pulseon_model::run::{Run, RunId, RunStatus};
use pulseon_model::types::ProjectId;

use crate::model::CatalogSnapshot;
use crate::query::CurveSnapshot;
use crate::worker::{Generation, ReadEvent, ReadKind, ReadRequest, ReadSnapshot};

pub const MAX_SELECTED_RUNS: usize = 10;

/// Matches a Run by name, identifier, or lifecycle status.
pub fn run_matches_filter(run: &Run, query: &str) -> bool {
    run_fields_match_filter(
        &run.name,
        run.run_id.as_str(),
        match run.status {
            RunStatus::Running => "running",
            RunStatus::Finished => "finished",
            RunStatus::Failed => "failed",
        },
        query,
    )
}

fn run_fields_match_filter(name: &str, run_id: &str, status: &str, query: &str) -> bool {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return true;
    }
    name.to_lowercase().contains(&query)
        || run_id.to_lowercase().contains(&query)
        || status.contains(&query)
}

/// Stable identities selected by the viewer independently of rendered widgets.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ViewerSelection {
    pub project_id: Option<ProjectId>,
    pub run_ids: Vec<RunId>,
    pub metric_key: Option<MetricKey>,
}

/// Whether a worker event changed the current viewer state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ApplyOutcome {
    Applied,
    IgnoredStale,
    IgnoredMismatched,
}

/// Invalid user selection transitions.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum SelectionError {
    #[error("at most {MAX_SELECTED_RUNS} Runs can be selected")]
    RunLimit,
}

/// GPUI-independent state reconciler for native read snapshots.
pub struct ViewerCore {
    selection: ViewerSelection,
    axis: AlignmentAxis,
    brush: Option<BrushState>,
    catalog: Option<CatalogSnapshot>,
    overview: Option<CurveSnapshot>,
    detail: Option<CurveSnapshot>,
    expected: [Option<Generation>; 3],
    last_error: Option<String>,
}

impl Default for ViewerCore {
    fn default() -> Self {
        Self {
            selection: ViewerSelection::default(),
            axis: AlignmentAxis::Step,
            brush: None,
            catalog: None,
            overview: None,
            detail: None,
            expected: [None; 3],
            last_error: None,
        }
    }
}

impl ViewerCore {
    pub fn new(selection: ViewerSelection) -> Self {
        Self {
            selection,
            ..Self::default()
        }
    }

    pub const fn selection(&self) -> &ViewerSelection {
        &self.selection
    }

    pub const fn axis(&self) -> AlignmentAxis {
        self.axis
    }

    pub const fn brush(&self) -> Option<BrushState> {
        self.brush
    }

    pub fn brush_mut(&mut self) -> Option<&mut BrushState> {
        self.brush.as_mut()
    }

    pub fn selected_viewport(&self) -> Option<AlignmentViewport> {
        let selected = self.brush?.selected();
        AlignmentViewport::new(
            selected.start().floor() as i64,
            selected.end().ceil() as i64,
        )
        .ok()
    }

    pub const fn catalog(&self) -> Option<&CatalogSnapshot> {
        self.catalog.as_ref()
    }

    pub const fn overview(&self) -> Option<&CurveSnapshot> {
        self.overview.as_ref()
    }

    pub const fn detail(&self) -> Option<&CurveSnapshot> {
        self.detail.as_ref()
    }

    pub fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }

    /// Clears all state associated with the currently open source.
    pub fn reset_source(&mut self) {
        *self = Self::default();
    }

    pub fn select_project(&mut self, project_id: Option<ProjectId>) {
        if self.selection.project_id == project_id {
            return;
        }
        self.selection = ViewerSelection {
            project_id,
            ..ViewerSelection::default()
        };
        if let Some(catalog) = self.catalog.as_mut() {
            catalog.runs.clear();
            catalog.metric_keys.clear();
        }
        self.clear_curves();
    }

    /// Toggles one Run while enforcing the product's comparison limit.
    ///
    /// # Errors
    ///
    /// Returns [`SelectionError::RunLimit`] when adding an eleventh Run.
    pub fn toggle_run(&mut self, run_id: RunId) -> Result<bool, SelectionError> {
        if let Some(index) = self
            .selection
            .run_ids
            .iter()
            .position(|selected| selected == &run_id)
        {
            self.selection.run_ids.remove(index);
            self.clear_curves();
            return Ok(false);
        }
        if self.selection.run_ids.len() == MAX_SELECTED_RUNS {
            return Err(SelectionError::RunLimit);
        }
        self.selection.run_ids.push(run_id);
        self.clear_curves();
        Ok(true)
    }

    pub fn select_metric(&mut self, metric_key: Option<MetricKey>) {
        if self.selection.metric_key != metric_key {
            self.selection.metric_key = metric_key;
            self.clear_curves();
        }
    }

    pub fn select_axis(&mut self, axis: AlignmentAxis) {
        if self.axis != axis {
            self.axis = axis;
            self.clear_curves();
        }
    }

    pub fn reset_view(&mut self) -> bool {
        let Some(brush) = self.brush.as_mut() else {
            return false;
        };
        brush.reset();
        true
    }

    /// Marks one request stream pending without clearing its current snapshot.
    pub fn begin(&mut self, generation: Generation, request: &ReadRequest) {
        self.expected[kind_index(request.kind())] = Some(generation);
        self.last_error = None;
    }

    pub fn is_pending(&self, kind: ReadKind) -> bool {
        self.expected[kind_index(kind)].is_some()
    }

    /// Applies only the result currently expected for its independent stream.
    pub fn apply(&mut self, event: ReadEvent) -> ApplyOutcome {
        let index = kind_index(event.kind);
        if self.expected[index] != Some(event.generation) {
            return ApplyOutcome::IgnoredStale;
        }
        if event
            .result
            .as_ref()
            .is_ok_and(|snapshot| snapshot.kind() != event.kind)
        {
            return ApplyOutcome::IgnoredMismatched;
        }
        self.expected[index] = None;
        match event.result {
            Ok(ReadSnapshot::Catalog(snapshot)) => self.apply_catalog(snapshot),
            Ok(ReadSnapshot::Overview(snapshot)) => self.apply_overview(snapshot),
            Ok(ReadSnapshot::Detail(snapshot)) => self.detail = Some(snapshot),
            Err(error) => self.last_error = Some(error.to_string()),
        }
        ApplyOutcome::Applied
    }

    fn apply_catalog(&mut self, snapshot: CatalogSnapshot) {
        let project_exists = self
            .selection
            .project_id
            .as_ref()
            .is_some_and(|project_id| {
                snapshot
                    .projects
                    .iter()
                    .any(|project| &project.project_id == project_id)
            });
        let previous = self.selection.clone();
        if !project_exists {
            self.selection = ViewerSelection::default();
        } else {
            self.selection.run_ids = snapshot
                .runs
                .iter()
                .filter(|run| self.selection.run_ids.contains(&run.run_id))
                .map(|run| run.run_id.clone())
                .collect();
            if self
                .selection
                .metric_key
                .as_ref()
                .is_some_and(|metric_key| !snapshot.metric_keys.iter().any(|key| key == metric_key))
            {
                self.selection.metric_key = None;
            }
        }
        if self.selection != previous {
            self.overview = None;
            self.detail = None;
            self.expected[kind_index(ReadKind::Overview)] = None;
            self.expected[kind_index(ReadKind::Detail)] = None;
        }
        self.catalog = Some(snapshot);
    }

    fn apply_overview(&mut self, snapshot: CurveSnapshot) {
        self.brush = snapshot.real_range.and_then(|range| {
            let home = AxisRange::new(range.start() as f64, range.end() as f64).ok()?;
            let previous = self.brush.map(BrushState::selected);
            let mut brush = BrushState::new(home).ok()?;
            if let Some(selected) = previous {
                brush.resize_start(selected.start()).ok()?;
                brush.resize_end(selected.end()).ok()?;
            }
            Some(brush)
        });
        self.overview = Some(snapshot);
    }

    fn clear_curves(&mut self) {
        self.brush = None;
        self.overview = None;
        self.detail = None;
        self.expected[kind_index(ReadKind::Overview)] = None;
        self.expected[kind_index(ReadKind::Detail)] = None;
        self.last_error = None;
    }
}

const fn kind_index(kind: ReadKind) -> usize {
    match kind {
        ReadKind::Catalog => 0,
        ReadKind::Overview => 1,
        ReadKind::Detail => 2,
    }
}

#[cfg(test)]
mod tests {
    use pulseon_model::alignment::AlignmentViewport;

    use super::*;

    fn curves() -> CurveSnapshot {
        CurveSnapshot {
            viewport: AlignmentViewport::new(0, 1).expect("test viewport should be valid"),
            point_budget: 2_000,
            real_range: None,
            series: Vec::new(),
        }
    }

    fn detail_event(generation: u64) -> ReadEvent {
        ReadEvent {
            generation: Generation(generation),
            kind: ReadKind::Detail,
            result: Ok(ReadSnapshot::Detail(curves())),
        }
    }

    #[test]
    fn detail_stays_visible_while_pending_and_stale_results_are_ignored() {
        let mut core = ViewerCore::default();
        let request = ReadRequest::Detail(crate::query::DetailRequest {
            selection: crate::query::CurveSelection {
                run_ids: Vec::new(),
                metric_key: MetricKey::from_string("loss"),
                axis: pulseon_model::alignment::AlignmentAxis::Step,
            },
            viewport: AlignmentViewport::new(0, 1).expect("test viewport should be valid"),
            physical_width: 1_000,
        });
        core.begin(Generation(1), &request);
        assert_eq!(core.apply(detail_event(1)), ApplyOutcome::Applied);
        core.begin(Generation(2), &request);

        assert!(core.detail().is_some() && core.is_pending(ReadKind::Detail));
        assert_eq!(core.apply(detail_event(1)), ApplyOutcome::IgnoredStale);
        assert!(core.detail().is_some() && core.is_pending(ReadKind::Detail));
    }

    #[test]
    fn refresh_removes_missing_selection_and_curve_snapshots() {
        let mut core = ViewerCore::new(ViewerSelection {
            project_id: Some(ProjectId::from_string("removed")),
            run_ids: vec![RunId::from_string("run-1")],
            metric_key: Some(MetricKey::from_string("loss")),
        });
        core.detail = Some(curves());
        let request = ReadRequest::Discover(crate::model::DiscoveryRequest::default());
        core.begin(Generation(1), &request);
        let event = ReadEvent {
            generation: Generation(1),
            kind: ReadKind::Catalog,
            result: Ok(ReadSnapshot::Catalog(CatalogSnapshot {
                projects: Vec::new(),
                runs: Vec::new(),
                metric_keys: Vec::new(),
            })),
        };

        assert_eq!(core.apply(event), ApplyOutcome::Applied);
        assert_eq!(core.selection(), &ViewerSelection::default());
        assert!(core.detail().is_none());
    }

    #[test]
    fn selection_transitions_clear_only_dependent_state() {
        let mut core = ViewerCore::default();
        core.select_project(Some(ProjectId::from_string("project-1")));
        assert!(
            core.toggle_run(RunId::from_string("run-1"))
                .expect("first Run should be selectable")
        );
        core.select_metric(Some(MetricKey::from_string("loss")));

        core.select_axis(AlignmentAxis::ElapsedTime);

        assert_eq!(core.axis(), AlignmentAxis::ElapsedTime);
        assert_eq!(core.selection().run_ids.len(), 1);
        assert_eq!(
            core.selection().metric_key.as_ref().map(MetricKey::as_str),
            Some("loss")
        );
        assert!(core.overview().is_none() && core.detail().is_none());
    }

    #[test]
    fn run_selection_enforces_the_ten_run_limit() {
        let mut core = ViewerCore::default();
        for index in 0..MAX_SELECTED_RUNS {
            core.toggle_run(RunId::from_string(format!("run-{index}")))
                .expect("first ten Runs should be selectable");
        }

        assert_eq!(
            core.toggle_run(RunId::from_string("run-10")),
            Err(SelectionError::RunLimit)
        );
        assert_eq!(core.selection().run_ids.len(), MAX_SELECTED_RUNS);
    }

    #[test]
    fn run_filter_matches_name_id_and_status_without_case() {
        assert!(
            ["loss", "run-42", "FAILED", ""]
                .into_iter()
                .all(|query| run_fields_match_filter("Loss Baseline", "RUN-42", "failed", query))
        );
        assert!(!run_fields_match_filter(
            "Loss Baseline",
            "RUN-42",
            "failed",
            "running"
        ));
    }
}
