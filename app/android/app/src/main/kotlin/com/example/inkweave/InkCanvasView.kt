package com.example.inkweave

import android.content.Context
import android.graphics.Color
import android.view.MotionEvent
import android.view.Surface
import android.view.SurfaceHolder
import android.view.SurfaceView
import android.util.Log
import android.view.View
import android.widget.FrameLayout
import android.widget.TextView
import io.flutter.plugin.platform.PlatformView

/**
 * The handwriting canvas: a native surface Flutter lays out but does not draw into, and does not
 * route input through.
 *
 * Both halves are the point (ADR-001, ADR-002). Input goes straight from `onTouchEvent` to the Rust
 * engine over JNI, so it never enters Flutter's pointer dispatcher — the dispatcher alone costs
 * 8.33 ms at 120 Hz, a third of §3.1's whole budget. And the surface is rendered by wgpu in Rust
 * rather than by Flutter, so the ink path does not pay for the framework either.
 *
 * The view is a `FrameLayout` rather than a bare `SurfaceView` so that the "engine is not enabled"
 * case has somewhere to say so; a SurfaceView with nothing drawn into it is an indistinguishable
 * white rectangle, which would look like a bug rather than an instruction.
 */
class InkCanvasView(
    context: Context,
    @Suppress("UNUSED_PARAMETER") viewId: Int,
) : FrameLayout(context), PlatformView, SurfaceHolder.Callback {

    private val surfaceView = SurfaceView(context)
    private val message = TextView(context).apply {
        setTextColor(Color.DKGRAY)
        textSize = 16f
        gravity = android.view.Gravity.CENTER
        setPadding(48, 48, 48, 48)
        visibility = View.GONE
    }

    /** The Rust side's handle, or 0 while there is nothing attached. */
    private var handle = 0L

    init {
        addView(surfaceView)
        addView(message)
        surfaceView.holder.addCallback(this)
        // The canvas owns the gesture: it must not be intercepted by anything above it.
        isFocusable = true
    }

    override fun getView(): View = this

    override fun dispose() {
        detach()
    }

    // --- surface lifecycle ---------------------------------------------------------------------

    override fun surfaceCreated(holder: SurfaceHolder) {
        // The frame is used rather than `surfaceView.width`, which is still 0 here: layout has not run
        // yet, and a 0-sized viewport would put every vertex outside the clip volume — the canvas would
        // show its paper and nothing else, which looks exactly like a renderer bug.
        val frame = holder.surfaceFrame
        attach(holder.surface, frame.width(), frame.height())
    }

    override fun surfaceChanged(holder: SurfaceHolder, format: Int, width: Int, height: Int) {
        val frame = holder.surfaceFrame
        if (handle == 0L) {
            attach(holder.surface, frame.width(), frame.height())
        } else {
            nativeResize(handle, frame.width(), frame.height())
        }
    }

    override fun surfaceDestroyed(holder: SurfaceHolder) {
        // The surface is about to go away, so the renderer holding it must go first.
        detach()
    }

    private fun attach(surface: Surface, width: Int, height: Int) {
        val result = nativeAttach(surface, width, height)
        if (result > 0L) {
            handle = result
            message.visibility = View.GONE
            surfaceView.visibility = View.VISIBLE
            return
        }
        // The engine not being enabled is an instruction; anything else is a fault worth reporting
        // rather than hiding behind an identical message.
        handle = 0L
        when (result) {
            ERR_NO_ENGINE -> showMessage(
                "手写引擎未启用。\n\n请先在「控制面板」中启用「手写引擎」插件，再回到这里。",
            )
            else -> showMessage(
                "画布未能初始化（代码 $result）。\n\n" +
                    "绘制表面或 GPU 设备不可用，详见日志。",
            )
        }
    }

    private fun detach() {
        if (handle != 0L) {
            nativeDetach(handle)
            handle = 0L
        }
    }

    private fun showMessage(text: String) {
        message.text = text
        message.visibility = View.VISIBLE
    }

    // --- input ---------------------------------------------------------------------------------

    /**
     * Raw pen samples straight to Rust.
     *
     * `getHistorical*` is read first and matters most at high sample rates: a stylus can report
     * 240 Hz while the UI thread runs at 120 Hz, so the points in between arrive batched here. Taking
     * only `event.x` would silently halve the resolution of every fast stroke.
     */
    override fun onTouchEvent(event: MotionEvent): Boolean {
        if (handle == 0L) {
            return super.onTouchEvent(event)
        }
        when (event.actionMasked) {
            MotionEvent.ACTION_DOWN, MotionEvent.ACTION_MOVE -> {
                for (i in 0 until event.historySize) {
                    nativeTouch(
                        handle,
                        PHASE_SAMPLE,
                        event.getHistoricalX(i),
                        event.getHistoricalY(i),
                        event.getHistoricalPressure(i),
                        event.getHistoricalEventTime(i),
                    )
                }
                nativeTouch(
                    handle,
                    PHASE_SAMPLE,
                    event.x,
                    event.y,
                    event.pressure,
                    event.eventTime,
                )
                // Only the last sample is followed by a redraw: the earlier ones are on screen
                // already by the time the frame lands.
                nativeInvalidate(handle)
            }

            MotionEvent.ACTION_UP -> {
                nativeTouch(handle, PHASE_FINISH, event.x, event.y, event.pressure, event.eventTime)
                // The stroke is in the document now, so the live buffer's copy must be replaced on
                // screen rather than left as the last thing drawn.
                nativeInvalidate(handle)
            }

            MotionEvent.ACTION_CANCEL -> {
                nativeTouch(handle, PHASE_CANCEL, event.x, event.y, event.pressure, event.eventTime)
                nativeInvalidate(handle)
            }

            else -> return false
        }
        return true
    }

    private external fun nativeAttach(surface: Surface, width: Int, height: Int): Long
    private external fun nativeDetach(handle: Long)
    private external fun nativeResize(handle: Long, width: Int, height: Int)
    private external fun nativeTouch(
        handle: Long,
        phase: Int,
        x: Float,
        y: Float,
        pressure: Float,
        timeMs: Long,
    )
    private external fun nativeInvalidate(handle: Long)

    companion object {
        const val VIEW_TYPE = "inkweave/ink-canvas"
        private const val TAG = "InkCanvas"
        private const val PHASE_SAMPLE = 0
        private const val PHASE_FINISH = 1
        private const val PHASE_CANCEL = 2

        /** Mirrors the codes in `app/rust/src/canvas.rs`; a failure must be able to say which step. */
        private const val ERR_NO_ENGINE = -1L

        init {
            // Already loaded by the flutter_rust_bridge bootstrap; loading again is a no-op.
            System.loadLibrary("inkweave_bridge")
        }
    }
}