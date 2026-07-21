use pulseon_model::metric::MetricKey;
use pulseon_model::run::RunId;
use pulseon_model::types::ProjectId;

use crate::model::CatalogSnapshot;
use crate::query::CurveSnapshot;
use crate::worker::{Generation, ReadEvent, ReadKind, ReadRequest, ReadSnapshot};

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

/// GPUI-independent state reconciler for native read snapshots.
#[derive(Default)]
pub struct ViewerCore {
    selection: ViewerSelection,
    catalog: Option<CatalogSnapshot>,
    overview: Option<CurveSnapshot>,
    detail: Option<CurveSnapshot>,
    expected: [Option<Generation>; 3],
    last_error: Option<String>,
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
            Ok(ReadSnapshot::Overview(snapshot)) => self.overview = Some(snapshot),
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
            self.selection
                .run_ids
                .retain(|run_id| snapshot.runs.iter().any(|run| &run.run_id == run_id));
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
}
