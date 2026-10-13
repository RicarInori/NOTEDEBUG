//! Inkweave host: composition root, platform service implementations and the `flutter_rust_bridge`
//! API surface.
//!
//! The Flutter side never touches the kernel directly — it calls the functions in [`api`], which
//! delegate to the same [`PluginRegistry`](inkweave_api::PluginRegistry) the in-app control panel
//! uses. That keeps one management path for both the UI and any future automation.
//!
//! # Generated bridge code
//!
//! `flutter_rust_bridge_codegen generate` writes `src/frb_generated.rs` and the Dart bindings.
//! Add the include below **after** running codegen; it is omitted here so that `cargo test` works
//! on a fresh checkout:
//!
//! ```ignore
//! mod frb_generated;
//! ```

pub mod api;
pub mod host;

pub use host::{app, bootstrap, AppHandle};
