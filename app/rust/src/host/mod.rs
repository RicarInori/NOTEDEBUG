//! Platform services and the composition root.
//!
//! Everything the host *must* provide but the kernel deliberately does not know about lives here:
//! background scheduling (§8.5) and memory governance (§8.6). They are provided as ordinary
//! services on the root scope, so plugins look them up exactly like any other capability.

mod memory;
mod scheduler;

pub use memory::HostMemoryGovernor;
pub use scheduler::HostBackgroundScheduler;

use cordis_rs::{Context, Plugin};
use inkweave_api::registry::{Contribution, Permissions, PluginInfo, PluginKind};
// `enable` / `disable` are trait methods; the concrete manager alone does not expose them.
use inkweave_api::PluginRegistry;
use inkweave_api::{MemoryHandle, SchedulerHandle, MEMORY, SCHEDULER};
use plugin_manager::PluginManager;
use std::future::Future;
use std::path::Path;
use std::sync::{Arc, OnceLock};
use tokio::runtime::Runtime;

/// Process-wide host state. Created once by [`bootstrap`].
pub struct AppHandle {
    pub root: Arc<Context>,
    /// Held directly (not only via the registry service) so the control panel can still operate
    /// after `plugin-manager` itself has been disabled — the self-degradation case of §8.3.
    pub registry: Arc<PluginManager>,
    /// Concrete handles for the diagnostics read-outs. The trait-object services are still what
    /// plugins consume; these are for the host's own API surface.
    pub scheduler: Arc<HostBackgroundScheduler>,
    pub memory: Arc<HostMemoryGovernor>,
    pub runtime: Runtime,
}

static APP: OnceLock<AppHandle> = OnceLock::new();

/// The initialised host. Panics if called before [`bootstrap`].
pub fn app() -> &'static AppHandle {
    APP.get()
        .expect("host is not initialised: call bootstrap() first")
}

/// Run a future to completion on the host runtime.
pub fn block_on<F: Future>(future: F) -> F::Output {
    app().runtime.block_on(future)
}

/// Build the kernel, register every preset plugin and enable the defaults.
///
/// `_data_dir` will hold `extensions/` and the document store once those crates land; it is
/// accepted now so the signature does not change under the Flutter layer later.
pub fn bootstrap(_data_dir: &Path) -> Result<(), String> {
    if APP.get().is_some() {
        return Ok(());
    }

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .map_err(|error| format!("failed to start the host runtime: {error}"))?;

    let root = Context::root();
    let registry = PluginManager::new(root.clone());
    let scheduler = Arc::new(HostBackgroundScheduler::new());
    let memory = Arc::new(HostMemoryGovernor::new());

    runtime.block_on(async {
        // Host-owned services. Whatever a plugin needs to *request* background work or report
        // memory goes through these two handles.
        root.provide(SCHEDULER, SchedulerHandle(scheduler.clone()))
            .await;
        root.provide(MEMORY, MemoryHandle(memory.clone())).await;
    });

    register_preset_plugins(&registry);

    let handle = AppHandle {
        root,
        registry,
        scheduler,
        memory,
        runtime,
    };
    APP.set(handle)
        .map_err(|_| "host was initialised concurrently")?;

    // Mount the presets. `plugin-manager` first so the registry service exists for the rest.
    block_on(async {
        for id in DEFAULT_ENABLED {
            if let Err(error) = app().registry.enable(id).await {
                // A failed preset must not take the host down; the control panel shows it as Failed.
                app()
                    .registry
                    .record_log(id, format!("启动时启用失败: {error}"));
            }
        }
    });

    Ok(())
}

/// Preset plugins, in mount order. Every one of these is toggleable from the control panel.
pub const DEFAULT_ENABLED: [&str; 8] = [
    "plugin-manager",
    "stylus-generic",
    "sync-webdav",
    "ink-engine",
    "pdfium-engine",
    "storage-sqlite",
    "nbk-migrate",
    "ext-loader",
];

