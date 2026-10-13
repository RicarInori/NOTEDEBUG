use crate::context::Context;
use crate::plugin::Plugin;
use crate::service::{ServiceKey, ServiceObj};
use futures::future::BoxFuture;
use parking_lot::Mutex;
use std::sync::atomic::{AtomicU64, AtomicU8, Ordering};
use std::sync::Arc;

/// A single inverse operation produced by a reversible effect.
///
/// Returns a boxed future so disposers may be asynchronous; wrap a synchronous closure with
/// [`sync_disposer`].
pub type Disposer = Box<dyn FnOnce() -> BoxFuture<'static, ()> + Send + 'static>;

/// Wrap a synchronous closure as a [`Disposer`].
pub fn sync_disposer<F>(f: F) -> Disposer
where
    F: FnOnce() + Send + 'static,
{
    Box::new(move || Box::pin(async move { f() }))
}

/// The six-state lifecycle of one plugin instance (DEV guide §2.1).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FiberState {
    /// Mounted, waiting for its injected services to become available.
    Pending,
    /// `apply()` is running.
    Loading,
    /// Running normally.
    Active,
    /// `apply()` failed; already-registered effects were recovered.
    Failed,
    /// Disposers are running.
    Unloading,
    /// Fully unloaded; never restarts.
    Disposed,
}

impl FiberState {
    pub const fn is_live(self) -> bool {
        matches!(
            self,
            FiberState::Pending | FiberState::Loading | FiberState::Active
        )
    }

    const fn to_u8(self) -> u8 {
        match self {
            FiberState::Pending => 0,
            FiberState::Loading => 1,
            FiberState::Active => 2,
            FiberState::Failed => 3,
            FiberState::Unloading => 4,
            FiberState::Disposed => 5,
        }
    }

    const fn from_u8(value: u8) -> Self {
        match value {
            0 => FiberState::Pending,
            1 => FiberState::Loading,
            2 => FiberState::Active,
            3 => FiberState::Failed,
            4 => FiberState::Unloading,
            _ => FiberState::Disposed,
        }
    }
}

/// Monotonic fiber identity, used for the UI bridge's `(plugin, generation)` de-duplication.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct FiberId(pub u64);

impl FiberId {
    pub fn next() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        FiberId(NEXT.fetch_add(1, Ordering::Relaxed))
    }
}

/// LIFO stack of disposers: the "recovery accumulator" of Cordis (DEV guide §2.6).
#[derive(Default)]
pub struct EffectStack {
    items: Vec<Disposer>,
}

impl EffectStack {
    pub fn push(&mut self, disposer: Disposer) {
        self.items.push(disposer);
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Run every disposer in reverse registration order.
    ///
    /// The stack is drained first so a disposer that re-registers cannot extend the drain.
    pub async fn recover(&mut self) {
        let drained = std::mem::take(&mut self.items);
        for disposer in drained.into_iter().rev() {
            disposer().await;
        }
    }
}

/// Lifecycle state machine for one plugin instance, holding its effects and children.
pub struct Fiber {
    id: FiberId,
    name: String,
    inject: Vec<ServiceKey>,
    state: AtomicU8,
    effects: Mutex<EffectStack>,
    /// `Arc<Context>` ↔ `Arc<Fiber>` is a deliberate two-way hold (ADR-004). `dispose()` takes
    /// this slot to break the cycle so the graph can actually be freed.
    ctx: Mutex<Option<Arc<Context>>>,
    plugin: Mutex<Option<Arc<dyn Plugin>>>,
    /// Injection snapshot taken when entering `Active` (ADR-004: hot path does not walk parents).
    resolved: Mutex<Vec<(ServiceKey, ServiceObj)>>,
    /// Strong handles: mounting a child inside `apply()` must keep it alive until this fiber
    /// unloads. `recover()` drains the list, so a disposed child is released immediately.
    children: Mutex<Vec<Arc<Fiber>>>,
}

impl Fiber {
    pub fn new(name: impl Into<String>, inject: Vec<ServiceKey>) -> Arc<Self> {
        Arc::new(Self {
            id: FiberId::next(),
            name: name.into(),
            inject,
            state: AtomicU8::new(FiberState::Pending.to_u8()),
            effects: Mutex::new(EffectStack::default()),
            ctx: Mutex::new(None),
            plugin: Mutex::new(None),
            resolved: Mutex::new(Vec::new()),
            children: Mutex::new(Vec::new()),
        })
    }

