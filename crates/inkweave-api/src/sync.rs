use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// The sync state machine of `sync-webdav` (DEV guide §5.5 / research 04 §4.3).
///
/// ```text
/// Idle → CheckLocal → CheckRemote → DiffCalc → {Upload | Download | Conflict}
///   → Apply → Verify → Cleanup → Idle
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncState {
    Idle,
    CheckLocal,
    CheckRemote,
    DiffCalc,
    Upload,
    Download,
    Conflict,
    Apply,
    Verify,
    Cleanup,
}

/// Per-file sync bookkeeping (research 04 §6.1 `SyncRecord`).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SyncRecord {
    pub id: String,
    pub local_etag: Option<String>,
    pub remote_etag: Option<String>,
    pub local_mtime_ms: u64,
    pub remote_mtime_ms: u64,
    pub is_deleted: bool,
    pub deleted_at_ms: Option<u64>,
    pub status: SyncStatus,
}

/// Tombstones are kept for 30 days before physical removal (DEV guide §5.5).
pub const TOMBSTONE_RETENTION_DAYS: u32 = 30;

/// 坚果云 starts returning 429 above 4 concurrent connections, so the hard cap is 4.
pub const MAX_CONCURRENT_TRANSFERS: u8 = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncStatus {
    Synced,
    Dirty,
    Conflict,
    Pending,
}

/// Conflict resolution. `LastWriteWins` uses millisecond timestamps plus a device id tie-breaker,
/// and degrades to `KeepBoth` when a local clock rollback of more than 5 minutes is detected.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConflictPolicy {
    LastWriteWins,
    KeepBoth,
    ThreeWayMerge,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SyncProviderConfig {
    pub server_url: String,
    pub username: String,
    /// Credential reference resolved from the platform secure store (Keystore / Keychain),
    /// never a plaintext password in memory longer than one request.
    pub credential_ref: String,
    pub remote_dir: String,
    pub policy: ConflictPolicy,
    pub wifi_only: bool,
    pub concurrency: u8,
}

impl Default for SyncProviderConfig {
    fn default() -> Self {
        Self {
            server_url: String::new(),
            username: String::new(),
            credential_ref: String::new(),
            remote_dir: "/inkweave/".to_string(),
            policy: ConflictPolicy::LastWriteWins,
            wifi_only: true,
            concurrency: MAX_CONCURRENT_TRANSFERS,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SyncReport {
    pub uploaded: u32,
    pub downloaded: u32,
    pub conflicts: u32,
    pub tombstones_purged: u32,
    pub finished_at_ms: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum SyncError {
    #[error("sync provider is not configured")]
    NotConfigured,

    #[error("authentication rejected by the server")]
    Unauthorized,

    #[error("the server is unreachable: {0}")]
    Unreachable(String),

    #[error("a concurrent sync is already running")]
    AlreadyRunning,

    #[error("the remote rejected a conditional write with 412 (etag mismatch)")]
    PreconditionFailed,
}

/// A sync backend. `sync-webdav` is the preset implementation; a future cloud provider plugs in
/// behind the same trait without any consumer changing (capability seam, §3.3).
#[async_trait]
pub trait SyncProvider: Send + Sync + 'static {
    fn state(&self) -> SyncState;

    fn config(&self) -> SyncProviderConfig;

    /// Persist configuration. The credential itself goes to the platform secure store.
    async fn save_config(&self, config: SyncProviderConfig) -> Result<(), SyncError>;

    /// Run one full synchronisation pass. Safe to call from a `BackgroundScheduler` task.
    async fn sync_now(&self) -> Result<SyncReport, SyncError>;
}
