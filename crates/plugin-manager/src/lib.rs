//! `plugin-manager` — the unified control panel's backend (ADR-008).
//!
//! It is deliberately **not** part of the kernel: it implements [`Plugin`] like every other
//! plugin, is mounted with `ctx.plugin()`, can be disabled, and provides the
//! [`PluginRegistry`] service on the root scope so any screen can consume it.
//!
//! Ownership model: this module is the only place that holds strong `Arc<Fiber>` handles, which is
//! what keeps mounted plugins alive. The kernel's ledger only tracks `Weak` references.

use async_trait::async_trait;
use cordis_rs::{Context, Fiber, FiberId, FiberState, Plugin, PluginError};
use ext_loader::native::{LoadError, NativeLibrary, NativeManifest};
use inkweave_api::registry::{
    kind_of_package, InstallRequest, Permissions, PluginInfo, PluginKind, PluginRegistry,
    PluginRuntimeState, RegistryError,
};
use inkweave_api::{RegistryHandle, SchedulerHandle, PLUGIN_REGISTRY, SCHEDULER};
use parking_lot::Mutex;
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// The contract version an extension must target (DEV guide §7.1 `inkweave_abi_v1`).
pub const SUPPORTED_ABI: &str = "inkweave_abi_v1";

/// How many state transitions a subscriber may fall behind before it is told it lagged. Small on
/// purpose: the payload is two words, and a UI that lags this far should re-read the snapshot anyway.
const STATE_CHANNEL_CAPACITY: usize = 64;

/// `[plugin]` section of an extension manifest (DEV guide §7.2).
#[derive(Debug, serde::Deserialize)]
pub struct ExtensionManifest {
    pub plugin: ManifestPluginSection,
    #[serde(default)]
    pub permissions: ManifestPermissions,
    #[serde(default)]
    pub contributes: ManifestContributes,
}

#[derive(Debug, serde::Deserialize)]
pub struct ManifestPluginSection {
    pub name: String,
    pub version: String,
    pub kind: String,
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub entry: String,
    #[serde(default)]
    pub abi: String,
}

#[derive(Debug, Default, serde::Deserialize)]
pub struct ManifestPermissions {
    #[serde(default)]
    pub network: bool,
    #[serde(default)]
    pub filesystem: bool,
    #[serde(default)]
    pub clipboard: bool,
}

impl ManifestPermissions {
    fn to_permissions(&self) -> Permissions {
        Permissions {
            network: self.network,
            filesystem: self.filesystem,
            clipboard: self.clipboard,
        }
    }
}

#[derive(Debug, Default, serde::Deserialize)]
pub struct ManifestContributes {
    #[serde(default)]
    pub commands: Vec<ManifestCommand>,
}

#[derive(Debug, serde::Deserialize)]
pub struct ManifestCommand {
    pub id: String,
    pub title: String,
}

/// How a registered plugin is produced at enable time.
enum Source {
    /// First-party crate: a constructor the composition root registered at startup. Boxed so it
    /// can capture shared state — `plugin-manager` reuses its own singleton instance this way.
    Internal(Box<dyn Fn() -> Arc<dyn Plugin> + Send + Sync>),
    /// Third-party WASM extension: its package path.
    Wasm(PathBuf),
    /// Third-party native extension: the copy of its library in the host's private storage, and the
    /// private data directory handed to it through the ABI.
    Native {
        library: PathBuf,
        files_dir: PathBuf,
    },
}

struct Entry {
    info: PluginInfo,
    source: Source,
    fiber: Option<Arc<Fiber>>,
}

/// Registry + lifecycle owner for every plugin in the process.
pub struct PluginManager {
    root: Arc<Context>,
    entries: Mutex<Vec<Entry>>,
    logs: Mutex<HashMap<String, VecDeque<String>>>,
    max_log_lines: usize,
    state_tx: tokio::sync::broadcast::Sender<PluginStateChange>,
    /// Root of the host's private extension storage. Third-party packages are copied in here, so a
    /// plugin keeps working after the file the user picked is gone (a picker cache can be evicted).
    extensions_dir: PathBuf,
}

