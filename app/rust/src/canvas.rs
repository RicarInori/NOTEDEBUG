//! The Android canvas: a platform view's `Surface` turned into a wgpu surface, and the pen input path
//! that feeds it (ADR-001, ADR-002).
//!
//! ## Why this is not in `ink-engine`
//!
//! The engine renders; it does not know about window systems. `ink_engine::SurfaceRenderer` accepts
//! anything implementing `HasWindowHandle`, so the platform-specific part reduces to getting from
//! Java's `Surface` to an `ANativeWindow` — which is what this module is, plus the `MotionEvent` to
//! `StrokePoint` conversion. The same engine then serves this canvas, a desktop window and an
//! offscreen thumbnail with no change.
//!
//! ## Threading
//!
//! Every entry point is called from the Android UI thread: `SurfaceHolder.Callback` and
//! `onTouchEvent` both run there, and the Kotlin view never hands the handle to another thread. The
//! handle is therefore a plain pointer with no synchronisation — documented as UI-thread-only rather
//! than wrapped in a `Mutex` that would suggest it is safe to share.
//!
//! ## What is deliberately not here yet
//!
//! ADR-002's shared-memory ring buffer. `nativeTouch` is a JNI call per sample, which is enough to be
//! correct and to see the ink; the zero-copy path is a latency optimisation to be made against real
//! measurements rather than a guess.

use crate::host;
use crate::logcat;
use ink_engine::{InkHandle, PageId, Stroke, StrokeId, StrokePoint, SurfaceRenderer};
use inkweave_api::stylus::PressureCurve;
use jni::objects::JObject;
use jni::sys::{jfloat, jint, jlong};
use jni::JNIEnv;
use ndk::native_window::NativeWindow;
use std::sync::Arc;

/// Failure codes `nativeAttach` returns instead of a handle.
///
/// A platform view that shows nothing is indistinguishable from one that is broken, and Rust's
/// `stderr` does not reach logcat, so the reason travels back as a number the view can print. A
/// pointer is never negative, so the sign is free to carry this.
pub const ERR_NO_ENGINE: jlong = -1;
pub const ERR_NO_WINDOW: jlong = -2;
pub const ERR_NO_RENDERER: jlong = -4;

/// Where the canvas draws. The document model is M2; until then every canvas is page 1.
const CANVAS_PAGE: PageId = 1;

/// The pen the canvas writes with until §3.5's brush plugins exist.
const INK_COLOR: u32 = 0xFF1B1B1F;
const INK_WIDTH_PX: f32 = 4.0;

/// Paper, as a clear colour.
const PAPER: [u8; 4] = [0xFF, 0xFF, 0xFF, 0xFF];

/// The stroke currently under the pen has no id yet; this one cannot collide with an allocated id.
const LIVE_STROKE: StrokeId = StrokeId(u64::MAX);

/// Pressure → width. §3.5 replaces this with the user's calibrated curve.
fn curve() -> PressureCurve {
    PressureCurve::linear()
}

/// One attached canvas, owned by the platform view and addressed by the `jlong` it was given.
struct Canvas {
    /// Held because the surface presents into this window; dropping it first would leave the surface
    /// pointing at freed memory.
    #[allow(dead_code)]
    window: NativeWindow,
    renderer: SurfaceRenderer,
    /// The samples of the stroke currently under the pen. Committed to the engine on lift, so an
    /// interrupted stroke is not silently kept.
    live: Vec<StrokePoint>,
}

impl Canvas {
    /// Draw the committed page plus whatever is under the pen, then present.
    ///
    /// Re-meshing the whole page per frame is honest but wasteful. It is the correct first step
    /// because it cannot desynchronise from the document, and §3.1's incremental ring buffer replaces
    /// it once there is a measurement to justify the complexity.
    fn render(&mut self) {
        let mut strokes = self.page_strokes();
        if !self.live.is_empty() {
            // Not in the document yet, so it is drawn from the buffer rather than the engine.
            strokes.push(Stroke::new(
                LIVE_STROKE,
                self.live.clone(),
                INK_COLOR,
                INK_WIDTH_PX,
            ));
        }
        self.renderer.render(&strokes, &curve(), PAPER);
    }

    fn page_strokes(&self) -> Vec<Stroke> {
        engine()
            .map(|engine| engine.strokes(CANVAS_PAGE))
            .unwrap_or_default()
    }
}

/// The ink engine, if the plugin is enabled.
///
/// `None` is a normal state, not a failure: note-taking is a plugin the user turns on, so the canvas
/// has to be able to say "enable it first" rather than pretend to work.
fn engine() -> Option<Arc<ink_engine::InkEngine>> {
    host::app()
        .root
        .get::<InkHandle>(inkweave_api::INK)
        .map(|handle| handle.0.clone())
}

/// Attach a surface. Returns 0 if anything about the surface or the GPU is unusable, which the Kotlin
/// view reports rather than showing a blank rectangle.
///
/// # Safety
/// Called by JNI from the UI thread with a live `android.view.Surface`.
#[no_mangle]
#[allow(non_snake_case)]
pub unsafe extern "system" fn Java_com_example_inkweave_InkCanvasView_nativeAttach(
    env: JNIEnv,
    _this: JObject,
    surface: JObject,
    width: jint,
    height: jint,
) -> jlong {
    // A panic here would abort the whole process, taking the notebook with it because a canvas
    // could not open. wgpu panics on several misuses, and a platform view is exactly where that must
    // not be fatal.
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        attach(env, surface, width, height)
    })) {
        Ok(result) => result,
        Err(_) => {
            logcat::log_error(
                "InkCanvas",
                "attach panicked; see InkweavePanic for the message",
            );
            ERR_NO_RENDERER
        }
    }
}

