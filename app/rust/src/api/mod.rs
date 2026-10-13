//! The `flutter_rust_bridge` API surface.
//!
//! Every function here is a thin delegate: the Dart layer calls these, and they call the same
//! [`PluginRegistry`](inkweave_api::PluginRegistry) that the in-app control panel renders. There is
//! exactly one management path, which is what makes the panel "unified" in practice and not just
//! in the mockup.

use crate::host;
use inkweave_api::registry::{InstallRequest, Permissions, PluginInfo};
use inkweave_api::runtime::{BackgroundScheduler, MemPressure, MemoryGovernor};
// The registry verbs live on the trait; the concrete `PluginManager` alone does not expose them.
use inkweave_api::PluginRegistry;
use std::path::Path;

/// Initialise the host. Must be called before any other function; idempotent.
pub fn bootstrap_host(data_dir: String) -> Result<(), String> {
    host::bootstrap(Path::new(&data_dir))
}

/// Contract version, shown in the about card.
pub fn api_version() -> String {
    inkweave_api::API_VERSION.to_string()
}

/// Everything the control panel renders, in one lock-free read (ADR-004).
pub fn plugin_cards() -> Result<Vec<PluginInfo>, String> {
    Ok(host::app().registry.snapshot())
}

/// Enable or disable a plugin. This is the "hot load / shutdown" verb pair of ADR-007.
pub fn set_plugin_enabled(id: String, enabled: bool) -> Result<(), String> {
    host::block_on(async move {
        let registry = &host::app().registry;
        let result = if enabled {
            registry.enable(&id).await
        } else {
            registry.disable(&id).await
        };
        result.map_err(|error| error.to_string())
    })
}

/// Install a WASM extension after the user accepted its permissions (§8.4).
pub fn install_extension(
    package_path: String,
    network: bool,
    filesystem: bool,
    clipboard: bool,
) -> Result<PluginInfo, String> {
    host::block_on(async move {
        host::app()
            .registry
            .install(InstallRequest {
                package_path,
                accepted: Permissions {
                    network,
                    filesystem,
                    clipboard,
                },
            })
            .await
            .map_err(|error| error.to_string())
    })
}

/// Uninstall a WASM extension. Internal modules return an error, shown verbatim by the UI.
pub fn uninstall_extension(id: String) -> Result<(), String> {
    host::block_on(async move {
        host::app()
            .registry
            .uninstall(&id)
            .await
            .map_err(|error| error.to_string())
    })
}

/// Tail of a plugin's log buffer for the "view log" action.
pub fn plugin_log(id: String, lines: u32) -> Result<Vec<String>, String> {
    Ok(host::app().registry.log_tail(&id, lines as usize))
}

/// Memory read-out for the panel header (§8.6).
#[derive(Clone, Debug)]
pub struct MemorySummary {
    pub process_used_mb: u32,
    pub pressure: String,
    pub over_budget: Vec<String>,
}

pub fn memory_summary() -> Result<MemorySummary, String> {
    let memory = &host::app().memory;
    let pressure = match memory.pressure() {
        MemPressure::None => "none",
        MemPressure::Background => "background",
        MemPressure::Low => "low",
        MemPressure::Critical => "critical",
    };
    Ok(MemorySummary {
        process_used_mb: memory.process_used_mb(),
        pressure: pressure.to_string(),
        over_budget: memory.over_budget(),
    })
}

/// One scheduled background task, for the panel's diagnostics.
#[derive(Clone, Debug)]
pub struct TaskSummary {
    pub owner: String,
    pub task: String,
    pub periodic: bool,
    pub interval_minutes: u32,
    pub requires_unmetered: bool,
}

