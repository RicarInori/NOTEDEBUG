//! A native plugin, used to exercise and document the C ABI.
//!
//! It is deliberately trivial: the point is the *boundary*, not the behaviour. It writes marker
//! files into the storage directory the host hands it, which makes the lifecycle externally
//! observable without inventing anything beyond the ABI — an integration test can therefore assert
//! that the table was read, that `enable` ran, and that `disable`/`destroy` ran on drop.

use inkweave_plugin_abi as abi;
use std::ffi::c_char;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

/// §7.2's manifest schema. The host parses this once, before deciding whether to load anything else.
const MANIFEST: &str = r#"
[plugin]
id = "com.inkweave.example.native"
name = "原生示例插件"
version = "0.1.0"
author = "Inkweave"
description = "演示原生动态库插件：只经宿主的函数指针表通信，不链接宿主任何 crate"

[permissions]
network = false
filesystem = true
clipboard = false
"#;

/// Plugin instance.
///
/// Holds no pointer into the host's table — the `log` function pointer is *copied* out at create
/// time — so the type is naturally `Send + Sync`, which [`abi::PluginHandle`] requires.
struct Instance {
    log: Option<unsafe extern "C" fn(u32, *const c_char, usize)>,
    /// Copied from the table, proving the plugin could read it.
    files_dir: Vec<u8>,
    enabled: AtomicBool,
    enables: AtomicU32,
}

impl Instance {
    fn say(&self, level: u32, message: &str) {
        if let Some(log) = self.log {
            // SAFETY: the host keeps this function alive for the process lifetime.
            unsafe { log(level, message.as_ptr() as *const c_char, message.len()) };
        }
    }

    /// Append a line to a marker file in the host-provided directory.
    ///
    /// Best effort: a plugin must not take down the host because a directory was missing.
    fn mark(&self, line: &str) {
        if self.files_dir.is_empty() {
            return;
        }
        let dir = String::from_utf8_lossy(&self.files_dir);
        let path = std::path::Path::new(dir.as_ref()).join("lifecycle.log");
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let previous = std::fs::read_to_string(&path).unwrap_or_default();
        let _ = std::fs::write(path, format!("{previous}{line}\n"));
    }
}

/// # Safety
/// `handle` must have come from [`inkweave_plugin_create`] and must not have been destroyed yet.
unsafe fn instance<'a>(handle: abi::PluginHandle) -> Option<&'a Instance> {
    if handle.is_null() {
        None
    } else {
        // SAFETY: guaranteed by the caller's contract with the host.
        Some(unsafe { &*(handle as *const Instance) })
    }
}

#[no_mangle]
pub extern "C" fn inkweave_plugin_abi_version() -> u32 {
    abi::ABI_VERSION
}

#[no_mangle]
pub extern "C" fn inkweave_plugin_manifest() -> abi::ByteRange {
    // The bytes live in the library's static data, so they are valid for the process lifetime.
    abi::ByteRange {
        ptr: MANIFEST.as_ptr(),
        len: MANIFEST.len(),
    }
}

/// # Safety
///
/// Called by the host right after loading this library. `host` must be null, or a valid and readable
/// pointer to an [`abi::HostInterface`] — this function cannot assume it is good, which is why the
/// very first thing it does is check.
#[no_mangle]
pub unsafe extern "C" fn inkweave_plugin_create(
    host: *const abi::HostInterface,
) -> abi::PluginHandle {
    let required = std::mem::size_of::<abi::HostInterface>() as u32;
    if !abi::host_interface_is_usable(host, required) {
        // Refusing is the correct response to a host older than this plugin needs.
        return std::ptr::null_mut();
    }

    let (log, files_dir) = {
        let table = &*host; // SAFETY: `host_interface_is_usable` proved the table is readable.
        let files_dir = if table.files_dir.is_null() || table.files_dir_len == 0 {
            Vec::new()
        } else {
            // SAFETY: the host promises `files_dir_len` readable bytes at `files_dir`.
            std::slice::from_raw_parts(table.files_dir as *const u8, table.files_dir_len).to_vec()
        };
        (table.log, files_dir)
    };

    let instance = Box::new(Instance {
        log,
        files_dir,
        enabled: AtomicBool::new(false),
        enables: AtomicU32::new(0),
    });
    Box::into_raw(instance) as abi::PluginHandle
}

/// # Safety
///
/// `handle` must be a pointer returned by [`inkweave_plugin_create`] that has not been destroyed.
#[no_mangle]
pub unsafe extern "C" fn inkweave_plugin_enable(handle: abi::PluginHandle) {
    // SAFETY: guaranteed by this function's contract with the host.
    let Some(instance) = instance(handle) else {
        return;
    };
    instance.enabled.store(true, Ordering::Release);
    instance.enables.fetch_add(1, Ordering::AcqRel);
    instance.mark("enabled");
    instance.say(abi::LOG_INFO, "原生示例插件已启用");
}

/// # Safety
///
/// `handle` must be a pointer returned by [`inkweave_plugin_create`] that has not been destroyed.
#[no_mangle]
pub unsafe extern "C" fn inkweave_plugin_disable(handle: abi::PluginHandle) {
    // SAFETY: guaranteed by this function's contract with the host.
    let Some(instance) = instance(handle) else {
        return;
    };
    instance.enabled.store(false, Ordering::Release);
    instance.mark("disabled");
    instance.say(abi::LOG_INFO, "原生示例插件已停用");
}

/// # Safety
///
/// `handle` must be a pointer returned by [`inkweave_plugin_create`], and this must be the only call
/// for it — the instance is freed here.
#[no_mangle]
pub unsafe extern "C" fn inkweave_plugin_destroy(handle: abi::PluginHandle) {
    if handle.is_null() {
        return;
    }
    // SAFETY: the host calls this exactly once per `create`, after `disable`, which is what makes
    // reconstructing the `Box` sound.
    let instance = Box::from_raw(handle as *mut Instance);
    instance.mark("destroyed");
    drop(instance);
}
