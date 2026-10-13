use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// Background work constraints, mapped onto `WorkManager` (Android) / `BGTaskScheduler` (iOS)
/// by the platform adapter. Plugins declare intent; the host owns the mechanism (DEV guide §8.5).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkConstraints {
    pub requires_network: bool,
    pub requires_unmetered: bool,
    pub requires_charging: bool,
    pub requires_battery_not_low: bool,
}

impl WorkConstraints {
    /// Wi-Fi-only periodic sync, the default for `sync-webdav`.
    pub const UNMETERED: Self = Self {
        requires_network: true,
        requires_unmetered: true,
        requires_charging: false,
        requires_battery_not_low: true,
    };

    /// Metadata-only sync allowed on cellular (research 04 §4.3).
    pub const METERED_OK: Self = Self {
        requires_network: true,
        requires_unmetered: false,
        requires_charging: false,
        requires_battery_not_low: true,
    };
}

/// Stable task name, namespaced per owner by the host (e.g. `webdav:sync`).
pub type TaskId = String;

/// The namespace a plugin owns for its background work.
///
/// Both sides derive it from this one function — the plugin when it enqueues, the host when it
/// cancels on disable/unload — so a plugin can never be left with work the host cannot reach.
pub fn task_owner(plugin_id: &str) -> String {
    format!("plugin:{plugin_id}")
}

#[derive(Debug, thiserror::Error)]
pub enum ScheduleError {
    #[error("task `{0}` is already scheduled with different parameters")]
    AlreadyScheduled(TaskId),

    #[error("the platform scheduler rejected the request: {0}")]
    Rejected(String),
}

/// The **only** sanctioned background-work channel (§8.5).
///
/// The host cancels everything owned by a plugin when it is disabled or unloaded, which is why
/// plugins never need (and never get) direct access to the platform scheduler.
#[async_trait]
pub trait BackgroundScheduler: Send + Sync + 'static {
    async fn enqueue_periodic(
        &self,
        owner: &str,
        task: TaskId,
        interval_minutes: u32,
        constraints: WorkConstraints,
    ) -> Result<(), ScheduleError>;

    async fn enqueue_one_shot(
        &self,
        owner: &str,
        task: TaskId,
        delay_seconds: u32,
        expedited: bool,
        constraints: WorkConstraints,
    ) -> Result<(), ScheduleError>;

    async fn cancel(&self, owner: &str, task: TaskId) -> Result<(), ScheduleError>;

    /// Cancel every task owned by a plugin. Called by the kernel's unload path, not by plugins.
    async fn cancel_owner(&self, owner: &str);

    /// Number of unfinished tasks, surfaced in the control panel card.
    fn unfinished(&self, owner: &str) -> u32;
}

/// Memory pressure levels the host broadcasts to plugins (§8.6).
///
/// Ordered: `Critical` > `Low` > `Background` > `None`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemPressure {
    #[default]
    None,
    /// App moved to the background (`TRIM_MEMORY_UI_HIDDEN`): stop animations, drop previews.
    Background,
    /// `TRIM_MEMORY_RUNNING_LOW`: release rebuildable caches (tiles, thumbnails, indices).
    Low,
    /// `TRIM_MEMORY_COMPLETE`: release everything rebuildable. The host also unloads idle plugins.
    Critical,
}

/// Tracks per-plugin memory budgets and the current pressure level.
///
/// Default extension budget is 64 MB (DEV guide §7.1); internal modules declare their own.
pub const DEFAULT_EXTENSION_BUDGET_MB: u32 = 64;

pub trait MemoryGovernor: Send + Sync + 'static {
    fn budget_mb(&self, owner: &str) -> u32;

    fn used_mb(&self, owner: &str) -> u32;

    fn set_budget_mb(&self, owner: &str, budget_mb: u32);

    /// Current process-wide pressure, as last broadcast.
    fn pressure(&self) -> MemPressure;

    /// Process-wide heap usage in MiB, for the control panel header.
    fn process_used_mb(&self) -> u32;
}
