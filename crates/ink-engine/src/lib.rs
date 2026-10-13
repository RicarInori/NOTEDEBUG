//! `ink-engine` — the handwriting engine (§3). **A plugin, not host code.**
//!
//! The note-taking capability is deliberately *not* part of the host: this crate is registered like
//! any other plugin and is **absent from `DEFAULT_ENABLED`**, so a fresh install has no note
//! capability until the user enables it in the control panel.
//!
//! ## Delivered so far
//!
//! The arithmetic and the edit log, both pure and both fully testable off-device:
//!
//! * [`stroke`] — the stroke model, the unit of persistence and of undo.
//! * [`pipeline`] — One Euro smoothing, 2 px equidistant resampling, pressure→width.
//! * [`undo`] — the document's edit log; see that module's note on why it is *not* a plugin effect.
//! * [`geometry`] / [`index`] — the exact tests and the per-page grid they are narrowed by.
//! * [`erase`] / [`select`] — §3.3's two eraser modes and §3.4's lasso and rectangle selection.
//!
//! ## Not here yet
//!
//! * the wgpu renderer: triangle-strip meshing, the incremental ring vertex buffer, one draw call;
//! * the highlighter's blend state and the `ink/stroke-committed` event;
//! * the Flutter canvas binding, and with it the live eraser preview the renderer owns.
//!
//! ## On the seam
//!
//! [`InkHandle`] wraps the concrete engine because the kernel's service table stores
//! `Arc<dyn Any + Send + Sync>` and `Arc<dyn Trait>` is not itself `Any`. When the first consumer
//! appears (the canvas bridge), the trait and the stroke vocabulary move to `inkweave-api` so that
//! consumers stop depending on this crate — the Definition/Provider split of §3.3.

pub mod erase;
pub mod geometry;
pub mod index;
pub mod pipeline;
pub mod select;
pub mod stroke;
pub mod undo;

use async_trait::async_trait;
use cordis_rs::{Context, Plugin, PluginError};
use inkweave_api::INK;
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::Arc;

pub use erase::{EraserMode, EraserPath};
pub use index::{StrokeIndex, GRID_CELL_PX};
pub use pipeline::{resample_equidistant, width_for, OneEuroFilter};
pub use select::SelectShape;
pub use stroke::{PageId, Stroke, StrokeId, StrokePoint};
pub use undo::{InkCommand, PageStore, UndoStack};

/// The guide's resampling step (§3.1).
pub const RESAMPLE_SPACING_PX: f32 = 2.0;

/// Service handle for the engine.
#[derive(Clone)]
pub struct InkHandle(pub Arc<InkEngine>);

/// Service key the engine is published under, re-exported for consumers.
pub const SERVICE_KEY: cordis_rs::ServiceKey = INK;

struct Inner {
    pages: PageStore,
    /// One log per page, as §3.2 specifies. Edits may span pages, so the log is keyed by page.
    histories: HashMap<PageId, UndoStack>,
    /// **One filter per axis.** A `OneEuroFilter` carries a single channel's state, so sharing one
    /// instance between x and y feeds the y estimate the x history — the two axes must stay
    /// independent or a straight horizontal stroke drifts sideways.
    filter_x: OneEuroFilter,
    filter_y: OneEuroFilter,
}

impl Inner {
    /// Run something against a page's history while it is detached from the map.
    ///
    /// `pages` and `histories` are always touched together, and two separate derefs of the guard would
    /// conflict even though the fields are disjoint — so the history is taken out, used, and put back.
    fn with_history<T>(
        &mut self,
        page: PageId,
        run: impl FnOnce(&mut UndoStack, &mut PageStore) -> T,
    ) -> T {
        let mut history = self.histories.remove(&page).unwrap_or_default();
        let outcome = run(&mut history, &mut self.pages);
        self.histories.insert(page, history);
        outcome
    }
}

