use std::collections::HashMap;

use gpui::{
    Bounds, Path, PathBuilder, Pixels, Point, Rgba, WindowAppearance, canvas, fill, point, px, rgb,
    size,
};
use pulseon_chart_core::{
    AxisRange, CanvasSize, PathCache, ScreenPoint, Viewport, hit_test_point, visible_y_range_for,
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
    revision: u64,
    viewport: [u64; 4],
    bounds: [u32; 4],
    dark: bool,
    partial: bool,
}

#[derive(Default)]
pub struct ChartAdapter {
    projection_cache: PathCache,
    gpui_paths: HashMap<String, (GpuiPathKey, Path<Pixels>)>,
    detail_bounds: Option<Bounds<Pixels>>,
}

struct PreparedChart {
    paths: Vec<(Path<Pixels>, Rgba)>,
}

impl ChartAdapter {
    pub fn clear(&mut self) {
        self.projection_cache.clear();
        self.gpui_paths.clear();
        self.detail_bounds = None;
    }

    fn prepare(
        &mut self,
        snapshot: &CurveSnapshot,
        revision: u64,
        viewport: Viewport,
        bounds: Bounds<Pixels>,
        appearance: WindowAppearance,
    ) -> PreparedChart {
        self.detail_bounds = Some(bounds);
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
            let path = if let Some((cached_key, path)) = self.gpui_paths.get(series.id().as_str())
                && cached_key == &key
            {
                path.clone()
            } else {
                let Ok(points) = self
                    .projection_cache
                    .path_for(series, revision, viewport, canvas)
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
                self.gpui_paths
                    .insert(series.id().as_str().to_owned(), (key, path.clone()));
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

pub fn detail_canvas(
    adapter: std::rc::Rc<std::cell::RefCell<ChartAdapter>>,
    snapshot: CurveSnapshot,
    revision: u64,
    viewport: Viewport,
) -> impl gpui::Styled + gpui::IntoElement {
    canvas(
        move |bounds, window, _| {
            adapter
                .borrow_mut()
                .prepare(&snapshot, revision, viewport, bounds, window.appearance())
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
