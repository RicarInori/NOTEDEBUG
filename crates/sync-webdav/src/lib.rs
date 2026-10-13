//! `sync-webdav` — preset plugin implementing the [`SyncProvider`] seam (DEV guide §5.5, §8.7).
//!
//! Two deliberate decisions from the research round are reflected here:
//!
//! 1. **No third-party WebDAV crate.** `reqwest_dav` has very few stars, so the plan is `reqwest`
//!    plus a hand-written WebDAV verb subset (PROPFIND / GET / PUT / DELETE / MKCOL / MOVE, ~300
//!    lines) with `quick-xml` for the multistatus body. The HTTP layer is therefore behind
//!    [`WebDavTransport`], which keeps it swappable and testable.
//! 2. **Concurrency is capped at 4.** 坚果云 replies 429 above four connections, so the cap is a
//!    constant in `inkweave-api`, not a tunable default.
//!
//! The transport implementation is the M4 deliverable; this crate ships the state machine, the
//! record bookkeeping and the plugin wiring, all of which are testable without a server.

use async_trait::async_trait;
use cordis_rs::{Context, Plugin, PluginError};
use inkweave_api::sync::{
    ConflictPolicy, SyncError, SyncProvider, SyncProviderConfig, SyncRecord, SyncReport, SyncState,
};
use inkweave_api::{SyncHandle, SYNC};
use parking_lot::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

/// Clock rollbacks beyond this are treated as suspicious, which disables Last-Write-Wins.
pub const CLOCK_ROLLBACK_TOLERANCE_MS: u64 = 5 * 60 * 1000;

/// Registry id of this plugin. The background-task namespace is derived from it via
/// [`inkweave_api::runtime::task_owner`], which is what lets the host cancel this plugin's work
/// when it is disabled.
pub const PLUGIN_ID: &str = "sync-webdav";

/// Periodic sync interval, in minutes.
pub const SYNC_INTERVAL_MINUTES: u32 = 15;

/// Name of the periodic sync task within this plugin's namespace.
pub const TASK_SYNC: &str = "sync";

/// One entry of a PROPFIND multistatus response.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteEntry {
    pub href: String,
    pub etag: Option<String>,
    pub mtime_ms: u64,
    pub is_collection: bool,
}

/// The HTTP surface the sync engine needs. Implemented with `reqwest` in M4.
#[async_trait]
pub trait WebDavTransport: Send + Sync + 'static {
    async fn propfind(&self, url: &str, depth: u8) -> Result<Vec<RemoteEntry>, SyncError>;

    /// Conditional write: `if_match` becomes `If-Match: "<etag>"`; a 412 maps to
    /// [`SyncError::PreconditionFailed`].
    async fn put(&self, url: &str, bytes: &[u8], if_match: Option<&str>) -> Result<(), SyncError>;

    async fn get(&self, url: &str) -> Result<Vec<u8>, SyncError>;

    /// `MKCOL`. A 405 ("already exists") is success, not an error.
    async fn mkcol(&self, url: &str) -> Result<(), SyncError>;

    async fn delete(&self, url: &str) -> Result<(), SyncError>;
}

/// Placeholder transport used when the platform layer has not installed a real one yet.
pub struct UnavailableTransport;

#[async_trait]
impl WebDavTransport for UnavailableTransport {
    async fn propfind(&self, _url: &str, _depth: u8) -> Result<Vec<RemoteEntry>, SyncError> {
        Err(SyncError::Unreachable(
            "尚未接入 HTTP 传输层（M4 交付：reqwest + quick-xml 的 WebDAV 动词子集）".to_string(),
        ))
    }

    async fn put(
        &self,
        _url: &str,
        _bytes: &[u8],
        _if_match: Option<&str>,
    ) -> Result<(), SyncError> {
        Err(SyncError::Unreachable("尚未接入 HTTP 传输层".to_string()))
    }

    async fn get(&self, _url: &str) -> Result<Vec<u8>, SyncError> {
        Err(SyncError::Unreachable("尚未接入 HTTP 传输层".to_string()))
    }

    async fn mkcol(&self, _url: &str) -> Result<(), SyncError> {
        Err(SyncError::Unreachable("尚未接入 HTTP 传输层".to_string()))
    }

    async fn delete(&self, _url: &str) -> Result<(), SyncError> {
        Err(SyncError::Unreachable("尚未接入 HTTP 传输层".to_string()))
    }
}