/// A state transition worth telling the UI about.
#[derive(Clone, Debug)]
pub struct PluginStateChange {
    pub plugin_id: String,
    pub state: PluginRuntimeState,
}

impl PluginManager {
    pub fn new(root: Arc<Context>, extensions_dir: PathBuf) -> Arc<Self> {
        let (state_tx, _) = tokio::sync::broadcast::channel(STATE_CHANNEL_CAPACITY);
        let manager = Arc::new(Self {
            root: root.clone(),
            entries: Mutex::new(Vec::new()),
            logs: Mutex::new(HashMap::new()),
            max_log_lines: 200,
            state_tx,
            extensions_dir,
        });

        // Mirror every fiber transition so the UI does not have to poll — including transitions with
        // no API call behind them, such as the dependency-driven `Pending → Active`.
        //
        // The observer holds a *weak* handle: the ledger outlives the manager in principle, and a
        // strong one would form the cycle manager → root → ledger → observer → manager.
        let weak = Arc::downgrade(&manager);
        root.observe_fiber_states(Arc::new(move |fiber_id, state| {
            if let Some(manager) = weak.upgrade() {
                manager.publish_state_change(fiber_id, state);
            }
        }));

        manager
    }

    /// Subscribe to state transitions. Each subscriber gets its own cursor; a slow one may lag and
    /// will be told so rather than blocking the kernel.
    pub fn state_changes(&self) -> tokio::sync::broadcast::Receiver<PluginStateChange> {
        self.state_tx.subscribe()
    }

    fn publish_state_change(&self, fiber_id: FiberId, state: FiberState) {
        let plugin_id = {
            let entries = self.entries.lock();
            entries
                .iter()
                .find(|entry| entry.fiber.as_ref().map(|fiber| fiber.id()) == Some(fiber_id))
                .map(|entry| entry.info.id.clone())
        };
        if let Some(plugin_id) = plugin_id {
            // Fails only when nobody is subscribed, which is normal before the UI attaches.
            let _ = self.state_tx.send(PluginStateChange {
                plugin_id,
                state: state.into(),
            });
        }
    }

    /// Register a first-party internal module. Called by the composition root during bootstrap.
    pub fn register_internal(
        &self,
        info: PluginInfo,
        factory: impl Fn() -> Arc<dyn Plugin> + Send + Sync + 'static,
    ) {
        self.entries.lock().push(Entry {
            info,
            source: Source::Internal(Box::new(factory)),
            fiber: None,
        });
    }

    /// Register a WASM extension discovered in the extensions directory.
    pub fn register_wasm(&self, info: PluginInfo, package_path: PathBuf) {
        self.entries.lock().push(Entry {
            info,
            source: Source::Wasm(package_path),
            fiber: None,
        });
    }

    /// Register a native extension already copied into the host's extension storage.
    ///
    /// Separate from [`Self::install`] because a native plugin's manifest only exists inside the
    /// library: it is read by [`Self::probe_native`] and passed in here, so the two paths cannot
    /// disagree about what was validated.
    pub fn register_native(&self, info: PluginInfo, library: PathBuf, files_dir: PathBuf) {
        self.entries.lock().push(Entry {
            info,
            source: Source::Native { library, files_dir },
            fiber: None,
        });
    }

    /// Read a native plugin file's compiled-in manifest without mounting it.
    ///
    /// This is what the "add plugin" confirmation dialog is fed by: the permission list the user is
    /// asked to accept must be the one the plugin declares, and for a native library that is only
    /// discoverable by opening it.
    pub fn probe_native(&self, path: &Path) -> Result<NativeManifest, RegistryError> {
        NativeLibrary::probe(path).map_err(load_error)
    }

    /// Mount `plugin-manager` itself. The returned handle is kept by the composition root.
    pub fn mount(self: &Arc<Self>) -> Arc<dyn Plugin> {
        Arc::new(PluginManagerPlugin {
            manager: self.clone(),
        })
    }

    /// Fraction of the extension budget / internal declaration that is currently in use.
    pub fn record_log(&self, owner: &str, line: impl Into<String>) {
        let mut logs = self.logs.lock();
        let buffer = logs.entry(owner.to_string()).or_default();
        buffer.push_back(line.into());
        while buffer.len() > self.max_log_lines {
            buffer.pop_front();
        }
    }

