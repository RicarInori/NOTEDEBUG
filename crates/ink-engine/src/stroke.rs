use std::fmt;

/// A page inside the current document. Pages are owned by `storage-sqlite`; the ink engine only
/// refers to them, so the id is opaque here.
pub type PageId = u64;

/// One sampled point of a stroke.
///
/// `t_us` is kept because both smoothing and width-from-speed depend on real time, not sample index.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StrokePoint {
    pub x: f32,
    pub y: f32,
    /// 0.0–1.0. Devices without pressure report 0.0, which [`crate::pipeline::width_for`] treats as
    /// "use the base width" rather than "invisible".
    pub pressure: f32,
    pub t_us: u64,
}

impl StrokePoint {
    pub fn new(x: f32, y: f32, pressure: f32, t_us: u64) -> Self {
        Self {
            x,
            y,
            pressure,
            t_us,
        }
    }

    pub fn distance_to(&self, other: &StrokePoint) -> f32 {
        let dx = self.x - other.x;
        let dy = self.y - other.y;
        (dx * dx + dy * dy).sqrt()
    }
}

/// Stable identity of a stroke, used by commands and by the eraser's hit list.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct StrokeId(pub u64);

impl fmt::Display for StrokeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "stroke#{}", self.0)
    }
}

/// A committed stroke: the unit of persistence and of undo (§3.2).
#[derive(Clone, Debug, PartialEq)]
pub struct Stroke {
    pub id: StrokeId,
    pub points: Vec<StrokePoint>,
    /// ARGB. Packed rather than a colour type so strokes stay trivially serialisable.
    pub color: u32,
    pub width_px: f32,
}

impl Stroke {
    pub fn new(id: StrokeId, points: Vec<StrokePoint>, color: u32, width_px: f32) -> Self {
        Self {
            id,
            points,
            color,
            width_px,
        }
    }

    /// Shift every point. The inverse of a translate command is the same with negated offsets, so
    /// this is all `InkCommand::Translate` needs.
    pub fn translate(&mut self, dx: f32, dy: f32) {
        for point in &mut self.points {
            point.x += dx;
            point.y += dy;
        }
    }

    /// Axis-aligned bounds as `(min_x, min_y, max_x, max_y)`.
    ///
    /// Used by the eraser and the selection tool for a cheap reject before the exact test (§3.3
    /// describes the grid index; this is what each grid cell stores).
    pub fn bounds(&self) -> Option<(f32, f32, f32, f32)> {
        let first = self.points.first()?;
        let mut bounds = (first.x, first.y, first.x, first.y);
        for point in &self.points[1..] {
            bounds.0 = bounds.0.min(point.x);
            bounds.1 = bounds.1.min(point.y);
            bounds.2 = bounds.2.max(point.x);
            bounds.3 = bounds.3.max(point.y);
        }
        Some(bounds)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stroke(points: &[(f32, f32)]) -> Stroke {
        Stroke::new(
            StrokeId(1),
            points
                .iter()
                .map(|(x, y)| StrokePoint::new(*x, *y, 0.5, 0))
                .collect(),
            0xFF1B1B1F,
            4.0,
        )
    }

    #[test]
    fn translate_shifts_every_point() {
        let mut s = stroke(&[(0.0, 0.0), (10.0, 0.0)]);
        s.translate(5.0, -2.0);
        assert_eq!(s.points[0].x, 5.0);
        assert_eq!(s.points[0].y, -2.0);
        assert_eq!(s.points[1].x, 15.0);
    }

    #[test]
    fn bounds_cover_all_points_and_an_empty_stroke_has_none() {
        let s = stroke(&[(3.0, 9.0), (-1.0, 2.0), (7.0, -4.0)]);
        assert_eq!(s.bounds(), Some((-1.0, -4.0, 7.0, 9.0)));
        assert_eq!(stroke(&[]).bounds(), None);
    }

    #[test]
    fn distance_is_euclidean() {
        let a = StrokePoint::new(0.0, 0.0, 0.0, 0);
        let b = StrokePoint::new(3.0, 4.0, 0.0, 0);
        assert_eq!(a.distance_to(&b), 5.0);
    }
}
