import 'dart:ui' show FramePhase;

import 'package:flutter/material.dart';
import 'package:flutter/scheduler.dart';

/// Debug-only handwriting latency instrumentation.
///
/// ## What this can and cannot measure (measured, not assumed)
///
/// The first version of this probe paired `PointerEvent.timeStamp` with
/// `FrameTiming.timestampInMicroseconds(FramePhase.rasterFinish)` and produced a p50 of
/// **26 570 ms** on the emulator — an obviously impossible "latency", because the two timestamps are
/// on *different clocks*: the pointer timestamp is app-relative while the frame timestamp comes from
/// the platform monotonic clock. The difference is a large constant offset, so the raw figure is not
/// a latency at all and the probe no longer presents it as one.
///
/// That rules out the cheap path. Only two things remain sound:
///
///  * **Frame-internal span** (`vsyncStart → rasterFinish`), below. Both endpoints come from the same
///    `FrameTiming`, so no cross-clock arithmetic is involved. This is the cost Flutter itself
///    controls, and it is a usable regression signal — but it excludes input dispatch, the
///    compositor and the panel.
///  * **Perfetto** (`input` + `frame_timeline` data sources), which puts input timestamps and frame
///    presentation on one system clock. That is the camera-free method ADR-002 can rely on.
///
/// **This probe therefore renders no verdict.** A budget comparison needs the input side, and the
/// input side needs system-level tracing. Anything else would be a number that looks like a
/// measurement and is not.
class LatencyProbe {
  final List<int> _frameSpansNanos = <int>[];
  final List<int> _inputToFrameNanos = <int>[];
  int? _pendingInputMicros;
  int _framesWithoutInput = 0;

  /// `vsyncStart → rasterFinish` per frame, in nanoseconds. Clock-safe.
  List<int> get frameSpansNanos => List.unmodifiable(_frameSpansNanos);

  /// Raw `frame − input` differences, in nanoseconds.
  ///
  /// Kept only as evidence that the two clocks disagree — **not** an input-to-display latency.
  List<int> get inputToFrameNanos => List.unmodifiable(_inputToFrameNanos);

  int get framesWithoutInput => _framesWithoutInput;

  /// Record an input batch. The earliest unconsumed timestamp is kept, so a stroke spanning several
  /// frames is attributed to the first event that had not yet been drawn.
  void noteInput(Duration timeStamp) {
    _pendingInputMicros ??= timeStamp.inMicroseconds;
  }

  /// Record one rendered frame.
  void noteFrame({required int vsyncMicros, required int rasterFinishMicros}) {
    _frameSpansNanos.add((rasterFinishMicros - vsyncMicros) * 1000);

    final pending = _pendingInputMicros;
    if (pending == null) {
      _framesWithoutInput++;
      return;
    }
    // A frame whose raster finished before the input arrived cannot be showing it.
    if (rasterFinishMicros < pending) return;
    _inputToFrameNanos.add((rasterFinishMicros - pending) * 1000);
    _pendingInputMicros = null;
  }

  void reset() {
    _frameSpansNanos.clear();
    _inputToFrameNanos.clear();
    _pendingInputMicros = null;
    _framesWithoutInput = 0;
  }

  /// Nearest-rank percentile over an arbitrary sample list. `p` in 0..1.
  static int? percentileOf(List<int> samples, double p) {
    if (samples.isEmpty) return null;
    final sorted = [...samples]..sort();
    final rank = (p * sorted.length).ceil().clamp(1, sorted.length);
    return sorted[rank - 1];
  }

  static String formatNanos(int? nanos) =>
      nanos == null ? '—' : '${(nanos / Duration.microsecondsPerMillisecond).toStringAsFixed(1)} ms';
}

/// Debug-only probe screen: draw, and watch the frame-internal span form.
///
/// Never routed to in release builds — see the `kDebugMode` guard where it is opened.
class LatencyProbeScreen extends StatefulWidget {
  const LatencyProbeScreen({super.key});

  @override
  State<LatencyProbeScreen> createState() => _LatencyProbeScreenState();
}

