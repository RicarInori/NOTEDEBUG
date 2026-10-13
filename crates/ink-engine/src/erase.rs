//! The eraser's two modes (§3.3). Pure functions over strokes and a journey: the engine owns the
//! undo log and id allocation, this module owns only the geometry.
//!
//! | mode | behaviour | undo cost |
//! |---|---|---|
//! | [`EraserMode::Stroke`] | the whole stroke dies | one removal |
//! | [`EraserMode::Pixel`] | the covered points are cut out, the rest survives as sub-strokes | remove + add |
//!
//! Pixel erasing cuts the **point sequence**, not the rendered pixels. §3.3 is explicit that this is
//! what keeps a stroke vector rather than a bitmap: the surviving runs keep their original samples,
//! pressure and width, so the result is still exportable to PDF as geometry.

use crate::geometry::{inflate, segment_distance, BBox, P};
use crate::index::StrokeIndex;
use crate::stroke::{Stroke, StrokeId, StrokePoint};
use std::collections::BTreeSet;

/// Which eraser the user selected.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EraserMode {
    /// §3.3's default: the eraser deletes whole strokes it touches.
    Stroke,
    /// §3.3's alternative: only the covered part of a stroke goes away.
    Pixel,
}

/// A journey of the eraser tip, in page coordinates. Only the positions are used — pressure and
/// time belong to a pen, not to an eraser.
#[derive(Clone, Debug, Default)]
pub struct EraserPath {
    points: Vec<P>,
}

impl EraserPath {
    pub fn from_points(points: impl IntoIterator<Item = P>) -> Self {
        Self {
            points: points.into_iter().collect(),
        }
    }

    /// The same journey, read from the sample type the pen pipeline produces.
    pub fn from_samples(samples: &[StrokePoint]) -> Self {
        Self::from_points(samples.iter().map(|p| (p.x, p.y)))
    }

    pub fn is_empty(&self) -> bool {
        self.points.is_empty()
    }

    pub fn points(&self) -> &[P] {
        &self.points
    }

    /// The journey's segments; a single-point journey contributes one degenerate segment so a tap
    /// still erases what is under it.
    fn segments(&self) -> impl Iterator<Item = (P, P)> + '_ {
        let points = &self.points;
        (0..points.len().saturating_sub(1)).map(move |i| (points[i], points[i + 1]))
    }

    /// A box per segment, grown by `radius`.
    ///
    /// **Why per segment rather than one box for the whole journey**: a box around a long journey
    /// would be enormous and would defeat the index; a box per segment keeps the query proportional
    /// to the area actually swept. It is also what makes [`candidates`] *complete*: if a stroke comes
    /// within `radius` of a segment, the touching point lies inside that segment's grown box, so the
    /// stroke is filed in one of the cells that box covers.
    fn swept_areas(&self, radius: f32) -> Vec<BBox> {
        match self.points.as_slice() {
            [] => Vec::new(),
            [only] => vec![inflate((only.0, only.1, only.0, only.1), radius)],
            _ => self
                .segments()
                .map(|(a, b)| {
                    inflate(
                        (a.0.min(b.0), a.1.min(b.1), a.0.max(b.0), a.1.max(b.1)),
                        radius,
                    )
                })
                .collect(),
        }
    }
}

/// Every stroke the index cannot rule out. A superset: callers must still run [`touches`].
pub fn candidates(index: &StrokeIndex, path: &EraserPath, radius_px: f32) -> BTreeSet<StrokeId> {
    index.query_any(path.swept_areas(radius_px))
}

/// Whether the eraser's radius reaches any part of the stroke.
pub fn touches(stroke: &Stroke, path: &EraserPath, radius_px: f32) -> bool {
    if stroke.points.is_empty() || path.is_empty() {
        return false;
    }
    for window in stroke.points.windows(2) {
        let a = (window[0].x, window[0].y);
        let b = (window[1].x, window[1].y);
        if reaches((a, b), path, radius_px) {
            return true;
        }
    }
    // A one-point stroke has no segments, so test the point itself.
    if let [only] = stroke.points.as_slice() {
        let p = (only.x, only.y);
        return reaches((p, p), path, radius_px);
    }
    false
}

fn reaches(segment: ((f32, f32), (f32, f32)), path: &EraserPath, radius_px: f32) -> bool {
    match path.points.as_slice() {
        [] => false,
        [only] => segment_distance(segment, ((only.0, only.1), (only.0, only.1))) <= radius_px,
        _ => path
            .segments()
            .any(|journey| segment_distance(segment, journey) <= radius_px),
    }
}

/// §3.3's pixel mode: the stroke's surviving runs, in order. Empty means the eraser covered all of it.
///
/// Cutting is done on the point sequence, which — thanks to §3.1's uniform 2 px resampling — lands
/// within one sample of the true intersection with the eraser's outline. Runs shorter than two points
/// are dropped: a lone sample cannot be rendered or exported, and keeping it would litter the page
/// with invisible strokes that the next undo has to carry around.
pub fn split(stroke: &Stroke, path: &EraserPath, radius_px: f32) -> Vec<Vec<StrokePoint>> {
    let mut survivors: Vec<Vec<StrokePoint>> = Vec::new();
    let mut current: Vec<StrokePoint> = Vec::new();

    for point in &stroke.points {
        let p = (point.x, point.y);
        let covered = reaches((p, p), path, radius_px);
        if covered {
            if current.len() >= 2 {
                survivors.push(std::mem::take(&mut current));
            } else {
                current.clear();
            }
        } else {
            current.push(*point);
        }
    }
    if current.len() >= 2 {
        survivors.push(current);
    }
    survivors
}

