//! `plugin-manager` — the unified control panel's backend (ADR-008).
//!
//! It is deliberately **not** part of the kernel: it implements [`Plugin`] like every other
//! plugin, is mounted with `ctx.plugin()`, can be disabled, and provides the
//! [`PluginRegistry`] service on the root scope so any screen can consume it.
//!
//! Ownership model: this module is the only place that holds strong `Arc<Fiber>` handles, which is
//! what keeps mounted plugins alive. The kernel's ledger only tracks `Weak` references.

use async_trait::async_trait;
use cordis_rs::{Context, Fiber, Plugin, PluginError};
use inkweave_api::registry::{
    InstallRequest, Permissions, PluginInfo, PluginRegistry, PluginRuntimeState, RegistryError,
};
use inkweave_api::{RegistryHandle, SchedulerHandle, PLUGIN_REGISTRY, SCHEDULER};
use parking_lot::Mutex;
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// The contract version an extension must target (DEV guide §7.1 `inkweave_abi_v1`).
pub const SUPPORTED_ABI: &str = "inkweave_abi_v1";

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
}

impl PluginManager {
    pub fn new(root: Arc<Context>) -> Arc<Self> {
        Arc::new(Self {
            root,
            entries: Mutex::new(Vec::new()),
            logs: Mutex::new(HashMap::new()),
            max_log_lines: 200,
        })
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
        let manifest = read_manifest(Path::new(&request.package_path))?;

        if !covers(&request.accepted, &manifest.permissions) {
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
                kind: inkweave_api::registry::PluginKind::Wasm,
                state: PluginRuntimeState::Installed,
                author: manifest.plugin.author.clone(),
                description: manifest.plugin.description.clone(),
                permissions: Permissions {
                    network: manifest.permissions.network,
                    filesystem: manifest.permissions.filesystem,
                    clipboard: manifest.permissions.clipboard,
                },
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
            self.register_wasm(info.clone(), PathBuf::from(&request.package_path));
            self.record_log(&id, "installed");
            self.enable(&id).await?;
            Ok(info)
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

        if !source.is_wasm {
            return Err(RegistryError::NotInstallable(id.to_string()));
        }

        self.disable(id).await?;

        {
            let mut entries = self.entries.lock();
            entries.retain(|entry| entry.info.id != id);
        }
        if let Some(path) = source.package_path {
            let parent = path.parent().map(Path::to_path_buf);
            if let Some(dir) = parent {
                // Best effort: the package directory is the extension's private storage.
                let _ = std::fs::remove_dir_all(dir);
            }
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

/// Fields needed to decide how a manifest can be uninstalled.
struct SourceSummary {
    is_wasm: bool,
    package_path: Option<PathBuf>,
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
        }
    }

    fn describe(&self) -> SourceSummary {
        match self {
            Source::Internal(_) => SourceSummary {
                is_wasm: false,
                package_path: None,
            },
            Source::Wasm(path) => SourceSummary {
                is_wasm: true,
                package_path: Some(path.clone()),
            },
        }
    }
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
    if !manifest.plugin.id.is_empty() {
        return manifest.plugin.id.clone();
    }
    // Fall back to a slug of the display name so a manifest without an explicit id still works.
    manifest
        .plugin
        .name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

fn covers(accepted: &Permissions, declared: &ManifestPermissions) -> bool {
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
