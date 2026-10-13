use crate::events::{
    payload, EventBus, EventName, ListenerId, Next, Payload, TypedBail, TypedEmit, TypedWaterfall,
};
use crate::fiber::{sync_disposer, Disposer, Fiber, FiberId, FiberState};
use crate::ledger::{Ledger, StateObserver};
use crate::plugin::{Plugin, PluginError, Result};
use crate::service::{downcast, ServiceKey, ServiceObj, ServiceTable};
use arc_swap::ArcSwap;
use futures::future::BoxFuture;
use std::any::Any;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// Identity of a scope, for diagnostics and the UI bridge.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct ContextId(pub u64);

impl ContextId {
    pub fn next() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        ContextId(NEXT.fetch_add(1, Ordering::Relaxed))
    }
}

/// Hierarchical service container, bound 1:1 to a [`Fiber`].
///
/// Service resolution walks self → parent → … → root (the "Proxy" analogue of Cordis). Reads are
/// lock-free; writes copy-on-write the whole [`ServiceTable`] (ADR-004).
pub struct Context {
    id: ContextId,
    parent: Option<Arc<Context>>,
    services: ArcSwap<ServiceTable>,
    events: Arc<EventBus>,
    fiber: Arc<Fiber>,
    ledger: Arc<Ledger>,
}

impl Context {
    /// Create the root scope. Callers mount plugins onto it.
    pub fn root() -> Arc<Context> {
        let ledger = Ledger::new();
        let fiber = Fiber::new("root", Vec::new());
        let ctx = Arc::new(Context {
            id: ContextId::next(),
            parent: None,
            services: ArcSwap::from_pointee(ServiceTable::default()),
            events: Arc::new(EventBus::default()),
            fiber: fiber.clone(),
            ledger,
        });
        fiber.attach_ctx(ctx.clone());
        ctx
    }

    /// Create an isolated child scope. Used by [`Context::plugin`]; also useful for per-window
    /// scopes (DEV guide §7.1 借鉴点 2).
    ///
    /// Takes `&Arc<Self>` because the child must hold a strong handle to its parent, and every
    /// `Context` is used behind an `Arc`.
    pub fn child(
        self: &Arc<Self>,
        name: impl Into<String>,
        inject: Vec<ServiceKey>,
    ) -> Arc<Context> {
        let fiber = Fiber::new(name, inject);
        let ctx = Arc::new(Context {
            id: ContextId::next(),
            parent: Some(Arc::clone(self)),
            services: ArcSwap::from_pointee(ServiceTable::default()),
            events: self.events.clone(),
            fiber: fiber.clone(),
            ledger: self.ledger.clone(),
        });
        fiber.attach_ctx(ctx.clone());
        self.ledger.track(&fiber);
        self.fiber.add_child(&fiber);
        ctx
    }

    pub fn id(&self) -> ContextId {
        self.id
    }

    pub fn fiber(&self) -> &Arc<Fiber> {
        &self.fiber
    }

    pub fn parent(&self) -> Option<&Arc<Context>> {
        self.parent.as_ref()
    }

    pub fn events(&self) -> &Arc<EventBus> {
        &self.events
    }

    // --- services -----------------------------------------------------------

