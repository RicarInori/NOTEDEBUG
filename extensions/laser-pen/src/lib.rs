//! Laser-pen extension skeleton (DEV guide §7, delivered at M6).
//!
//! A laser pointer is a good first extension: it is pure presentation, needs no filesystem or
//! network access, and disappears on its own. That makes it a clean way to validate the host
//! function ABI without granting anything sensitive.
//!
//! **Nothing here is wired up yet.** The ABI itself is an explicit M6 work item (2–3 weeks), so
//! this file exists to pin down the shape an extension is written against:
//!
//! * an extension never touches the GPU directly — it submits a WGSL fragment shader through
//!   `ink_register_brush`;
//! * it subscribes to stroke events through `ink_on_stroke_committed` rather than polling;
//! * anything it would need from the platform (files, network, clipboard) arrives as a host
//!   function call and is gated by the manifest's declared permissions.
//!
//! Build (once the ABI lands):
//!
//! ```bash
//! cargo build --release --target wasm32-unknown-unknown
//! ```
//!
//! Then package `laser_pen.wasm` with this directory's `manifest.toml` and install it through the
//! app's "添加插件" entry.

/// Host function the extension calls to register its fragment shader.
///
/// Declared here as documentation of the boundary; the real import is generated from the ABI
/// header at M6.
pub const ABI_REGISTER_BRUSH: &str = "ink_register_brush";

/// WGSL fragment shader the laser brush would submit. Kept as a literal so the shader's shape is
/// reviewable alongside the ABI that carries it.
pub const LASER_FRAGMENT_WGSL: &str = r#"
// Laser pen: additive-ish glow that does not accumulate on self-overlap.
@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let glow = 1.0 - smoothstep(0.0, 1.0, length(in.local - vec2<f32>(0.0, 0.0)));
    return vec4<f32>(1.0, 0.25, 0.35, glow * 0.85);
}
"#;

#[no_mangle]
pub extern "C" fn inkweave_extension_abi_version() -> u32 {
    // Bumped in lockstep with `inkweave_abi_v1`; the host refuses anything it does not know.
    1
}