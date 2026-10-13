import 'package:flutter/foundation.dart'
    show Factory, TargetPlatform, defaultTargetPlatform, kIsWeb;
import 'package:flutter/gestures.dart'
    show EagerGestureRecognizer, OneSequenceGestureRecognizer;
import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart' show PlatformViewHitTestBehavior;
import 'package:flutter/services.dart' show SystemChrome, SystemUiMode;

/// The handwriting canvas.
///
/// On Android this is a **native surface hosted in a platform view**, not a Flutter canvas
/// (ADR-001, ADR-002): both halves of the input-to-pixel path have to stay out of Flutter, because
/// `SmoothPointerDataDispatcher` alone adds 8.33 ms at 120 Hz — a third of the total budget — and a
/// Flutter-drawn surface would pay for the framework as well.
///
/// The platform view owns the gesture outright: its `onTouchEvent` forwards raw pen samples
/// (including the historical batch a 240 Hz stylus produces) straight to Rust over JNI, and wgpu
/// draws into the same surface. Nothing about a stroke passes through this widget, which is why it
/// is a blank rectangle here.
///
/// The `PointerDeviceKind` list below only tells Flutter to hand stylus events to the platform view
/// rather than treating them as its own; it is a routing hint, not the input path.
///
/// ## Other platforms
///
/// The iOS and desktop surfaces do not exist yet, and this screen deliberately does **not** fall back
/// to a Flutter canvas there: drawing one would measure the wrong input path and hide the very risk
/// this architecture exists to avoid.
class CanvasHost extends StatelessWidget {
  const CanvasHost({super.key});

  @override
  Widget build(BuildContext context) {
    final isAndroid = !kIsWeb && defaultTargetPlatform == TargetPlatform.android;
    return Scaffold(
      appBar: AppBar(title: const Text('手写画布')),
      // The canvas is a writing surface, not a page: the system's own bars would take input away
      // from the pen and give the stroke less room than it measured.
      body: isAndroid ? const _NativeInkCanvas() : const _UnavailableCanvas(),
    );
  }
}

/// The Android platform view.
class _NativeInkCanvas extends StatefulWidget {
  const _NativeInkCanvas();

  @override
  State<_NativeInkCanvas> createState() => _NativeInkCanvasState();
}

class _NativeInkCanvasState extends State<_NativeInkCanvas> {
  @override
  void initState() {
    super.initState();
    // Immersive: with the status and navigation bars gone the canvas is the whole window, and the
    // edges are usable by the pen.
    SystemChrome.setEnabledSystemUIMode(SystemUiMode.immersiveSticky);
  }

  @override
  void dispose() {
    SystemChrome.setEnabledSystemUIMode(SystemUiMode.edgeToEdge);
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return AndroidView(
      viewType: 'inkweave/ink-canvas',
      // `EagerGestureRecognizer` claims the whole sequence for the platform view, which is the point:
      // Flutter still lays the view out, and must not also consume the pen.
      gestureRecognizers: <Factory<OneSequenceGestureRecognizer>>{
        Factory<OneSequenceGestureRecognizer>(() => EagerGestureRecognizer()),
      },
      hitTestBehavior: PlatformViewHitTestBehavior.opaque,
    );
  }
}

/// What every platform without a native surface shows.
class _UnavailableCanvas extends StatelessWidget {
  const _UnavailableCanvas();

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return Padding(
      padding: const EdgeInsets.all(24),
      child: Center(
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            Icon(Icons.draw_outlined, size: 56, color: theme.colorScheme.primary),
            const SizedBox(height: 16),
            Text('画布待接入', style: theme.textTheme.titleLarge),
            const SizedBox(height: 8),
            Text(
              '原生 Surface 输入路径目前只在 Android 上实现。\n'
              '此平台不提供 Flutter 侧画布，以免测到错误的输入路径（ADR-002）。',
              textAlign: TextAlign.center,
              style: theme.textTheme.bodyMedium
                  ?.copyWith(color: theme.colorScheme.onSurfaceVariant),
            ),
          ],
        ),
      ),
    );
  }
}