    /// Register a service in this scope.
    ///
    /// Async because satisfying a dependency may let `Pending` fibers activate, and replacing an
    /// existing provider restarts its dependents (DEV guide §2.2 invariant 3).
    pub async fn provide<T: Send + Sync + 'static>(&self, key: ServiceKey, service: T) {
        let replaced = {
            let current = self.services.load();
            let mut next = (**current).clone();
            let previous = next.insert(key, Arc::new(service));
            self.services.store(Arc::new(next));
            previous
        };
        if replaced.is_some() {
            self.ledger.restart_dependents(key).await;
        }
        self.ledger.activate_pending().await;
    }

    /// Remove a service. Dependents that injected it fall back to `Pending`.
    pub async fn remove_service(&self, key: ServiceKey) {
        let removed = {
            let current = self.services.load();
            let mut next = (**current).clone();
            let previous = next.remove(key);
            self.services.store(Arc::new(next));
            previous
        };
        if removed.is_some() {
            self.ledger.restart_dependents(key).await;
        }
        self.ledger.activate_pending().await;
    }

    /// Walk up the parent chain and return the topmost scope as an owned handle.
    ///
    /// `None` means this scope *is* the root (no owning `Arc` is reachable from `&self`).
    fn arc_root(&self) -> Option<Arc<Context>> {
        let mut top = self.parent.clone()?;
        while let Some(next) = top.parent.clone() {
            top = next;
        }
        Some(top)
    }

    /// Provide a service visible to the **whole tree** (registered on the root scope).
    ///
    /// This is the shape a plugin uses to expose a capability to its siblings — `storage`, `ink`
    /// and `plugin-manager`'s `PluginRegistry` all do this. Registration is automatically revoked
    /// when the providing fiber unloads, and dependents are re-evaluated.
    ///
    /// Called on the root scope itself there is no owning fiber, so the service simply lives for
    /// the process lifetime (use [`Context::remove_service`] to revoke it manually).
    pub async fn provide_shared<T: Send + Sync + 'static>(&self, key: ServiceKey, service: T) {
        match self.arc_root() {
            None => self.provide(key, service).await,
            Some(root) => {
                root.provide(key, service).await;
                let root_on_unload = root.clone();
                self.fiber.push_effect(Box::new(move || {
                    let root = root_on_unload.clone();
                    Box::pin(async move { root.remove_service(key).await })
                }));
            }
        }
    }

    /// Resolve along the parent chain, untyped.
    pub fn resolve(&self, key: ServiceKey) -> Option<ServiceObj> {
        if let Some(service) = self.services.load().get(key) {
            return Some(service);
        }
        self.parent.as_ref().and_then(|parent| parent.resolve(key))
    }

    /// Resolve along the parent chain, type-checked.
    pub fn get<T: Any + Send + Sync>(&self, key: ServiceKey) -> Option<Arc<T>> {
        downcast::<T>(&self.resolve(key)?)
    }

    /// Resolve type-checked, panicking with a clear message when absent.
    ///
    /// Only for plugins that already declared the key in [`Plugin::inject`], where absence is a
    /// programmer error rather than a runtime condition.
    pub fn expect<T: Any + Send + Sync>(&self, key: ServiceKey) -> Arc<T> {
        self.get::<T>(key)
            .unwrap_or_else(|| panic!("service `{key}` is not available in this scope"))
    }

    pub fn has(&self, key: ServiceKey) -> bool {
        self.resolve(key).is_some()
    }

    /// Keys provided by *this* scope only (not inherited).
    pub fn provided_keys(&self) -> Vec<ServiceKey> {
        self.services.load().keys().collect()
    }

    // --- effects ------------------------------------------------------------

    /// Register a reversible effect owned by this scope's fiber (DEV guide §2.6).
    ///
    /// Every registration in this crate goes through here, which is why plugin authors never
    /// write cleanup code: unload unwinds the stack LIFO.
    pub fn effect<F>(&self, cleanup: F)
    where
        F: FnOnce() + Send + 'static,
    {
        self.fiber.push_effect(sync_disposer(cleanup));
    }

    /// Register an asynchronous disposer.
    pub fn effect_async(&self, disposer: Disposer) {
        self.fiber.push_effect(disposer);
    }

    // --- events -------------------------------------------------------------

    pub fn on_emit<T, F>(&self, name: EventName, listener: F) -> ListenerId
    where
        T: Any + Send + Sync + 'static,
        F: Fn(&T) + Send + Sync + 'static,
    {
        let id = self
            .events
            .on_emit(name, Arc::new(TypedEmit::<T>(Box::new(listener))));
        self.off_on_unload(move |events| events.off_emit(name, id));
        id
    }

    pub fn on_bail<T, F>(&self, name: EventName, listener: F) -> ListenerId
    where
        T: Any + Send + Sync + 'static,
        F: Fn(&T) -> Option<Payload> + Send + Sync + 'static,
    {
        let id = self
            .events
            .on_bail(name, Arc::new(TypedBail::<T>(Box::new(listener))));
        self.off_on_unload(move |events| events.off_bail(name, id));
        id
    }

    pub fn on_waterfall<T, F>(&self, name: EventName, listener: F) -> ListenerId
    where
        T: Any + Send + Sync + Clone + 'static,
        F: Fn(T, Next) -> BoxFuture<'static, Payload> + Send + Sync + 'static,
    {
        let id = self
            .events
            .on_waterfall(name, Arc::new(TypedWaterfall::<T>(Box::new(listener))));
        self.off_on_unload(move |events| events.off_waterfall(name, id));
        id
    }

    pub fn emit<T: Any + Send + Sync>(&self, name: EventName, value: T) {
        self.events.emit(name, payload(value));
    }

    pub fn bail<T: Any + Send + Sync>(&self, name: EventName, value: T) -> Option<Payload> {
        self.events.bail(name, payload(value))
    }

    /// Run the onion pipeline for an event. Returns the final payload.
    pub async fn waterfall<T: Any + Send + Sync + Clone>(
        &self,
        name: EventName,
        value: T,
    ) -> Payload {
        let input = payload(value);
        self.events.waterfall(name, input.clone(), input).await
    }

    fn off_on_unload(&self, remove: impl Fn(&Arc<EventBus>) -> bool + Send + 'static) {
        let events = self.events.clone();
        self.fiber.push_effect(sync_disposer(move || {
            remove(&events);
        }));
    }

    // --- plugins ------------------------------------------------------------

    /// Mount a plugin as a child scope: **Load + Enable** in the four-verb model (ADR-007).
    ///
    /// Returns the plugin's [`Fiber`]. If dependencies are missing the fiber stays `Pending` and
    /// the caller still gets a handle — it activates later when a provider appears.
    pub async fn plugin(self: &Arc<Self>, plugin: Arc<dyn Plugin>) -> Result<Arc<Fiber>> {
        if self.fiber.state() == FiberState::Disposed {
            return Err(PluginError::Disposed);
        }
        let plugin_name = plugin.name().to_string();
        let ctx = self.child(plugin_name, plugin.inject());
        let fiber = ctx.fiber().clone();
        fiber.set_plugin(plugin);
        self.ledger.activate_pending().await;
        Ok(fiber)
    }

    /// Unload a mounted plugin: **Disable + Unload** (ADR-007).
    ///
    /// Its services vanish with its scope, so dependents are re-evaluated afterwards.
    pub async fn unload(&self, fiber: &Arc<Fiber>) {
        let orphaned = fiber
            .ctx()
            .map(|ctx| ctx.provided_keys())
            .unwrap_or_default();
        fiber.dispose().await;
        for key in orphaned {
            self.ledger.restart_dependents(key).await;
        }
        self.ledger.activate_pending().await;
    }

    /// Number of fibers currently tracked by the tree (used by the disposer audit in tests).
    pub fn tracked_fibers(&self) -> usize {
        self.ledger.tracked_count()
    }

    /// Observe every fiber state transition in this tree.
    ///
    /// This is the seam the lifecycle bridge (DEV guide §2.4) is built on: the host registers one
    /// observer and forwards transitions to the UI, so `Pending`/`Loading`/`Unloading`/`Disposed`
    /// are visible even when nothing called an API.
    pub fn observe_fiber_states(&self, observer: StateObserver) {
        self.ledger.observe(observer);
    }

    /// Called by [`Fiber::set_state`]; not part of the plugin-facing API.
    pub fn notify_fiber_state(&self, id: FiberId, state: FiberState) {
        self.ledger.notify_state(id, state);
    }
}