/// One command if there is one, a batch that undoes as a unit if there are several, and nothing if the
/// gesture was a no-op.
fn grouped(page: PageId, mut commands: Vec<InkCommand>) -> Option<InkCommand> {
    match commands.len() {
        0 => None,
        1 => commands.pop(),
        _ => Some(InkCommand::Batch { page, commands }),
    }
}

/// What an eraser journey did, so the caller can report it without re-reading the page.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EraseOutcome {
    /// Strokes the eraser deleted outright.
    pub removed: usize,
    /// Strokes that survived the journey as two or more sub-strokes.
    pub split: usize,
}

impl EraseOutcome {
    /// Whether the journey produced an edit worth recording.
    pub fn changed(&self) -> bool {
        self.removed > 0 || self.split > 0
    }
}

/// The engine's public surface: commit a stroke, edit the document, undo it.
pub struct InkEngine {
    inner: Mutex<Inner>,
}

impl Default for InkEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl InkEngine {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(Inner {
                pages: PageStore::new(),
                histories: HashMap::new(),
                filter_x: OneEuroFilter::standard(),
                filter_y: OneEuroFilter::standard(),
            }),
        }
    }

    /// Run a raw sample batch through the pipeline and commit the resulting stroke.
    ///
    /// The width curve is *not* applied here: each point keeps its pressure and the renderer applies
    /// [`width_for`] per point. Baking it in would destroy the pressure data the eraser's split and
    /// the PDF export both need.
    pub fn commit_stroke(
        &self,
        page: PageId,
        samples: &[StrokePoint],
        color: u32,
        base_width_px: f32,
    ) -> Option<StrokeId> {
        if samples.is_empty() {
            return None;
        }
        let mut guard = self.inner.lock();
        let inner = &mut *guard;

        // Smooth along each axis; carrying filter state across strokes would drag the start.
        inner.filter_x.reset();
        inner.filter_y.reset();
        let smoothed: Vec<StrokePoint> = samples
            .iter()
            .map(|point| {
                let seconds = point.t_us as f32 / 1_000_000.0;
                StrokePoint {
                    x: inner.filter_x.filter(point.x, seconds),
                    y: inner.filter_y.filter(point.y, seconds),
                    pressure: point.pressure,
                    t_us: point.t_us,
                }
            })
            .collect();

        let points = resample_equidistant(&smoothed, RESAMPLE_SPACING_PX);
        let id = inner.pages.alloc_id();
        let stroke = Stroke::new(id, points, color, base_width_px);
        inner.with_history(page, |history, pages| {
            history.commit(pages, InkCommand::Add { page, stroke });
        });
        Some(id)
    }

    pub fn strokes(&self, page: PageId) -> Vec<Stroke> {
        self.inner.lock().pages.strokes(page).to_vec()
    }

    pub fn stroke_count(&self, page: PageId) -> usize {
        self.inner.lock().pages.count(page)
    }

    /// Erase a whole stroke, the default mode of §3.3. The command carries the stroke so undo can
    /// restore it unchanged.
    pub fn erase_stroke(&self, page: PageId, id: StrokeId) -> bool {
        let mut guard = self.inner.lock();
        let inner = &mut *guard;
        let Some(stroke) = inner.pages.take(page, id) else {
            return false;
        };
        inner.with_history(page, |history, pages| {
            history.commit(pages, InkCommand::Remove { page, stroke });
        });
        true
    }

    /// §3.3's eraser, both modes. The whole journey is **one** undo step, however many strokes it hit.
    ///
    /// The grid is rebuilt per call rather than maintained across edits. An erase happens once per
    /// gesture, and a rebuild cannot drift out of step with the undo log mutating the page behind its
    /// back — which incremental maintenance can, and which would show up as an eraser that quietly
    /// stops hitting strokes. A maintained index is a renderer concern, where it is queried per frame.
    pub fn erase(
        &self,
        page: PageId,
        path: &EraserPath,
        mode: EraserMode,
        radius_px: f32,
    ) -> EraseOutcome {
        let mut guard = self.inner.lock();
        let inner = &mut *guard;

        // A snapshot, so the page can be mutated (new ids, removals) while the journey is resolved.
        let strokes = inner.pages.strokes(page).to_vec();
        let index = StrokeIndex::build(&strokes);
        let candidates = erase::candidates(&index, path, radius_px);

        let mut commands = Vec::new();
        let mut outcome = EraseOutcome::default();
        for stroke in strokes.iter().filter(|s| candidates.contains(&s.id)) {
            match mode {
                EraserMode::Stroke => {
                    if erase::touches(stroke, path, radius_px) {
                        commands.push(InkCommand::Remove {
                            page,
                            stroke: stroke.clone(),
                        });
                        outcome.removed += 1;
                    }
                }
                EraserMode::Pixel => {
                    let survivors = erase::split(stroke, path, radius_px);
                    if erase::unchanged(stroke, &survivors) {
                        continue;
                    }
                    // Remove then re-add the surviving runs: the stroke keeps its colour and width,
                    // and undo puts the original back point for point.
                    commands.push(InkCommand::Remove {
                        page,
                        stroke: stroke.clone(),
                    });
                    let fragments = survivors.len();
                    for points in survivors {
                        let id = inner.pages.alloc_id();
                        commands.push(InkCommand::Add {
                            page,
                            stroke: Stroke::new(id, points, stroke.color, stroke.width_px),
                        });
                    }
                    if fragments == 0 {
                        // The journey covered the stroke completely, so nothing came back.
                        outcome.removed += 1;
                    } else {
                        outcome.split += 1;
                    }
                }
            }
        }

        if let Some(command) = grouped(page, commands) {
            inner.with_history(page, |history, pages| {
                history.commit(pages, command);
            });
        }
        outcome
    }

    /// §3.4's lasso and rectangle hit test. Read-only: acting on a selection is a separate command, so
    /// dragging a box over the page never edits the document.
    pub fn select(&self, page: PageId, shape: &SelectShape) -> Vec<StrokeId> {
        let guard = self.inner.lock();
        let strokes = guard.pages.strokes(page);
        let index = StrokeIndex::build(strokes);
        select::select(&index, strokes, shape)
    }

    /// Delete a selection as a single undo step (§3.4).
    pub fn delete_strokes(&self, page: PageId, ids: &[StrokeId]) -> usize {
        let mut guard = self.inner.lock();
        let inner = &mut *guard;

        let doomed: Vec<Stroke> = inner
            .pages
            .strokes(page)
            .iter()
            .filter(|stroke| ids.contains(&stroke.id))
            .cloned()
            .collect();
        let removed = doomed.len();

        let commands = doomed
            .into_iter()
            .map(|stroke| InkCommand::Remove { page, stroke })
            .collect();
        if let Some(command) = grouped(page, commands) {
            inner.with_history(page, |history, pages| {
                history.commit(pages, command);
            });
        }
        removed
    }

    pub fn translate(&self, page: PageId, ids: Vec<StrokeId>, dx: f32, dy: f32) {
        let mut guard = self.inner.lock();
        let inner = &mut *guard;
        // Recorded before the move: the inverse cannot be recomputed from the offsets without
        // drifting (see `InkCommand::Translate`).
        let before: Vec<Stroke> = inner
            .pages
            .strokes(page)
            .iter()
            .filter(|stroke| ids.contains(&stroke.id))
            .cloned()
            .collect();
        let command = InkCommand::Translate {
            page,
            ids,
            dx,
            dy,
            before,
        };
        inner.with_history(page, |history, pages| {
            history.commit(pages, command);
        });
    }

    pub fn undo(&self, page: PageId) -> bool {
        let mut guard = self.inner.lock();
        guard.with_history(page, |history, pages| history.undo(pages))
    }

    pub fn redo(&self, page: PageId) -> bool {
        let mut guard = self.inner.lock();
        guard.with_history(page, |history, pages| history.redo(pages))
    }

    pub fn undo_depth(&self, page: PageId) -> usize {
        self.inner
            .lock()
            .histories
            .get(&page)
            .map(UndoStack::undo_depth)
            .unwrap_or(0)
    }
}

