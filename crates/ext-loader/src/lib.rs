//! `ext-loader` — dynamic plugin loading (ADR-003, v2.3 revision).
//!
//! Two channels, deliberately different:
//!
//! * **[`native`]** — a `cdylib` the user imports, opened with `dlopen`. It can reach the GPU, which
//!   is exactly why the note engine has to use this channel: §7.1 rules WASM out for rendering.
//! * **WASM** — still planned for third-party logic that needs a hard sandbox. The host-function ABI
//!   is an M6 work item and nothing here loads a `.wasm` yet; the constants below fix that contract's
//!   surface so the review happens against a checklist rather than prose.
//!
//! What makes the native channel defensible despite Rust having no stable ABI is that **nothing
//! Rust-shaped crosses the boundary** — see `inkweave-plugin-abi`.

pub mod native;

use cordis_rs::{Context, FnPlugin, Plugin};
use std::sync::Arc;

/// The ABI version string a WASM extension must declare (§7.1). The native channel versions itself
/// through [`inkweave_plugin_abi::ABI_VERSION`] instead, because it can be checked from the binary.
pub const WASM_ABI_VERSION: &str = "inkweave_abi_v1";

/// Default memory ceiling for a WASM extension, in MiB.
pub const DEFAULT_MEMORY_LIMIT_MB: u32 = 64;

/// Host functions the WASM sandbox will implement at M6. Listed so the ABI review has a concrete
/// checklist.
pub mod host_functions {
    /// `ink_register_brush(name_ptr, name_len, wgsl_ptr, wgsl_len) -> u32`
    pub const REGISTER_BRUSH: &str = "ink_register_brush";

    /// `ink_on_stroke_committed(callback_fn)`
    pub const ON_STROKE_COMMITTED: &str = "ink_on_stroke_committed";

    /// `storage_read(path_ptr, path_len, out_ptr) -> i32`
    pub const STORAGE_READ: &str = "storage_read";

    /// `stylus_capabilities() -> u64`
    pub const STYLUS_CAPABILITIES: &str = "stylus_capabilities";
}

/// Registration entry for the composition root.
///
/// The loader is a library the host *calls*, not a service it publishes, so this registers the name
/// (it shows in the control panel like every other internal module) without providing anything yet.
/// A service handle appears when the import flow needs one.
pub fn plugin() -> Arc<dyn Plugin> {
    FnPlugin::new("ext-loader", |_ctx: &Arc<Context>| {}).shared()
}
