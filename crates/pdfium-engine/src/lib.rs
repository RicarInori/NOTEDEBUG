//! `pdfium-engine` — the PDF base layer (DEV guide §4). **Placeholder crate.**
//!
//! Ships in M3, behind [`inkweave_api::PDF`]:
//!
//! * **Handle pool** (ADR-005): PDFium is not thread-safe, so N render workers each hold an
//!   independent `FPDF_DOCUMENT` for the same file; pool size is `min(2, cpu_cores / 4)`. The LRU
//!   caches rendered **tiles**, never handles.
//! * **Single writer thread** for every edit (annotations, page management, save), coordinated
//!   with the render workers by a read/write gate that pauses rendering of the edited document.
//! * **Tile rendering**: 512×512, four LOD levels, bitmap LRU of 2 pages under memory pressure and
//!   4–6 normally, viewport culling.
//! * **Export**: ink annotations stay editable as a custom layer, and are flattened on export via
//!   `FPDF_ANNOT_INK` / path objects followed by `FPDFPage_Flatten()`.
//!
//! Licence note: PDFium is Apache-2.0; MuPDF was rejected as AGPL (§0 red line).

use cordis_rs::{Context, FnPlugin, Plugin};
use std::sync::Arc;

/// Rendering tile size recommended by research 03.
pub const TILE_SIZE: u32 = 512;

/// Number of LOD levels above and below the base level.
pub const LEVELS_OF_DETAIL: u32 = 4;

/// Bitmap cache tiers: [low memory, normal].
pub const CACHE_PAGES: [u32; 2] = [2, 6];

pub fn plugin() -> Arc<dyn Plugin> {
    FnPlugin::new("pdfium-engine", |_ctx: &Arc<Context>| {
        // M3 mounts: provide_shared(PDF, PdfiumEngine::new(pool_size))
    })
    .shared()
}
