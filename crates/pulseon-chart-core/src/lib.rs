//! Renderer-agnostic models and geometry for PulseOn metric charts.

#![forbid(unsafe_code)]

mod downsample;
mod interaction;

use std::error::Error;
use std::fmt;

pub use downsample::lttb;
pub use interaction::{Hit, PathCache, ScreenPoint, SelectionState, ZoomState, hit_test};

/// Errors produced while constructing or transforming chart data.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ChartError {
    EmptySeriesId,
    InvalidPoint { index: usize },
    UnsortedSeries { index: usize },
    InvalidRange,
    InvalidOutputRange,
    InvalidCanvasSize,
    InvalidDownsampleThreshold,
    InvalidTransform,
}

impl fmt::Display for ChartError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptySeriesId => formatter.write_str("series id must not be empty"),
            Self::InvalidPoint { index } => {
                write!(formatter, "series point at index {index} is not finite")
            }
            Self::UnsortedSeries { index } => write!(
                formatter,
                "series x values must be nondecreasing (out of order at index {index})"
            ),
            Self::InvalidRange => formatter.write_str("axis range must be finite and increasing"),
            Self::InvalidOutputRange => {
                formatter.write_str("scale output range must be finite and non-empty")
            }
            Self::InvalidCanvasSize => {
                formatter.write_str("canvas dimensions must be finite and positive")
            }
            Self::InvalidDownsampleThreshold => {
                formatter.write_str("downsample threshold must be at least three")
            }
            Self::InvalidTransform => formatter.write_str("chart transform inputs are invalid"),
        }
    }
}

impl Error for ChartError {}

/// Stable identity for one chart series.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SeriesId(String);

impl SeriesId {
    /// Creates a non-empty series identifier.
    ///
    /// # Errors
    ///
    /// Returns [`ChartError::EmptySeriesId`] when the value is blank.
    pub fn new(value: impl Into<String>) -> Result<Self, ChartError> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(ChartError::EmptySeriesId);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for SeriesId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// One point in chart data coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DataPoint {
    pub x: f64,
    pub y: f64,
}

impl DataPoint {
    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }
}

/// An immutable, x-ordered chart series.
#[derive(Clone, Debug, PartialEq)]
pub struct Series {
    id: SeriesId,
    points: Vec<DataPoint>,
}

impl Series {
    /// Creates a series whose finite points are ordered by x coordinate.
    ///
    /// # Errors
    ///
    /// Returns an error for non-finite coordinates or decreasing x values.
    pub fn new(id: SeriesId, points: Vec<DataPoint>) -> Result<Self, ChartError> {
        for (index, point) in points.iter().enumerate() {
            if !point.x.is_finite() || !point.y.is_finite() {
                return Err(ChartError::InvalidPoint { index });
            }
            if index > 0 && point.x < points[index - 1].x {
                return Err(ChartError::UnsortedSeries { index });
            }
        }
        Ok(Self { id, points })
    }

    pub fn id(&self) -> &SeriesId {
        &self.id
    }

    pub fn points(&self) -> &[DataPoint] {
        &self.points
    }

    /// Returns points intersecting an x range plus one neighbor on each side.
    pub fn visible_points(&self, x_range: AxisRange) -> &[DataPoint] {
        let first = self.points.partition_point(|point| point.x < x_range.start);
        let after = self.points.partition_point(|point| point.x <= x_range.end);
        if first == self.points.len() || after == 0 {
            return &[];
        }
        &self.points[first.saturating_sub(1)..after.saturating_add(1).min(self.points.len())]
    }
}

/// A finite, increasing range on one data axis.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AxisRange {
    pub start: f64,
    pub end: f64,
}

impl AxisRange {
    /// Creates a finite range where `start < end`.
    ///
    /// # Errors
    ///
    /// Returns [`ChartError::InvalidRange`] for an empty or non-finite range.
    pub fn new(start: f64, end: f64) -> Result<Self, ChartError> {
        if !start.is_finite() || !end.is_finite() || start >= end {
            return Err(ChartError::InvalidRange);
        }
        Ok(Self { start, end })
    }

    pub fn span(self) -> f64 {
        self.end - self.start
    }
}

/// The data-coordinate rectangle currently visible to a renderer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Viewport {
    pub x: AxisRange,
    pub y: AxisRange,
}

impl Viewport {
    pub const fn new(x: AxisRange, y: AxisRange) -> Self {
        Self { x, y }
    }
}

/// Positive pixel dimensions for a chart surface.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CanvasSize {
    pub width: f64,
    pub height: f64,
}

impl CanvasSize {
    /// Creates finite, positive canvas dimensions.
    ///
    /// # Errors
    ///
    /// Returns [`ChartError::InvalidCanvasSize`] for invalid dimensions.
    pub fn new(width: f64, height: f64) -> Result<Self, ChartError> {
        if !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0 {
            return Err(ChartError::InvalidCanvasSize);
        }
        Ok(Self { width, height })
    }
}

/// Maps one data range onto an arbitrary output interval.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LinearScale {
    domain: AxisRange,
    output_start: f64,
    output_end: f64,
}

