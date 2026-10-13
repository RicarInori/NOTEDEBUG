//! Triangle meshing (§3.1's "Triangle Strip 网格化（切线平均法线，圆角 join）").
//!
//! This is the last pure step before the GPU: a stroke and a pressure curve in, a vertex/index pair
//! out. Everything the renderer needs and nothing about wgpu, which is why it can be tested to the
//! pixel off-device.
//!
//! ## Why per-segment normals instead of one averaged normal per point
//!
//! The literal reading of "切线平均法线" is a mitred ribbon: one normal per point, the average of the
//! two adjacent segment directions. That construction is one vertex pair per point and it *pinches*
//! on the inside of a turn while shooting a spike out of the outside, and the spike grows without
//! bound as the turn sharpens. Real ink renderers avoid it for exactly that reason.
//!
//! So the ribbon is built from **per-segment** normals — each segment contributes a plain rectangle
//! whose corners are exactly `half_width` from the centreline — and every joint gets an explicit
//! **round join**: a fan of radius `half_width` filling the wedge the two rectangles leave open. The
//! result has no mitre limit to tune, never self-intersects far from the centreline, and gives the
//! round joins §3.1 asks for. Caps are the same fan, swept over half a turn.
//!
//! The invariant this buys is worth stating because the tests check it: **every vertex is at most
//! `half_width` away from the centreline**. There is no configuration of input where a vertex shoots
//! off toward infinity.

use crate::pipeline::width_for;
use crate::stroke::{Stroke, StrokePoint};
use inkweave_api::stylus::PressureCurve;

/// Steps used to round a join or a cap.
///
/// Eight puts the chord error of a half turn at `r · (1 − cos(π/16)) ≈ 0.02 r` — under a tenth of a
/// pixel at the half-widths a pen produces — which is finer than what anti-aliasing can resolve.
pub const ROUND_SEGMENTS: usize = 8;

/// Samples closer than this count as the same point. They contribute no travel, and keeping them
/// would mean dividing by a zero-length segment.
const DUPLICATE_EPSILON: f32 = 1e-3;

/// Directions closer than this are treated as parallel, so a straight run emits no join fan at all.
const ANGLE_EPSILON: f32 = 1e-4;

/// One vertex. Deliberately not interleaved as a byte blob yet: the layout the GPU wants is a
/// renderer decision, and keeping it as struct-of-fields is what makes the mesh assertable.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MeshVertex {
    pub x: f32,
    pub y: f32,
    /// Distance along the stroke from its first sample, in pixels — §3.5 stretches the pencil texture's
    /// noise along this, which is why it is a length and not a normalised 0..1.
    pub u: f32,
    /// Half the stroke width here. Carried so a shader can round the caps and fade the edges without
    /// a second attribute.
    pub half_width: f32,
}

/// A stroke's geometry, ready to upload. One vertex buffer, one index buffer, one draw call.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StrokeMesh {
    pub vertices: Vec<MeshVertex>,
    pub indices: Vec<u32>,
}

impl StrokeMesh {
    pub fn is_empty(&self) -> bool {
        self.indices.is_empty()
    }

    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }

    /// Total area covered by the triangles.
    ///
    /// The joins and caps overlap the rectangles slightly, so this is the *drawn* area rather than the
    /// exact swept area — which is what makes it useful as a coverage check and, later, for deciding
    /// when a stroke has become too small to be worth a draw call.
    pub fn area(&self) -> f32 {
        let mut total = 0.0;
        for triangle in self.indices.as_chunks::<3>().0 {
            let a = self.vertices[triangle[0] as usize];
            let b = self.vertices[triangle[1] as usize];
            let c = self.vertices[triangle[2] as usize];
            let cross = (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x);
            total += cross.abs() * 0.5;
        }
        total
    }

    fn push_vertex(&mut self, x: f32, y: f32, u: f32, half_width: f32) -> u32 {
        let index = self.vertices.len() as u32;
        self.vertices.push(MeshVertex {
            x,
            y,
            u,
            half_width,
        });
        index
    }

    /// A wedge of triangles sharing `centre`.
    ///
    /// `sweep` may be negative, and the triangles are wound to stay counter-clockwise either way:
    /// back-face culling is a renderer option only if the winding is consistent, and an inconsistent
    /// winding is the kind of bug that shows up as a stroke disappearing at one particular angle.
    fn fan(&mut self, centre: (f32, f32), radius: f32, start: f32, sweep: f32, u: f32) {
        if radius <= 0.0 || sweep == 0.0 {
            return;
        }
        let centre_index = self.push_vertex(centre.0, centre.1, u, radius);
        let steps = ROUND_SEGMENTS.max(1);

        let mut arc = Vec::with_capacity(steps + 1);
        for step in 0..=steps {
            let angle = start + sweep * (step as f32 / steps as f32);
            let (sin, cos) = angle.sin_cos();
            arc.push(self.push_vertex(centre.0 + cos * radius, centre.1 + sin * radius, u, radius));
        }
        for step in 0..steps {
            if sweep >= 0.0 {
                self.indices
                    .extend([centre_index, arc[step], arc[step + 1]]);
            } else {
                self.indices
                    .extend([centre_index, arc[step + 1], arc[step]]);
            }
        }
    }
}

