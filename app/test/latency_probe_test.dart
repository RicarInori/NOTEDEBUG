import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:inkweave/modules/debug/latency_probe.dart';

/// The probe's arithmetic is the only part testable without a device, and it is the part a wrong
/// answer would quietly corrupt: an off-by-one in the percentile, or a leaked pending input, would
/// produce a plausible-looking number that means nothing.
///
/// Note what is *not* asserted anywhere: that the input-to-frame figure is a latency. It is not —
/// the two timestamps are on different clocks — so the tests only pin the pairing mechanism and the
/// frame-internal span, which is the number the screen labels as valid.
void main() {
  group('LatencyProbe', () {
    test('a frame records its own span, which involves one clock only', () {
      final p = LatencyProbe();
      p.noteFrame(vsyncMicros: 1000, rasterFinishMicros: 9000);
      expect(p.frameSpansNanos.single, 8 * 1000 * 1000);
    });

    test('an input is attributed to the first frame that could show it', () {
      final p = LatencyProbe();
      p.noteInput(const Duration(microseconds: 1000));
      p.noteInput(const Duration(microseconds: 1500)); // same batch — the earliest one wins

      p.noteFrame(vsyncMicros: 0, rasterFinishMicros: 500); // predates the input
      expect(p.inputToFrameNanos, isEmpty);
      expect(p.framesWithoutInput, 0, reason: 'the input was still pending, not absent');

      p.noteFrame(vsyncMicros: 0, rasterFinishMicros: 11000);
      expect(p.inputToFrameNanos.single, 10 * 1000 * 1000);
    });

    test('frames with nothing pending are counted, never paired', () {
      final p = LatencyProbe();
      p.noteFrame(vsyncMicros: 0, rasterFinishMicros: 5000);
      expect(p.inputToFrameNanos, isEmpty);
      expect(p.framesWithoutInput, 1);
    });

    test('percentiles use nearest rank', () {
      final samples = [for (var i = 1; i <= 10; i++) i * 1000 * 1000];
      expect(LatencyProbe.percentileOf(samples, 0.5), 5 * 1000 * 1000);
      expect(LatencyProbe.percentileOf(samples, 0.95), 10 * 1000 * 1000);
      expect(LatencyProbe.percentileOf(samples, 0.0), 1 * 1000 * 1000);
      expect(LatencyProbe.percentileOf(const [], 0.5), isNull);
    });

    test('the frame-span statistic follows the frames, not the input', () {
      final p = LatencyProbe();
      for (var i = 0; i < 28; i++) {
        p.noteFrame(vsyncMicros: 0, rasterFinishMicros: 1000);
      }
      for (var i = 0; i < 2; i++) {
        p.noteFrame(vsyncMicros: 0, rasterFinishMicros: 50 * 1000);
      }
      // Two of thirty samples reach p95; one would not.
      expect(LatencyProbe.percentileOf(p.frameSpansNanos, 0.95), 50 * 1000 * 1000);
    });

    test('reset clears both sample sets and the pending input', () {
      final p = LatencyProbe();
      p.noteInput(Duration.zero);
      p.noteFrame(vsyncMicros: 0, rasterFinishMicros: 1000);
      p.noteInput(const Duration(microseconds: 9000));
      expect(p.frameSpansNanos, hasLength(1));

      p.reset();

      expect(p.frameSpansNanos, isEmpty);
      expect(p.inputToFrameNanos, isEmpty);
      // The stale pending input must not be paired with the next frame.
      p.noteFrame(vsyncMicros: 0, rasterFinishMicros: 20000);
      expect(p.inputToFrameNanos, isEmpty);
      expect(p.framesWithoutInput, 1);
    });
  });

  group('LatencyProbeScreen', () {
    // Guards a mistake that only running the app revealed: `View.of` is an inherited lookup, so
    // calling it from initState throws and paints the whole screen red. Opening the screen is the
    // only way to catch that, and this does exactly that.
    testWidgets('opens without an exception and shows its read-out', (tester) async {
      await tester.pumpWidget(const MaterialApp(home: LatencyProbeScreen()));
      await tester.pump();

      expect(tester.takeException(), isNull);
      expect(find.text('手写延迟探针'), findsOneWidget);
      expect(find.textContaining('帧内管线'), findsOneWidget);
      // It must never claim a verdict it cannot support.
      expect(find.textContaining('本探针不给结论'), findsOneWidget);
    });

    testWidgets('draws a stroke without throwing', (tester) async {
      await tester.pumpWidget(const MaterialApp(home: LatencyProbeScreen()));
      await tester.pump();

      final gesture = await tester.startGesture(const Offset(100, 400));
      await gesture.moveTo(const Offset(300, 600));
      await gesture.moveTo(const Offset(500, 300));
      await gesture.up();
      await tester.pump();

      expect(tester.takeException(), isNull);
    });
  });
}