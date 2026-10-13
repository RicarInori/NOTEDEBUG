//! The exact tests the eraser and the selection tool need (§3.3, §3.4).
//!
//! Plain `f32` tuples and free functions on purpose: the eraser runs these per stroke segment per
//! eraser segment, and §3.1's hot-path discipline forbids allocation and indirection here.

/// A point, as `(x, y)`.
pub type P = (f32, f32);

/// Axis-aligned bounds as `(min_x, min_y, max_x, max_y)` — the shape a grid cell stores.
pub type BBox = (f32, f32, f32, f32);

/// Grow a box by `radius` on every side. Used to turn "within the eraser radius" into a box query,
/// which is what makes the grid a *complete* filter rather than a heuristic.
pub fn inflate(bounds: BBox, radius: f32) -> BBox {
    (
        bounds.0 - radius,
        bounds.1 - radius,
        bounds.2 + radius,
        bounds.3 + radius,
    )
}

pub fn bounds_overlap(a: BBox, b: BBox) -> bool {
    a.0 <= b.2 && b.0 <= a.2 && a.1 <= b.3 && b.1 <= a.3
}

/// Squared distance from `p` to the segment `a`–`b`.
///
/// Squared rather than actual distance so callers can compare against `radius * radius` and skip the
/// square root; the eraser's radius is the only quantity ever compared.
pub fn point_segment_distance_sq(p: P, a: P, b: P) -> f32 {
    let vx = b.0 - a.0;
    let vy = b.1 - a.1;
    let len_sq = vx * vx + vy * vy;
    // A degenerate segment is a point, so the projection is undefined but the answer is not.
    let t = if len_sq <= f32::EPSILON {
        0.0
    } else {
        (((p.0 - a.0) * vx + (p.1 - a.1) * vy) / len_sq).clamp(0.0, 1.0)
    };
    let dx = p.0 - (a.0 + t * vx);
    let dy = p.1 - (a.1 + t * vy);
    dx * dx + dy * dy
}

pub fn point_segment_distance(p: P, a: P, b: P) -> f32 {
    point_segment_distance_sq(p, a, b).sqrt()
}

/// Twice the signed area of the triangle `a`, `b`, `c`. The sign is the only thing used.
fn orientation(a: P, b: P, c: P) -> f32 {
    (b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0)
}

fn on_segment(a: P, b: P, p: P) -> bool {
    // Only called when `p` is already known to be collinear with `a`–`b`.
    p.0 >= a.0.min(b.0) && p.0 <= a.0.max(b.0) && p.1 >= a.1.min(b.1) && p.1 <= a.1.max(b.1)
}

/// Whether segments `a` and `b` share any point, including the collinear-touching cases.
pub fn segments_intersect(a: (P, P), b: (P, P)) -> bool {
    let d1 = orientation(b.0, b.1, a.0);
    let d2 = orientation(b.0, b.1, a.1);
    let d3 = orientation(a.0, a.1, b.0);
    let d4 = orientation(a.0, a.1, b.1);

    // Straddling on both sides is the ordinary crossing.
    if ((d1 > 0.0) != (d2 > 0.0)) && ((d3 > 0.0) != (d4 > 0.0)) {
        return true;
    }
    (d1 == 0.0 && on_segment(b.0, b.1, a.0))
        || (d2 == 0.0 && on_segment(b.0, b.1, a.1))
        || (d3 == 0.0 && on_segment(a.0, a.1, b.0))
        || (d4 == 0.0 && on_segment(a.0, a.1, b.1))
}

/// Distance between two segments: zero if they cross, otherwise the closest approach of an endpoint
/// to the other segment. Those four cases cover every configuration, including nearly-parallel ones.
pub fn segment_distance(a: (P, P), b: (P, P)) -> f32 {
    if segments_intersect(a, b) {
        return 0.0;
    }
    let candidates = [
        point_segment_distance_sq(a.0, b.0, b.1),
        point_segment_distance_sq(a.1, b.0, b.1),
        point_segment_distance_sq(b.0, a.0, a.1),
        point_segment_distance_sq(b.1, a.0, a.1),
    ];
    candidates.into_iter().fold(f32::INFINITY, f32::min).sqrt()
}