/// Mesh one stroke. `curve` is the user's pressure calibration; the width curve is applied here
/// rather than baked into the points, because the eraser's split and the PDF export both need the
/// pressure values intact.
pub fn mesh_stroke(stroke: &Stroke, curve: &PressureCurve) -> StrokeMesh {
    let points = without_duplicates(&stroke.points);
    if points.is_empty() {
        return StrokeMesh::default();
    }

    let half_widths: Vec<f32> = points
        .iter()
        .map(|point| width_for(point.pressure, curve, stroke.width_px) * 0.5)
        .collect();
    let along = arc_lengths(&points);

    let mut mesh = StrokeMesh::default();

    // A pen touch that produced one sample is a dot, not nothing.
    if let [only] = points.as_slice() {
        mesh.fan(
            (only.x, only.y),
            half_widths[0],
            0.0,
            std::f32::consts::TAU,
            along[0],
        );
        return mesh;
    }

    for (i, window) in points.windows(2).enumerate() {
        let (from, to) = (window[0], window[1]);
        let normal = segment_normal(from, to);
        let (half_from, half_to) = (half_widths[i], half_widths[i + 1]);

        let left_from = mesh.push_vertex(
            from.x + normal.0 * half_from,
            from.y + normal.1 * half_from,
            along[i],
            half_from,
        );
        let right_from = mesh.push_vertex(
            from.x - normal.0 * half_from,
            from.y - normal.1 * half_from,
            along[i],
            half_from,
        );
        let right_to = mesh.push_vertex(
            to.x - normal.0 * half_to,
            to.y - normal.1 * half_to,
            along[i + 1],
            half_to,
        );
        let left_to = mesh.push_vertex(
            to.x + normal.0 * half_to,
            to.y + normal.1 * half_to,
            along[i + 1],
            half_to,
        );
        mesh.indices.extend([
            left_from, right_from, right_to, left_from, right_to, left_to,
        ]);
    }

    for i in 1..points.len() - 1 {
        let previous = segment_normal(points[i - 1], points[i]);
        let next = segment_normal(points[i], points[i + 1]);
        let turn = turn_direction(
            segment_direction(points[i - 1], points[i]),
            segment_direction(points[i], points[i + 1]),
        );

        // The wedge to fill is on the *outside* of the turn. With `normal` being the left normal, the
        // outside is the right side of travel when turning left and the left side when turning right.
        let (from, to) = if turn > 0.0 {
            (negate(previous), negate(next))
        } else {
            (previous, next)
        };
        let start = angle_of(from);
        let sweep = wrap_angle(angle_of(to) - start);
        if sweep.abs() < ANGLE_EPSILON {
            continue; // collinear: the two rectangles already meet
        }
        mesh.fan(
            (points[i].x, points[i].y),
            half_widths[i],
            start,
            sweep,
            along[i],
        );
    }

    // Half-turns at the ends. The start cap sweeps away from the direction of travel and the end cap
    // away from the direction of arrival, which is what makes both of them cover the far side.
    let last = points.len() - 1;
    let start_normal = segment_normal(points[0], points[1]);
    mesh.fan(
        (points[0].x, points[0].y),
        half_widths[0],
        angle_of(start_normal),
        std::f32::consts::PI,
        along[0],
    );
    let end_normal = segment_normal(points[last - 1], points[last]);
    mesh.fan(
        (points[last].x, points[last].y),
        half_widths[last],
        angle_of(end_normal),
        -std::f32::consts::PI,
        along[last],
    );

    mesh
}