fn register_preset_plugins(registry: &Arc<PluginManager>) {
    // `plugin-manager` reuses its own singleton so `enable`/`disable` always target one instance.
    let manager = registry.clone();
    registry.register_internal(
        internal_info(
            "plugin-manager",
            "统一控制面板",
            "插件启停、装入、卸载与运行观测的唯一入口",
            Permissions::NONE,
        ),
        move || manager.mount(),
    );

    registry.register_internal(
        internal_info(
            "stylus-generic",
            "手写笔系统",
            "低延迟手写输入、压感与掌拒；厂商适配器按能力位图激活时替换本 provider",
            Permissions::NONE,
        ),
        || {
            let (plugin, _driver) = stylus_drivers::StylusProviderPlugin::new();
            plugin as Arc<dyn Plugin>
        },
    );

    registry.register_internal(
        internal_info(
            "sync-webdav",
            "WebDAV 同步",
            "六态同步状态机、etag 双校验与 LWW 冲突解决",
            Permissions {
                network: true,
                filesystem: true,
                clipboard: false,
            },
        ),
        || {
            // The HTTP transport (reqwest + quick-xml) is the M4 deliverable; until then the
            // provider validates configuration and reports the transport as unavailable.
            let provider =
                sync_webdav::WebDavSyncProvider::new(Arc::new(sync_webdav::UnavailableTransport));
            sync_webdav::SyncWebDavPlugin::new(provider) as Arc<dyn Plugin>
        },
    );

    registry.register_internal(
        internal_info(
            "ink-engine",
            "手写引擎",
            "笔画管线、工具与画布渲染（M1）",
            Permissions::NONE,
        ),
        ink_engine::plugin,
    );
    registry.register_internal(
        internal_info(
            "pdfium-engine",
            "PDF 底座",
            "句柄池、tile 渲染、批注与导出（M3）",
            Permissions::NONE,
        ),
        pdfium_engine::plugin,
    );
    registry.register_internal(
        internal_info(
            "storage-sqlite",
            "文档存储",
            "SQLite + FTS5、.nbk 容器与 undo 日志（M2）",
            Permissions::NONE,
        ),
        storage_sqlite::plugin,
    );
    registry.register_internal(
        internal_info(
            "nbk-migrate",
            "格式迁移",
            ".nbk 版本迁移链（M2）",
            Permissions::NONE,
        ),
        || cordis_rs::FnPlugin::new("nbk-migrate", |_ctx: &Arc<Context>| {}).shared(),
    );
    registry.register_internal(
        internal_info(
            "ext-loader",
            "扩展沙箱",
            "wasmtime 沙箱与 inkweave ABI（M6）",
            Permissions::NONE,
        ),
        ext_loader::plugin,
    );
}

fn internal_info(id: &str, name: &str, description: &str, permissions: Permissions) -> PluginInfo {
    PluginInfo {
        id: id.to_string(),
        name: name.to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        kind: PluginKind::Internal,
        state: inkweave_api::PluginRuntimeState::Installed,
        author: "Inkweave".to_string(),
        description: description.to_string(),
        permissions,
        capabilities: Default::default(),
        memory_budget_mb: match id {
            "stylus-generic" => 64,
            "sync-webdav" => 48,
            "pdfium-engine" => 128,
            "ink-engine" => 96,
            _ => 32,
        },
        memory_used_mb: 0,
        background_tasks: 0,
        contributions: match id {
            "stylus-generic" => vec![Contribution {
                id: "stylus.canvas".to_string(),
                title: "手写板".to_string(),
                kind: inkweave_api::ContributionKind::SettingsView,
            }],
            "sync-webdav" => vec![Contribution {
                id: "sync.settings".to_string(),
                title: "WebDAV 同步".to_string(),
                kind: inkweave_api::ContributionKind::SettingsView,
            }],
            _ => Vec::new(),
        },
        last_error: None,
    }
}
