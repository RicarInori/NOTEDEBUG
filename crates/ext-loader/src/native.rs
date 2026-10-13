//! The native (cdylib) loader — ADR-003, v2.3 revision.
//!
//! Loading arbitrary native code is the riskiest thing this project does, so the order of checks
//! matters: open the library, read its **compiled-in** ABI version symbol, refuse a mismatch, and
//! only then read anything else. The manifest's own `abi` field is informational — a string in a
//! TOML file cannot be trusted about a struct layout, whereas the exported symbol is produced by the
//! same compilation as the code that will read that struct.

use cordis_rs::{Context, Plugin, PluginError};
use inkweave_plugin_abi as abi;
use libloading::{Library, Symbol};
use parking_lot::Mutex;
use std::ffi::{c_char, CString};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// What the import flow needs to show the user before anything is loaded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NativeManifest {
    pub id: String,
    pub name: String,
    pub version: String,
    pub author: String,
    pub description: String,
    /// Human-readable permission labels, resolved from the manifest's booleans.
    pub permissions: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    // `message`, not `source`: thiserror treats a field named `source` as the error chain and would
    // require it to implement `std::error::Error`.
    #[error("cannot open {path}: {message}")]
    Open { path: String, message: String },

    #[error("the library does not export `{0}`")]
    MissingSymbol(&'static str),

    #[error("ABI mismatch: the plugin was built for {wanted}, this host provides {found}")]
    AbiMismatch { wanted: u32, found: u32 },

    #[error("the plugin's manifest is unusable: {0}")]
    Manifest(String),

    #[error("the plugin refused to create an instance")]
    Refused,
}

/// Resolved entry points.
///
/// The function pointers are *copied out* of their symbols rather than borrowed, because
/// `libloading::Symbol` is tied to the library by a lifetime and storing both together would be a
/// self-referential struct. Copies are safe here precisely because an `extern "C" fn` pointer is a
/// value: what keeps them callable is the library staying loaded.
///
/// **Field order is load-bearing.** Rust drops fields top to bottom, so `library` is declared last
/// and therefore dropped last; declaring it first would leave the pointers above it dangling.
struct Symbols {
    create: abi::CreateFn,
    enable: abi::LifecycleFn,
    disable: abi::LifecycleFn,
    destroy: abi::LifecycleFn,
    /// Never read: held purely so the library stays loaded for the pointers above. Dropping it early
    /// would leave them dangling, which is why the field exists at all.
    #[allow(dead_code)]
    library: Library,
}

/// `Library` has no `Debug`, so this prints what is actually useful — which plugin, from where.
impl std::fmt::Debug for NativeLibrary {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("NativeLibrary")
            .field("id", &self.manifest.id)
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

/// An opened plugin library plus its manifest. Holding this keeps the code loaded.
pub struct NativeLibrary {
    symbols: Symbols,
    manifest: NativeManifest,
    path: PathBuf,
}

impl NativeLibrary {
    /// Open and validate. Nothing from the library is called except its ABI version symbol until the
    /// version check has passed.
    pub fn open(path: &Path) -> Result<Self, LoadError> {
        // SAFETY: loading a native library runs its initialisers. The caller is the host, which only
        // ever passes a path inside its own private storage; this is the documented risk of the
        // native plugin channel (ADR-003 v2.3), not an oversight.
        let library = unsafe { Library::new(path) }.map_err(|error| LoadError::Open {
            path: path.display().to_string(),
            message: error.to_string(),
        })?;

        let found = {
            // SAFETY: the symbol is read, called once, and released. The signature is checked by
            // the loader only insofar as the name matches; the version it returns is what the rest
            // of this file trusts.
            let symbol: libloading::Symbol<abi::AbiVersionFn> =
                unsafe { library.get(abi::SYM_ABI_VERSION) }
                    .map_err(|_| LoadError::MissingSymbol("inkweave_plugin_abi_version"))?;
            unsafe { symbol() }
        };
        if found != abi::ABI_VERSION {
            return Err(LoadError::AbiMismatch {
                wanted: abi::ABI_VERSION,
                found,
            });
        }

        let manifest = {
            // SAFETY: by now the ABI version matches, so the manifest signature is the one this
            // host was compiled against. The plugin keeps the bytes alive for the process lifetime.
            let symbol: libloading::Symbol<abi::ManifestFn> =
                unsafe { library.get(abi::SYM_MANIFEST) }
                    .map_err(|_| LoadError::MissingSymbol("inkweave_plugin_manifest"))?;
            let blob = unsafe { symbol() };
            parse_manifest(blob)?
        };

        // The patterns below are the only place symbols are resolved: each is copied out immediately, and
        // `Symbols` keeps the library loaded for as long as the pointers are reachable.
        let create = {
            // SAFETY: the ABI version already matched, so this symbol has the signature this host was
            // compiled against.
            let symbol: Symbol<abi::CreateFn> = unsafe { library.get(abi::SYM_CREATE) }
                .map_err(|_| LoadError::MissingSymbol("inkweave_plugin_create"))?;
            *symbol
        };
        let enable = {
            // SAFETY: as above.
            let symbol: Symbol<abi::LifecycleFn> = unsafe { library.get(abi::SYM_ENABLE) }
                .map_err(|_| LoadError::MissingSymbol("inkweave_plugin_enable"))?;
            *symbol
        };
        let disable = {
            // SAFETY: as above.
            let symbol: Symbol<abi::LifecycleFn> = unsafe { library.get(abi::SYM_DISABLE) }
                .map_err(|_| LoadError::MissingSymbol("inkweave_plugin_disable"))?;
            *symbol
        };
        let destroy = {
            // SAFETY: as above.
            let symbol: Symbol<abi::LifecycleFn> = unsafe { library.get(abi::SYM_DESTROY) }
                .map_err(|_| LoadError::MissingSymbol("inkweave_plugin_destroy"))?;
            *symbol
        };

        Ok(Self {
            symbols: Symbols {
                create,
                enable,
                disable,
                destroy,
                library,
            },
            manifest,
            path: path.to_path_buf(),
        })
    }