class _LatencyProbeScreenState extends State<LatencyProbeScreen> {
  LatencyProbe? _probe;
  final List<Offset> _stroke = <Offset>[];
  bool _measuring = false;

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    // `View.of` is an inherited lookup, and inherited lookups are illegal in initState. This is the
    // first legal place for it — and it must only ever build the probe once.
    if (_probe != null) return;
    _probe = LatencyProbe();
    SchedulerBinding.instance.addTimingsCallback(_onTimings);
    _measuring = true;
  }

  @override
  void dispose() {
    _measuring = false;
    SchedulerBinding.instance.removeTimingsCallback(_onTimings);
    super.dispose();
  }

  void _onTimings(List<FrameTiming> timings) {
    final probe = _probe;
    if (!_measuring || probe == null) return;
    for (final timing in timings) {
      probe.noteFrame(
        vsyncMicros: timing.timestampInMicroseconds(FramePhase.vsyncStart),
        rasterFinishMicros: timing.timestampInMicroseconds(FramePhase.rasterFinish),
      );
    }
    if (mounted) setState(() {});
  }

  void _notePointer(PointerEvent event) {
    // Recorded to know a frame was input-driven, and to expose the clock mismatch — see the class
    // documentation.
    _probe?.noteInput(event.timeStamp);
  }

  @override
  Widget build(BuildContext context) {
    final probe = _probe;
    if (probe == null) return const Scaffold(body: SizedBox.shrink());

    return Scaffold(
      appBar: AppBar(
        title: const Text('手写延迟探针'),
        actions: [
          IconButton(
            tooltip: '清空采样',
            onPressed: () => setState(probe.reset),
            icon: const Icon(Icons.restart_alt),
          ),
        ],
      ),
      body: Column(
        children: [
          _Stats(probe: probe),
          Expanded(
            child: Listener(
              onPointerDown: _notePointer,
              onPointerMove: _notePointer,
              child: GestureDetector(
                onPanStart: (details) => setState(() => _stroke
                  ..clear()
                  ..add(details.localPosition)),
                onPanUpdate: (details) => setState(() => _stroke.add(details.localPosition)),
                child: Container(
                  color: Theme.of(context).colorScheme.surfaceContainerHighest,
                  child: CustomPaint(
                    painter: _StrokePainter(_stroke),
                    size: Size.infinite,
                  ),
                ),
              ),
            ),
          ),
        ],
      ),
    );
  }
}

class _Stats extends StatelessWidget {
  const _Stats({required this.probe});

  final LatencyProbe probe;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final spans = probe.frameSpansNanos;
    final raw = probe.inputToFrameNanos;

    return Padding(
      padding: const EdgeInsets.fromLTRB(16, 12, 16, 8),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Text('帧内管线 vsync→栅格完成（有效）', style: theme.textTheme.titleSmall),
          Text(
            'n=${spans.length} · p50 ${LatencyProbe.formatNanos(LatencyProbe.percentileOf(spans, 0.5))} · '
            'p95 ${LatencyProbe.formatNanos(LatencyProbe.percentileOf(spans, 0.95))} · '
            'max ${LatencyProbe.formatNanos(LatencyProbe.percentileOf(spans, 1.0))}',
            style: theme.textTheme.bodyMedium,
          ),
          const SizedBox(height: 10),
          Text('输入→帧（原始差值，含时钟偏移，不可用作延迟）',
              style: theme.textTheme.titleSmall?.copyWith(color: theme.colorScheme.error)),
          Text(
            'n=${raw.length} · 最小 ${LatencyProbe.formatNanos(LatencyProbe.percentileOf(raw, 0.0))} · '
            '最大 ${LatencyProbe.formatNanos(LatencyProbe.percentileOf(raw, 1.0))} · '
            '无输入帧 ${probe.framesWithoutInput}',
            style: theme.textTheme.bodySmall,
          ),
          const SizedBox(height: 8),
          Text(
            '本探针不给结论：输入与帧时间戳不同时基。免摄像机测输入延迟需 Perfetto（input + frame_timeline）。'
            '见 ADR-002 补充。',
            style: theme.textTheme.bodySmall
                ?.copyWith(color: theme.colorScheme.onSurfaceVariant),
          ),
        ],
      ),
    );
  }
}

class _StrokePainter extends CustomPainter {
  _StrokePainter(this.points);

  final List<Offset> points;

  @override
  void paint(Canvas canvas, Size size) {
    if (points.length < 2) return;
    final paint = Paint()
      ..color = const Color(0xFF1B1B1F)
      ..strokeWidth = 4
      ..strokeCap = StrokeCap.round
      ..style = PaintingStyle.stroke;
    final path = Path()..moveTo(points.first.dx, points.first.dy);
    for (final point in points.skip(1)) {
      path.lineTo(point.dx, point.dy);
    }
    canvas.drawPath(path, paint);
  }

  @override
  bool shouldRepaint(_StrokePainter oldDelegate) => true;
}