use crate::fiber::{Fiber, FiberState};
use crate::service::{ServiceKey, ServiceObj};
use parking_lot::Mutex;
use std::sync::{Arc, Weak};

/// Registry of every fiber in the tree, plus the driver for reactive injection.
///
/// Keeps only `Weak` references, so the ledger never extends a plugin's lifetime: whoever mounted
/// the plugin (the `plugin-manager`) owns the strong handle.
#[derive(Default)]
pub struct Ledger {
    fibers: Mutex<Vec<Weak<Fiber>>>,
    /// Re-entrancy guard: `provide()` may be called from inside `apply()`, which itself runs
    /// inside `activate_pending()`. We must not recurse.
    activating: Mutex<bool>,
}

impl Ledger {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn track(&self, fiber: &Arc<Fiber>) {
        self.fibers.lock().push(Arc::downgrade(fiber));
    }

    /// Number of fibers still alive (Pending/Loading/Active/Failed/Unloading).
    pub fn tracked_count(&self) -> usize {
        self.live().len()
    }

    fn live(&self) -> Vec<Arc<Fiber>> {
        let mut guard = self.fibers.lock();
        guard.retain(|weak| weak.strong_count() > 0);
        guard.iter().filter_map(Weak::upgrade).collect()
    }

    /// Try to bring a single fiber to `Active` (or `Failed`).
    ///
    /// Snapshot semantics: injected services are resolved **once** here and cached on the fiber,
    /// so the running hot path never walks the parent chain (ADR-004).
    pub async fn activate(&self, fiber: Arc<Fiber>) {
        let Some(ctx) = fiber.ctx() else { return };
        let Some(plugin) = fiber.plugin() else { return };

        let mut resolved: Vec<(ServiceKey, ServiceObj)> = Vec::with_capacity(fiber.inject().len());
        for key in fiber.inject() {
            match ctx.resolve(*key) {
                Some(service) => resolved.push((*key, service)),
                None => {
                    // A dependency is missing: stay Pending, no error (DEV guide §2.4).
                    fiber.set_state(FiberState::Pending);
                    return;
                }
            }
        }
        fiber.set_resolved(resolved);
        fiber.set_state(FiberState::Loading);
        match plugin.apply(&ctx).await {
            Ok(()) => fiber.set_state(FiberState::Active),
            Err(_) => {
                // A failed fiber keeps nothing: unwind what `apply` already registered.
                fiber.recover().await;
                fiber.set_state(FiberState::Failed);
            }
        }
    }

    /// Activate every `Pending` fiber whose dependencies are now satisfiable.
    pub async fn activate_pending(&self) {
        {
            let mut guard = self.activating.lock();
            if *guard {
                return; // already inside a pass; it will re-scan below
            }
            *guard = true;
        }

        loop {
            let pending: Vec<Arc<Fiber>> = self
                .live()
                .into_iter()
                .filter(|f| f.state() == FiberState::Pending)
                .collect();
            if pending.is_empty() {
                break;
            }
            let mut progressed = false;
            for fiber in pending {
                let before = fiber.state();
                self.activate(fiber.clone()).await;
                if fiber.state() != before {
                    progressed = true;
                }
            }
            if !progressed {
                break; // nothing can progress without a new `provide`
            }
        }

        *self.activating.lock() = false;
    }

    /// Provider replacement: restart every `Active` fiber that injected `key`
    /// (DEV guide §2.2 invariant 3 — the stylus provider swap and sync plugin toggle rely on this).
    pub async fn restart_dependents(&self, key: ServiceKey) {
        let dependents: Vec<Arc<Fiber>> = self
            .live()
            .into_iter()
            .filter(|f| f.state() == FiberState::Active && f.inject().contains(&key))
            .collect();

        for fiber in dependents {
            // Recover the old episode, then re-apply against the new provider. The plugin
            // instance is retained (only `dispose` drops it).
            fiber.recover().await;
            fiber.set_state(FiberState::Pending);
            self.activate(fiber).await;
        }
    }
}
