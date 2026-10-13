//! The `flutter_rust_bridge` API surface.
//!
//! Every function here is a thin delegate: the Dart layer calls these, and they call the same
//! [`PluginRegistry`](inkweave_api::PluginRegistry) that the in-app control panel renders. There is
//! exactly one management path, which is what makes the panel "unified" in practice and not just
//! in the mockup.
//!
//! # Why this module owns its own wire types
//!
//! The kernel and the API contract live in other crates, and frb's only way to use a foreign type
//! is the `#[frb(mirror(...))]` attribute, which requires re-listing every field — duplication in a
//! different notation. So the bridge declares explicit DTOs here and converts. That also decouples
//! the wire format from internal types: reshaping `PluginInfo` cannot silently change the Dart API.

use crate::host;
use inkweave_api::registry::InstallRequest;
use inkweave_api::runtime::{BackgroundScheduler, MemPressure, MemoryGovernor};
// The registry verbs live on the trait; the concrete `PluginManager` alone does not expose them.
use inkweave_api::PluginRegistry;
use std::path::Path;

// --- wire types -------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PluginKindDto {
    Internal,
    Wasm,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PluginStateDto {
    Installed,
    Pending,
    Loading,
    Active,
    Failed,
    Unloading,
    Disposed,
}

#[derive(Clone, Debug)]
pub struct PermissionsDto {
    pub network: bool,
    pub filesystem: bool,
    pub clipboard: bool,
}

impl From<inkweave_api::registry::Permissions> for PermissionsDto {
    fn from(value: inkweave_api::registry::Permissions) -> Self {
        Self {
            network: value.network,
            filesystem: value.filesystem,
            clipboard: value.clipboard,
        }
    }
}

#[derive(Clone, Debug)]
pub struct StylusCapabilitiesDto {
    pub pressure: bool,
    pub tilt: bool,
    pub hover: bool,
    pub eraser: bool,
    pub predict_stroke: bool,
    pub recognize_shape: bool,
    pub recognize_handwriting: bool,
}

