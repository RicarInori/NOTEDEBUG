//! Canonical service keys.
//!
//! Keys are shared vocabulary: the provider and every consumer reference these `const`s, so a
//! typo is a compile error rather than a silently missing service.

use cordis_rs::ServiceKey;

/// `plugin-manager` — the unified control panel's data source (ADR-008).
pub const PLUGIN_REGISTRY: ServiceKey = ServiceKey::new("plugin-registry");

/// `stylus-drivers` — active stylus provider (generic or vendor adapter, §8.7).
pub const STYLUS: ServiceKey = ServiceKey::new("stylus");

/// `ink-engine` — stroke pipeline and canvas rendering.
pub const INK: ServiceKey = ServiceKey::new("ink");

/// `pdfium-engine` — PDF base layer.
pub const PDF: ServiceKey = ServiceKey::new("pdf");

/// `storage-sqlite` — document storage, `.nbk` I/O and FTS.
pub const STORAGE: ServiceKey = ServiceKey::new("storage");

/// `sync-webdav` — sync provider (§5.5).
pub const SYNC: ServiceKey = ServiceKey::new("sync");

/// Host-provided background task channel (§8.5). Always available.
pub const SCHEDULER: ServiceKey = ServiceKey::new("scheduler");

/// Host-provided memory governance (§8.6). Always available.
pub const MEMORY: ServiceKey = ServiceKey::new("memory");
