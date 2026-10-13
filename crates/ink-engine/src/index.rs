//! The per-page spatial index the eraser and the selection tool query (§3.3: "每页一个 Grid 索引，
//! cell=64px").
//!
//! ## Why a coarse filter is allowed to be wrong in only one direction
//!
//! A grid can safely return **too much** and never too little, because the exact geometric test
//! filters the candidates afterwards. The reverse — silently dropping a stroke that is in fact within
//! the eraser's radius — is the failure mode that would break §3.4's "笔画擦除命中率 100%" promise, and
//! it is what the insertion rule below is designed to make impossible.
//!
//! A stroke is filed under **every cell its bounding box overlaps**, not under the cells its point
//! sample happens to land in. Sampling would let a long segment slip through a cell that no endpoint
//! touches; a bounding box cannot miss it, at the cost of holding a diagonal stroke in more cells
//! than it geometrically needs.

use crate::geometry::{BBox, P};
use crate::stroke::{Stroke, StrokeId};
use std::collections::{BTreeSet, HashMap};

/// Cell edge length from §3.3. 64 px is a few stroke widths, so a typical stroke occupies one or two
/// cells and an eraser query touches a handful.
pub const GRID_CELL_PX: f32 = 64.0;

/// A page's strokes, bucketed by cell.
///
/// Queries return a [`BTreeSet`] so results are ordered — hit lists feed the undo log, and an
/// unordered one would make a test's failure impossible to reproduce.
pub struct StrokeIndex {
    cell: f32,
    cells: HashMap<(i32, i32), Vec<StrokeId>>,
    /// Kept so `insert`/`remove` can undo an insertion without re-deriving the boxes.
    bounds: HashMap<StrokeId, BBox>,
}

impl Default for StrokeIndex {
    fn default() -> Self {
        Self::new(GRID_CELL_PX)
    }
}

impl StrokeIndex {
    pub fn new(cell: f32) -> Self {
        Self {
            // A non-positive cell would divide by zero and then produce an infinite cell range.
            cell: if cell > 0.0 { cell } else { GRID_CELL_PX },
            cells: HashMap::new(),
            bounds: HashMap::new(),
        }
    }

    /// File every stroke on a page. Rebuilding is cheap enough at the frequencies that matter here
    /// (once per eraser stroke, once per selection) and, unlike incremental maintenance, cannot drift
    /// out of step with the undo log mutating the page behind its back.
    pub fn build(strokes: &[Stroke]) -> Self {
        let mut index = Self::default();
        for stroke in strokes {
            index.insert(stroke);
        }
        index
    }

    pub fn insert(&mut self, stroke: &Stroke) {
        let Some(bounds) = stroke.bounds() else {
            return;
        };
        for cell in self.cells_of(bounds) {
            self.cells.entry(cell).or_default().push(stroke.id);
        }
        self.bounds.insert(stroke.id, bounds);
    }

    pub fn len(&self) -> usize {
        self.bounds.len()
    }

    pub fn is_empty(&self) -> bool {
        self.bounds.is_empty()
    }

    /// Every id whose box could overlap `area`. A superset of the true answer, by construction.
    pub fn query(&self, area: BBox) -> BTreeSet<StrokeId> {
        let mut hits = BTreeSet::new();
        for cell in self.cells_of(area) {
            if let Some(ids) = self.cells.get(&cell) {
                hits.extend(ids.iter().copied());
            }
        }
        hits
    }

    /// Every id whose box could overlap any of the given areas. The eraser uses this to query each
    /// of its segments separately — see [`crate::erase`] for why that is what makes the filter
    /// complete rather than merely plausible.
    pub fn query_any(&self, areas: impl IntoIterator<Item = BBox>) -> BTreeSet<StrokeId> {
        let mut hits = BTreeSet::new();
        for area in areas {
            hits.extend(self.query(area));
        }
        hits
    }

    fn cells_of(&self, bounds: BBox) -> Vec<(i32, i32)> {
        let (min_x, min_y) = self.cell_of((bounds.0, bounds.1));
        let (max_x, max_y) = self.cell_of((bounds.2, bounds.3));
        let mut cells =
            Vec::with_capacity(((max_x - min_x + 1) as usize) * ((max_y - min_y + 1) as usize));
        for x in min_x..=max_x {
            for y in min_y..=max_y {
                cells.push((x, y));
            }
        }
        cells
    }

    fn cell_of(&self, point: P) -> (i32, i32) {
        (
            (point.0 / self.cell).floor() as i32,
            (point.1 / self.cell).floor() as i32,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::{bounds_overlap, inflate};
    use crate::stroke::StrokePoint;

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

    #[test]
    fn a_stroke_is_found_from_a_box_that_overlaps_its_bounds() {
        let s = stroke(1, &[(0.0, 0.0), (10.0, 0.0)]);
        let index = StrokeIndex::build(std::slice::from_ref(&s));
        assert_eq!(index.len(), 1);

        let hits = index.query((-5.0, -5.0, 5.0, 5.0));
        assert!(hits.contains(&StrokeId(1)));
        assert!(index.query((500.0, 500.0, 600.0, 600.0)).is_empty());
    }

    #[test]
    fn an_empty_stroke_is_indexed_nowhere() {
        let s = stroke(1, &[]);
        let index = StrokeIndex::build(std::slice::from_ref(&s));
        assert!(index.is_empty());
        assert!(index.query((-1e6, -1e6, 1e6, 1e6)).is_empty());
    }

    #[test]
    fn a_stroke_spanning_cells_is_filed_in_all_of_them() {
        // Crosses the origin diagonally, so it must be reachable from cells on both sides.
        let s = stroke(1, &[(-70.0, -70.0), (70.0, 70.0)]);
        let index = StrokeIndex::build(std::slice::from_ref(&s));

        for probe in [(-60.0, -60.0), (0.0, 0.0), (60.0, 60.0)] {
            let area = (probe.0, probe.1, probe.0, probe.1);
            assert!(
                index.query(area).contains(&StrokeId(1)),
                "a query at {probe:?} must still find the stroke"
            );
        }
    }

    /// The property the eraser depends on: for a randomly placed probe, every stroke whose geometry
    /// comes within `radius` of the probe's box must be in the candidate set. This is what makes the
    /// grid a complete filter, and it is checked by brute force against the true answer.
    #[test]
    fn the_query_never_drops_a_stroke_that_could_be_hit() {
        let strokes: Vec<Stroke> = (0..40)
            .map(|i| {
                let x = (i % 8) as f32 * 97.0 - 300.0;
                let y = (i / 8) as f32 * 83.0 - 200.0;
                stroke(i as u64, &[(x, y), (x + 55.0, y + 31.0)])
            })
            .collect();
        let index = StrokeIndex::build(&strokes);

        let radius = 12.0;
        // Sweep probes across and beyond the cloud. The step is coprime with the 64 px cell so the
        // sweep walks across cell boundaries rather than re-testing the same inside-cell positions.
        for probe in (0..=77).flat_map(|i| (0..=59).map(move |j| (i, j))) {
            let px = -420.0 + probe.0 as f32 * 11.0;
            let py = -320.0 + probe.1 as f32 * 11.0;
            let box_ = (px, py, px, py);
            let candidates = index.query(inflate(box_, radius));

            for s in &strokes {
                let truth = s
                    .bounds()
                    .map(|b| bounds_overlap(inflate(b, radius), box_))
                    .unwrap_or(false);
                if truth {
                    assert!(
                        candidates.contains(&s.id),
                        "stroke {} within {radius} of {:?} was filtered out",
                        s.id,
                        (px, py)
                    );
                }
            }
        }
    }
}
