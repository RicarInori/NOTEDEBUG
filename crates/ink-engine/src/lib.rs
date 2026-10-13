//! `ink-engine` — the handwriting engine (DEV guide §3). **Placeholder crate.**
//!
//! Mounted as a real plugin today so it appears in, and is toggleable from, the control panel.
//! Its service ([`inkweave_api::INK`]) is intentionally **not** provided yet: providing a marker
//! under the real key would let consumers bind to something that cannot render.
//!
//! Ships in M1:
//!
//! * the low-latency pipeline — historical/coalesced sample extraction, One Euro filter
//!   (`min_cutoff = 1.0`, `beta = 0.01`), 2 px equidistant resampling, pressure→width, triangle
//!   strip meshing, incremental ring vertex buffer, single draw call;
//! * tools: pen, highlighter (multiply blend, stencil-guarded self-overlap), pixel/whole-stroke
//!   eraser, lasso and rectangle selection;
//! * undo/redo as kernel effects: every [`InkCommand`] carries its own inverse, which is exactly
//!   what the disposer stack already models.
//!
//! Hot-path discipline (§3.1): no heap allocation, no lock contention, no logging in release.

use cordis_rs::{Context, FnPlugin, Plugin};
use std::sync::Arc;

/// The command grammar of the undo stack. Kept here so the shape is reviewable before M1.
#[derive(Clone, Debug)]
pub enum InkCommand {
    AddStroke { page: u64, stroke_id: u64 },
    RemoveStroke { page: u64, stroke_id: u64 },
    TransformStrokes { page: u64, ids: Vec<u64> },
}

/// Construct the plugin for the composition root.
pub fn plugin() -> Arc<dyn Plugin> {
    FnPlugin::new("ink-engine", |_ctx: &Arc<Context>| {
        // M1 mounts: provide_shared(INK, InkEngine::new(..))
    })
    .shared()
}