    pub fn id(&self) -> FiberId {
        self.id
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn inject(&self) -> &[ServiceKey] {
        &self.inject
    }

    pub fn state(&self) -> FiberState {
        FiberState::from_u8(self.state.load(Ordering::Acquire))
    }

    /// Only the kernel may transition states; exposed for the ledger and tests.
    ///
    /// Every transition is forwarded to the ledger's observers, so a host can mirror plugin state
    /// without polling — including transitions with no API call behind them, such as the
    /// dependency-driven `Pending → Active`.
    pub fn set_state(&self, next: FiberState) {
        self.transition(next, self.ctx());
    }

    fn transition(&self, next: FiberState, ctx: Option<Arc<Context>>) {
        self.state.store(next.to_u8(), Ordering::Release);
        if let Some(ctx) = ctx {
            ctx.notify_fiber_state(self.id, next);
        }
    }

    pub fn attach_ctx(&self, ctx: Arc<Context>) {
        *self.ctx.lock() = Some(ctx);
    }

    pub fn ctx(&self) -> Option<Arc<Context>> {
        self.ctx.lock().clone()
    }

    pub fn set_plugin(&self, plugin: Arc<dyn Plugin>) {
        *self.plugin.lock() = Some(plugin);
    }

    pub fn plugin(&self) -> Option<Arc<dyn Plugin>> {
        self.plugin.lock().clone()
    }

    pub fn push_effect(&self, disposer: Disposer) {
        self.effects.lock().push(disposer);
    }

    /// Number of pending disposers. The disposer audit asserts this reaches 0 after unload.
    pub fn effect_count(&self) -> usize {
        self.effects.lock().len()
    }

    pub fn add_child(&self, child: &Arc<Fiber>) {
        self.children.lock().push(child.clone());
    }

    pub fn children(&self) -> Vec<Arc<Fiber>> {
        let guard = self.children.lock();
        guard.clone()
    }

    pub fn set_resolved(&self, resolved: Vec<(ServiceKey, ServiceObj)>) {
        *self.resolved.lock() = resolved;
    }

    /// Read a service from this fiber's activation snapshot.
    pub fn resolved(&self, key: ServiceKey) -> Option<ServiceObj> {
        self.resolved
            .lock()
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, v)| v.clone())
    }

    /// Cascade: dispose children (youngest first) then unwind own effects LIFO.
    pub async fn recover(&self) {
        let children: Vec<Arc<Fiber>> = {
            let mut guard = self.children.lock();
            guard.drain(..).collect()
        };
        for child in children.into_iter().rev() {
            // Boxed because `recover` and `dispose` are mutually recursive; without the
            // indirection the compiler cannot size the futures.
            Box::pin(child.dispose()).await;
        }
        let mut stack = {
            let mut guard = self.effects.lock();
            std::mem::take(&mut *guard)
        };
        stack.recover().await;
    }

    /// Full teardown. Idempotent. Breaks the Context↔Fiber cycle at the end.
    pub async fn dispose(&self) {
        if self.state() == FiberState::Disposed {
            return;
        }
        // Read the context up front: it is cleared below to break the cycle, and the final
        // `Disposed` transition must still be observable after that.
        let ctx = self.ctx();
        self.transition(FiberState::Unloading, ctx.clone());
        self.recover().await;
        self.plugin.lock().take();
        self.resolved.lock().clear();
        self.ctx.lock().take();
        self.transition(FiberState::Disposed, ctx);
    }
}
