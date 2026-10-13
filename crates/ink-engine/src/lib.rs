//! `ink-engine` — the handwriting engine (§3). **A plugin, not host code.**
//!
//! The note-taking capability is deliberately *not* part of the host: this crate is registered like
//! any other plugin and is **absent from `DEFAULT_ENABLED`**, so a fresh install has no note
//! capability until the user enables it in the control panel.
//!
//! ## Delivered in this slice (M1, part 1)
//!
//! The arithmetic and the edit log, both pure and both fully testable off-device:
//!
//! * [`stroke`] — the stroke model, the unit of persistence and of undo.
//! * [`pipeline`] — One Euro smoothing, 2 px equidistant resampling, pressure→width.
//! * [`undo`] — the document's edit log; see that module's note on why it is *not* a plugin effect.
//!
//! ## Not here yet
//!
//! * the wgpu renderer: triangle-strip meshing, the incremental ring vertex buffer, one draw call;
//! * the tools: highlighter blending, the two eraser modes, lasso/rectangle selection;
//! * the `ink/stroke-committed` event and the Flutter canvas binding.
//!
//! ## On the seam
//!
//! [`InkHandle`] wraps the concrete engine because the kernel's service table stores
//! `Arc<dyn Any + Send + Sync>` and `Arc<dyn Trait>` is not itself `Any`. When the first consumer
//! appears (the canvas bridge), the trait and the stroke vocabulary move to `inkweave-api` so that
//! consumers stop depending on this crate — the Definition/Provider split of §3.3.

pub mod pipeline;
pub mod stroke;
pub mod undo;

use async_trait::async_trait;
use cordis_rs::{Context, Plugin, PluginError};
use inkweave_api::INK;
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::Arc;

pub use pipeline::{resample_equidistant, width_for, OneEuroFilter};
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
    filter: OneEuroFilter,
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
                filter: OneEuroFilter::standard(),
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
        // Borrow the inner struct once: `inner.pages` and `inner.histories` are touched together,
        // and two separate derefs of the guard would conflict even though the fields are disjoint.
        let inner = &mut *guard;

        // Smooth along the stroke; carrying filter state across strokes would drag the start.
        inner.filter.reset();
        let smoothed: Vec<StrokePoint> = samples
            .iter()
            .map(|point| {
                let seconds = point.t_us as f32 / 1_000_000.0;
                StrokePoint {
                    x: inner.filter.filter(point.x, seconds),
                    y: inner.filter.filter(point.y, seconds),
                    pressure: point.pressure,
                    t_us: point.t_us,
                }
            })
            .collect();

        let points = resample_equidistant(&smoothed, RESAMPLE_SPACING_PX);
        let id = inner.pages.alloc_id();
        let stroke = Stroke::new(id, points, color, base_width_px);
        let command = InkCommand::Add { page, stroke };
        inner
            .histories
            .entry(page)
            .or_default()
            .commit(&mut inner.pages, command);
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
        // Borrow the inner struct once: `inner.pages` and `inner.histories` are touched together,
        // and two separate derefs of the guard would conflict even though the fields are disjoint.
        let inner = &mut *guard;
        let Some(stroke) = inner.pages.take(page, id) else {
            return false;
        };
        let command = InkCommand::Remove { page, stroke };
        let mut history = inner.histories.remove(&page).unwrap_or_default();
        history.commit(&mut inner.pages, command);
        inner.histories.insert(page, history);
        true
    }

    pub fn translate(&self, page: PageId, ids: Vec<StrokeId>, dx: f32, dy: f32) {
        let mut guard = self.inner.lock();
        // Borrow the inner struct once: `inner.pages` and `inner.histories` are touched together,
        // and two separate derefs of the guard would conflict even though the fields are disjoint.
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
        let mut history = inner.histories.remove(&page).unwrap_or_default();
        history.commit(&mut inner.pages, command);
        inner.histories.insert(page, history);
    }

    pub fn undo(&self, page: PageId) -> bool {
        let mut guard = self.inner.lock();
        // Borrow the inner struct once: `inner.pages` and `inner.histories` are touched together,
        // and two separate derefs of the guard would conflict even though the fields are disjoint.
        let inner = &mut *guard;
        let mut history = inner.histories.remove(&page).unwrap_or_default();
        let changed = history.undo(&mut inner.pages);
        inner.histories.insert(page, history);
        changed
    }

    pub fn redo(&self, page: PageId) -> bool {
        let mut guard = self.inner.lock();
        // Borrow the inner struct once: `inner.pages` and `inner.histories` are touched together,
        // and two separate derefs of the guard would conflict even though the fields are disjoint.
        let inner = &mut *guard;
        let mut history = inner.histories.remove(&page).unwrap_or_default();
        let changed = history.redo(&mut inner.pages);
        inner.histories.insert(page, history);
        changed
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
}