impl LinearScale {
    /// Creates a scale from a validated domain to a non-empty output interval.
    ///
    /// # Errors
    ///
    /// Returns [`ChartError::InvalidOutputRange`] for an invalid output interval.
    pub fn new(domain: AxisRange, output_start: f64, output_end: f64) -> Result<Self, ChartError> {
        if !output_start.is_finite() || !output_end.is_finite() || output_start == output_end {
            return Err(ChartError::InvalidOutputRange);
        }
        Ok(Self {
            domain,
            output_start,
            output_end,
        })
    }

    pub fn map(self, value: f64) -> f64 {
        let ratio = (value - self.domain.start) / self.domain.span();
        self.output_start + ratio * (self.output_end - self.output_start)
    }

    pub fn invert(self, value: f64) -> f64 {
        let ratio = (value - self.output_start) / (self.output_end - self.output_start);
        self.domain.start + ratio * self.domain.span()
    }
}

/// Generates human-scale linear tick values across an axis range.
pub fn linear_ticks(range: AxisRange, target_count: usize) -> Vec<f64> {
    if target_count == 0 {
        return Vec::new();
    }
    let target_count = target_count.min(1_024);
    let raw_step = range.span() / target_count as f64;
    let magnitude = 10_f64.powf(raw_step.log10().floor());
    let normalized = raw_step / magnitude;
    let factor = if normalized <= 1.0 {
        1.0
    } else if normalized <= 2.0 {
        2.0
    } else if normalized <= 5.0 {
        5.0
    } else {
        10.0
    };
    let step = factor * magnitude;
    if !step.is_finite() || step <= 0.0 {
        return vec![range.start, range.end];
    }

    let mut value = (range.start / step).ceil() * step;
    let mut ticks = Vec::new();
    while value <= range.end + step * 1e-10 && ticks.len() <= target_count + 2 {
        ticks.push(if value == -0.0 { 0.0 } else { value });
        value += step;
    }
    ticks
}

/// Builds a downsampled path in top-left-origin screen coordinates.
///
/// # Errors
///
/// Returns an error when `max_points` is below three or scales are invalid.
pub fn build_path(
    series: &Series,
    viewport: Viewport,
    canvas: CanvasSize,
    max_points: Option<usize>,
) -> Result<Vec<ScreenPoint>, ChartError> {
    let visible = series.visible_points(viewport.x);
    let points = match max_points {
        Some(threshold) if visible.len() > threshold => lttb(visible, threshold)?,
        Some(threshold) if threshold < 3 => {
            return Err(ChartError::InvalidDownsampleThreshold);
        }
        _ => visible.to_vec(),
    };
    let x_scale = LinearScale::new(viewport.x, 0.0, canvas.width)?;
    let y_scale = LinearScale::new(viewport.y, canvas.height, 0.0)?;
    Ok(points
        .into_iter()
        .map(|point| ScreenPoint::new(x_scale.map(point.x), y_scale.map(point.y)))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn range(start: f64, end: f64) -> AxisRange {
        AxisRange::new(start, end).expect("test range should be valid")
    }

    #[test]
    fn series_rejects_non_finite_and_unsorted_points() {
        let id = SeriesId::new("loss").expect("test id should be valid");
        let invalid = Series::new(id.clone(), vec![DataPoint::new(0.0, f64::NAN)]);
        let unsorted = Series::new(id, vec![DataPoint::new(2.0, 1.0), DataPoint::new(1.0, 2.0)]);

        assert_eq!(invalid, Err(ChartError::InvalidPoint { index: 0 }));
        assert_eq!(unsorted, Err(ChartError::UnsortedSeries { index: 1 }));
    }

    #[test]
    fn visible_points_keeps_boundary_neighbors_for_connected_paths() {
        let series = Series::new(
            SeriesId::new("loss").expect("test id should be valid"),
            (0..=5)
                .map(|value| DataPoint::new(f64::from(value), f64::from(value)))
                .collect(),
        )
        .expect("test series should be valid");

        assert_eq!(
            series.visible_points(range(2.2, 3.2)),
            &series.points()[2..=4]
        );
    }

    #[test]
    fn linear_scale_maps_and_inverts_reversed_output() {
        let scale =
            LinearScale::new(range(0.0, 10.0), 100.0, 0.0).expect("test scale should be valid");

        assert_eq!(scale.map(2.5), 75.0);
        assert_eq!(scale.invert(75.0), 2.5);
    }

    #[test]
    fn linear_ticks_uses_nice_steps() {
        assert_eq!(linear_ticks(range(0.3, 9.1), 5), vec![2.0, 4.0, 6.0, 8.0]);
    }

    #[test]
    fn build_path_inverts_the_y_axis() {
        let series = Series::new(
            SeriesId::new("loss").expect("test id should be valid"),
            vec![DataPoint::new(0.0, 0.0), DataPoint::new(10.0, 10.0)],
        )
        .expect("test series should be valid");
        let viewport = Viewport::new(range(0.0, 10.0), range(0.0, 10.0));
        let canvas = CanvasSize::new(200.0, 100.0).expect("test canvas should be valid");

        assert_eq!(
            build_path(&series, viewport, canvas, None).expect("path should build"),
            vec![ScreenPoint::new(0.0, 100.0), ScreenPoint::new(200.0, 0.0)]
        );
    }
}
