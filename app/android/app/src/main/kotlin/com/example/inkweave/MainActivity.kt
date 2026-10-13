package com.example.inkweave

import io.flutter.embedding.android.FlutterActivity
import io.flutter.embedding.engine.FlutterEngine

class MainActivity : FlutterActivity() {
    /**
     * Register the handwriting canvas.
     *
     * A platform view rather than a Flutter widget because both halves of the contract are things
     * Flutter cannot do here (ADR-001, ADR-002): the ink is rendered by wgpu into a native surface
     * so a stroke can be on screen one frame after the stylus reports it, and the touch stream goes
     * straight from `onTouchEvent` to Rust without passing through Flutter's pointer dispatcher.
     */
    override fun configureFlutterEngine(flutterEngine: FlutterEngine) {
        super.configureFlutterEngine(flutterEngine)
        flutterEngine.platformViewsController.registry
            .registerViewFactory(InkCanvasView.VIEW_TYPE, InkCanvasFactory())
    }
}