    fn scheduler(&self) -> Option<Arc<SchedulerHandle>> {
        self.root.get::<SchedulerHandle>(SCHEDULER)
    }

    /// Install a WASM package. Its manifest sits beside the code, so everything can be validated
    /// before any of it runs — and in this build, before any of it is even loaded.
    async fn install_wasm(
        &self,
        path: &Path,
        accepted: Permissions,
    ) -> Result<PluginInfo, RegistryError> {
        let manifest = read_manifest(path)?;

        if !covers(&accepted, &manifest.permissions.to_permissions()) {
            return Err(RegistryError::PermissionDenied(
                "扩展声明的权限未被用户全部接受",
            ));
        }
        if manifest.plugin.kind != "wasm" {
            return Err(RegistryError::InvalidPackage(format!(
                "本入口只接受 WASM 扩展，收到 kind = `{}`",
                manifest.plugin.kind
            )));
        }
        if manifest.plugin.abi != SUPPORTED_ABI {
            return Err(RegistryError::AbiMismatch {
                wanted: manifest.plugin.abi.clone(),
                found: SUPPORTED_ABI.to_string(),
            });
        }

        let id = manifest_id(&manifest);
        {
            let entries = self.entries.lock();
            if entries.iter().any(|entry| entry.info.id == id) {
                return Err(RegistryError::AlreadyInstalled(id));
            }
        }

        #[cfg(not(feature = "wasm-runtime"))]
        {
            // Everything above is the real validation path. Mounting needs the sandbox + host
            // function ABI, which is the M6 deliverable.
            Err(RegistryError::InvalidPackage(
                "本构建未启用 wasm-runtime；WASM 扩展的沙箱装载通道于 M6 接入（DEV guide §7.1）"
                    .to_string(),
            ))
        }

        #[cfg(feature = "wasm-runtime")]
        {
            let info = PluginInfo {
                id: id.clone(),
                name: manifest.plugin.name.clone(),
                version: manifest.plugin.version.clone(),
                kind: PluginKind::Wasm,
                state: PluginRuntimeState::Installed,
                author: manifest.plugin.author.clone(),
                description: manifest.plugin.description.clone(),
                permissions: manifest.permissions.to_permissions(),
                capabilities: Default::default(),
                memory_budget_mb: inkweave_api::runtime::DEFAULT_EXTENSION_BUDGET_MB,
                memory_used_mb: 0,
                background_tasks: 0,
                contributions: manifest
                    .contributes
                    .commands
                    .iter()
                    .map(|command| inkweave_api::registry::Contribution {
                        id: command.id.clone(),
                        title: command.title.clone(),
                        kind: inkweave_api::ContributionKind::Command,
                    })
                    .collect(),
                last_error: None,
            };
            self.register_wasm(info.clone(), path.to_path_buf());
            self.record_log(&id, "installed");
            self.enable(&id).await?;
            Ok(info)
        }
    }