    pub fn manifest(&self) -> &NativeManifest {
        &self.manifest
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Wrap the library as an ordinary cordis plugin.
    ///
    /// `files_dir` becomes the plugin's private storage root, handed over through the ABI; the host
    /// deletes it on uninstall.
    pub fn into_plugin(self, files_dir: PathBuf) -> Result<Arc<dyn Plugin>, LoadError> {
        let files_dir = CString::new(files_dir.to_string_lossy().as_bytes())
            .map_err(|_| LoadError::Manifest("the storage path contains a NUL byte".to_string()))?;
        // Built once and stored, not built per call: a plugin is allowed to keep the pointer it was
        // handed, so the table must outlive `create` — as long as the plugin itself does.
        let host = abi::HostInterface {
            size: std::mem::size_of::<abi::HostInterface>() as u32,
            _reserved: 0,
            log: Some(host_log),
            files_dir: files_dir.as_ptr(),
            files_dir_len: files_dir.as_bytes().len(),
        };
        Ok(Arc::new(NativePluginAdapter {
            name: self.manifest.name.clone(),
            manifest: self.manifest,
            symbols: self.symbols,
            files_dir,
            host: SendHost(host),
            handle: Mutex::new(None),
        }))
    }
}

/// The ABI's handle, made `Send` **by contract**.
///
/// A raw pointer is neither `Send` nor `Sync`, so without this wrapper the adapter could not be a
/// `Plugin` at all — the kernel requires `Send + Sync`. The ABI states that a plugin must be
/// thread-safe and that the host may call lifecycle functions from any thread, so moving the handle
/// between threads is permitted by the plugin's own contract rather than assumed here.
struct SendHandle(abi::PluginHandle);
unsafe impl Send for SendHandle {}

/// The host table, `Send + Sync` **by construction**.
///
/// It is written once in [`NativeLibrary::into_plugin`] and only read afterwards, and the strings it
/// points at are owned by the adapter and outlive it. Sharing a read-only table across threads is
/// therefore sound; if a future capability needs interior mutability, that capability's own type
/// must carry the synchronisation, not this wrapper.
struct SendHost(abi::HostInterface);
unsafe impl Send for SendHost {}
unsafe impl Sync for SendHost {}

/// Adapter that presents a native library as a [`Plugin`].
///
/// The kernel has no `onEnable`/`onDisable`/`onUnload` hooks — a plugin's teardown *is* its effect
/// stack — so the ABI's four lifecycle symbols map like this:
///
/// | ABI | kernel |
/// | --- | --- |
/// | `create`, `enable` | `apply` (the plugin is mounted and therefore running) |
/// | `disable`, `destroy` | `Drop`, which the kernel triggers when it releases the plugin |
///
/// `apply` is idempotent: a dependency-driven restart recovers the effects and re-applies without
/// dropping the plugin, so a second `create` would leak the first instance.
struct NativePluginAdapter {
    /// Cached so [`Plugin::name`] can return a `&str`.
    name: String,
    manifest: NativeManifest,
    symbols: Symbols,
    /// Kept alive because `host.files_dir` points into it. Never read through the field: the pointer
    /// handed to the plugin is what matters.
    #[allow(dead_code)]
    files_dir: CString,
    /// Kept alive because a plugin may hold the pointer handed to `create`.
    host: SendHost,
    handle: Mutex<Option<SendHandle>>,
}

impl NativePluginAdapter {
    fn create(&self) -> Result<abi::PluginHandle, LoadError> {
        // SAFETY: the ABI version was checked at load time, so `create` has the signature this host
        // was compiled against, and `self.host` outlives the plugin.
        let handle = unsafe { (self.symbols.create)(&self.host.0) };
        if handle.is_null() {
            return Err(LoadError::Refused);
        }
        Ok(handle)
    }

