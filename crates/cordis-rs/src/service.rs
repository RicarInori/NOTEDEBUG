use std::any::Any;
use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

/// Type-erased service handle stored in a [`ServiceTable`](crate::service::ServiceTable).
pub type ServiceObj = Arc<dyn Any + Send + Sync>;

/// Stable identifier for a service slot, e.g. `ServiceKey::new("ink")`.
///
/// Keys are `&'static str` so that lookups are a pointer-free string compare and keys can be
/// `const` items shared across crates.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct ServiceKey(pub &'static str);

impl ServiceKey {
    pub const fn new(name: &'static str) -> Self {
        Self(name)
    }

    pub const fn as_str(self) -> &'static str {
        self.0
    }
}

impl fmt::Debug for ServiceKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ServiceKey({})", self.0)
    }
}

impl fmt::Display for ServiceKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

/// The mutable part of a [`Context`](crate::context::Context): services registered in one scope.
///
/// Cloning is cheap-ish and is exactly what copy-on-write registration does (ADR-004).
#[derive(Clone, Default)]
pub struct ServiceTable {
    map: HashMap<ServiceKey, ServiceObj>,
}

impl ServiceTable {
    pub fn insert(&mut self, key: ServiceKey, service: ServiceObj) -> Option<ServiceObj> {
        self.map.insert(key, service)
    }

    pub fn remove(&mut self, key: ServiceKey) -> Option<ServiceObj> {
        self.map.remove(&key)
    }

    pub fn get(&self, key: ServiceKey) -> Option<ServiceObj> {
        self.map.get(&key).cloned()
    }

    pub fn contains(&self, key: ServiceKey) -> bool {
        self.map.contains_key(&key)
    }

    pub fn keys(&self) -> impl Iterator<Item = ServiceKey> + '_ {
        self.map.keys().copied()
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

/// Downcast a type-erased service handle, returning `None` on type mismatch.
pub fn downcast<T: Any + Send + Sync>(obj: &ServiceObj) -> Option<Arc<T>> {
    obj.clone().downcast::<T>().ok()
}
