//! The stroke pipeline: smooth, resample, and turn pressure into width (§3.1).
//!
//! Only the parts that do not need a GPU live here. Rendering — triangle-strip meshing, the ring
//! vertex buffer, the single draw call — belongs with the wgpu backend and is not in this crate yet.
//!
//! Every function here is pure, which is deliberate: this is the hot path's arithmetic, and the
//! reason it can be unit-tested is that none of it touches a device.

use crate::stroke::StrokePoint;
use inkweave_api::stylus::PressureCurve;

/// One Euro filter, the smoothing the guide specifies (`min_cutoff = 1.0`, `beta = 0.01`).
///
/// Chosen over a plain low-pass because it adapts: slow movement is smoothed hard, fast movement is
/// followed closely, so a stroke neither jitters when held still nor lags when flicked.
#[derive(Clone, Debug)]
pub struct OneEuroFilter {
    min_cutoff: f32,
    beta: f32,
    d_cutoff: f32,
    previous: Option<f32>,
    previous_derivative: f32,
    /// Time of the previous sample. Kept so `filter` stays allocation-free and handles a variable
    /// sample rate correctly.
    last_t: f32,
}

impl OneEuroFilter {
    pub fn new(min_cutoff: f32, beta: f32, d_cutoff: f32) -> Self {
        Self {
            min_cutoff,
            beta,
            d_cutoff,
            previous: None,
            previous_derivative: 0.0,
            last_t: 0.0,
        }
    }

    /// The parameters from the guide's pipeline.
    pub fn standard() -> Self {
        Self::new(1.0, 0.01, 1.0)
    }

    /// `t_seconds` is the sample's own time, so a variable sample rate is handled correctly.
    pub fn filter(&mut self, value: f32, t_seconds: f32) -> f32 {
        let Some(previous) = self.previous else {
            self.previous = Some(value);
            self.last_t = t_seconds;
            return value;
        };
        let dt = t_seconds - self.last_t;
        if !dt.is_finite() || dt <= 0.0 {
            // Two samples with the same timestamp (or a non-finite one): nothing to differentiate
            // against, so pass through.
            return previous;
        }

        let derivative = (value - previous) / dt;
        let smoothed_derivative = low_pass(
            derivative,
            self.previous_derivative,
            alpha(self.d_cutoff, dt),
        );
        self.previous_derivative = smoothed_derivative;

        let cutoff = self.min_cutoff + self.beta * smoothed_derivative.abs();
        let smoothed = low_pass(value, previous, alpha(cutoff, dt));
        self.previous = Some(smoothed);
        self.last_t = t_seconds;
        smoothed
    }

    /// Reset between strokes — carrying state across a pen lift would drag the next stroke's start.
    pub fn reset(&mut self) {
        self.previous = None;
        self.previous_derivative = 0.0;
        self.last_t = 0.0;
    }
}

/// Exponential smoothing weight for a cutoff frequency and timestep.
fn alpha(cutoff: f32, dt: f32) -> f32 {
    let tau = 1.0 / (2.0 * std::f32::consts::PI * cutoff.max(f32::EPSILON));
    1.0 / (1.0 + tau / dt)
}

fn low_pass(value: f32, previous: f32, alpha: f32) -> f32 {
    alpha * value + (1.0 - alpha) * previous
}

/// Emit a point every `spacing` pixels along the polyline, interpolating position, pressure and
/// time.
///
/// Resampling before meshing is what keeps segment length uniform, so stroke width and any
/// texture along the stroke do not depend on how fast the pen moved (§3.1's 2 px step).
/// The final point is always kept, so the stroke ends where the pen did rather than at the last
/// multiple of `spacing`.
pub fn resample_equidistant(points: &[StrokePoint], spacing: f32) -> Vec<StrokePoint> {
    if points.len() < 2 || !spacing.is_finite() || spacing <= 0.0 {
        return points.to_vec();
    }

    let mut out = Vec::with_capacity(points.len());
    out.push(points[0]);
    let mut carry = 0.0_f32;

    for pair in points.windows(2) {
        let (from, to) = (pair[0], pair[1]);
        let segment = from.distance_to(&to);
        if segment <= 0.0 {
            continue;
        }
        let mut travelled = spacing - carry;
        while travelled <= segment {
            let t = travelled / segment;
            out.push(lerp(from, to, t));
            travelled += spacing;
        }
        carry = segment - (travelled - spacing);
        if carry >= spacing {
            carry = 0.0;
        }
    }

    if let Some(&last) = points.last() {
        let needs_last = out
            .last()
            .map(|p| p.distance_to(&last) > f32::EPSILON)
            .unwrap_or(true);
        if needs_last {
            out.push(last);
        }
    }
    out
}

fn lerp(from: StrokePoint, to: StrokePoint, t: f32) -> StrokePoint {
    StrokePoint {
        x: from.x + (to.x - from.x) * t,
        y: from.y + (to.y - from.y) * t,
        pressure: from.pressure + (to.pressure - from.pressure) * t,
        t_us: from.t_us + ((to.t_us as i64 - from.t_us as i64) as f32 * t) as u64,
    }
}