/// Drop samples that repeat the previous one. They describe no travel, so a segment between them
/// would have no direction to speak of.
fn without_duplicates(points: &[StrokePoint]) -> Vec<StrokePoint> {
    let mut kept: Vec<StrokePoint> = Vec::with_capacity(points.len());
    for point in points {
        match kept.last() {
            Some(previous) if previous.distance_to(point) <= DUPLICATE_EPSILON => continue,
            _ => kept.push(*point),
        }
    }
    kept
}

/// Distance along the stroke at each sample.
fn arc_lengths(points: &[StrokePoint]) -> Vec<f32> {
    let mut along = Vec::with_capacity(points.len());
    let mut travelled = 0.0;
    along.push(0.0);
    for window in points.windows(2) {
        travelled += window[0].distance_to(&window[1]);
        along.push(travelled);
    }
    along
}

/// The left normal of a segment: the direction rotated a quarter turn counter-clockwise.
fn segment_normal(from: StrokePoint, to: StrokePoint) -> (f32, f32) {
    let (dx, dy) = (to.x - from.x, to.y - from.y);
    let length = (dx * dx + dy * dy).sqrt();
    if length <= f32::EPSILON {
        return (0.0, 0.0);
    }
    (-dy / length, dx / length)
}

fn segment_direction(from: StrokePoint, to: StrokePoint) -> (f32, f32) {
    let (dx, dy) = (to.x - from.x, to.y - from.y);
    let length = (dx * dx + dy * dy).sqrt();
    if length <= f32::EPSILON {
        return (0.0, 0.0);
    }
    (dx / length, dy / length)
}

/// Positive for a left turn, negative for a right turn, zero when straight.
fn turn_direction(previous: (f32, f32), next: (f32, f32)) -> f32 {
    previous.0 * next.1 - previous.1 * next.0
}

fn negate(v: (f32, f32)) -> (f32, f32) {
    (-v.0, -v.1)
}

fn angle_of(v: (f32, f32)) -> f32 {
    v.1.atan2(v.0)
}