/// Background work currently registered with the host scheduler (§8.5).
pub fn scheduler_tasks() -> Result<Vec<TaskSummary>, String> {
    let snapshot = host::app().scheduler.snapshot();
    let mut tasks: Vec<TaskSummary> = snapshot
        .into_iter()
        .flat_map(|(owner, list)| {
            list.into_iter().map(move |scheduled| TaskSummary {
                owner: owner.clone(),
                task: scheduled.task,
                periodic: scheduled.periodic,
                interval_minutes: scheduled.interval_minutes,
                requires_unmetered: scheduled.constraints.requires_unmetered,
            })
        })
        .collect();
    tasks.sort_by(|left, right| (&left.owner, &left.task).cmp(&(&right.owner, &right.task)));
    Ok(tasks)
}

/// Unfinished task count for one owner, used by the plugin card.
pub fn unfinished_tasks(owner: String) -> Result<u32, String> {
    Ok(host::app().scheduler.unfinished(&owner))
}

#[cfg(test)]
mod tests {
    use super::*;
    use inkweave_api::PluginRuntimeState;
    use std::sync::Mutex;

    /// The host is process-global, so these tests must not race each other.
    static TEST_LOCK: Mutex<()> = Mutex::new(());

    fn boot() -> std::sync::MutexGuard<'static, ()> {
        let guard = TEST_LOCK.lock().unwrap_or_else(|error| error.into_inner());
        bootstrap_host(std::env::temp_dir().to_string_lossy().to_string()).expect("host boots");
        guard
    }

    fn find(id: &str) -> PluginInfo {
        plugin_cards()
            .expect("snapshot")
            .into_iter()
            .find(|card| card.id == id)
            .unwrap_or_else(|| panic!("`{id}` is registered"))
    }

    #[test]
    fn presets_are_registered_and_enabled() {
        let _guard = boot();
        assert!(
            plugin_cards()
                .expect("snapshot")
                .iter()
                .any(|card| card.id == "plugin-manager"),
            "the control panel's own backend must appear in the list"
        );

        // The host is process-global and shared with the other tests, so set the state this test
        // asserts rather than assuming a particular test order.
        for id in ["stylus-generic", "sync-webdav"] {
            set_plugin_enabled(id.to_string(), true).expect("enable preset");
            assert_eq!(
                find(id).state,
                PluginRuntimeState::Active,
                "{id} must be active"
            );
        }
    }

    #[test]
    fn disabling_and_reenabling_a_preset_round_trips() {
        let _guard = boot();

        set_plugin_enabled("sync-webdav".to_string(), false).expect("disable");
        assert_eq!(find("sync-webdav").state, PluginRuntimeState::Installed);

        set_plugin_enabled("sync-webdav".to_string(), true).expect("enable");
        assert_eq!(find("sync-webdav").state, PluginRuntimeState::Active);
    }

    #[test]
    fn internal_modules_cannot_be_uninstalled() {
        let _guard = boot();
        let error = uninstall_extension("ink-engine".to_string()).expect_err("must refuse");
        assert!(
            error.contains("internal module"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn enabling_an_unknown_plugin_errors_instead_of_panicking() {
        let _guard = boot();
        let error = set_plugin_enabled("does.not.exist".to_string(), true).expect_err("must fail");
        assert!(
            error.contains("unknown plugin"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn enabling_sync_webdav_registers_a_background_task() {
        let _guard = boot();
        set_plugin_enabled("sync-webdav".to_string(), true).expect("enable");

        let tasks = scheduler_tasks().expect("scheduler snapshot");
        assert!(
            tasks
                .iter()
                .any(|task| task.task == "sync" && task.requires_unmetered),
            "a Wi-Fi-only periodic sync must be registered: {tasks:?}"
        );

        // Disabling must cancel the plugin's work — the host enforces this, not the plugin.
        set_plugin_enabled("sync-webdav".to_string(), false).expect("disable");
        assert_eq!(
            unfinished_tasks(inkweave_api::runtime::task_owner("sync-webdav")).expect("count"),
            0,
            "the host must cancel every task the plugin registered under its own namespace"
        );

        // Leave the host as the other tests expect to find it.
        set_plugin_enabled("sync-webdav".to_string(), true).expect("restore");
    }
}