/// The body of [`Java_com_example_inkweave_InkCanvasView_nativeAttach`], separated so the JNI entry
/// point can catch a panic around it.
unsafe fn attach(env: JNIEnv, surface: JObject, width: jint, height: jint) -> jlong {
    if engine().is_none() {
        logcat::log_error(
            "InkCanvas",
            "the ink-engine plugin is not enabled, so there is no engine to draw into",
        );
        return ERR_NO_ENGINE;
    }

    // SAFETY: `surface` is a live `android.view.Surface` owned by the caller's SurfaceView. The
    // resulting window takes its own reference, released when the `Canvas` below is dropped.
    let Some(window) =
        (unsafe { NativeWindow::from_surface(env.get_native_interface(), surface.as_raw()) })
    else {
        logcat::log_error(
            "InkCanvas",
            "ANativeWindow_fromSurface returned null: the Surface was not usable yet",
        );
        return ERR_NO_WINDOW;
    };

    let size = (width.max(1) as u32, height.max(1) as u32);
    // SAFETY: `window` is moved into the `Canvas` below, so it outlives the renderer that presents
    // into it — which is exactly what `SurfaceRenderer::from_window` requires.
    let renderer = match unsafe { SurfaceRenderer::from_window(&window, size) } {
        Ok(renderer) => renderer,
        Err(error) => {
            logcat::log_error(
                "InkCanvas",
                &format!("SurfaceRenderer::from_window failed: {error}"),
            );
            return ERR_NO_RENDERER;
        }
    };

    let mut canvas = Box::new(Canvas {
        window,
        renderer,
        live: Vec::new(),
    });
    canvas.render();
    Box::into_raw(canvas) as jlong
}

/// # Safety
/// `handle` must be a value returned by `nativeAttach` that has not been detached.
#[no_mangle]
#[allow(non_snake_case)]
pub unsafe extern "system" fn Java_com_example_inkweave_InkCanvasView_nativeDetach(
    _env: JNIEnv,
    _this: JObject,
    handle: jlong,
) {
    if handle == 0 {
        return;
    }
    let Some(canvas) = (unsafe { (handle as *mut Canvas).as_mut() }) else {
        return;
    };
    // An in-progress stroke is dropped rather than committed: the pen was still down when the surface
    // went away, so there is no honest way to claim the gesture finished.
    canvas.live.clear();
    // SAFETY: this is the matching release for the `Box::into_raw` in `nativeAttach`.
    drop(unsafe { Box::from_raw(handle as *mut Canvas) });
}

/// # Safety
/// `handle` must be a value returned by `nativeAttach` that has not been detached.
#[no_mangle]
#[allow(non_snake_case)]
pub unsafe extern "system" fn Java_com_example_inkweave_InkCanvasView_nativeResize(
    _env: JNIEnv,
    _this: JObject,
    handle: jlong,
    width: jint,
    height: jint,
) {
    let Some(canvas) = (unsafe { (handle as *mut Canvas).as_mut() }) else {
        return;
    };
    canvas
        .renderer
        .resize((width.max(1) as u32, height.max(1) as u32));
    canvas.render();
}

/// One pen sample. `phase` is 0 for a move, 1 for the end of a stroke, 2 for a cancellation.
///
/// Deliberately does not draw: a stylus can report 240 samples a second while the display shows 120
/// frames, so rendering per sample would throw away the extra work every time. The platform view
/// calls [`Java_com_example_inkweave_InkCanvasView_nativeInvalidate`] once per `MotionEvent` batch.
///
/// # Safety
/// `handle` must be a value returned by `nativeAttach` that has not been detached.
#[no_mangle]
#[allow(non_snake_case)]
pub unsafe extern "system" fn Java_com_example_inkweave_InkCanvasView_nativeTouch(
    _env: JNIEnv,
    _this: JObject,
    handle: jlong,
    phase: jint,
    x: jfloat,
    y: jfloat,
    pressure: jfloat,
    time_ms: jlong,
) {
    let Some(canvas) = (unsafe { (handle as *mut Canvas).as_mut() }) else {
        return;
    };

    match phase {
        // Lift, or a cancellation that discards what was drawn.
        1 | 2 => {
            if phase == 1 {
                canvas.live.push(sample(x, y, pressure, time_ms));
                if let Some(engine) = engine() {
                    // Buffering the gesture and committing once is what makes one stroke one undo
                    // step: §3.2 measures undo granularity in strokes, not in samples.
                    engine.commit_stroke(CANVAS_PAGE, &canvas.live, INK_COLOR, INK_WIDTH_PX);
                }
            }
            canvas.live.clear();
        }
        _ => canvas.live.push(sample(x, y, pressure, time_ms)),
    }
}

/// Draw and present. Called once per input batch, not once per sample.
///
/// # Safety
/// `handle` must be a value returned by `nativeAttach` that has not been detached.
#[no_mangle]
#[allow(non_snake_case)]
pub unsafe extern "system" fn Java_com_example_inkweave_InkCanvasView_nativeInvalidate(
    _env: JNIEnv,
    _this: JObject,
    handle: jlong,
) {
    if let Some(canvas) = unsafe { (handle as *mut Canvas).as_mut() } {
        canvas.render();
    }
}

fn sample(x: f32, y: f32, pressure: f32, time_ms: jlong) -> StrokePoint {
    StrokePoint::new(
        x,
        y,
        // A device that reports no pressure reads as 0, which `width_for` treats as "use the base
        // width" — the §3.8 degradation the pipeline already implements.
        pressure.clamp(0.0, 1.0),
        // `MotionEvent.getEventTime` is milliseconds since boot: monotonic and comparable across the
        // process, which is all the One Euro filter needs.
        (time_ms.max(0) as u64) * 1_000,
    )
}