/// Ray casting. The polygon is implicitly closed (last vertex joins the first), and a lasso that the
/// user drew without lifting is therefore already a valid polygon.
pub fn point_in_polygon(p: P, polygon: &[P]) -> bool {
    if polygon.len() < 3 {
        return false;
    }
    let mut inside = false;
    let mut j = polygon.len() - 1;
    for i in 0..polygon.len() {
        let (xi, yi) = polygon[i];
        let (xj, yj) = polygon[j];
        // Half-open on `y` so a vertex is counted exactly once.
        if (yi > p.1) != (yj > p.1) {
            let t = (p.1 - yi) / (yj - yi);
            if p.0 < xi + t * (xj - xi) {
                inside = !inside;
            }
        }
        j = i;
    }
    inside
}

/// Bounds of a point run, or `None` for an empty one.
pub fn points_bounds(points: &[P]) -> Option<BBox> {
    let (first_x, first_y) = *points.first()?;
    let mut bounds = (first_x, first_y, first_x, first_y);
    for (x, y) in &points[1..] {
        bounds.0 = bounds.0.min(*x);
        bounds.1 = bounds.1.min(*y);
        bounds.2 = bounds.2.max(*x);
        bounds.3 = bounds.3.max(*y);
    }
    Some(bounds)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn point_to_segment_distance_clamps_to_the_endpoints() {
        // Perpendicular foot lands inside the segment.
        assert!((point_segment_distance((5.0, 3.0), (0.0, 0.0), (10.0, 0.0)) - 3.0).abs() < 1e-5);
        // Foot would land past the end, so the endpoint is nearest.
        assert!((point_segment_distance((13.0, 4.0), (0.0, 0.0), (10.0, 0.0)) - 5.0).abs() < 1e-5);
        // A zero-length segment behaves as a point.
        assert!((point_segment_distance((0.0, 3.0), (0.0, 0.0), (0.0, 0.0)) - 3.0).abs() < 1e-5);
    }

    #[test]
    fn crossing_segments_are_zero_apart() {
        let a = ((0.0, 0.0), (10.0, 10.0));
        let b = ((0.0, 10.0), (10.0, 0.0));
        assert!(segments_intersect(a, b));
        assert_eq!(segment_distance(a, b), 0.0);
    }

    #[test]
    fn parallel_segments_report_the_gap_between_them() {
        let a = ((0.0, 0.0), (10.0, 0.0));
        let b = ((0.0, 4.0), (10.0, 4.0));
        assert!(!segments_intersect(a, b));
        assert!((segment_distance(a, b) - 4.0).abs() < 1e-5);
    }

    #[test]
    fn touching_endpoints_count_as_intersecting() {
        let a = ((0.0, 0.0), (5.0, 0.0));
        let b = ((5.0, 0.0), (5.0, 5.0));
        assert!(segments_intersect(a, b));
    }

    #[test]
    fn collinear_overlap_counts_as_intersecting() {
        let a = ((0.0, 0.0), (10.0, 0.0));
        let b = ((5.0, 0.0), (15.0, 0.0));
        assert!(segments_intersect(a, b));
    }

    #[test]
    fn point_in_polygon_handles_inside_outside_and_the_boundary_region() {
        let square = [(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)];
        assert!(point_in_polygon((5.0, 5.0), &square));
        assert!(!point_in_polygon((15.0, 5.0), &square));
        assert!(!point_in_polygon((-1.0, 5.0), &square));

        // A concave shape, so a convexity assumption cannot sneak in.
        let arrow = [
            (0.0, 0.0),
            (10.0, 0.0),
            (10.0, 10.0),
            (5.0, 4.0),
            (0.0, 10.0),
        ];
        assert!(point_in_polygon((5.0, 1.0), &arrow));
        assert!(!point_in_polygon((5.0, 8.0), &arrow), "inside the notch");
    }

    #[test]
    fn a_degenerate_polygon_contains_nothing() {
        assert!(!point_in_polygon((0.0, 0.0), &[(0.0, 0.0), (1.0, 1.0)]));
    }

    #[test]
    fn inflating_a_box_keeps_the_radius_inside_the_query() {
        let box_ = (10.0, 10.0, 20.0, 20.0);
        assert_eq!(inflate(box_, 5.0), (5.0, 5.0, 25.0, 25.0));
        assert!(bounds_overlap(inflate(box_, 5.0), (0.0, 0.0, 21.0, 21.0)));
        assert!(!bounds_overlap(box_, (25.0, 25.0, 30.0, 30.0)));
    }
}
