//! Lasso and rectangle selection (§3.4). Pure hit-testing: what the user then does with the selection
//! (move, delete, copy) is expressed as ordinary commands, so a selection never edits the document.
//!
//! The rule is §3.4's: a stroke counts as selected when **more than half** of its sampled points lie
//! inside the shape. Sampling the points rather than testing the curve keeps it cheap and matches how
//! the user judges it — they see the samples, not the ideal curve.

use crate::geometry::{point_in_polygon, P};
use crate::index::StrokeIndex;
use crate::stroke::{Stroke, StrokeId};

/// The fraction of a stroke's samples that must be inside for it to count as selected (§3.4).
pub const SELECTION_THRESHOLD: f32 = 0.5;

/// The two shapes the selection tool offers.
#[derive(Clone, Debug, PartialEq)]
pub enum SelectShape {
    /// A freehand loop. Treated as closed, so the user never has to meet their own start point.
    Lasso(Vec<P>),
    /// A dragged rectangle, which is just a four-vertex polygon.
    Rectangle { min: P, max: P },
}

impl SelectShape {
    /// The shape as a closed polygon, so both variants share one containment test.
    pub fn polygon(&self) -> Vec<P> {
        match self {
            SelectShape::Lasso(points) => points.clone(),
            SelectShape::Rectangle { min, max } => vec![
                (min.0, min.1),
                (max.0, min.1),
                (max.0, max.1),
                (min.0, max.1),
            ],
        }
    }

    /// The shape's bounding box — the grid query that narrows the exact test.
    pub fn bounds(&self) -> Option<(f32, f32, f32, f32)> {
        crate::geometry::points_bounds(&self.polygon())
    }

    pub fn from_rectangle(corner_a: P, corner_b: P) -> Self {
        SelectShape::Rectangle {
            min: (corner_a.0.min(corner_b.0), corner_a.1.min(corner_b.1)),
            max: (corner_a.0.max(corner_b.0), corner_a.1.max(corner_b.1)),
        }
    }
}

/// Whether `stroke` is more than half inside `shape`. An empty polygon selects nothing.
pub fn is_selected(stroke: &Stroke, polygon: &[P]) -> bool {
    if polygon.len() < 3 || stroke.points.is_empty() {
        return false;
    }
    let inside = stroke
        .points
        .iter()
        .filter(|point| point_in_polygon((point.x, point.y), polygon))
        .count();
    // Integer arithmetic on purpose: `inside / total > 0.5` in f32 would be a rounding question.
    inside * 2 > stroke.points.len()
}

