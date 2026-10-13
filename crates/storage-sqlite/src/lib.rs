//! `storage-sqlite` — local persistence (DEV guide §5). **Placeholder crate.**
//!
//! Ships in M2, behind [`inkweave_api::STORAGE`]:
//!
//! * library model — folders (unlimited nesting), notebooks, pages, tags (many-to-many),
//!   favourites, quick notes, an unfiled buffer;
//! * `.nbk` container I/O — a ZIP holding `manifest.json`, `document.pdf`, per-page metadata and
//!   binary stroke files (`~14 bytes/point`), `undo_log.bin` and WebP thumbnails;
//! * format migration via the `nbk-migrate` chain, with a `.bak` copy taken before any in-place
//!   migration, and read-only mode for files newer than the code understands;
//! * autosave — whichever comes first of 3 s idle after a stroke commit, 30 s since the last save,
//!   page switch, backgrounding, or a manual save. Crash recovery replays the undo log to the last
//!   checkpoint using the `clean_shutdown` flag in the manifest;
//! * full-text search over SQLite FTS5, with a jieba tokenizer plugin for Chinese.

use cordis_rs::{Context, FnPlugin, Plugin};
use std::sync::Arc;

/// Autosave triggers (DEV guide §5.4).
pub const AUTOSAVE_IDLE_MS: u64 = 3_000;
pub const AUTOSAVE_MAX_AGE_MS: u64 = 30_000;

/// Bytes per normalised stroke point in the `.nbk` binary encoding.
pub const BYTES_PER_POINT: u32 = 14;

pub fn plugin() -> Arc<dyn Plugin> {
    FnPlugin::new("storage-sqlite", |_ctx: &Arc<Context>| {
        // M2 mounts: provide_shared(STORAGE, SqliteStore::open(data_dir))
    })
    .shared()
}
