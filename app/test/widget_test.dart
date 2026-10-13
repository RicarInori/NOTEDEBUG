import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:inkweave/data/plugin_repository.dart';
import 'package:inkweave/main.dart';

/// Smoke test for the app shell: it boots and exposes exactly the three top-level surfaces.
///
/// Written against the in-memory repository so it exercises the UI without the Rust host.
void main() {
  testWidgets('the shell boots with the three top-level surfaces', (tester) async {
    await tester.pumpWidget(InkweaveApp(repository: DemoPluginRepository()));
    await tester.pump(const Duration(milliseconds: 16));

    expect(find.byType(NavigationBar), findsOneWidget);
    for (final label in ['文件库', '控制面板', '设置']) {
      expect(find.text(label), findsWidgets, reason: 'the $label destination must exist');
    }
  });
}