/// Publishes the engine under [`SERVICE_KEY`].
///
/// A real [`Plugin`] implementation rather than [`cordis_rs::FnPlugin`], because `provide_shared` is
/// async and `FnPlugin` takes a synchronous closure.
struct InkEnginePlugin {
    engine: Arc<InkEngine>,
}

#[async_trait]
impl Plugin for InkEnginePlugin {
    fn name(&self) -> &str {
        "ink-engine"
    }

    async fn apply(&self, ctx: &Arc<Context>) -> Result<(), PluginError> {
        ctx.provide_shared(SERVICE_KEY, InkHandle(self.engine.clone()))
            .await;
        Ok(())
    }
}

/// Create an engine and the plugin that publishes it.
///
/// Returns both so a caller can hold the same handle the plugin publishes — the plugin is the seam,
/// not the owner. Re-enabling after a disable republishes the same engine, so in-progress work
/// survives an enable/disable cycle, which is what a user would expect.
pub fn shared() -> (Arc<InkEngine>, Arc<dyn Plugin>) {
    let engine = Arc::new(InkEngine::new());
    let plugin: Arc<dyn Plugin> = Arc::new(InkEnginePlugin {
        engine: engine.clone(),
    });
    (engine, plugin)
}

/// Convenience for callers that do not need the handle.
pub fn plugin() -> Arc<dyn Plugin> {
    shared().1
}

