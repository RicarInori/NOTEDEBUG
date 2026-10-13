use inkweave_api::runtime::{MemPressure, MemoryGovernor, DEFAULT_EXTENSION_BUDGET_MB};
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU8, Ordering};

/// Host implementation of [`MemoryGovernor`] (§8.6).
///
/// Budgets are declared per owner; the platform layer feeds real numbers in through
/// [`HostMemoryGovernor::set_used_mb`] and [`HostMemoryGovernor::set_pressure`], which the Android
/// `ComponentCallbacks2.onTrimMemory` / iOS memory-warning adapters will call.
pub struct HostMemoryGovernor {
    budgets: Mutex<HashMap<String, u32>>,
    used: Mutex<HashMap<String, u32>>,
    pressure: AtomicU8,
}

impl Default for HostMemoryGovernor {
    fn default() -> Self {
        Self::new()
    }
}

impl HostMemoryGovernor {
    pub fn new() -> Self {
        Self {
            budgets: Mutex::new(HashMap::new()),
            used: Mutex::new(HashMap::new()),
            pressure: AtomicU8::new(pressure_to_u8(MemPressure::None)),
        }
    }

    /// Called by the platform adapter when it samples a plugin's footprint.
    pub fn set_used_mb(&self, owner: &str, used_mb: u32) {
        self.used.lock().insert(owner.to_string(), used_mb);
    }

    /// Called by the platform adapter on a system memory notification.
    ///
    /// `Critical` is what triggers the host's idle-plugin unloading policy; broadcasting the level
    /// to each plugin is the kernel's job via `onTrimMemory`.
    pub fn set_pressure(&self, pressure: MemPressure) {
        self.pressure
            .store(pressure_to_u8(pressure), Ordering::Release);
    }

    /// Owners whose measured usage exceeds their budget. The control panel flags these.
    pub fn over_budget(&self) -> Vec<String> {
        let budgets = self.budgets.lock();
        let used = self.used.lock();
        used.iter()
            .filter(|(owner, usage)| {
                budgets
                    .get(*owner)
                    .map(|budget| **usage > *budget)
                    .unwrap_or(false)
            })
            .map(|(owner, _)| owner.clone())
            .collect()
    }
}

impl MemoryGovernor for HostMemoryGovernor {
    fn budget_mb(&self, owner: &str) -> u32 {
        self.budgets
            .lock()
            .get(owner)
            .copied()
            .unwrap_or(DEFAULT_EXTENSION_BUDGET_MB)
    }

    fn used_mb(&self, owner: &str) -> u32 {
        self.used.lock().get(owner).copied().unwrap_or(0)
    }

    fn set_budget_mb(&self, owner: &str, budget_mb: u32) {
        self.budgets.lock().insert(owner.to_string(), budget_mb);
    }

    fn pressure(&self) -> MemPressure {
        u8_to_pressure(self.pressure.load(Ordering::Acquire))
    }

    fn process_used_mb(&self) -> u32 {
        // Fed by the platform adapter; zero until that wiring exists.
        self.used.lock().values().copied().sum()
    }
}

fn pressure_to_u8(pressure: MemPressure) -> u8 {
    match pressure {
        MemPressure::None => 0,
        MemPressure::Background => 1,
        MemPressure::Low => 2,
        MemPressure::Critical => 3,
    }
}

fn u8_to_pressure(value: u8) -> MemPressure {
    match value {
        1 => MemPressure::Background,
        2 => MemPressure::Low,
        3 => MemPressure::Critical,
        _ => MemPressure::None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn budgets_default_for_unknown_extensions() {
        let governor = HostMemoryGovernor::new();
        assert_eq!(
            governor.budget_mb("com.acme.plugin"),
            DEFAULT_EXTENSION_BUDGET_MB
        );
    }

    #[test]
    fn over_budget_owners_are_reported() {
        let governor = HostMemoryGovernor::new();
        governor.set_budget_mb("ink-engine", 96);
        governor.set_used_mb("ink-engine", 120);
        governor.set_budget_mb("storage", 32);
        governor.set_used_mb("storage", 10);

        assert_eq!(governor.over_budget(), vec!["ink-engine".to_string()]);
    }

    #[test]
    fn pressure_round_trips_through_the_atomic() {
        let governor = HostMemoryGovernor::new();
        assert_eq!(governor.pressure(), MemPressure::None);
        governor.set_pressure(MemPressure::Critical);
        assert_eq!(governor.pressure(), MemPressure::Critical);
        assert!(governor.pressure() > MemPressure::Low);
    }
}
