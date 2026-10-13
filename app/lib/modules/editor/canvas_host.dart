import 'package:flutter/material.dart';

/// Placeholder for the handwriting canvas.
///
/// The real canvas is a **native surface hosted in a platform view**, not Flutter (ADR-001,
/// ADR-002): the input path has to bypass Flutter's pointer dispatcher entirely, because
/// `SmoothPointerDataDispatcher` alone adds 8.33 ms at 120 Hz — a third of the total budget.
///
/// The contract to implement, once the Spike-0 latency proof is green:
///
/// * **Android** — a `SurfaceView` whose `onTouchEvent` reads `getHistorical*` batches, filters on
///   `TOOL_TYPE_STYLUS`, applies palm rejection, and writes normalised samples straight into the
///   shared ring buffer over JNI. Rendering is wgpu/Vulkan into the same surface.
/// * **iOS** — a `UIView` capturing `touchesBegan/Moved` including predicted touches, calling Rust
///   over FFI, rendering into a `CAMetalLayer`.
/// * **Fallback architecture B** — if the platform view misses the latency budget by more than
///   30%, the native surface becomes the base layer and Flutter overlays it with a transparent
///   background, trading layout integration for a clean input path.
///
/// Until one of those exists, this screen deliberately does not fake a drawing surface: a Flutter
/// canvas here would measure the wrong input path and hide the very risk Spike-0 exists to test.
class CanvasHost extends StatelessWidget {
  const CanvasHost({super.key});

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);

    return Scaffold(
      appBar: AppBar(title: const Text('手写画布')),
      body: Padding(
        padding: const EdgeInsets.all(24),
        child: Center(
          child: Column(
            mainAxisSize: MainAxisSize.min,
            children: [
              Icon(
                Icons.draw_outlined,
                size: 56,
                color: theme.colorScheme.primary,
              ),
              const SizedBox(height: 16),
              Text('画布待接入', style: theme.textTheme.titleLarge),
              const SizedBox(height: 8),
              Text(
                '原生 Surface 输入路径由 Spike-0 延迟验证通过后接入（ADR-002）。\n'
                '在此之前不提供 Flutter 侧画布，以免测到错误的输入路径。',
                textAlign: TextAlign.center,
                style: theme.textTheme.bodyMedium
                    ?.copyWith(color: theme.colorScheme.onSurfaceVariant),
              ),
            ],
          ),
        ),
      ),
    );
  }
}