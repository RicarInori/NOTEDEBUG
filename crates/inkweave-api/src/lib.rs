//! The **Definition** layer (DEV guide §2 §3.3).
//!
//! Providers implement these traits; consumers depend only on them. Neither side imports the
//! other, so a backend can be replaced without touching a single consumer — that is the whole
//! point of the capability-seam pattern.
//!
//! ```text
//!   Definition (this crate)  →  Provider (a plugin)  →  Consumer (another plugin)
//! ```
//!
//! Nothing here knows about Flutter, wgpu, SQLite or WebDAV. Platform specifics live behind
//! [`BackgroundScheduler`](runtime::BackgroundScheduler) and [`MemoryGovernor`](runtime::MemoryGovernor).

pub mod handles;
pub mod keys;
pub mod registry;
pub mod runtime;
pub mod stylus;
pub mod sync;

pub use handles::{MemoryHandle, RegistryHandle, SchedulerHandle, StylusHandle, SyncHandle};
pub use keys::*;
pub use registry::{
    Contribution, ContributionKind, InstallRequest, PluginInfo, PluginKind, PluginRegistry,
    PluginRuntimeState, RegistryError,
};
pub use runtime::{
    BackgroundScheduler, MemPressure, MemoryGovernor, ScheduleError, TaskId, WorkConstraints,
};
pub use stylus::{
    ButtonAction, InputPhase, PressureCurve, StylusButtonEvent, StylusButtonKind,
    StylusCapabilities, StylusData, StylusDriver, StylusInputEvent, StylusVendor,
};
pub use sync::{
    ConflictPolicy, SyncError, SyncProvider, SyncProviderConfig, SyncRecord, SyncReport, SyncState,
    SyncStatus,
};

/// Build stamp surfaced in the control panel's about card.
pub const API_VERSION: &str = env!("CARGO_PKG_VERSION");