/// Fold an angle into `(-π, π]`, so a join always sweeps the short way round.
fn wrap_angle(angle: f32) -> f32 {
    let tau = std::f32::consts::TAU;
    let mut wrapped = angle % tau;
    if wrapped > std::f32::consts::PI {
        wrapped -= tau;
    } else if wrapped <= -std::f32::consts::PI {
        wrapped += tau;
    }
    wrapped
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stroke(points: &[(f32, f32)], pressure: &[f32], width: f32) -> Stroke {
        assert_eq!(points.len(), pressure.len());
        Stroke::new(
            crate::stroke::StrokeId(1),
            points
                .iter()
                .zip(pressure)
                .enumerate()
                .map(|(i, ((x, y), p))| StrokePoint::new(*x, *y, *p, i as u64 * 2000))
                .collect(),
            0xFF000000,
            width,
        )
    }

    /// A horizontal run of samples 2 px apart, all at the full pressure.
    fn straight(length: f32, width: f32) -> Stroke {
        let count = (length / 2.0) as usize + 1;
        let points: Vec<(f32, f32)> = (0..count).map(|i| (i as f32 * 2.0, 0.0)).collect();
        let pressure = vec![1.0; count];
        stroke(&points, &pressure, width)
    }

    fn curve() -> PressureCurve {
        PressureCurve::linear()
    }

    fn vertex(mesh: &StrokeMesh, index: u32) -> MeshVertex {
        mesh.vertices[index as usize]
    }

    fn triangle(mesh: &StrokeMesh, i: usize) -> (MeshVertex, MeshVertex, MeshVertex) {
        let t = &mesh.indices[i * 3..i * 3 + 3];
        (vertex(mesh, t[0]), vertex(mesh, t[1]), vertex(mesh, t[2]))
    }

    fn cross(a: MeshVertex, b: MeshVertex, c: MeshVertex) -> f32 {
        (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x)
    }

    /// Whether a point lies inside any triangle, edges counted as inside.
    fn contains(mesh: &StrokeMesh, p: (f32, f32)) -> bool {
        for i in 0..mesh.triangle_count() {
            let (a, b, c) = triangle(mesh, i);
            let d1 = cross(
                a,
                b,
                MeshVertex {
                    x: p.0,
                    y: p.1,
                    u: 0.0,
                    half_width: 0.0,
                },
            );
            let d2 = cross(
                b,
                c,
                MeshVertex {
                    x: p.0,
                    y: p.1,
                    u: 0.0,
                    half_width: 0.0,
                },
            );
            let d3 = cross(
                c,
                a,
                MeshVertex {
                    x: p.0,
                    y: p.1,
                    u: 0.0,
                    half_width: 0.0,
                },
            );
            let has_negative = d1 < -1e-6 || d2 < -1e-6 || d3 < -1e-6;
            let has_positive = d1 > 1e-6 || d2 > 1e-6 || d3 > 1e-6;
            if !(has_negative && has_positive) {
                return true;
            }
        }
        false
    }

    #[test]
    fn a_straight_stroke_covers_its_length_plus_the_caps() {
        let mesh = mesh_stroke(&straight(20.0, 10.0), &curve());
        assert!(!mesh.is_empty());

        let body = 20.0 * 10.0;
        let caps = std::f32::consts::PI * 5.0 * 5.0;
        assert!(
            mesh.area() > body,
            "the caps must add area, got {}",
            mesh.area()
        );
        assert!(
            mesh.area() < body + caps,
            "a polygonal fan is inscribed in the disc, got {} vs {}",
            mesh.area(),
            body + caps
        );
        // Within a couple of percent of exact: the fan error, not a shape error.
        assert!((mesh.area() - (body + caps)).abs() < (body + caps) * 0.05);
    }

    #[test]
    fn every_vertex_stays_within_half_a_width_of_the_centreline() {
        // Corners: 30°, 90° and a near-reversal. A mitred ribbon spikes on the last two.
        let points = [
            (0.0, 0.0),
            (10.0, 0.0),
            (20.0, 8.0),
            (30.0, 8.0),
            (30.0, -8.0),
        ];
        let pressure = vec![1.0; points.len()];
        let mesh = mesh_stroke(&stroke(&points, &pressure, 12.0), &curve());

        let centreline: Vec<(f32, f32)> = points.to_vec();
        for v in &mesh.vertices {
            let distance = distance_to_polyline((v.x, v.y), &centreline);
            assert!(
                distance <= v.half_width + 1e-3,
                "vertex {:?} is {} from the centreline but only {} wide",
                (v.x, v.y),
                distance,
                v.half_width
            );
        }
    }

    /// Point-to-polyline distance, the reference the invariant above is measured against.
    fn distance_to_polyline(p: (f32, f32), polyline: &[(f32, f32)]) -> f32 {
        let mut best = f32::INFINITY;
        for window in polyline.windows(2) {
            best = best.min(crate::geometry::point_segment_distance(
                p, window[0], window[1],
            ));
        }
        best
    }

    #[test]
    fn a_sharp_turn_is_filled_on_the_outside() {
        // A right angle: the wedge at (10, 0) on the far side of the corner must be covered, and it is
        // exactly what a per-point averaged normal leaves hollow.
        let points = [(0.0, 0.0), (10.0, 0.0), (10.0, 10.0)];
        let pressure = vec![1.0; points.len()];
        let mesh = mesh_stroke(&stroke(&points, &pressure, 10.0), &curve());

        // The outer quadrant at the corner is x > 10 and y < 0.
        let probe = (12.5, -2.5);
        assert!(
            contains(&mesh, probe),
            "the outside of the corner must be filled"
        );
        // And the inner quadrant is covered too, by the rectangles overlapping.
        assert!(contains(&mesh, (9.0, 1.0)));
    }

    #[test]
    fn pressure_shows_up_as_width() {
        // 0.25 → 1.0 through a linear curve, so the stroke starts at a quarter width.
        let points: Vec<(f32, f32)> = (0..=20).map(|i| (i as f32 * 2.0, 0.0)).collect();
        let pressure: Vec<f32> = (0..=20).map(|i| 0.25 + 0.75 * i as f32 / 20.0).collect();
        let mesh = mesh_stroke(&stroke(&points, &pressure, 20.0), &curve());

        let extent = |target_u: f32| -> f32 {
            mesh.vertices
                .iter()
                .filter(|v| (v.u - target_u).abs() < 0.6)
                .map(|v| v.half_width)
                .fold(0.0_f32, f32::max)
        };
        let at_start = extent(0.0);
        let at_end = extent(40.0);

        assert!(
            (at_start - 2.5).abs() < 0.2,
            "a quarter of a 20 px stroke is 5 px wide, got {at_start}"
        );
        assert!((at_end - 10.0).abs() < 0.2, "got {at_end}");
    }

    #[test]
    fn u_runs_from_zero_to_the_stroke_length_without_going_backwards() {
        let mesh = mesh_stroke(
            &stroke(
                &[(0.0, 0.0), (10.0, 0.0), (10.0, 12.0)],
                &[1.0, 1.0, 1.0],
                8.0,
            ),
            &curve(),
        );

        let mut highest = 0.0_f32;
        for v in &mesh.vertices {
            assert!(v.u >= -1e-4, "u must not go negative: {}", v.u);
            // Joins and caps are emitted after the ribbon, so the vertex list is not sorted by u;
            // what must hold is that u starts at zero and never exceeds the total arc length.
            assert!(
                v.u <= 22.0 + 1e-3,
                "u must not exceed the arc length: {}",
                v.u
            );
            highest = highest.max(v.u);
        }
        assert!((highest - 22.0).abs() < 1e-3, "got {highest}");
        assert!(mesh.vertices.iter().any(|v| v.u.abs() < 1e-4));
    }

    #[test]
    fn every_triangle_is_counter_clockwise() {
        let cases: Vec<Stroke> = vec![
            straight(20.0, 8.0),
            stroke(
                &[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0)],
                &[1.0, 1.0, 1.0],
                10.0,
            ),
            stroke(
                &[
                    (0.0, 0.0),
                    (10.0, 6.0),
                    (20.0, 6.0),
                    (30.0, -4.0),
                    (30.0, 6.0),
                ],
                &[1.0; 5],
                8.0,
            ),
            stroke(&[(4.0, 4.0)], &[1.0], 8.0),
        ];
        for case in &cases {
            let mesh = mesh_stroke(case, &curve());
            assert!(!mesh.is_empty());
            for i in 0..mesh.triangle_count() {
                let (a, b, c) = triangle(&mesh, i);
                assert!(
                    cross(a, b, c) > 0.0,
                    "triangle {i} winds the wrong way in {case:?}"
                );
            }
        }
    }

    #[test]
    fn a_single_sample_becomes_a_dot() {
        let mesh = mesh_stroke(&stroke(&[(3.0, 4.0)], &[1.0], 10.0), &curve());
        assert!(!mesh.is_empty());
        // A full turn, so the fan is a disc inscribed in the circle of radius 5.
        assert!(mesh.area() > 70.0 && mesh.area() < std::f32::consts::PI * 25.0);
        for v in &mesh.vertices {
            let distance = ((v.x - 3.0).powi(2) + (v.y - 4.0).powi(2)).sqrt();
            assert!(distance <= 5.0 + 1e-3, "a dot must stay inside its radius");
        }
    }

    #[test]
    fn degenerate_input_meshes_to_nothing_rather_than_nan() {
        assert!(mesh_stroke(&stroke(&[], &[], 8.0), &curve()).is_empty());

        // Repeats describe no travel: they collapse to the one point they represent, which is a dot.
        let repeated = stroke(&[(2.0, 2.0), (2.0, 2.0), (2.0, 2.0)], &[1.0, 1.0, 1.0], 6.0);
        let mesh = mesh_stroke(&repeated, &curve());
        assert!(!mesh.is_empty());
        assert!(mesh
            .vertices
            .iter()
            .all(|v| v.x.is_finite() && v.y.is_finite()));
    }

    #[test]
    fn a_straight_run_emits_no_join_fans() {
        // A collinear pair leaves the two rectangles meeting already, so the mesh is exactly the 20
        // segment quads plus the two caps.
        let mesh = mesh_stroke(&straight(40.0, 8.0), &curve());
        assert_eq!(
            mesh.triangle_count(),
            20 * 2 + 2 * ROUND_SEGMENTS,
            "20 segments give 20 quads, plus 8 triangles per cap"
        );
    }

    #[test]
    fn the_caps_are_the_shape_of_the_ends() {
        let mesh = mesh_stroke(&straight(20.0, 10.0), &curve());

        // A cap reaches half a width past the end...
        assert!(contains(&mesh, (-4.5, 0.0)), "behind the start");
        assert!(!contains(&mesh, (-6.0, 0.0)), "but not arbitrarily far");
        assert!(contains(&mesh, (24.5, 0.0)), "ahead of the end");
        assert!(!contains(&mesh, (26.0, 0.0)));

        // ...and it is *round*: the corner of the square a flat cap would occupy is outside, while a
        // point on the circle at the same diagonal is inside.
        assert!(
            contains(&mesh, (-3.0, 3.0)),
            "on the circle at 135 degrees, inside"
        );
        assert!(
            !contains(&mesh, (-4.5, 4.5)),
            "the square cap's corner must not be covered"
        );
    }
}
