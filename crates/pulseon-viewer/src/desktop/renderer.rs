use std::collections::HashMap;
use std::sync::Arc;

use gpui::{
    Bounds, Path, PathBuilder, Pixels, Point, Rgba, WindowAppearance, canvas, fill, point, px, rgb,
    rgba, size,
};
use pulseon_chart_core::{
    AxisRange, BrushState, CanvasSize, LinearScale, PathCache, ScreenPoint, Viewport,
    hit_test_point, visible_y_range_for,
};
use pulseon_model::alignment::AlignmentAxis;
use pulseon_model::comparison::EvidenceCompleteness;
use pulseon_viewer::query::CurveSnapshot;

#[derive(Clone, Debug)]
pub struct HoverPoint {
    pub run_name: String,
    pub metric_key: String,
    pub axis_value: i64,
    pub step: i64,
    pub value: f64,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct GpuiPathKey {
    kind: ChartKind,
    revision: u64,
    viewport: [u64; 4],
    bounds: [u32; 4],
    dark: bool,
    partial: bool,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum ChartKind {
    Detail,
    Overview,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BrushDragTarget {
    Start,
    End,
    Window,
}

#[derive(Default)]
pub struct ChartAdapter {
    detail_projection_cache: PathCache,
    overview_projection_cache: PathCache,
    gpui_paths: HashMap<(ChartKind, String), (GpuiPathKey, Path<Pixels>)>,
    detail_bounds: Option<Bounds<Pixels>>,
    overview_bounds: Option<Bounds<Pixels>>,
}

struct PreparedChart {
    paths: Vec<(Path<Pixels>, Rgba)>,
}

impl ChartAdapter {
    pub fn clear(&mut self) {
        self.detail_projection_cache.clear();
        self.overview_projection_cache.clear();
        self.gpui_paths.clear();
        self.detail_bounds = None;
        self.overview_bounds = None;
    }

    fn prepare(
        &mut self,
        snapshot: &CurveSnapshot,
        kind: ChartKind,
        revision: u64,
        viewport: Viewport,
        bounds: Bounds<Pixels>,
        appearance: WindowAppearance,
    ) -> PreparedChart {
        match kind {
            ChartKind::Detail => self.detail_bounds = Some(bounds),
            ChartKind::Overview => self.overview_bounds = Some(bounds),
        }
        let Ok(canvas) =
            CanvasSize::new(f64::from(bounds.size.width), f64::from(bounds.size.height))
        else {
            return PreparedChart { paths: Vec::new() };
        };
        let dark = matches!(
            appearance,
            WindowAppearance::Dark | WindowAppearance::VibrantDark
        );
        let mut paths = Vec::new();
        for (index, curve) in snapshot.series.iter().enumerate() {
            let Some(series) = curve.chart_series.as_ref() else {
                continue;
            };
            let partial = curve.evidence.completeness == EvidenceCompleteness::Partial;
            let key = GpuiPathKey {
                kind,
                revision,
                viewport: [
                    viewport.x.start().to_bits(),
                    viewport.x.end().to_bits(),
                    viewport.y.start().to_bits(),
                    viewport.y.end().to_bits(),
                ],
                bounds: [
                    f32::from(bounds.origin.x).to_bits(),
                    f32::from(bounds.origin.y).to_bits(),
                    f32::from(bounds.size.width).to_bits(),
                    f32::from(bounds.size.height).to_bits(),
                ],
                dark,
                partial,
            };
            let cache_id = (kind, series.id().as_str().to_owned());
            let path = if let Some((cached_key, path)) = self.gpui_paths.get(&cache_id)
                && cached_key == &key
            {
                path.clone()
            } else {
                let projection_cache = match kind {
                    ChartKind::Detail => &mut self.detail_projection_cache,
                    ChartKind::Overview => &mut self.overview_projection_cache,
                };
                let Ok(points) = projection_cache.path_for(series, revision, viewport, canvas)
                else {
                    continue;
                };
                let mut builder = PathBuilder::stroke(px(2.));
                if partial {
                    builder = builder.dash_array(&[px(7.), px(4.)]);
                }
                for (point_index, projected) in points.iter().enumerate() {
                    let position = point(
                        bounds.origin.x + px(projected.x as f32),
                        bounds.origin.y + px(projected.y as f32),
                    );
                    if point_index == 0 {
                        builder.move_to(position);
                    } else {
                        builder.line_to(position);
                    }
                }
                let Ok(path) = builder.build() else {
                    continue;
                };
                self.gpui_paths.insert(cache_id, (key, path.clone()));
                path
            };
            paths.push((path, series_color(index)));
        }
        PreparedChart { paths }
    }

    pub fn hit_test(
        &self,
        snapshot: &CurveSnapshot,
        viewport: Viewport,
        cursor: Point<Pixels>,
    ) -> Option<HoverPoint> {
        let bounds = self.detail_bounds?;
        let canvas =
            CanvasSize::new(f64::from(bounds.size.width), f64::from(bounds.size.height)).ok()?;
        let local = ScreenPoint::new(
            f64::from(cursor.x - bounds.origin.x),
            f64::from(cursor.y - bounds.origin.y),
        );
        let mut nearest = None;
        for curve in &snapshot.series {
            let Some(series) = curve.chart_series.as_ref() else {
                continue;
            };
            let Some(hit) = hit_test_point(series, viewport, canvas, local, 8.).ok()? else {
                continue;
            };
            if nearest
                .as_ref()
                .is_some_and(|(distance, _): &(f64, HoverPoint)| *distance <= hit.distance)
            {
                continue;
            }
            let aligned = curve.evidence.points.get(hit.point_index)?;
            nearest = Some((
                hit.distance,
                HoverPoint {
                    run_name: curve.run.name.clone(),
                    metric_key: aligned.point.metric_key.as_str().to_owned(),
                    axis_value: aligned.axis_value,
                    step: aligned.point.step.value(),
                    value: aligned.point.value_f64,
                },
            ));
        }
        nearest.map(|(_, point)| point)
    }

    pub fn detail_axis_at(&self, range: AxisRange, cursor: Point<Pixels>) -> Option<f64> {
        axis_at(self.detail_bounds?, range, cursor)
    }

    pub fn detail_pan_delta(
        &self,
        range: AxisRange,
        previous_x: f64,
        cursor: Point<Pixels>,
    ) -> Option<f64> {
        let bounds = self.detail_bounds?;
        let current = axis_at(bounds, range, cursor)?;
        let scale = LinearScale::new(
            range,
            f64::from(bounds.origin.x),
            f64::from(bounds.origin.x + bounds.size.width),
        )
        .ok()?;
        Some(scale.invert(previous_x) - current)
    }

    pub fn overview_axis_at(&self, brush: BrushState, cursor: Point<Pixels>) -> Option<f64> {
        axis_at(self.overview_bounds?, brush.home(), cursor)
    }

    pub fn brush_target(
        &self,
        brush: BrushState,
        cursor: Point<Pixels>,
    ) -> Option<BrushDragTarget> {
        let bounds = self.overview_bounds?;
        let scale = LinearScale::new(
            brush.home(),
            f64::from(bounds.origin.x),
            f64::from(bounds.origin.x + bounds.size.width),
        )
        .ok()?;
        let x = f64::from(cursor.x);
        let start = scale.map(brush.selected().start());
        let end = scale.map(brush.selected().end());
        if (x - start).abs() <= 8. {
            Some(BrushDragTarget::Start)
        } else if (x - end).abs() <= 8. {
            Some(BrushDragTarget::End)
        } else if x > start && x < end {
            Some(BrushDragTarget::Window)
        } else {
            None
        }
    }

    pub fn physical_widths(&self, scale_factor: f32) -> (Option<u32>, Option<u32>) {
        (
            physical_width(self.overview_bounds, scale_factor),
            physical_width(self.detail_bounds, scale_factor),
        )
    }
}

fn axis_at(bounds: Bounds<Pixels>, range: AxisRange, cursor: Point<Pixels>) -> Option<f64> {
    if cursor.x < bounds.origin.x || cursor.x > bounds.origin.x + bounds.size.width {
        return None;
    }
    LinearScale::new(
        range,
        f64::from(bounds.origin.x),
        f64::from(bounds.origin.x + bounds.size.width),
    )
    .ok()
    .map(|scale| scale.invert(f64::from(cursor.x)))
}

fn physical_width(bounds: Option<Bounds<Pixels>>, scale_factor: f32) -> Option<u32> {
    let width = f32::from(bounds?.size.width) * scale_factor;
    (width.is_finite() && width >= 1.).then(|| width.round() as u32)
}

pub fn detail_viewport(snapshot: &CurveSnapshot, selected: Option<AxisRange>) -> Option<Viewport> {
    let snapshot_x = AxisRange::new(
        snapshot.viewport.start() as f64,
        snapshot.viewport.end() as f64,
    )
    .ok()?;
    let x = selected
        .filter(|selected| {
            snapshot_x.start() <= selected.start() && snapshot_x.end() >= selected.end()
        })
        .unwrap_or(snapshot_x);
    visible_y_range_for(
        snapshot
            .series
            .iter()
            .filter_map(|curve| curve.chart_series.as_ref()),
        x,
    )
    .ok()
    .flatten()
    .map(|y| Viewport::new(x, y))
}

pub fn selection_covered(snapshot: &CurveSnapshot, selected: AxisRange) -> bool {
    snapshot.viewport.start() as f64 <= selected.start()
        && snapshot.viewport.end() as f64 >= selected.end()
}

pub fn overview_viewport(snapshot: &CurveSnapshot, brush: BrushState) -> Option<Viewport> {
    visible_y_range_for(
        snapshot
            .series
            .iter()
            .filter_map(|curve| curve.chart_series.as_ref()),
        brush.home(),
    )
    .ok()
    .flatten()
    .map(|y| Viewport::new(brush.home(), y))
}

pub fn detail_canvas(
    adapter: std::rc::Rc<std::cell::RefCell<ChartAdapter>>,
    snapshot: Arc<CurveSnapshot>,
    revision: u64,
    viewport: Viewport,
) -> impl gpui::Styled + gpui::IntoElement {
    canvas(
        move |bounds, window, _| {
            let resized = adapter.borrow().detail_bounds != Some(bounds);
            let prepared = adapter.borrow_mut().prepare(
                &snapshot,
                ChartKind::Detail,
                revision,
                viewport,
                bounds,
                window.appearance(),
            );
            if resized {
                window.request_animation_frame();
            }
            prepared
        },
        move |bounds, prepared, window, _| {
            let grid = rgb(0xe5e7eb);
            for index in 0..=5 {
                let ratio = index as f32 / 5.;
                let x = bounds.origin.x + bounds.size.width * ratio;
                let y = bounds.origin.y + bounds.size.height * ratio;
                window.paint_quad(fill(
                    Bounds::new(point(x, bounds.origin.y), size(px(1.), bounds.size.height)),
                    grid,
                ));
                window.paint_quad(fill(
                    Bounds::new(point(bounds.origin.x, y), size(bounds.size.width, px(1.))),
                    grid,
                ));
            }
            for (path, color) in prepared.paths {
                window.paint_path(path, color);
            }
        },
    )
}

pub fn overview_canvas(
    adapter: std::rc::Rc<std::cell::RefCell<ChartAdapter>>,
    snapshot: Arc<CurveSnapshot>,
    revision: u64,
    viewport: Viewport,
    brush: BrushState,
) -> impl gpui::Styled + gpui::IntoElement {
    canvas(
        move |bounds, window, _| {
            let resized = adapter.borrow().overview_bounds != Some(bounds);
            let prepared = adapter.borrow_mut().prepare(
                &snapshot,
                ChartKind::Overview,
                revision,
                viewport,
                bounds,
                window.appearance(),
            );
            if resized {
                window.request_animation_frame();
            }
            prepared
        },
        move |bounds, prepared, window, _| {
            for (path, color) in prepared.paths {
                window.paint_path(path, color);
            }
            let start_ratio =
                ((brush.selected().start() - brush.home().start()) / brush.home().span()) as f32;
            let end_ratio =
                ((brush.selected().end() - brush.home().start()) / brush.home().span()) as f32;
            let start = bounds.origin.x + bounds.size.width * start_ratio;
            let end = bounds.origin.x + bounds.size.width * end_ratio;
            window.paint_quad(fill(
                Bounds::new(
                    point(start, bounds.origin.y),
                    size(end - start, bounds.size.height),
                ),
                rgba(0x2563eb24),
            ));
            for x in [start, end] {
                window.paint_quad(fill(
                    Bounds::new(
                        point(x - px(4.), bounds.origin.y),
                        size(px(8.), bounds.size.height),
                    ),
                    rgb(0x2563eb),
                ));
            }
        },
    )
}

pub fn series_color(index: usize) -> Rgba {
    const COLORS: [u32; 10] = [
        0x2563eb, 0xdc2626, 0x059669, 0x7c3aed, 0xea580c, 0x0891b2, 0xdb2777, 0x65a30d, 0x4f46e5,
        0x9333ea,
    ];
    rgb(COLORS[index % COLORS.len()])
}

pub const fn axis_value_label(axis: AlignmentAxis) -> &'static str {
    match axis {
        AlignmentAxis::Step => "step",
        AlignmentAxis::ElapsedTime => "elapsed_ms",
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use pulseon_core::engine::client::NativeClient;
    use pulseon_model::alignment::AlignmentViewport;
    use pulseon_model::metric::MetricKey;
    use pulseon_model::run::RunId;
    use pulseon_model::types::ProjectId;
    use pulseon_viewer::query::{CurveSelection, DetailRequest};
    use pulseon_viewer::worker::{Generation, ReadRequest, ReadSnapshot, ReadWorker};

    use super::*;

    fn range(start: f64, end: f64) -> AxisRange {
        AxisRange::new(start, end).expect("test range should be valid")
    }

    #[test]
    fn detail_snapshot_coverage_distinguishes_transient_ranges() {
        let snapshot = CurveSnapshot {
            viewport: AlignmentViewport::new(10, 20).expect("test viewport should be valid"),
            point_budget: 2_000,
            real_range: None,
            series: Vec::new(),
        };

        assert!(selection_covered(&snapshot, range(12., 18.)));
        assert!(!selection_covered(&snapshot, range(5., 15.)));
    }

    #[test]
    fn brush_hit_testing_distinguishes_handles_and_window() {
        let mut adapter = ChartAdapter {
            overview_bounds: Some(Bounds::new(point(px(0.), px(0.)), size(px(100.), px(40.)))),
            ..ChartAdapter::default()
        };
        let mut brush = BrushState::new(range(0., 100.)).expect("brush should be valid");
        brush.resize_start(20.).expect("start should resize");
        brush.resize_end(80.).expect("end should resize");

        assert_eq!(
            [20., 50., 80.].map(|x| adapter.brush_target(brush, point(px(x), px(10.)))),
            [
                Some(BrushDragTarget::Start),
                Some(BrushDragTarget::Window),
                Some(BrushDragTarget::End),
            ]
        );
        adapter.overview_bounds = None;
        assert_eq!(adapter.brush_target(brush, point(px(50.), px(10.))), None);
    }

    #[test]
    fn physical_width_uses_display_scale() {
        let bounds = Bounds::new(point(px(0.), px(0.)), size(px(400.), px(40.)));

        assert_eq!(physical_width(Some(bounds), 2.), Some(800));
    }

    #[test]
    fn first_ten_series_colors_are_distinct() {
        for left in 0..10 {
            for right in left + 1..10 {
                assert_ne!(series_color(left), series_color(right));
            }
        }
    }

    #[test]
    fn hover_maps_a_rendered_point_back_to_stored_evidence()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = tempfile::tempdir()?;
        let client = NativeClient::open(root.path())?;
        let project = client.create_project("viewer", Some(ProjectId::from_string("project")))?;
        let run = client.create_run(
            &project.project_id,
            "baseline",
            Some(RunId::from_string("run")),
        )?;
        client
            .run_handle(run.clone())
            .log_metric_at_step("loss", 7, 1.25)?;
        client.finish_run(&run.run_id)?;
        client.shutdown(None)?;
        let worker = ReadWorker::spawn(root.path())?;
        worker.submit(
            Generation(1),
            ReadRequest::Detail(DetailRequest {
                selection: CurveSelection {
                    run_ids: vec![run.run_id],
                    metric_key: MetricKey::from_string("loss"),
                    axis: AlignmentAxis::Step,
                },
                viewport: AlignmentViewport::new(7, 8)?,
                physical_width: 100,
            }),
        )?;
        let event = worker.recv_timeout(Duration::from_secs(10))?;
        let ReadSnapshot::Detail(snapshot) = event.result? else {
            return Err("worker returned the wrong snapshot kind".into());
        };
        let viewport = detail_viewport(&snapshot, None).ok_or("detail should be drawable")?;
        let adapter = ChartAdapter {
            detail_bounds: Some(Bounds::new(point(px(0.), px(0.)), size(px(100.), px(100.)))),
            ..ChartAdapter::default()
        };

        let hover = adapter
            .hit_test(&snapshot, viewport, point(px(0.), px(50.)))
            .ok_or("stored point should be hit")?;

        assert_eq!(
            (
                hover.run_name.as_str(),
                hover.metric_key.as_str(),
                hover.axis_value,
                hover.step,
                hover.value,
            ),
            ("baseline", "loss", 7, 7, 1.25)
        );
        Ok(())
    }
}
