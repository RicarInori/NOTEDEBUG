use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// Mirror of the kernel's `FiberState`, in a serde-friendly form for the UI bridge.
///
/// The kernel deliberately does not depend on serde; this DTO is the boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginRuntimeState {
    Pending,
    Loading,
    Active,
    Failed,
    Unloading,
    Disposed,
    /// Installed on disk but this run has never mounted it.
    Installed,
}

impl From<cordis_rs::FiberState> for PluginRuntimeState {
    fn from(state: cordis_rs::FiberState) -> Self {
        match state {
            cordis_rs::FiberState::Pending => Self::Pending,
            cordis_rs::FiberState::Loading => Self::Loading,
            cordis_rs::FiberState::Active => Self::Active,
            cordis_rs::FiberState::Failed => Self::Failed,
            cordis_rs::FiberState::Unloading => Self::Unloading,
            cordis_rs::FiberState::Disposed => Self::Disposed,
        }
    }
}

/// How a plugin is distributed. Determines which of the four verbs (ADR-007) apply.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginKind {
    /// First-party crate compiled into the host. Can be enabled/disabled, not uninstalled.
    Internal,
    /// Third-party WASM extension. Can be enabled, disabled, installed and uninstalled.
    Wasm,
}

/// Declared capabilities. Rendered as a permission list in the control panel; enforced by the
/// WASM host-function ABI for extensions (DEV guide §7.1).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Permissions {
    pub network: bool,
    pub filesystem: bool,
    pub clipboard: bool,
}

impl Permissions {
    pub const NONE: Self = Self {
        network: false,
        filesystem: false,
        clipboard: false,
    };

    /// Human-readable list for the confirmation dialog in §8.4.
    pub fn labels(&self) -> Vec<&'static str> {
        let mut out = Vec::new();
        if self.network {
            out.push("网络");
        }
        if self.filesystem {
            out.push("文件");
        }
        if self.clipboard {
            out.push("剪贴板");
        }
        out
    }
}

/// A UI entry a plugin contributes. The host renders it with Material 3 components; plugins never
/// build their own management chrome (§8.3).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContributionKind {
    Command,
    SettingsView,
    MenuItem,
    Brush,
    Template,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Contribution {
    pub id: String,
    pub title: String,
    pub kind: ContributionKind,
}

/// Everything the control panel renders for one plugin.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PluginInfo {
    pub id: String,
    pub name: String,
    pub version: String,
    pub kind: PluginKind,
    pub state: PluginRuntimeState,
    pub author: String,
    pub description: String,
    pub permissions: Permissions,
    pub capabilities: crate::stylus::StylusCapabilities,
    pub memory_budget_mb: u32,
    pub memory_used_mb: u32,
    pub background_tasks: u32,
    pub contributions: Vec<Contribution>,
    pub last_error: Option<String>,
}

impl PluginInfo {
    /// Whether the control panel's "uninstall" action applies (WASM only, §8.4).
    pub fn is_uninstallable(&self) -> bool {
        self.kind == PluginKind::Wasm
    }
}

/// Request produced by the "add plugin" entry after the user confirmed the permission list.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InstallRequest {
    /// Absolute path of the extension package in the app's private storage.
    pub package_path: String,
    /// Permissions the user accepted; must cover the manifest's declaration.
    pub accepted: Permissions,
}

#[derive(Debug, thiserror::Error)]
pub enum RegistryError {
    #[error("unknown plugin `{0}`")]
    UnknownPlugin(String),

    #[error("`{0}` is an internal module and cannot be installed or uninstalled")]
    NotInstallable(String),

    #[error("contract version mismatch: extension wants `{wanted}`, host provides `{found}`")]
    AbiMismatch { wanted: String, found: String },

    #[error("the package is not a valid Inkweave extension: {0}")]
    InvalidPackage(String),

    #[error("the user declined a required permission: {0}")]
    PermissionDenied(&'static str),

    #[error("extension `{0}` is already installed")]
    AlreadyInstalled(String),

    #[error("failed to mount the plugin: {0}")]
    MountFailed(String),
}

/// The **Definition** behind the unified control panel (ADR-008).
///
/// Provider: the `plugin-manager` internal module. Consumers: the settings screens and, through
/// the lifecycle bridge, the Flutter control panel. Consumers must never mutate another plugin
/// directly — every state change goes through these verbs.
#[async_trait]
pub trait PluginRegistry: Send + Sync + 'static {
    /// Lock-free snapshot for rendering (ADR-004: reads never block registration).
    fn snapshot(&self) -> Vec<PluginInfo>;

    fn state(&self, id: &str) -> Option<PluginRuntimeState>;

    /// Enable = mount + apply (ADR-007). Idempotent.
    async fn enable(&self, id: &str) -> Result<(), RegistryError>;

    /// Disable = recover effects and cancel owned work, keeping the plugin installed.
    async fn disable(&self, id: &str) -> Result<(), RegistryError>;

    /// Install a WASM extension and mount it. Validates manifest, ABI version and signature (§8.4).
    async fn install(&self, request: InstallRequest) -> Result<PluginInfo, RegistryError>;

    /// Uninstall a WASM extension: dispose the fiber and delete its package. Internal modules
    /// return [`RegistryError::NotInstallable`].
    async fn uninstall(&self, id: &str) -> Result<(), RegistryError>;

    /// Tail of the plugin's log buffer, for the "view log" action.
    fn log_tail(&self, id: &str, lines: usize) -> Vec<String>;
}
