//! `stylus-drivers` — preset plugin providing the generic stylus provider (DEV guide §6, §8.7).
//!
//! Standard platform APIs cover 60–70% of what a stylus can do, so this is the fallback every
//! device gets. Vendor adapters (`stylus-xiaomi`, `stylus-huawei`, `apple-pencil`) are compiled
//! into the host and activated at runtime by providing the same [`STYLUS`] key; the kernel's
//! provider-replacement path then restarts whichever plugins injected it (typically the ink
//! engine's prediction and shape-recognition consumers).
//!
//! Disabling this plugin removes the input listener and releases the canvas bitmap caches.

use async_trait::async_trait;
use cordis_rs::{Context, Plugin, PluginError};
use inkweave_api::stylus::{
    PressureCurve, StylusCapabilities, StylusDriver, StylusInputEvent, StylusVendor,
};
use inkweave_api::{StylusHandle, STYLUS};
use parking_lot::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Bounded record of recent samples. Stands in for the shared ring buffer that the real platform
/// layer writes into over FFI (DEV guide §3.1) — the schema is what matters here.
const SAMPLE_WINDOW: usize = 512;

pub struct GenericStylusDriver {
    connected: AtomicBool,
    pressure_curve: Mutex<PressureCurve>,
    tilt_sensitivity: Mutex<f32>,
    recent: Mutex<Vec<StylusInputEvent>>,
}

impl GenericStylusDriver {
    /// Create a driver as a shared handle (it is always used behind an `Arc`).
    #[allow(clippy::new_ret_no_self)]
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            connected: AtomicBool::new(false),
            pressure_curve: Mutex::new(PressureCurve::linear()),
            tilt_sensitivity: Mutex::new(1.0),
            recent: Mutex::new(Vec::with_capacity(SAMPLE_WINDOW)),
        })
    }

    /// Samples captured by the current session, oldest first.
    pub fn recent(&self) -> Vec<StylusInputEvent> {
        self.recent.lock().clone()
    }

    pub fn set_connected(&self, connected: bool) {
        self.connected.store(connected, Ordering::Release);
    }
}

#[async_trait]
impl StylusDriver for GenericStylusDriver {
    fn vendor(&self) -> StylusVendor {
        StylusVendor::Generic
    }

    fn capabilities(&self) -> StylusCapabilities {
        StylusCapabilities::STANDARD
    }

    fn is_connected(&self) -> bool {
        self.connected.load(Ordering::Acquire)
    }

    async fn set_sensitivity(&self, pressure: Option<PressureCurve>, tilt: Option<f32>) {
        if let Some(curve) = pressure {
            *self.pressure_curve.lock() = curve;
        }
        if let Some(value) = tilt {
            *self.tilt_sensitivity.lock() = value.clamp(0.0, 2.0);
        }
    }

    fn pressure_curve(&self) -> PressureCurve {
        self.pressure_curve.lock().clone()
    }

    /// Hot path: maps raw pressure through the user's curve and stores the sample.
    ///
    /// The real implementation must not allocate per event (object pool) — noted here because the
    /// bounded `Vec` below is a skeleton stand-in, not the shipping design.
    fn feed(&self, events: &[StylusInputEvent]) {
        let curve = self.pressure_curve.lock().clone();
        let mut recent = self.recent.lock();
        for event in events {
            let mut point = event.point;
            point.pressure = curve.apply(point.pressure);
            recent.push(StylusInputEvent {
                stroke_id: event.stroke_id,
                point,
                phase: event.phase,
            });
        }
        let overflow = recent.len().saturating_sub(SAMPLE_WINDOW);
        if overflow > 0 {
            recent.drain(0..overflow);
        }
    }
}

/// The plugin wrapper: provides the stylus service on the root scope.
pub struct StylusProviderPlugin {
    driver: Arc<GenericStylusDriver>,
}

impl StylusProviderPlugin {
    pub fn new() -> (Arc<Self>, Arc<GenericStylusDriver>) {
        let driver = GenericStylusDriver::new();
        (
            Arc::new(Self {
                driver: driver.clone(),
            }),
            driver,
        )
    }
}

#[async_trait]
impl Plugin for StylusProviderPlugin {
    fn name(&self) -> &str {
        "stylus-generic"
    }

    async fn apply(&self, ctx: &Arc<Context>) -> Result<(), PluginError> {
        self.driver.set_connected(true);
        ctx.provide_shared(
            STYLUS,
            StylusHandle(self.driver.clone() as Arc<dyn StylusDriver>),
        )
        .await;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use inkweave_api::stylus::{InputPhase, StylusData};

    #[tokio::test]
    async fn feed_applies_the_pressure_curve() {
        let driver = GenericStylusDriver::new();
        driver
            .set_sensitivity(Some(PressureCurve::ease_in()), None)
            .await;

        driver.feed(&[StylusInputEvent {
            stroke_id: 1,
            point: StylusData {
                pressure: 0.5,
                ..StylusData::default()
            },
            phase: InputPhase::Move,
        }]);

        let recent = driver.recent();
        assert_eq!(recent.len(), 1);
        // ease_in maps 0.5 -> 0.25
        assert!((recent[0].point.pressure - 0.25).abs() < 1e-4);
    }

    #[tokio::test]
    async fn sample_window_is_bounded() {
        let driver = GenericStylusDriver::new();
        let batch: Vec<StylusInputEvent> = (0..(SAMPLE_WINDOW + 100))
            .map(|index| StylusInputEvent {
                stroke_id: index as u64,
                point: StylusData::default(),
                phase: InputPhase::Move,
            })
            .collect();
        driver.feed(&batch);
        assert_eq!(driver.recent().len(), SAMPLE_WINDOW);
    }
}