impl From<inkweave_api::stylus::StylusCapabilities> for StylusCapabilitiesDto {
    fn from(value: inkweave_api::stylus::StylusCapabilities) -> Self {
        Self {
            pressure: value.pressure,
            tilt: value.tilt,
            hover: value.hover,
            eraser: value.eraser,
            predict_stroke: value.predict_stroke,
            recognize_shape: value.recognize_shape,
            recognize_handwriting: value.recognize_handwriting,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContributionKindDto {
    Command,
    SettingsView,
    MenuItem,
    Brush,
    Template,
}

#[derive(Clone, Debug)]
pub struct ContributionDto {
    pub id: String,
    pub title: String,
    pub kind: ContributionKindDto,
}

/// One plugin, exactly as the control panel needs it.
#[derive(Clone, Debug)]
pub struct PluginCardDto {
    pub id: String,
    pub name: String,
    pub version: String,
    pub kind: PluginKindDto,
    pub state: PluginStateDto,
    pub author: String,
    pub description: String,
    pub permissions: PermissionsDto,
    pub capabilities: StylusCapabilitiesDto,
    pub memory_budget_mb: u32,
    pub memory_used_mb: u32,
    pub background_tasks: u32,
    pub contributions: Vec<ContributionDto>,
    pub last_error: Option<String>,
}

impl From<inkweave_api::registry::PluginInfo> for PluginCardDto {
    fn from(value: inkweave_api::registry::PluginInfo) -> Self {
        use inkweave_api::registry::{ContributionKind, PluginKind, PluginRuntimeState};

        Self {
            id: value.id,
            name: value.name,
            version: value.version,
            kind: match value.kind {
                PluginKind::Internal => PluginKindDto::Internal,
                PluginKind::Wasm => PluginKindDto::Wasm,
            },
            state: match value.state {
                PluginRuntimeState::Installed => PluginStateDto::Installed,
                PluginRuntimeState::Pending => PluginStateDto::Pending,
                PluginRuntimeState::Loading => PluginStateDto::Loading,
                PluginRuntimeState::Active => PluginStateDto::Active,
                PluginRuntimeState::Failed => PluginStateDto::Failed,
                PluginRuntimeState::Unloading => PluginStateDto::Unloading,
                PluginRuntimeState::Disposed => PluginStateDto::Disposed,
            },
            author: value.author,
            description: value.description,
            permissions: value.permissions.into(),
            capabilities: value.capabilities.into(),
            memory_budget_mb: value.memory_budget_mb,
            memory_used_mb: value.memory_used_mb,
            background_tasks: value.background_tasks,
            contributions: value
                .contributions
                .into_iter()
                .map(|contribution| ContributionDto {
                    id: contribution.id,
                    title: contribution.title,
                    kind: match contribution.kind {
                        ContributionKind::Command => ContributionKindDto::Command,
                        ContributionKind::SettingsView => ContributionKindDto::SettingsView,
                        ContributionKind::MenuItem => ContributionKindDto::MenuItem,
                        ContributionKind::Brush => ContributionKindDto::Brush,
                        ContributionKind::Template => ContributionKindDto::Template,
                    },
                })
                .collect(),
            last_error: value.last_error,
        }
    }
}

/// Memory read-out for the panel header (§8.6).
#[derive(Clone, Debug)]
pub struct MemorySummary {
    pub process_used_mb: u32,
    pub pressure: String,
    pub over_budget: Vec<String>,
}

/// One scheduled background task, for the panel's diagnostics (§8.5).
#[derive(Clone, Debug)]
pub struct TaskSummary {
    pub owner: String,
    pub task: String,
    pub periodic: bool,
    pub interval_minutes: u32,
    pub requires_unmetered: bool,
}

// --- API --------------------------------------------------------------------

/// Initialise the host. Must be called before any other function; idempotent.
pub fn bootstrap_host(data_dir: String) -> Result<(), String> {
    host::bootstrap(Path::new(&data_dir))
}

/// Contract version, shown in the about card.
pub fn api_version() -> String {
    inkweave_api::API_VERSION.to_string()
}

/// Everything the control panel renders, in one lock-free read (ADR-004).
pub fn plugin_cards() -> Result<Vec<PluginCardDto>, String> {
    Ok(host::app()
        .registry
        .snapshot()
        .into_iter()
        .map(PluginCardDto::from)
        .collect())
}

/// Enable or disable a plugin. This is the "hot load / shutdown" verb pair of ADR-007.
///
/// A successful change is remembered on disk, so the choice survives a restart — `bootstrap` reads
/// the same file and leaves disabled presets alone.
pub fn set_plugin_enabled(id: String, enabled: bool) -> Result<(), String> {
    let remembered = id.clone();
    host::block_on(async move {
        let registry = &host::app().registry;
        let result = if enabled {
            registry.enable(&id).await
        } else {
            registry.disable(&id).await
        };
        if result.is_ok() {
            host::app().state.set_disabled(&remembered, !enabled);
        }
        result.map_err(|error| error.to_string())
    })
}

/// Install a WASM extension after the user accepted its permissions (§8.4).
pub fn install_extension(
    package_path: String,
    network: bool,
    filesystem: bool,
    clipboard: bool,
) -> Result<PluginCardDto, String> {
    host::block_on(async move {
        host::app()
            .registry
            .install(InstallRequest {
                package_path,
                accepted: inkweave_api::registry::Permissions {
                    network,
                    filesystem,
                    clipboard,
                },
            })
            .await
            .map(PluginCardDto::from)
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
    use std::sync::Mutex;

    /// The host is process-global, so these tests must not race each other.
    static TEST_LOCK: Mutex<()> = Mutex::new(());

    fn boot() -> std::sync::MutexGuard<'static, ()> {
        let guard = TEST_LOCK.lock().unwrap_or_else(|error| error.into_inner());
        // A fresh directory per test-binary run: the host now persists disabled plugins, so a
        // shared temp dir would let one run's state leak into the next and fail unrelated tests.
        let data_dir =
            std::env::temp_dir().join(format!("inkweave-bridge-tests-{}", std::process::id()));
        bootstrap_host(data_dir.to_string_lossy().to_string()).expect("host boots");
        guard
    }

    fn find(id: &str) -> PluginCardDto {
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
                PluginStateDto::Active,
                "{id} must be active"
            );
        }
    }

    #[test]
    fn the_wire_types_carry_what_the_panel_renders() {
        let _guard = boot();
        let stylus = find("stylus-generic");
        assert_eq!(stylus.kind, PluginKindDto::Internal);
        assert!(
            stylus.memory_budget_mb > 0,
            "the budget is shown on the card"
        );
        assert!(
            !stylus.contributions.is_empty(),
            "the card offers a way into the plugin's own view"
        );

        let sync = find("sync-webdav");
        assert!(
            sync.permissions.network,
            "a declared permission must survive the bridge"
        );
    }

    #[test]
    fn disabling_and_reenabling_a_preset_round_trips() {
        let _guard = boot();

        set_plugin_enabled("sync-webdav".to_string(), false).expect("disable");
        assert_eq!(find("sync-webdav").state, PluginStateDto::Installed);

        set_plugin_enabled("sync-webdav".to_string(), true).expect("enable");
        assert_eq!(find("sync-webdav").state, PluginStateDto::Active);
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
    fn disabling_a_plugin_is_remembered_for_the_next_launch() {
        let _guard = boot();

        set_plugin_enabled("sync-webdav".to_string(), false).expect("disable");
        assert!(
            host::app().state.disabled_plugins().contains("sync-webdav"),
            "the choice must be persisted, or bootstrap would re-enable it next launch"
        );

        // Enabling again must clear it, otherwise the file would accumulate stale entries.
        set_plugin_enabled("sync-webdav".to_string(), true).expect("enable");
        assert!(!host::app().state.disabled_plugins().contains("sync-webdav"));
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
