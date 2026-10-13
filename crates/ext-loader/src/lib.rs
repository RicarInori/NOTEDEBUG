//! `ext-loader` — the WASM extension sandbox (DEV guide §7). **Placeholder crate.**
//!
//! This is the piece that makes third-party plugins genuinely dynamic: `install` / `uninstall`
//! from the "添加插件" entry loads and drops code at runtime, which internal modules (compiled in
//! as crates, ADR-003) deliberately cannot do.
//!
//! The ABI design is an explicit **M6 work item (2–3 weeks)**, so nothing here is wired up yet.
//! What the crate fixes now is the contract surface so the rest of the tree can be written against
//! it:
//!
//! * **`inkweave_abi_v1`** — extensions declare the version they target; a mismatch is refused at
//!   install time with [`inkweave_api::RegistryError::AbiMismatch`].
//! * **Host functions only.** WASM cannot reach the GPU or platform APIs, so every capability
//!   arrives through a host function call, gated by the manifest's declared permissions.
//! * **Limits** — 64 MB memory ceiling and fuel metering, both enforced by the sandbox, not by the
//!   extension.

use cordis_rs::{Context, FnPlugin, Plugin};
use std::sync::Arc;

/// The ABI version string extensions must declare.
pub const ABI_VERSION: &str = "inkweave_abi_v1";

/// Default memory ceiling for an extension, in MiB.
pub const DEFAULT_MEMORY_LIMIT_MB: u32 = 64;

/// Host functions to be implemented in M6. Listed as constants so the ABI review can happen
/// against a concrete checklist rather than prose.
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

pub fn plugin() -> Arc<dyn Plugin> {
    FnPlugin::new("ext-loader", |_ctx: &Arc<Context>| {
        // M6: create the wasmtime engine, install the host functions, expose an install hook.
    })
    .shared()
}