    /// Install a native (`cdylib`) package.
    ///
    /// The picked file is copied into the host's private storage before anything is mounted, and the
    /// copy is re-read to check that it is byte-for-byte the plugin the user consented to. A native
    /// library runs in-process with no sandbox (ADR-003 v2.3), so the permission list is a
    /// disclosure rather than a boundary — which is exactly why the artifact we validate must be the
    /// artifact we mount.
    async fn install_native(
        &self,
        picked: &Path,
        accepted: Permissions,
    ) -> Result<PluginInfo, RegistryError> {
        let previewed = self.probe_native(picked)?;
        if !covers(&accepted, &previewed.permissions) {
            return Err(RegistryError::PermissionDenied(
                "原生插件声明的权限未被用户全部接受",
            ));
        }

        let id = slug(&previewed.id, &previewed.name);
        {
            let entries = self.entries.lock();
            if entries.iter().any(|entry| entry.info.id == id) {
                return Err(RegistryError::AlreadyInstalled(id));
            }
        }

        let dir = self.extensions_dir.join(safe_component(&id));
        let library = dir.join(format!("plugin.{}", library_extension(picked)));
        let files_dir = dir.join("files");

        let staged = self
            .stage_native(picked, &library, &files_dir)
            .map_err(|error| RegistryError::InvalidPackage(format!("无法装入插件文件：{error}")))?;

        // Re-read from the copy: this is the file `enable` will dlopen, so it is the one whose
        // declared permissions and identity have to match what the user was shown.
        let installed = match self.probe_native(&staged) {
            Ok(manifest) => manifest,
            Err(error) => {
                let _ = std::fs::remove_dir_all(&dir);
                return Err(error);
            }
        };
        if installed != previewed {
            let _ = std::fs::remove_dir_all(&dir);
            return Err(RegistryError::InvalidPackage(
                "插件文件在确认之后发生了变化，已中止装入".to_string(),
            ));
        }

        let info = PluginInfo {
            id: id.clone(),
            name: installed.name.clone(),
            version: installed.version.clone(),
            kind: PluginKind::Native,
            state: PluginRuntimeState::Installed,
            author: installed.author.clone(),
            description: installed.description.clone(),
            permissions: installed.permissions,
            capabilities: Default::default(),
            memory_budget_mb: inkweave_api::runtime::DEFAULT_EXTENSION_BUDGET_MB,
            memory_used_mb: 0,
            background_tasks: 0,
            contributions: Vec::new(),
            last_error: None,
        };
        self.register_native(info.clone(), library, files_dir);

        if let Err(error) = self.enable(&id).await {
            // A plugin that failed to mount must not leave a card behind, nor its files on disk.
            self.entries.lock().retain(|entry| entry.info.id != id);
            let _ = std::fs::remove_dir_all(&dir);
            return Err(error);
        }
        self.record_log(&id, "installed");
        Ok(info)
    }

    /// Copy the picked library into the extension's own directory, creating the layout
    /// `<extensions>/<id>/plugin.<ext>` plus `<extensions>/<id>/files/`.
    fn stage_native(
        &self,
        picked: &Path,
        library: &Path,
        files_dir: &Path,
    ) -> std::io::Result<PathBuf> {
        std::fs::create_dir_all(files_dir)?;
        std::fs::copy(picked, library)?;
        Ok(library.to_path_buf())
    }
}

#[async_trait]
impl PluginRegistry for PluginManager {
    fn snapshot(&self) -> Vec<PluginInfo> {
        let scheduler = self.scheduler();
        let entries = self.entries.lock();
        entries
            .iter()
            .map(|entry| {
                let mut info = entry.info.clone();
                info.state = match &entry.fiber {
                    Some(fiber) => fiber.state().into(),
                    None => PluginRuntimeState::Installed,
                };
                info.background_tasks = scheduler
                    .as_ref()
                    .map(|handle| {
                        handle
                            .0
                            .unfinished(&inkweave_api::runtime::task_owner(&info.id))
                    })
                    .unwrap_or(0);
                info
            })
            .collect()
    }

    fn state(&self, id: &str) -> Option<PluginRuntimeState> {
        let entries = self.entries.lock();
        entries
            .iter()
            .find(|entry| entry.info.id == id)
            .map(|entry| match &entry.fiber {
                Some(fiber) => fiber.state().into(),
                None => PluginRuntimeState::Installed,
            })
    }

    async fn enable(&self, id: &str) -> Result<(), RegistryError> {
        let plugin = {
            let mut entries = self.entries.lock();
            let entry = entries
                .iter_mut()
                .find(|entry| entry.info.id == id)
                .ok_or_else(|| RegistryError::UnknownPlugin(id.to_string()))?;

            if entry
                .fiber
                .as_ref()
                .map(|fiber| fiber.state().is_live())
                .unwrap_or(false)
            {
                return Ok(()); // already enabled — the verb is idempotent
            }
            entry.fiber = None;
            entry.source.build()?
        };

        let fiber = self
            .root
            .plugin(plugin)
            .await
            .map_err(|error: PluginError| RegistryError::MountFailed(error.to_string()))?;

        {
            let mut entries = self.entries.lock();
            if let Some(entry) = entries.iter_mut().find(|entry| entry.info.id == id) {
                entry.fiber = Some(fiber);
            }
        }
        self.record_log(id, "enabled");
        Ok(())
    }