/// Whether a split left the stroke alone. Compared by point sequence rather than by count, because a
/// stroke that was cut and immediately rejoined is not an edit and must not reach the undo log.
pub fn unchanged(original: &Stroke, survivors: &[Vec<StrokePoint>]) -> bool {
    survivors.len() == 1 && survivors[0] == original.points
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stroke(id: u64, points: &[(f32, f32)]) -> Stroke {
        Stroke::new(
            StrokeId(id),
            points
                .iter()
                .map(|(x, y)| StrokePoint::new(*x, *y, 0.5, 0))
                .collect(),
            0xFF000000,
            4.0,
        )
    }

    /// A horizontal run of samples 2 px apart, as the pipeline would produce.
    fn run(id: u64, from: f32, to: f32, y: f32) -> Stroke {
        let mut points = Vec::new();
        let mut x = from;
        while x <= to {
            points.push((x, y));
            x += 2.0;
        }
        stroke(id, &points)
    }

    #[test]
    fn a_journey_that_misses_the_stroke_does_not_touch_it() {
        let s = run(1, 0.0, 100.0, 0.0);
        let path = EraserPath::from_points([(50.0, 40.0), (50.0, 30.0)]);
        assert!(!touches(&s, &path, 8.0));
        assert!(!touches(&s, &path, 1.0));

        // And only just inside the radius.
        let near = EraserPath::from_points([(50.0, 7.0)]);
        assert!(touches(&s, &near, 8.0));
        assert!(!touches(&s, &near, 5.0));
    }

    #[test]
    fn a_tap_is_a_journey_of_one_point() {
        let s = run(1, 0.0, 100.0, 0.0);
        let tap = EraserPath::from_points([(50.0, 2.0)]);
        assert!(touches(&s, &tap, 8.0));
    }

    #[test]
    fn pixel_mode_keeps_the_two_ends_and_drops_the_middle() {
        let s = run(1, 0.0, 100.0, 0.0);
        // Sweep a radius-10 eraser straight through the middle.
        let path = EraserPath::from_points([(40.0, -20.0), (60.0, 20.0)]);

        let survivors = split(&s, &path, 10.0);
        assert_eq!(survivors.len(), 2, "the stroke must survive as two ends");

        // Survivors are the original samples, not a re-derived approximation.
        for run_ in &survivors {
            for point in run_ {
                assert!(
                    s.points.contains(point),
                    "a split must not invent sample points"
                );
            }
        }
        // The erased stretch is gone.
        assert!(survivors[0].last().unwrap().x < 45.0);
        assert!(survivors[1].first().unwrap().x > 55.0);
        assert!(!unchanged(&s, &survivors));
    }

    #[test]
    fn pixel_mode_that_misses_leaves_one_identical_run() {
        let s = run(1, 0.0, 100.0, 0.0);
        let path = EraserPath::from_points([(50.0, 60.0), (50.0, 80.0)]);
        let survivors = split(&s, &path, 10.0);
        assert!(
            unchanged(&s, &survivors),
            "an untouched stroke must not become an edit"
        );
    }

    #[test]
    fn covering_a_stroke_entirely_leaves_nothing() {
        let s = run(1, 0.0, 20.0, 0.0);
        let path = EraserPath::from_points([(-50.0, 0.0), (50.0, 0.0)]);
        assert!(split(&s, &path, 10.0).is_empty());
    }

    #[test]
    fn a_one_point_fragment_is_discarded_rather_than_kept_invisible() {
        // Erase everything except the very last sample.
        let s = run(1, 0.0, 10.0, 0.0);
        let path = EraserPath::from_points([(-10.0, 0.0), (9.0, 0.0)]);
        let survivors = split(&s, &path, 5.0);
        assert!(
            survivors.iter().all(|run_| run_.len() >= 2),
            "no fragment may be a single sample: {survivors:?}"
        );
    }

    #[test]
    fn candidates_include_everything_the_exact_test_would_hit() {
        let strokes: Vec<Stroke> = (0..30)
            .map(|i| {
                run(
                    i as u64,
                    -200.0 + i as f32 * 13.0,
                    -180.0 + i as f32 * 13.0,
                    i as f32 * 9.0,
                )
            })
            .collect();
        let index = StrokeIndex::build(&strokes);
        let radius = 9.0;

        for y in (-20..=280).step_by(5) {
            let path = EraserPath::from_points([(-300.0, y as f32), (300.0, y as f32)]);
            let candidates = candidates(&index, &path, radius);
            for s in &strokes {
                if touches(s, &path, radius) {
                    assert!(
                        candidates.contains(&s.id),
                        "the index dropped stroke {} from a journey at y={y}",
                        s.id
                    );
                }
            }
        }
    }
}