#[cfg(test)]
mod tests {
    use super::*;
    use cordis_rs::Context;

    fn samples() -> Vec<StrokePoint> {
        (0..=10)
            .map(|i| StrokePoint::new(i as f32 * 3.0, 0.0, 0.5, i as u64 * 2000))
            .collect()
    }

    #[tokio::test]
    async fn the_plugin_publishes_the_engine_under_the_ink_key() {
        let (engine, plugin) = shared();
        let root = Context::root();
        root.plugin(plugin).await.expect("mount");

        let published = root
            .get::<InkHandle>(SERVICE_KEY)
            .expect("the ink service must be reachable from the root scope");
        assert!(Arc::ptr_eq(&published.0, &engine));
    }

    #[test]
    fn committing_a_stroke_resamples_it() {
        let engine = InkEngine::new();
        let id = engine
            .commit_stroke(1, &samples(), 0xFF000000, 4.0)
            .expect("committed");
        assert_eq!(engine.stroke_count(1), 1);

        let stroke = &engine.strokes(1)[0];
        assert_eq!(stroke.id, id);
        // The point count is deliberately not asserted: with the guide's One Euro parameters the
        // filter smooths heavily at 500 Hz sampling, so the smoothed path is much shorter than the
        // raw samples suggest. What resampling guarantees is a *uniform* grid, so that is what is
        // checked — the pen-speed independence §3.1 asks for.
        assert!(stroke.points.len() >= 2, "got {}", stroke.points.len());
        let gaps: Vec<f32> = stroke
            .points
            .windows(2)
            .map(|window| window[0].distance_to(&window[1]))
            .collect();
        for gap in &gaps[..gaps.len() - 1] {
            assert!((gap - RESAMPLE_SPACING_PX).abs() < 0.05, "uneven gap {gap}");
        }
    }

    #[test]
    fn an_empty_batch_commits_nothing() {
        let engine = InkEngine::new();
        assert_eq!(engine.commit_stroke(1, &[], 0, 4.0), None);
        assert_eq!(engine.stroke_count(1), 0);
    }

