use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// Normalised stylus sample. One struct covers Android / iOS / Web after platform conversion
/// (research 02 §5; research 05 `StylusInputEvent`).
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
pub struct StylusData {
    pub x: f32,
    pub y: f32,
    /// 0.0–1.0. Devices without pressure report a synthesised value; see [`PressureCurve`].
    pub pressure: f32,
    /// 0–π/2 radians.
    pub altitude: f32,
    /// 0–2π radians.
    pub azimuth: f32,
    /// 0–359 degrees.
    pub twist: f32,
    pub timestamp_us: u64,
}

impl StylusData {
    /// W3C Pointer Events L3 mapping from Android's pressure/tilt/orientation triple.
    pub fn from_android(pressure: f32, tilt: f32, orientation: f32) -> Self {
        Self {
            pressure,
            altitude: tilt,
            azimuth: orientation,
            ..Self::default()
        }
    }

    /// iOS `UITouch` mapping (Apple Pencil reports altitude and azimuth directly).
    pub fn from_ios(pressure: f32, altitude: f32, azimuth: f32) -> Self {
        Self {
            pressure,
            altitude,
            azimuth,
            ..Self::default()
        }
    }

    /// Web `PointerEvent` mapping (`tiltX` / `tiltY` in degrees).
    pub fn from_web(pressure: f32, tilt_x: f32, tilt_y: f32) -> Self {
        let altitude = tilt_x.hypot(tilt_y).to_radians();
        let azimuth = tilt_y.atan2(tilt_x);
        Self {
            pressure,
            altitude,
            azimuth,
            ..Self::default()
        }
    }
}

/// User-calibrated pressure response. Applied after the One Euro filter and before width mapping
/// (DEV guide §3.1).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PressureCurve {
    pub points: Vec<(f32, f32)>,
}

impl PressureCurve {
    pub fn linear() -> Self {
        Self {
            points: vec![(0.0, 0.0), (1.0, 1.0)],
        }
    }

    pub fn ease_in() -> Self {
        Self {
            points: vec![(0.0, 0.0), (0.5, 0.25), (1.0, 1.0)],
        }
    }

    pub fn ease_out() -> Self {
        Self {
            points: vec![(0.0, 0.0), (0.5, 0.75), (1.0, 1.0)],
        }
    }

    /// Piecewise-linear interpolation over `points`; input and output are clamped to 0.0–1.0.
    pub fn apply(&self, pressure: f32) -> f32 {
        let p = pressure.clamp(0.0, 1.0);
        if self.points.len() < 2 {
            return p;
        }
        let mut previous = self.points[0];
        if p <= previous.0 {
            return previous.1.clamp(0.0, 1.0);
        }
        for point in &self.points[1..] {
            if p <= point.0 {
                let span = (point.0 - previous.0).max(f32::EPSILON);
                let t = (p - previous.0) / span;
                return (previous.1 + (point.1 - previous.1) * t).clamp(0.0, 1.0);
            }
            previous = *point;
        }
        previous.1.clamp(0.0, 1.0)
    }
}

impl Default for PressureCurve {
    fn default() -> Self {
        Self::linear()
    }
}

/// Capability bitmap. The UI shows or hides features from this — it never branches on vendor
/// (DEV guide §6).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StylusCapabilities {
    pub pressure: bool,
    pub tilt: bool,
    pub hover: bool,
    pub eraser: bool,
    pub predict_stroke: bool,
    pub recognize_shape: bool,
    pub recognize_handwriting: bool,
}

impl StylusCapabilities {
    /// What the standard platform API can be relied on for, per research 05 (60–70% coverage).
    pub const STANDARD: Self = Self {
        pressure: true,
        tilt: true,
        hover: false,
        eraser: true,
        predict_stroke: false,
        recognize_shape: false,
        recognize_handwriting: false,
    };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StylusVendor {
    Generic,
    Samsung,
    Xiaomi,
    Huawei,
    Apple,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InputPhase {
    Down,
    Move,
    Up,
    Cancel,
}

/// One normalised input event from the native capture path.
///
/// The platform layer batch-reads `getHistorical*` / coalesced touches and pushes these across
/// FFI into a shared ring buffer; this type is the schema of that buffer's entries.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct StylusInputEvent {
    pub stroke_id: u64,
    pub point: StylusData,
    pub phase: InputPhase,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StylusButtonKind {
    Primary,
    Secondary,
    Eraser,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ButtonAction {
    Press,
    Release,
    DoubleTap,
    LongPress,
    Squeeze,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct StylusButtonEvent {
    pub kind: StylusButtonKind,
    pub action: ButtonAction,
}

/// A stylus provider. `stylus-generic` is the preset fallback; vendor adapters replace it at
/// runtime by providing the same key, which makes dependents restart via the kernel's reactive
/// injection (DEV guide §6, §8.7).
#[async_trait]
pub trait StylusDriver: Send + Sync + 'static {
    fn vendor(&self) -> StylusVendor;

    fn capabilities(&self) -> StylusCapabilities;

    fn is_connected(&self) -> bool;

    /// Update user sensitivity. `None` leaves that dimension untouched.
    async fn set_sensitivity(&self, pressure: Option<PressureCurve>, tilt: Option<f32>);

    /// Current pressure response, for the settings UI.
    fn pressure_curve(&self) -> PressureCurve;

    /// Feed a batch of normalised samples. Hot path: must not allocate or log.
    fn feed(&self, events: &[StylusInputEvent]);
}