    async fn disable(&self, id: &str) -> Result<(), RegistryError> {
        let fiber = {
            let entries = self.entries.lock();
            entries
                .iter()
                .find(|entry| entry.info.id == id)
                .ok_or_else(|| RegistryError::UnknownPlugin(id.to_string()))?
                .fiber
                .clone()
        };

        if let Some(fiber) = fiber {
            // Unload recovers every effect, which also revokes the plugin's provided services and
            // sends its dependents back to Pending.
            self.root.unload(&fiber).await;
        }

        // Cancel the plugin's background work. This is the host enforcing §8.5, not the plugin
        // cleaning up after itself. The namespace comes from the shared helper, so it always
        // matches whatever the plugin registered under.
        if let Some(scheduler) = self.scheduler() {
            scheduler
                .0
                .cancel_owner(&inkweave_api::runtime::task_owner(id))
                .await;
        }

        {
            let mut entries = self.entries.lock();
            if let Some(entry) = entries.iter_mut().find(|entry| entry.info.id == id) {
                entry.fiber = None;
            }
        }
        self.record_log(id, "disabled");
        Ok(())
    }

    async fn install(&self, request: InstallRequest) -> Result<PluginInfo, RegistryError> {
        let path = Path::new(&request.package_path);
        // The file the user picked decides the channel; the two are installed differently because a
        // WASM package carries its manifest beside it while a native one carries it inside.
        match kind_of_package(path) {
            Some(PluginKind::Native) => self.install_native(path, request.accepted).await,
            Some(PluginKind::Wasm) => self.install_wasm(path, request.accepted).await,
            _ => Err(RegistryError::InvalidPackage(format!(
                "无法识别的插件包类型：{}（支持 .wasm/.inkext 与 .so/.dll/.dylib）",
                path.display()
            ))),
        }
    }

    async fn uninstall(&self, id: &str) -> Result<(), RegistryError> {
        let source = {
            let entries = self.entries.lock();
            entries
                .iter()
                .find(|entry| entry.info.id == id)
                .ok_or_else(|| RegistryError::UnknownPlugin(id.to_string()))?
                .source
                .describe()
        };

        if !source.removable {
            return Err(RegistryError::NotInstallable(id.to_string()));
        }

        // Unload first: a native library cannot be deleted while it is mapped, and unloading is what
        // runs the plugin's `disable`/`destroy` through the adapter's `Drop`.
        self.disable(id).await?;

        {
            let mut entries = self.entries.lock();
            entries.retain(|entry| entry.info.id != id);
        }
        if let Some(dir) = source.package_dir {
            // Best effort: the plugin directory is the extension's private storage, both for its
            // package and for anything it wrote.
            let _ = std::fs::remove_dir_all(dir);
        }
        self.logs.lock().remove(id);
        Ok(())
    }

    fn log_tail(&self, id: &str, lines: usize) -> Vec<String> {
        let logs = self.logs.lock();
        logs.get(id)
            .map(|buffer| {
                let skip = buffer.len().saturating_sub(lines);
                buffer.iter().skip(skip).cloned().collect()
            })
            .unwrap_or_default()
    }
}

/// Fields needed to decide how a registered plugin can be uninstalled.
struct SourceSummary {
    /// Internal modules are compiled in, so there is nothing on disk to remove.
    removable: bool,
    /// The extension's private directory: its package and whatever it wrote. Removed on uninstall.
    package_dir: Option<PathBuf>,
}