    #[test]
    fn erase_then_undo_restores_the_same_stroke() {
        let engine = InkEngine::new();
        let id = engine
            .commit_stroke(1, &samples(), 0xFF000000, 4.0)
            .expect("committed");

        assert!(engine.erase_stroke(1, id));
        assert_eq!(engine.stroke_count(1), 0);

        assert!(engine.undo(1));
        assert_eq!(engine.stroke_count(1), 1);
        assert_eq!(
            engine.strokes(1)[0].id,
            id,
            "the same stroke, not a new one"
        );
    }

    #[test]
    fn erasing_something_absent_reports_false_and_logs_nothing() {
        let engine = InkEngine::new();
        assert!(!engine.erase_stroke(1, StrokeId(99)));
        assert_eq!(
            engine.undo_depth(1),
            0,
            "a no-op must not become an undo step"
        );
    }

    #[test]
    fn undo_and_redo_walk_the_history() {
        let engine = InkEngine::new();
        engine.commit_stroke(1, &samples(), 0, 4.0);
        engine.commit_stroke(1, &samples(), 0, 4.0);
        assert_eq!(engine.undo_depth(1), 2);

        assert!(engine.undo(1));
        assert_eq!(engine.stroke_count(1), 1);
        assert!(engine.redo(1));
        assert_eq!(engine.stroke_count(1), 2);
        assert!(!engine.redo(1));
    }

    #[test]
    fn translate_is_undone_exactly() {
        let engine = InkEngine::new();
        let id = engine
            .commit_stroke(1, &samples(), 0, 4.0)
            .expect("committed");
        let before = engine.strokes(1)[0].points.clone();

        engine.translate(1, vec![id], 12.0, -7.0);
        assert_ne!(engine.strokes(1)[0].points, before);

        engine.undo(1);
        assert_eq!(engine.strokes(1)[0].points, before);
    }

    #[test]
    fn pages_have_independent_histories() {
        let engine = InkEngine::new();
        engine.commit_stroke(1, &samples(), 0, 4.0);
        engine.commit_stroke(2, &samples(), 0, 4.0);

        engine.undo(1);
        assert_eq!(engine.stroke_count(1), 0);
        assert_eq!(
            engine.stroke_count(2),
            1,
            "undo must not reach across pages"
        );
    }

    // --- §3.3 / §3.4 tools ---------------------------------------------------

    /// A horizontal sample run of the shape the pen pipeline produces.
    fn line(y: f32) -> Vec<StrokePoint> {
        (0..=40)
            .map(|k| StrokePoint::new(k as f32 * 4.0, y, 0.5, k as u64 * 2000))
            .collect()
    }

    /// Regression: a `OneEuroFilter` holds one channel's state, so running a single instance over x
    /// and then y made the y estimate follow the x history — the second call arrives with `dt == 0`
    /// and returns the x estimate. Every stroke collapsed into a narrow horizontal band, which is
    /// invisible in a unit test that only checks spacing and shows up as an eraser that hits
    /// everything at once.
    #[test]
    fn smoothing_keeps_the_two_axes_independent() {
        let engine = InkEngine::new();

        // A straight run at y = 200 must stay at y = 200 all the way along.
        engine.commit_stroke(1, &line(200.0), 0xFF000000, 4.0);
        // And a diagonal must stay diagonal.
        let diagonal: Vec<StrokePoint> = (0..=40)
            .map(|k| {
                let v = k as f32 * 4.0;
                StrokePoint::new(v, v, 0.5, k as u64 * 2000)
            })
            .collect();
        engine.commit_stroke(1, &diagonal, 0xFF000000, 4.0);

        let strokes = engine.strokes(1);
        for point in &strokes[0].points {
            assert!(
                (point.y - 200.0).abs() < 1e-3,
                "the y channel was dragged by x: {point:?}"
            );
        }
        for point in &strokes[1].points {
            assert!(
                (point.y - point.x).abs() < 1e-2,
                "a diagonal must not be sheared: {point:?}"
            );
        }
    }