/// The strokes inside the shape, in id order.
///
/// The grid narrows the candidates and the exact test decides, exactly as the eraser does — but here
/// a stroke whose *points* are outside can still be selected if most samples are in, so the coarse
/// filter must not be the only test.
pub fn select(index: &StrokeIndex, strokes: &[Stroke], shape: &SelectShape) -> Vec<StrokeId> {
    let Some(bounds) = shape.bounds() else {
        return Vec::new();
    };
    let polygon = shape.polygon();
    let candidates = index.query(bounds);

    strokes
        .iter()
        .filter(|stroke| candidates.contains(&stroke.id))
        .filter(|stroke| is_selected(stroke, &polygon))
        .map(|stroke| stroke.id)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stroke::StrokePoint;

    fn stroke_at(id: u64, from: f32, to: f32, y: f32) -> Stroke {
        let mut points = Vec::new();
        let mut x = from;
        while x <= to {
            points.push(StrokePoint::new(x, y, 0.5, 0));
            x += 2.0;
        }
        Stroke::new(StrokeId(id), points, 0xFF000000, 4.0)
    }

    fn square() -> SelectShape {
        SelectShape::from_rectangle((0.0, 0.0), (100.0, 100.0))
    }

    #[test]
    fn a_stroke_mostly_inside_is_selected() {
        let inside = stroke_at(1, 20.0, 80.0, 50.0);
        assert!(is_selected(&inside, &square().polygon()));
    }

    #[test]
    fn a_stroke_mostly_outside_is_not_selected() {
        let outside = stroke_at(1, 90.0, 300.0, 50.0);
        assert!(!is_selected(&outside, &square().polygon()));
    }

    #[test]
    fn exactly_half_is_not_selected() {
        // The square is cut in half at x = 50, with 100 samples straddling the cut.
        let left_half = SelectShape::from_rectangle((0.0, 0.0), (50.0, 100.0));
        let points: Vec<StrokePoint> = (0..100)
            .map(|i| StrokePoint::new(i as f32, 50.0, 0.5, 0))
            .collect();
        let straddling = Stroke::new(StrokeId(9), points, 0xFF000000, 4.0);

        assert!(
            !is_selected(&straddling, &left_half.polygon()),
            "half is not a majority"
        );

        // One more sample inside tips it over.
        let mut tipped_points = straddling.points.clone();
        tipped_points.push(StrokePoint::new(10.0, 60.0, 0.5, 0));
        let tipped = Stroke::new(StrokeId(9), tipped_points, 0xFF000000, 4.0);
        assert!(is_selected(&tipped, &left_half.polygon()));
    }

    #[test]
    fn a_lasso_selects_what_it_encloses() {
        let diamond =
            SelectShape::Lasso(vec![(50.0, 0.0), (100.0, 50.0), (50.0, 100.0), (0.0, 50.0)]);
        // Centred: inside. Near the corner of the bounding box: outside the diamond.
        assert!(is_selected(
            &stroke_at(1, 40.0, 60.0, 50.0),
            &diamond.polygon()
        ));
        assert!(!is_selected(
            &stroke_at(2, 2.0, 10.0, 4.0),
            &diamond.polygon()
        ));
    }

    #[test]
    fn an_open_lasso_is_closed_for_the_user() {
        // Three vertices, never reconnecting to the first: still a triangle.
        let path = SelectShape::Lasso(vec![(0.0, 0.0), (100.0, 0.0), (100.0, 100.0)]);
        assert!(is_selected(
            &stroke_at(1, 40.0, 60.0, 20.0),
            &path.polygon()
        ));
        assert!(!is_selected(
            &stroke_at(2, 10.0, 20.0, 80.0),
            &path.polygon()
        ));
    }

    #[test]
    fn the_index_never_loses_a_stroke_the_exact_test_would_select() {
        let strokes: Vec<Stroke> = (0..20)
            .map(|i| stroke_at(i as u64, i as f32 * 30.0, i as f32 * 30.0 + 20.0, 50.0))
            .collect();
        let index = StrokeIndex::build(&strokes);
        let shape = square();
        let polygon = shape.polygon();

        let brute_force: Vec<StrokeId> = strokes
            .iter()
            .filter(|stroke| is_selected(stroke, &polygon))
            .map(|stroke| stroke.id)
            .collect();

        assert_eq!(
            select(&index, &strokes, &shape),
            brute_force,
            "the coarse filter must only ever remove work, never a result"
        );
        assert_eq!(
            brute_force,
            vec![StrokeId(0), StrokeId(1), StrokeId(2)],
            "strokes running from x = 0/30/60 are the ones over half inside the square"
        );
    }

    #[test]
    fn a_degenerate_shape_selects_nothing() {
        let strokes = vec![stroke_at(1, 0.0, 10.0, 0.0)];
        let index = StrokeIndex::build(&strokes);
        assert!(select(&index, &strokes, &SelectShape::Lasso(vec![(0.0, 0.0)])).is_empty());
        assert!(select(&index, &strokes, &SelectShape::Lasso(Vec::new())).is_empty());
    }

    #[test]
    fn a_rectangle_is_normalised_from_any_two_corners() {
        let dragged_upwards = SelectShape::from_rectangle((100.0, 100.0), (0.0, 0.0));
        assert_eq!(dragged_upwards, square());
    }
}