impl Source {
    fn build(&self) -> Result<Arc<dyn Plugin>, RegistryError> {
        match self {
            Source::Internal(factory) => Ok(factory()),
            Source::Wasm(path) => {
                #[cfg(feature = "wasm-runtime")]
                {
                    // M6: instantiate the wasmtime module and wrap it in an adapter plugin.
                    Err(RegistryError::InvalidPackage(format!(
                        "wasm-runtime 装载尚未实现: {}",
                        path.display()
                    )))
                }
                #[cfg(not(feature = "wasm-runtime"))]
                {
                    Err(RegistryError::InvalidPackage(format!(
                        "本构建未启用 wasm-runtime: {}",
                        path.display()
                    )))
                }
            }
            // Opening happens per mount, not once at install: `enable` is what the control panel's
            // switch drives, and a plugin that was unloaded must be loadable again.
            Source::Native { library, files_dir } => NativeLibrary::open(library)
                .map_err(load_error)?
                .into_plugin(files_dir.clone())
                .map_err(load_error),
        }
    }

    fn describe(&self) -> SourceSummary {
        match self {
            Source::Internal(_) => SourceSummary {
                removable: false,
                package_dir: None,
            },
            Source::Wasm(path) => SourceSummary {
                removable: true,
                package_dir: path.parent().map(Path::to_path_buf),
            },
            Source::Native { library, .. } => SourceSummary {
                removable: true,
                package_dir: library.parent().map(Path::to_path_buf),
            },
        }
    }
}

/// Map a loader failure onto the registry's error surface. The ABI version mismatch keeps its own
/// variant because it is the one case the UI should explain differently from "bad package".
fn load_error(error: LoadError) -> RegistryError {
    match error {
        LoadError::AbiMismatch { wanted, found } => RegistryError::AbiMismatch {
            wanted: wanted.to_string(),
            found: found.to_string(),
        },
        other => RegistryError::InvalidPackage(other.to_string()),
    }
}

/// A plugin id used as a directory name: anything that is not safe in a path component is folded to
/// `-`, so an id from a manifest can never escape the extensions directory.
fn safe_component(id: &str) -> String {
    id.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '-'
            }
        })
        .collect()
}

/// The extension to store a copied library under. `.so`/`.dll`/`.dylib` all preserve their own
/// suffix; anything else falls back to `so`.
fn library_extension(picked: &Path) -> String {
    picked
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_else(|| "so".to_string())
}

/// Read and parse an extension manifest from a directory or a manifest file.
fn read_manifest(path: &Path) -> Result<ExtensionManifest, RegistryError> {
    let manifest_path = if path.is_dir() {
        path.join("manifest.toml")
    } else {
        path.to_path_buf()
    };
    if !manifest_path.exists() {
        return Err(RegistryError::InvalidPackage(format!(
            "缺少 manifest.toml: {}",
            manifest_path.display()
        )));
    }
    let raw = std::fs::read_to_string(&manifest_path)
        .map_err(|error| RegistryError::InvalidPackage(format!("无法读取 manifest: {error}")))?;
    toml::from_str(&raw)
        .map_err(|error| RegistryError::InvalidPackage(format!("manifest 解析失败: {error}")))
}

fn manifest_id(manifest: &ExtensionManifest) -> String {
    slug(&manifest.plugin.id, &manifest.plugin.name)
}

/// The id a plugin is registered under, falling back to a slug of the display name so a manifest
/// without an explicit id still works.
fn slug(id: &str, name: &str) -> String {
    if !id.is_empty() {
        return id.to_string();
    }
    name.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

/// Whether the user accepted everything the plugin declared. A plugin that asks for a capability the
/// user withheld is refused outright rather than run with a silently narrowed grant.
fn covers(accepted: &Permissions, declared: &Permissions) -> bool {
    (!declared.network || accepted.network)
        && (!declared.filesystem || accepted.filesystem)
        && (!declared.clipboard || accepted.clipboard)
}

/// The plugin wrapper that mounts `plugin-manager` on the kernel and exports the registry service.
struct PluginManagerPlugin {
    manager: Arc<PluginManager>,
}

#[async_trait]
impl Plugin for PluginManagerPlugin {
    fn name(&self) -> &str {
        "plugin-manager"
    }

    async fn apply(&self, ctx: &Arc<Context>) -> Result<(), PluginError> {
        ctx.provide_shared(
            PLUGIN_REGISTRY,
            RegistryHandle(self.manager.clone() as Arc<dyn PluginRegistry>),
        )
        .await;
        Ok(())
    }
}