    /// A vertical journey through the middle of a stroke, which must therefore erase it.
    fn crossing(stroke: &Stroke) -> EraserPath {
        let mid = stroke.points[stroke.points.len() / 2];
        EraserPath::from_points([(mid.x, mid.y - 10.0), (mid.x, mid.y + 10.0)])
    }

    /// The strokes of a page, ordered, so comparisons do not depend on whether an undo re-appended
    /// something at the end — see the note on ordering in [`undo`].
    fn sorted(engine: &InkEngine, page: PageId) -> Vec<Stroke> {
        let mut strokes = engine.strokes(page);
        strokes.sort_by_key(|stroke| stroke.id);
        strokes
    }

    /// M1's acceptance criterion, stated as the test: every stroke in a test set is hit by a journey
    /// that crosses it — no misses and no over-reach onto a neighbour.
    #[test]
    fn the_stroke_eraser_hits_every_stroke_in_a_test_set() {
        let engine = InkEngine::new();
        // Far enough apart that a 20 px journey with radius 6 cannot reach a neighbour.
        let ids: Vec<StrokeId> = (0..20)
            .map(|i| {
                engine
                    .commit_stroke(1, &line(i as f32 * 200.0), 0xFF000000, 4.0)
                    .expect("committed")
            })
            .collect();
        assert_eq!(engine.stroke_count(1), 20);

        for (hit, id) in ids.iter().enumerate() {
            let stroke = engine
                .strokes(1)
                .into_iter()
                .find(|stroke| stroke.id == *id)
                .unwrap_or_else(|| panic!("{id} should still be present"));
            let outcome = engine.erase(1, &crossing(&stroke), EraserMode::Stroke, 6.0);
            assert_eq!(
                outcome.removed,
                1,
                "stroke {id} (hit {} of {}) must be erased exactly once",
                hit + 1,
                ids.len()
            );
            assert_eq!(engine.stroke_count(1), ids.len() - hit - 1);
        }
        assert_eq!(engine.stroke_count(1), 0);
    }

    #[test]
    fn one_eraser_journey_is_one_undo_step_however_many_strokes_it_hits() {
        let engine = InkEngine::new();
        for i in 0..3 {
            engine.commit_stroke(1, &line(i as f32 * 100.0), 0xFF000000, 4.0);
        }
        let before = sorted(&engine, 1);
        let depth = engine.undo_depth(1);

        // One vertical sweep across all three.
        let mid_x = before[0].points[before[0].points.len() / 2].x;
        let path = EraserPath::from_points([(mid_x, -40.0), (mid_x, 240.0)]);
        let outcome = engine.erase(1, &path, EraserMode::Stroke, 8.0);

        assert_eq!(
            outcome.removed, 3,
            "the journey must reach all three strokes"
        );
        assert_eq!(engine.stroke_count(1), 0);
        assert_eq!(
            engine.undo_depth(1),
            depth + 1,
            "three strokes, but only one user action"
        );

        assert!(engine.undo(1));
        assert_eq!(
            sorted(&engine, 1),
            before,
            "one undo brings all three back, unchanged"
        );
    }

    #[test]
    fn pixel_erasing_cuts_the_stroke_and_undo_restores_it_point_for_point() {
        let engine = InkEngine::new();
        engine.commit_stroke(1, &line(0.0), 0xFF112233, 6.0);
        let original = engine.strokes(1)[0].clone();
        assert!(original.points.len() > 4, "need a stroke worth cutting");

        let mid = original.points[original.points.len() / 2];
        let path = EraserPath::from_points([(mid.x, -30.0), (mid.x, 30.0)]);
        let outcome = engine.erase(1, &path, EraserMode::Pixel, 10.0);
        assert_eq!(outcome.split, 1);
        assert_eq!(
            outcome.removed, 0,
            "the ends survive, so nothing was removed"
        );

        let fragments = engine.strokes(1);
        assert!(
            fragments.len() >= 2,
            "the stroke must survive as its two ends, got {}",
            fragments.len()
        );
        for fragment in &fragments {
            assert_eq!(fragment.color, original.color);
            assert_eq!(fragment.width_px, original.width_px);
            // Every surviving sample is one of the originals: a cut is not a re-render.
            for point in &fragment.points {
                assert!(original.points.contains(point));
            }
        }

        assert!(engine.undo(1));
        assert_eq!(
            engine.strokes(1),
            vec![original],
            "undo restores the stroke as it was, not as two pieces"
        );
    }