pub struct WebDavSyncProvider {
    transport: Arc<dyn WebDavTransport>,
    config: Mutex<SyncProviderConfig>,
    state: Mutex<SyncState>,
    /// Ordered log of state transitions; the settings UI shows the tail, tests assert the order.
    transitions: Mutex<Vec<SyncState>>,
    records: Mutex<Vec<SyncRecord>>,
    finished_at_ms: AtomicU64,
}

impl WebDavSyncProvider {
    pub fn new(transport: Arc<dyn WebDavTransport>) -> Arc<Self> {
        Arc::new(Self {
            transport,
            config: Mutex::new(SyncProviderConfig::default()),
            state: Mutex::new(SyncState::Idle),
            transitions: Mutex::new(Vec::new()),
            records: Mutex::new(Vec::new()),
            finished_at_ms: AtomicU64::new(0),
        })
    }

    pub fn transitions(&self) -> Vec<SyncState> {
        self.transitions.lock().clone()
    }

    pub fn records(&self) -> Vec<SyncRecord> {
        self.records.lock().clone()
    }

    fn advance(&self, next: SyncState) {
        *self.state.lock() = next;
        self.transitions.lock().push(next);
    }

    /// Last-Write-Wins is unsafe when the device clock has moved backwards by more than five
    /// minutes; in that case the caller falls back to keeping both versions.
    pub fn effective_policy(&self, now_ms: u64, remote_mtime_ms: u64) -> ConflictPolicy {
        let configured = self.config.lock().policy;
        if configured == ConflictPolicy::LastWriteWins
            && remote_mtime_ms > now_ms + CLOCK_ROLLBACK_TOLERANCE_MS
        {
            ConflictPolicy::KeepBoth
        } else {
            configured
        }
    }

    /// One pass of the state machine.
    ///
    /// Full diff/apply logic arrives with the transport; the skeleton walks every state so the
    /// lifecycle is observable and the error path returns to `Idle`.
    async fn run_pass(&self) -> Result<SyncReport, SyncError> {
        let config = self.config.lock().clone();
        if config.server_url.is_empty() {
            return Err(SyncError::NotConfigured);
        }

        self.advance(SyncState::CheckLocal);
        let local_entries: Vec<SyncRecord> = self.records.lock().clone();

        self.advance(SyncState::CheckRemote);
        let remote = self
            .transport
            .propfind(&config.server_url, 1)
            .await
            .inspect_err(|_| {
                // Any transport failure aborts the pass; the machine never sticks mid-flight.
                self.advance(SyncState::Idle);
            })?;

        if !remote.is_empty() {
            self.advance(SyncState::DiffCalc);
        }

        let mut report = SyncReport::default();
        for _record in &local_entries {
            // Replaced by the real diff in M4.
        }

        self.advance(SyncState::Apply);
        self.advance(SyncState::Verify);
        self.advance(SyncState::Cleanup);

        let now = now_ms();
        report.finished_at_ms = now;
        self.finished_at_ms.store(now, Ordering::Release);
        self.advance(SyncState::Idle);
        Ok(report)
    }
}

#[async_trait]
impl SyncProvider for WebDavSyncProvider {
    fn state(&self) -> SyncState {
        *self.state.lock()
    }

    fn config(&self) -> SyncProviderConfig {
        self.config.lock().clone()
    }

    async fn save_config(&self, config: SyncProviderConfig) -> Result<(), SyncError> {
        // The password itself never lands here: `credential_ref` names an entry in the platform
        // secure store (Android Keystore / iOS Keychain).
        let mut current = self.config.lock();
        *current = config;
        Ok(())
    }

    async fn sync_now(&self) -> Result<SyncReport, SyncError> {
        self.run_pass().await
    }
}

/// The plugin wrapper: provides the sync service and schedules periodic work.
pub struct SyncWebDavPlugin {
    provider: Arc<WebDavSyncProvider>,
}

impl SyncWebDavPlugin {
    pub fn new(provider: Arc<WebDavSyncProvider>) -> Arc<Self> {
        Arc::new(Self { provider })
    }
}

#[async_trait]
impl Plugin for SyncWebDavPlugin {
    fn name(&self) -> &str {
        "sync-webdav"
    }

