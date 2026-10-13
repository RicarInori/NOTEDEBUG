//! The C ABI shared by the host and every native plugin (ADR-003, v2.3 revision).
//!
//! Rust has no stable ABI, so nothing Rust-flavoured may cross a `dlopen` boundary: no `String`, no
//! `Vec`, no trait objects, no generics. What crosses here is `#[repr(C)]` structs, function
//! pointers and `extern "C"` symbols, and **both sides compile this crate**, so the layouts agree by
//! construction rather than by luck.
//!
//! The plugin never links a host crate. Everything it can do is a function pointer it receives in
//! [`HostInterface`].

#![allow(non_camel_case_types)]

use std::os::raw::{c_char, c_void};

/// Bump on any breaking change to a struct layout or a symbol signature.
///
/// The host checks this before touching anything else, and refuses a mismatch. A plugin built
/// against a different version is rejected rather than allowed to misread a struct.
pub const ABI_VERSION: u32 = 1;

/// Host capabilities, handed to the plugin by `inkweave_plugin_create`.
///
/// **Valid for the plugin's whole lifetime**, not just the `create` call: a plugin is allowed to
/// keep the pointer it was given. The host owns the table and the strings it points at.
///
/// `size` lets the plugin detect a *newer* host (it can then ignore entries it does not know) and a
/// *older* one (it must refuse). New capabilities are always appended, so extending this struct is
/// backward compatible — that is the whole reason it is a table rather than individual symbols.
#[repr(C)]
pub struct HostInterface {
    /// `size_of::<HostInterface>()` as compiled by the host.
    pub size: u32,
    /// Reserved so the struct grows on 8-byte boundaries; must be zero.
    pub _reserved: u32,
    /// `log(level, message_ptr, message_len)`; levels: 0 debug, 1 info, 2 warn, 3 error.
    ///
    /// Messages are UTF-8 and deliberately *not* NUL-terminated, so a plugin cannot accidentally
    /// rely on termination.
    pub log: Option<unsafe extern "C" fn(level: u32, message: *const c_char, len: usize)>,
    /// Root of the plugin's private storage. Not NUL-terminated; use `files_dir_len`.
    ///
    /// The plugin must not write outside it — the host deletes this tree when the plugin is
    /// uninstalled.
    pub files_dir: *const c_char,
    pub files_dir_len: usize,
}

/// A borrowed byte range. The host copies it immediately; the plugin keeps ownership and must keep
/// it alive until the call returns.
#[repr(C)]
pub struct ByteRange {
    pub ptr: *const u8,
    pub len: usize,
}

/// Opaque plugin instance. One per `create`; every other lifecycle call takes it.
///
/// **A plugin must be thread-safe.** The host may call `enable`, `disable` and `destroy` from
/// different threads — a fiber is disposed wherever the disposal happens to run — so an instance
/// must not assume it is only ever touched by the thread that created it.
pub type PluginHandle = *mut c_void;

// --- exported symbols ---------------------------------------------------------------------------
//
// A plugin is any shared library that exports these, with exactly these names and signatures.
// `entry` is *not* configurable: keeping it fixed is what lets the host find the plugin at all.

pub const SYM_ABI_VERSION: &[u8] = b"inkweave_plugin_abi_version\0";
pub const SYM_MANIFEST: &[u8] = b"inkweave_plugin_manifest\0";
pub const SYM_CREATE: &[u8] = b"inkweave_plugin_create\0";
pub const SYM_ENABLE: &[u8] = b"inkweave_plugin_enable\0";
pub const SYM_DISABLE: &[u8] = b"inkweave_plugin_disable\0";
pub const SYM_DESTROY: &[u8] = b"inkweave_plugin_destroy\0";

/// `fn() -> u32` — the ABI the plugin was compiled against.
pub type AbiVersionFn = unsafe extern "C" fn() -> u32;

/// `fn() -> ByteRange` — the manifest as TOML (§7.2's schema), valid for the process lifetime.
pub type ManifestFn = unsafe extern "C" fn() -> ByteRange;

/// `fn(*const HostInterface) -> PluginHandle` — construct the instance. Returning null means the
/// plugin refused to start; the host reports it as a load failure rather than mounting a half-built
/// plugin.
pub type CreateFn = unsafe extern "C" fn(host: *const HostInterface) -> PluginHandle;

/// `fn(PluginHandle)` — enable, disable, destroy. All three are idempotent.
pub type LifecycleFn = unsafe extern "C" fn(handle: PluginHandle);

// --- log levels ---------------------------------------------------------------------------------

pub const LOG_DEBUG: u32 = 0;
pub const LOG_INFO: u32 = 1;
pub const LOG_WARN: u32 = 2;
pub const LOG_ERROR: u32 = 3;

/// Whether a host interface a plugin received is one it can trust.
///
/// `required` is the size the plugin was compiled against. A host smaller than that is too old to
/// promise the fields the plugin reads.
///
/// # Safety
///
/// `host` must be null, or a valid and readable pointer to a [`HostInterface`]. This is the first
/// thing a plugin does, so it cannot assume the pointer is good.
#[inline]
pub unsafe fn host_interface_is_usable(host: *const HostInterface, required: u32) -> bool {
    if host.is_null() {
        return false;
    }
    // SAFETY: guaranteed by the caller — reading `size` is the only way to learn whether the rest may
    // be read at all.
    (*host).size >= required
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::size_of;

    #[test]
    fn the_interface_size_is_what_the_plugin_will_compare_against() {
        // Pinned so that a layout change is a deliberate act: every plugin compares this number.
        #[cfg(target_pointer_width = "64")]
        {
            assert_eq!(size_of::<HostInterface>(), 32);
            assert_eq!(size_of::<ByteRange>(), 16);
        }
    }

    #[test]
    fn a_null_or_too_small_interface_is_rejected() {
        let required = size_of::<HostInterface>() as u32;
        // SAFETY: a null pointer is explicitly allowed by the function's contract.
        assert!(!unsafe { host_interface_is_usable(std::ptr::null(), required) });

        let host = HostInterface {
            size: 8,
            _reserved: 0,
            log: None,
            files_dir: std::ptr::null(),
            files_dir_len: 0,
        };
        // SAFETY: `host` is a live local for the duration of these calls.
        assert!(!unsafe { host_interface_is_usable(&host, required) });
        assert!(
            unsafe { host_interface_is_usable(&host, 8) },
            "an older plugin accepts a smaller host"
        );
    }
}
