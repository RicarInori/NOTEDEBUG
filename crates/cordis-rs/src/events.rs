use arc_swap::ArcSwap;
use futures::future::BoxFuture;
use std::any::Any;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// Event names follow the `namespace/action` convention, e.g. `note/before-save`.
pub type EventName = &'static str;

/// Type-erased event payload. Wrap with [`payload`], read with [`payload_as`].
pub type Payload = Arc<dyn Any + Send + Sync>;

/// Continuation passed to a [`WaterfallListener`]; calling it yields the downstream result.
pub type Next = Arc<dyn Fn() -> BoxFuture<'static, Payload> + Send + Sync>;

/// Identifier of a registered listener, used to remove it when the owning fiber unloads.
pub type ListenerId = u64;

fn next_listener_id() -> ListenerId {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

pub fn payload<T: Any + Send + Sync>(value: T) -> Payload {
    Arc::new(value)
}

pub fn payload_as<T: Any + Send + Sync>(value: &Payload) -> Option<&T> {
    value.as_ref().downcast_ref::<T>()
}

/// Broadcast listener: cannot veto and does not return a value.
pub trait EmitListener: Send + Sync {
    fn call(&self, payload: &Payload);
}

/// Short-circuit listener: returning `Some` stops the chain (used for `note/can-delete` guards).
pub trait BailListener: Send + Sync {
    fn call(&self, payload: &Payload) -> Option<Payload>;
}

/// Around-middleware listener: it may call `next` and post-process, or return early to veto.
pub trait WaterfallListener: Send + Sync {
    fn call(&self, payload: Payload, next: Next) -> BoxFuture<'static, Payload>;
}

type EmitMap = HashMap<EventName, Vec<(ListenerId, Arc<dyn EmitListener>)>>;
type BailMap = HashMap<EventName, Vec<(ListenerId, Arc<dyn BailListener>)>>;
type WaterfallMap = HashMap<EventName, Vec<(ListenerId, Arc<dyn WaterfallListener>)>>;

/// Typed event bus. Dispatch tables are `ArcSwap` so `emit` never blocks registration
/// (ADR-004: listeners are snapshotted, then called without holding any lock).
#[derive(Default)]
pub struct EventBus {
    emits: ArcSwap<EmitMap>,
    bails: ArcSwap<BailMap>,
    waterfalls: ArcSwap<WaterfallMap>,
}

impl EventBus {
    pub fn on_emit(&self, name: EventName, listener: Arc<dyn EmitListener>) -> ListenerId {
        let id = next_listener_id();
        let current = self.emits.load();
        let mut next = (**current).clone();
        next.entry(name).or_default().push((id, listener));
        self.emits.store(Arc::new(next));
        id
    }

    pub fn off_emit(&self, name: EventName, id: ListenerId) -> bool {
        let current = self.emits.load();
        let mut next = (**current).clone();
        let removed = match next.get_mut(name) {
            Some(list) => {
                let before = list.len();
                list.retain(|(listener_id, _)| *listener_id != id);
                list.len() != before
            }
            None => false,
        };
        if removed {
            self.emits.store(Arc::new(next));
        }
        removed
    }

    pub fn on_bail(&self, name: EventName, listener: Arc<dyn BailListener>) -> ListenerId {
        let id = next_listener_id();
        let current = self.bails.load();
        let mut next = (**current).clone();
        next.entry(name).or_default().push((id, listener));
        self.bails.store(Arc::new(next));
        id
    }

    pub fn off_bail(&self, name: EventName, id: ListenerId) -> bool {
        let current = self.bails.load();
        let mut next = (**current).clone();
        let removed = match next.get_mut(name) {
            Some(list) => {
                let before = list.len();
                list.retain(|(listener_id, _)| *listener_id != id);
                list.len() != before
            }
            None => false,
        };
        if removed {
            self.bails.store(Arc::new(next));
        }
        removed
    }

    pub fn on_waterfall(
        &self,
        name: EventName,
        listener: Arc<dyn WaterfallListener>,
    ) -> ListenerId {
        let id = next_listener_id();
        let current = self.waterfalls.load();
        let mut next = (**current).clone();
        next.entry(name).or_default().push((id, listener));
        self.waterfalls.store(Arc::new(next));
        id
    }

    pub fn off_waterfall(&self, name: EventName, id: ListenerId) -> bool {
        let current = self.waterfalls.load();
        let mut next = (**current).clone();
        let removed = match next.get_mut(name) {
            Some(list) => {
                let before = list.len();
                list.retain(|(listener_id, _)| *listener_id != id);
                list.len() != before
            }
            None => false,
        };
        if removed {
            self.waterfalls.store(Arc::new(next));
        }
        removed
    }

    /// Broadcast. Listeners run in registration order; no listener can stop the others.
    pub fn emit(&self, name: EventName, event: Payload) {
        let listeners: Vec<Arc<dyn EmitListener>> = self
            .emits
            .load()
            .get(name)
            .map(|list| list.iter().map(|(_, l)| l.clone()).collect())
            .unwrap_or_default();
        for listener in listeners {
            listener.call(&event);
        }
    }

    /// First listener to return `Some` wins and the rest are skipped.
    pub fn bail(&self, name: EventName, event: Payload) -> Option<Payload> {
        let listeners: Vec<Arc<dyn BailListener>> = self
            .bails
            .load()
            .get(name)
            .map(|list| list.iter().map(|(_, l)| l.clone()).collect())
            .unwrap_or_default();
        for listener in listeners {
            if let Some(decision) = listener.call(&event) {
                return Some(decision);
            }
        }
        None
    }

    /// Onion-style middleware chain. `default` is what the innermost continuation yields —
    /// pass the input again for the common "pipeline transforms the value" case.
    pub async fn waterfall(&self, name: EventName, input: Payload, default: Payload) -> Payload {
        let listeners: Vec<Arc<dyn WaterfallListener>> = self
            .waterfalls
            .load()
            .get(name)
            .map(|list| list.iter().map(|(_, l)| l.clone()).collect())
            .unwrap_or_default();
        if listeners.is_empty() {
            return default;
        }
        build_chain(listeners, input, default)().await
    }
}

/// Compose the remaining listeners into a [`Next`] continuation.
fn build_chain(rest: Vec<Arc<dyn WaterfallListener>>, input: Payload, default: Payload) -> Next {
    let mut iter = rest.into_iter();
    match iter.next() {
        None => Arc::new(move || {
            let terminal = default.clone();
            Box::pin(async move { terminal })
        }),
        Some(head) => {
            let tail: Vec<Arc<dyn WaterfallListener>> = iter.collect();
            Arc::new(move || {
                let next = build_chain(tail.clone(), input.clone(), default.clone());
                let listener = head.clone();
                let event = input.clone();
                Box::pin(async move { listener.call(event, next).await })
            })
        }
    }
}

/// Closure shape of a typed emit listener.
pub type EmitFn<T> = Box<dyn Fn(&T) + Send + Sync>;

/// Closure shape of a typed bail listener.
pub type BailFn<T> = Box<dyn Fn(&T) -> Option<Payload> + Send + Sync>;

/// Closure shape of a typed waterfall listener.
pub type WaterfallFn<T> = Box<dyn Fn(T, Next) -> BoxFuture<'static, Payload> + Send + Sync>;

/// Adapts a typed closure into an [`EmitListener`]; payloads of another type are ignored.
pub struct TypedEmit<T: Any + Send + Sync>(pub EmitFn<T>);

impl<T: Any + Send + Sync> EmitListener for TypedEmit<T> {
    fn call(&self, event: &Payload) {
        if let Some(value) = payload_as::<T>(event) {
            (self.0)(value);
        }
    }
}

/// Adapts a typed closure into a [`BailListener`].
pub struct TypedBail<T: Any + Send + Sync>(pub BailFn<T>);

impl<T: Any + Send + Sync> BailListener for TypedBail<T> {
    fn call(&self, event: &Payload) -> Option<Payload> {
        payload_as::<T>(event).and_then(|value| (self.0)(value))
    }
}

/// Adapts a typed closure into a [`WaterfallListener`].
///
/// The closure receives the current value and the continuation, and returns the new payload —
/// exactly the Cordis contract, where a listener that only observes must still call `next()`.
pub struct TypedWaterfall<T: Any + Send + Sync + Clone>(pub WaterfallFn<T>);

impl<T: Any + Send + Sync + Clone> WaterfallListener for TypedWaterfall<T> {
    fn call(&self, event: Payload, next: Next) -> BoxFuture<'static, Payload> {
        match payload_as::<T>(&event) {
            Some(value) => (self.0)(value.clone(), next),
            None => Box::pin(async move { event }),
        }
    }
}