    async fn apply(&self, ctx: &Arc<Context>) -> Result<(), PluginError> {
        ctx.provide_shared(
            SYNC,
            SyncHandle(self.provider.clone() as Arc<dyn SyncProvider>),
        )
        .await;

        // Background scheduling is requested through the host channel only (§8.5). The host maps
        // the task name back to this provider's `sync_now` and cancels it when we unload — the
        // namespace must come from the shared helper so the two sides cannot disagree.
        if let Some(scheduler) = ctx.get::<inkweave_api::SchedulerHandle>(inkweave_api::SCHEDULER) {
            let constraints = if self.provider.config().wifi_only {
                inkweave_api::WorkConstraints::UNMETERED
            } else {
                inkweave_api::WorkConstraints::METERED_OK
            };
            let owner = inkweave_api::runtime::task_owner(PLUGIN_ID);
            let _ = scheduler
                .0
                .enqueue_periodic(
                    &owner,
                    TASK_SYNC.to_string(),
                    SYNC_INTERVAL_MINUTES,
                    constraints,
                )
                .await;
        }
        Ok(())
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// In-memory transport so the state machine is testable without a server.
    struct MemoryTransport {
        entries: Vec<RemoteEntry>,
        fail: bool,
    }

    #[async_trait]
    impl WebDavTransport for MemoryTransport {
        async fn propfind(&self, _url: &str, _depth: u8) -> Result<Vec<RemoteEntry>, SyncError> {
            if self.fail {
                return Err(SyncError::Unreachable("offline".to_string()));
            }
            Ok(self.entries.clone())
        }

        async fn put(
            &self,
            _url: &str,
            _bytes: &[u8],
            _if_match: Option<&str>,
        ) -> Result<(), SyncError> {
            Ok(())
        }

        async fn get(&self, _url: &str) -> Result<Vec<u8>, SyncError> {
            Ok(Vec::new())
        }

        async fn mkcol(&self, _url: &str) -> Result<(), SyncError> {
            Ok(())
        }

        async fn delete(&self, _url: &str) -> Result<(), SyncError> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn unconfigured_provider_refuses_to_sync() {
        let provider = WebDavSyncProvider::new(Arc::new(UnavailableTransport));
        let error = provider.sync_now().await.expect_err("must refuse");
        assert!(matches!(error, SyncError::NotConfigured));
        assert_eq!(provider.state(), SyncState::Idle);
    }

    #[tokio::test]
    async fn configured_provider_walks_the_full_state_machine() {
        let provider = WebDavSyncProvider::new(Arc::new(MemoryTransport {
            entries: vec![RemoteEntry {
                href: "/inkweave/".to_string(),
                etag: Some("abc".to_string()),
                mtime_ms: 1,
                is_collection: true,
            }],
            fail: false,
        }));
        provider
            .save_config(SyncProviderConfig {
                server_url: "https://dav.example.com/remote.php/dav".to_string(),
                username: "alice".to_string(),
                credential_ref: "keystore:webdav".to_string(),
                ..SyncProviderConfig::default()
            })
            .await
            .expect("config saves");

        let report = provider.sync_now().await.expect("pass succeeds");
        assert_eq!(report.uploaded, 0);

        assert_eq!(
            provider.transitions(),
            vec![
                SyncState::CheckLocal,
                SyncState::CheckRemote,
                SyncState::DiffCalc,
                SyncState::Apply,
                SyncState::Verify,
                SyncState::Cleanup,
                SyncState::Idle,
            ]
        );
        assert_eq!(provider.state(), SyncState::Idle);
    }

    #[tokio::test]
    async fn transport_failure_returns_the_machine_to_idle() {
        let provider = WebDavSyncProvider::new(Arc::new(MemoryTransport {
            entries: Vec::new(),
            fail: true,
        }));
        provider
            .save_config(SyncProviderConfig {
                server_url: "https://dav.example.com".to_string(),
                ..SyncProviderConfig::default()
            })
            .await
            .expect("config saves");

        let error = provider.sync_now().await.expect_err("transport fails");
        assert!(matches!(error, SyncError::Unreachable(_)));
        assert_eq!(
            provider.state(),
            SyncState::Idle,
            "a failed pass must not leave the machine mid-flight"
        );
    }

    #[test]
    fn clock_rollback_disables_last_write_wins() {
        let provider = WebDavSyncProvider::new(Arc::new(UnavailableTransport));
        let now = 1_000_000_000_000;
        assert_eq!(
            provider.effective_policy(now, now),
            ConflictPolicy::LastWriteWins
        );
        assert_eq!(
            provider.effective_policy(now, now + CLOCK_ROLLBACK_TOLERANCE_MS + 1),
            ConflictPolicy::KeepBoth,
            "a remote timestamp far in the future means our clock rolled back"
        );
    }
}