    #[test]
    fn pixel_mode_removes_a_stroke_it_covers_completely() {
        let engine = InkEngine::new();
        engine.commit_stroke(1, &line(0.0), 0xFF000000, 4.0);
        let points = engine.strokes(1)[0].points.clone();
        let first = points.first().expect("non-empty").x - 30.0;
        let last = points.last().expect("non-empty").x + 30.0;

        // Run the eraser along the stroke, past both ends.
        let path = EraserPath::from_points([(first, 0.0), (last, 0.0)]);
        let outcome = engine.erase(1, &path, EraserMode::Pixel, 12.0);

        assert_eq!(outcome.removed, 1);
        assert_eq!(outcome.split, 0);
        assert_eq!(
            engine.stroke_count(1),
            0,
            "no invisible single-sample fragments may be left behind"
        );

        assert!(engine.undo(1));
        assert_eq!(engine.stroke_count(1), 1);
    }

    #[test]
    fn an_eraser_journey_that_hits_nothing_records_nothing() {
        let engine = InkEngine::new();
        engine.commit_stroke(1, &line(0.0), 0xFF000000, 4.0);
        let depth = engine.undo_depth(1);

        let path = EraserPath::from_points([(10_000.0, 10_000.0), (10_100.0, 10_100.0)]);
        let outcome = engine.erase(1, &path, EraserMode::Stroke, 8.0);

        assert_eq!(outcome, EraseOutcome::default());
        assert!(!outcome.changed());
        assert_eq!(engine.undo_depth(1), depth, "no edit, no undo press");
        assert_eq!(engine.stroke_count(1), 1);
    }

    #[test]
    fn a_selection_can_be_deleted_and_undone_as_one_step() {
        let engine = InkEngine::new();
        for i in 0..4 {
            engine.commit_stroke(1, &line(i as f32 * 300.0), 0xFF000000, 4.0);
        }
        let all = engine.strokes(1);
        let first = all[0].bounds().expect("non-empty");
        let second = all[1].bounds().expect("non-empty");

        // A box around the first two strokes only.
        let shape = SelectShape::from_rectangle(
            (first.0.min(second.0) - 10.0, first.1.min(second.1) - 10.0),
            (first.2.max(second.2) + 10.0, first.3.max(second.3) + 10.0),
        );
        let selected = engine.select(1, &shape);
        assert_eq!(selected, vec![all[0].id, all[1].id]);

        let depth = engine.undo_depth(1);
        assert_eq!(engine.delete_strokes(1, &selected), 2);
        assert_eq!(engine.stroke_count(1), 2);
        assert_eq!(
            engine.undo_depth(1),
            depth + 1,
            "one press for the selection"
        );

        assert!(engine.undo(1));
        assert_eq!(engine.stroke_count(1), 4);
    }

    #[test]
    fn deleting_an_empty_selection_is_not_an_edit() {
        let engine = InkEngine::new();
        engine.commit_stroke(1, &line(0.0), 0xFF000000, 4.0);
        let depth = engine.undo_depth(1);

        assert_eq!(engine.delete_strokes(1, &[StrokeId(999)]), 0);
        assert_eq!(engine.undo_depth(1), depth);
        assert_eq!(engine.stroke_count(1), 1);
    }
}
