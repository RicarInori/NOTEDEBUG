//! Inkweave host: composition root, platform service implementations and the `flutter_rust_bridge`
//! API surface.
//!
//! The Flutter side never touches the kernel directly — it calls the functions in [`api`], which
//! delegate to the same [`PluginRegistry`](inkweave_api::PluginRegistry) the in-app control panel
//! uses. That keeps one management path for both the UI and any future automation.
//!
//! # Generated bridge code
//!
//! `crates/frb_generated.rs` and `app/lib/bridge/*.dart` are produced by `flutter_rust_bridge_codegen
//! generate` (2.13.0, matching the `flutter_rust_bridge` dependency exactly). They are generated
//! artifacts and are gitignored — regenerate them after any change to [`api`] rather than editing
//! them.

mod frb_generated;

pub mod api;
pub mod host;

pub use host::{app, bootstrap, AppHandle};
