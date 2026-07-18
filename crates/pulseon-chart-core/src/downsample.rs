use crate::{ChartError, DataPoint};

/// Downsamples ordered points with Largest-Triangle-Three-Buckets.
///
/// # Errors
///
/// Returns [`ChartError::InvalidDownsampleThreshold`] below three points.
pub fn lttb(points: &[DataPoint], threshold: usize) -> Result<Vec<DataPoint>, ChartError> {
    if threshold < 3 {
        return Err(ChartError::InvalidDownsampleThreshold);
    }
    if points.len() <= threshold {
        return Ok(points.to_vec());
    }

    let bucket_width = (points.len() - 2) as f64 / (threshold - 2) as f64;
    let mut sampled = Vec::with_capacity(threshold);
    let mut selected_index = 0;
    sampled.push(points[selected_index]);

    for bucket in 0..threshold - 2 {
        let average_start =
            (((bucket + 1) as f64 * bucket_width).floor() as usize + 1).min(points.len());
        let average_end =
            (((bucket + 2) as f64 * bucket_width).floor() as usize + 1).min(points.len());
        let average_points = &points[average_start..average_end];
        let (average_x, average_y) = if average_points.is_empty() {
            let last = points[points.len() - 1];
            (last.x, last.y)
        } else {
            let (x_sum, y_sum) = average_points
                .iter()
                .fold((0.0, 0.0), |(x_sum, y_sum), point| {
                    (x_sum + point.x, y_sum + point.y)
                });
            let count = average_points.len() as f64;
            (x_sum / count, y_sum / count)
        };

        let candidate_start =
            ((bucket as f64 * bucket_width).floor() as usize + 1).min(points.len() - 1);
        let candidate_end = ((((bucket + 1) as f64 * bucket_width).floor() as usize + 1)
            .min(points.len() - 1))
        .max(candidate_start + 1);
        let selected = points[selected_index];
        let mut largest_area = -1.0;

        for (offset, candidate) in points[candidate_start..candidate_end].iter().enumerate() {
            let area = ((selected.x - average_x) * (candidate.y - selected.y)
                - (selected.x - candidate.x) * (average_y - selected.y))
                .abs();
            if area > largest_area {
                largest_area = area;
                selected_index = candidate_start + offset;
            }
        }
        sampled.push(points[selected_index]);
    }

    sampled.push(points[points.len() - 1]);
    Ok(sampled)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lttb_reduces_points_and_preserves_endpoints() {
        let points: Vec<_> = (0..100)
            .map(|value| {
                let value = f64::from(value);
                DataPoint::new(value, (value / 10.0).sin())
            })
            .collect();

        let sampled = lttb(&points, 12).expect("threshold should be valid");

        assert_eq!(sampled.len(), 12);
        assert_eq!(sampled.first(), points.first());
        assert_eq!(sampled.last(), points.last());
    }

    #[test]
    fn lttb_returns_short_input_unchanged() {
        let points = vec![DataPoint::new(0.0, 1.0), DataPoint::new(1.0, 2.0)];

        assert_eq!(lttb(&points, 3), Ok(points));
    }

    #[test]
    fn lttb_preserves_a_salient_peak() {
        let points = vec![
            DataPoint::new(0.0, 0.0),
            DataPoint::new(1.0, 0.0),
            DataPoint::new(2.0, 100.0),
            DataPoint::new(3.0, 0.0),
            DataPoint::new(4.0, 0.0),
        ];

        assert!(
            lttb(&points, 3)
                .expect("threshold should be valid")
                .contains(&DataPoint::new(2.0, 100.0))
        );
    }

    #[test]
    fn lttb_rejects_thresholds_that_cannot_preserve_shape() {
        assert_eq!(
            lttb(&[DataPoint::new(0.0, 0.0)], 2),
            Err(ChartError::InvalidDownsampleThreshold)
        );
    }
}