    fn call(&self, function: abi::LifecycleFn) {
        let guard = self.handle.lock();
        if let Some(handle) = guard.as_ref() {
            // SAFETY: the handle came from this library's `create`, and the ABI version matched.
            unsafe { function(handle.0) };
        }
    }
}

impl Drop for NativePluginAdapter {
    fn drop(&mut self) {
        // `disable` then `destroy`, matching the ABI's own ordering, and only if an instance exists.
        if let Some(handle) = self.handle.lock().take() {
            // SAFETY: the handle is destroyed exactly once — `take` above guarantees it even if the
            // object were dropped twice, which ownership already prevents.
            unsafe {
                (self.symbols.disable)(handle.0);
                (self.symbols.destroy)(handle.0);
            }
        }
    }
}

/// Host logging entry point handed to plugins. A plain `fn`, so it can be a function pointer.
///
/// Messages are UTF-8 and not NUL-terminated; the plugin owns the buffer for the duration of the
/// call. Routing these into the per-plugin log buffer happens when the import flow is wired up.
unsafe extern "C" fn host_log(level: u32, message: *const c_char, len: usize) {
    if message.is_null() || len == 0 {
        return;
    }
    // SAFETY: the ABI promises a readable buffer of `len` bytes for the duration of this call.
    let text =
        String::from_utf8_lossy(unsafe { std::slice::from_raw_parts(message as *const u8, len) });
    let label = match level {
        abi::LOG_ERROR => "error",
        abi::LOG_WARN => "warn",
        abi::LOG_INFO => "info",
        _ => "debug",
    };
    eprintln!("[native plugin {label}] {text}");
}

fn parse_manifest(blob: abi::ByteRange) -> Result<NativeManifest, LoadError> {
    if blob.ptr.is_null() || blob.len == 0 {
        return Err(LoadError::Manifest("empty manifest".to_string()));
    }
    // SAFETY: the plugin guarantees the range is readable for the process lifetime.
    let bytes = unsafe { std::slice::from_raw_parts(blob.ptr, blob.len) };
    let text = std::str::from_utf8(bytes)
        .map_err(|error| LoadError::Manifest(format!("manifest is not UTF-8: {error}")))?;
    let file: ManifestFile = toml::from_str(text)
        .map_err(|error| LoadError::Manifest(format!("manifest is not valid TOML: {error}")))?;

    let mut permissions = Vec::new();
    if file.permissions.network {
        permissions.push("网络".to_string());
    }
    if file.permissions.filesystem {
        permissions.push("文件".to_string());
    }
    if file.permissions.clipboard {
        permissions.push("剪贴板".to_string());
    }

    Ok(NativeManifest {
        id: file.plugin.id,
        name: file.plugin.name,
        version: file.plugin.version,
        author: file.plugin.author,
        description: file.plugin.description,
        permissions,
    })
}

/// §7.2's manifest schema, the subset a native plugin needs. The exported symbol names are fixed by
/// the ABI, so there is no `entry` field.
#[derive(serde::Deserialize)]
struct ManifestFile {
    plugin: PluginSection,
    #[serde(default)]
    permissions: PermissionsSection,
}

#[derive(serde::Deserialize)]
struct PluginSection {
    id: String,
    name: String,
    version: String,
    #[serde(default)]
    author: String,
    #[serde(default)]
    description: String,
}

#[derive(serde::Deserialize, Default)]
struct PermissionsSection {
    #[serde(default)]
    network: bool,
    #[serde(default)]
    filesystem: bool,
    #[serde(default)]
    clipboard: bool,
}

#[async_trait::async_trait]
impl Plugin for NativePluginAdapter {
    fn name(&self) -> &str {
        &self.name
    }

    /// Idempotent on purpose: a dependency-driven restart recovers the effects and re-applies
    /// without dropping the plugin, so creating a second instance here would leak the first.
    async fn apply(&self, _ctx: &Arc<Context>) -> Result<(), PluginError> {
        let reused = {
            let guard = self.handle.lock();
            guard.is_some()
        };
        if reused {
            self.call(self.symbols.enable);
            return Ok(());
        }

        let handle = self.create().map_err(|error| PluginError::Apply {
            name: self.manifest.id.clone(),
            message: error.to_string(),
        })?;
        *self.handle.lock() = Some(SendHandle(handle));
        self.call(self.symbols.enable);
        Ok(())
    }
}