/// Pressure → width.
///
/// Devices without pressure report `pressure == 0.0` for every point; falling back to the base
/// width there is what keeps such a device usable instead of drawing nothing (§3.8's degradation
/// table). A real `0.0` from a pressure-sensitive device is indistinguishable, and the curve is
/// calibrated so that the low end is the thin end anyway.
pub fn width_for(pressure: f32, curve: &PressureCurve, base_width_px: f32) -> f32 {
    if pressure <= 0.0 {
        return base_width_px;
    }
    (base_width_px * curve.apply(pressure)).max(0.5)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn point(x: f32, y: f32, pressure: f32, t_us: u64) -> StrokePoint {
        StrokePoint::new(x, y, pressure, t_us)
    }

    #[test]
    fn the_first_sample_passes_through_untouched() {
        let mut filter = OneEuroFilter::standard();
        assert_eq!(filter.filter(42.0, 0.0), 42.0);
    }

    #[test]
    fn a_held_pen_settles_exactly_on_the_signal() {
        let mut filter = OneEuroFilter::standard();
        filter.reset();
        let mut value = 0.0;
        for step in 0..200 {
            // 2 ms apart, all at 10.0
            value = filter.filter(10.0, step as f32 * 0.002);
        }
        assert!(
            (value - 10.0).abs() < 1e-3,
            "a constant signal must converge, got {value}"
        );
    }

    #[test]
    fn a_step_is_smoothed_rather_than_followed_instantly() {
        let mut filter = OneEuroFilter::standard();
        filter.filter(0.0, 0.0);
        let first_after_step = filter.filter(100.0, 0.002);
        assert!(
            first_after_step > 0.0 && first_after_step < 100.0,
            "the step must be attenuated, got {first_after_step}"
        );
    }

    #[test]
    fn equal_timestamps_do_not_divide_by_zero() {
        let mut filter = OneEuroFilter::standard();
        filter.filter(1.0, 0.5);
        assert_eq!(
            filter.filter(2.0, 0.5),
            1.0,
            "same instant => keep the previous estimate"
        );
    }

    #[test]
    fn resampling_puts_points_on_a_uniform_grid_and_keeps_the_last_one() {
        // A 30 px straight segment resampled every 10 px: 0, 10, 20, 30 — the last kept even though
        // it lands on the grid here, and the case where it does not is covered separately.
        let points = vec![point(0.0, 0.0, 0.0, 0), point(30.0, 0.0, 0.0, 3000)];
        let out = resample_equidistant(&points, 10.0);
        let xs: Vec<f32> = out.iter().map(|p| p.x).collect();
        assert_eq!(xs, vec![0.0, 10.0, 20.0, 30.0]);
    }

    #[test]
    fn resampling_keeps_an_off_grid_endpoint() {
        let points = vec![point(0.0, 0.0, 0.0, 0), point(25.0, 0.0, 0.0, 2500)];
        let out = resample_equidistant(&points, 10.0);
        let xs: Vec<f32> = out.iter().map(|p| p.x).collect();
        assert_eq!(xs, vec![0.0, 10.0, 20.0, 25.0], "the endpoint must survive");
    }

    #[test]
    fn resampling_interpolates_pressure_and_time() {
        let points = vec![point(0.0, 0.0, 0.0, 0), point(20.0, 0.0, 1.0, 2000)];
        let out = resample_equidistant(&points, 10.0);
        let middle = out[1];
        assert!((middle.pressure - 0.5).abs() < 1e-6);
        assert_eq!(middle.t_us, 1000);
    }

    #[test]
    fn resampling_leaves_degenerate_input_alone() {
        let single = vec![point(1.0, 1.0, 0.5, 0)];
        assert_eq!(resample_equidistant(&single, 10.0), single);
        assert_eq!(resample_equidistant(&[], 10.0), Vec::<StrokePoint>::new());
        // Duplicate points describe no travel, so they collapse to the one point they represent
        // rather than being emitted twice.
        let duplicated = vec![point(1.0, 1.0, 0.5, 0), point(1.0, 1.0, 0.5, 0)];
        assert_eq!(
            resample_equidistant(&duplicated, 10.0),
            vec![point(1.0, 1.0, 0.5, 0)]
        );
    }

    #[test]
    fn width_falls_back_to_the_base_for_a_pressure_less_device() {
        let curve = PressureCurve::linear();
        assert_eq!(width_for(0.0, &curve, 4.0), 4.0);
    }

    #[test]
    fn width_follows_the_curve_and_never_reaches_zero() {
        let curve = PressureCurve::linear();
        assert!((width_for(1.0, &curve, 4.0) - 4.0).abs() < 1e-6);
        assert!((width_for(0.5, &curve, 4.0) - 2.0).abs() < 1e-6);
        // A curve that maps pressure to 0 still leaves a hairline, not an invisible stroke.
        let flattening = PressureCurve {
            points: vec![(0.0, 0.0), (1.0, 0.0)],
        };
        assert_eq!(width_for(1.0, &flattening, 4.0), 0.5);
    }
}
