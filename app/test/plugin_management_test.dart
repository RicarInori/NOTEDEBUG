import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:inkweave/data/models.dart';
import 'package:inkweave/data/plugin_repository.dart';
import 'package:inkweave/modules/plugins/control_panel_screen.dart';
import 'package:inkweave/modules/plugins/plugin_card_view.dart';

/// The switch inside the card for [pluginName].
Finder switchFor(String pluginName) => find.descendant(
      of: find.ancestor(
        of: find.text(pluginName),
        matching: find.byType(PluginCardView),
      ),
      matching: find.byType(Switch),
    );

/// Pump the panel and let its (immediate) async load settle.
///
/// Deliberately avoids `pumpAndSettle`: the initial state shows a `CircularProgressIndicator`,
/// whose animation never settles, so `pumpAndSettle` would time out rather than fail usefully.
Future<void> settle(WidgetTester tester, {int frames = 6}) async {
  for (var i = 0; i < frames; i++) {
    await tester.pump(const Duration(milliseconds: 16));
  }
}

Future<void> pumpPanel(WidgetTester tester, PluginRepository repository) async {
  await tester.pumpWidget(MaterialApp(home: ControlPanelScreen(repository: repository)));
  await settle(tester);
}

void main() {
  group('control panel', () {
    testWidgets('renders the first cards and the add-plugin entry', (tester) async {
      final repository = DemoPluginRepository();
      await pumpPanel(tester, repository);

      expect(find.text('控制面板'), findsOneWidget);
      expect(find.text('添加插件'), findsOneWidget);
      expect(find.textContaining('已安装'), findsOneWidget);

      // Only the first card sits above the fold, so assert on that one by name and use a
      // non-exact count for the rest: a ListView never builds its offscreen children.
      expect(find.text('统一控制面板'), findsOneWidget);
      expect(find.byType(PluginCardView), findsWidgets);
    });

    testWidgets('disabling a plugin goes through the repository, not widget state',
        (tester) async {
      final repository = DemoPluginRepository();
      await pumpPanel(tester, repository);

      await tester.tap(switchFor('手写笔系统'));
      await settle(tester);

      // The registry is the source of truth...
      final stylus = (await repository.cards())
          .firstWhere((card) => card.id == 'stylus-generic');
      expect(
        stylus.state,
        PluginRuntimeState.installed,
        reason: 'the switch must drive the registry, not local widget state',
      );

      // ...and the header reflects the refreshed snapshot. The card itself has moved into the
      // "已停用" section, which a ListView does not build while it sits offscreen.
      expect(find.textContaining('启用 7 个'), findsOneWidget);
    });

    testWidgets('uninstall is offered only for WASM extensions', (tester) async {
      final repository = DemoPluginRepository();
      await pumpPanel(tester, repository);

      // Every preset in the demo set is an internal module, so no delete affordance appears.
      expect(find.byIcon(Icons.delete_outline), findsNothing);
      expect(
        (await repository.cards()).every((card) => !card.isUninstallable),
        isTrue,
      );
    });
  });

  group('demo repository', () {
    test('install adds an uninstallable extension; uninstall removes it', () async {
      final repository = DemoPluginRepository();
      final before = (await repository.cards()).length;

      final installed = await repository.install(
        '/tmp/example.wasm',
        const Permissions(network: true),
      );
      expect(installed.kind, PluginKind.wasm);
      expect(installed.isUninstallable, isTrue);
      expect(installed.permissions.labels, ['网络']);
      expect((await repository.cards()).length, before + 1);

      await repository.uninstall(installed.id);
      expect((await repository.cards()).length, before);
    });

    test('memory and task read-outs are exposed for the panel header', () async {
      final repository = DemoPluginRepository();
      final memory = await repository.memory();
      expect(memory.processUsedMb, greaterThan(0));

      final tasks = await repository.tasks();
      expect(tasks, isNotEmpty);
      expect(tasks.first.periodic, isTrue);
    });
  });
}