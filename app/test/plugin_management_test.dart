import 'dart:async';

import 'package:flutter/foundation.dart' show TargetPlatform, debugDefaultTargetPlatformOverride;
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:inkweave/bridge/api.dart';
import 'package:inkweave/data/plugin_extensions.dart';
import 'package:inkweave/data/plugin_repository.dart';
import 'package:inkweave/modules/plugins/add_plugin_dialog.dart';
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

Future<void> pumpPanel(
  WidgetTester tester,
  PluginRepository repository, {
  double height = 3200,
}) async {
  // A phone-sized viewport only builds the first card or two, so an assertion about "any card"
  // would silently be about whichever ones a ListView happened to build. Widen it.
  tester.view.physicalSize = Size(1200, height);
  tester.view.devicePixelRatio = 1.0;
  addTearDown(tester.view.reset);

  await tester.pumpWidget(MaterialApp(home: ControlPanelScreen(repository: repository)));
  await settle(tester);
}

/// A repository whose state stream the test can drive, standing in for the kernel's ledger observer.
class StreamingRepository extends DemoPluginRepository {
  final controller = StreamController<PluginStateChangeDto>.broadcast();

  @override
  Stream<PluginStateChangeDto> stateChanges() => controller.stream;
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
        PluginStateDto.installed,
        reason: 'the switch must drive the registry, not local widget state',
      );

      // ...and the header reflects the refreshed snapshot. The card itself has moved into the
      // "已停用" section, which a ListView does not build while it sits offscreen.
      // 7 of 8 are enabled to begin with — ink-engine ships disabled, since note-taking is a plugin
      // the user turns on.
      expect(find.textContaining('启用 6 个'), findsOneWidget);
    });

    testWidgets('a declared contribution opens the screen the registry maps it to', (tester) async {
      // The canvas is a native surface on Android and an explanatory placeholder anywhere else, and
      // `flutter_test` runs as Android by default. Pinning a platform with no surface is what lets
      // this assert something real rather than the platform-view plumbing a host test cannot host.
      // Reset in a `finally`: the framework checks its debug variables when the body returns, which
      // is before any `addTearDown` callback would run.
      debugDefaultTargetPlatformOverride = TargetPlatform.windows;
      try {
        final repository = DemoPluginRepository();
        await pumpPanel(tester, repository);

        // The stylus plugin declares `stylus.canvas`, which the host maps to the canvas screen.
        await tester.tap(find.text('打开「手写板」'));
        await settle(tester);

        expect(find.text('手写画布'), findsOneWidget);
        expect(find.text('画布待接入'), findsOneWidget);
      } finally {
        debugDefaultTargetPlatformOverride = null;
      }
    });

    testWidgets('a contribution with no Dart screen says so instead of doing nothing',
        (tester) async {
      final repository = DemoPluginRepository();
      await pumpPanel(tester, repository);

      // `panel.plugins` is declared by the plugin-manager card; nothing registers a screen for it.
      await tester.tap(find.text('打开「控制面板」'));
      await settle(tester);

      expect(find.textContaining('尚未提供可用的界面'), findsOneWidget);
    });

    testWidgets('a kernel state change refreshes the panel without any user action',
        (tester) async {
      final repository = StreamingRepository();
      addTearDown(repository.controller.close);
      await pumpPanel(tester, repository);

      // Something outside this screen changes the registry — a dependency arriving, the host
      // tearing a fiber down. The panel must follow without the user touching anything.
      await repository.setEnabled('stylus-generic', false);
      repository.controller.add(const PluginStateChangeDto(
        pluginId: 'stylus-generic',
        state: PluginStateDto.disposed,
      ));
      await settle(tester);

      expect(find.textContaining('启用 6 个'), findsOneWidget);
    });

    testWidgets('uninstall is not offered for compiled-in modules', (tester) async {
      final repository = DemoPluginRepository();
      await pumpPanel(tester, repository);

      // Every preset in the demo set is an internal module, so no delete affordance appears.
      expect(find.byIcon(Icons.delete_outline), findsNothing);
      expect(
        (await repository.cards()).every((card) => !card.isUninstallable),
        isTrue,
      );
    });

    testWidgets('an imported native plugin lands in the extensions section', (tester) async {
      final repository = DemoPluginRepository();
      await repository.install(
        '/tmp/libexample_native.so',
        const PermissionsDto(network: false, filesystem: true, clipboard: false),
      );
      await pumpPanel(tester, repository, height: 6000);

      expect(find.text('已装入的扩展'), findsOneWidget);
      expect(
        find.text('v0.1.0 · 原生扩展'),
        findsOneWidget,
        reason: 'the card must say which channel the plugin came through',
      );
    });
  });

  group('add-plugin confirmation', () {
    CheckboxListTile boxFor(WidgetTester tester, String title) =>
        tester.widget<CheckboxListTile>(
          find.ancestor(
            of: find.text(title),
            matching: find.byType(CheckboxListTile),
          ),
        );

    testWidgets('a native manifest prefills the boxes with the plugin\'s own request',
        (tester) async {
      await tester.pumpWidget(
        const MaterialApp(
          home: Scaffold(
            body: AddPluginDialog(
              packageName: 'libexample_native.so',
              manifest: NativeManifestDto(
                path: '/tmp/libexample_native.so',
                id: 'com.inkweave.example.native',
                name: '示例原生插件',
                version: '0.1.0',
                author: 'Inkweave',
                description: '演示原生动态库插件',
                permissions: PermissionsDto(
                  network: false,
                  filesystem: true,
                  clipboard: false,
                ),
              ),
            ),
          ),
        ),
      );

      // The dialog must describe the plugin, not just the file name the user picked.
      expect(find.text('示例原生插件'), findsOneWidget);
      expect(find.textContaining('libexample_native.so · v0.1.0'), findsOneWidget);
      expect(find.text('演示原生动态库插件'), findsOneWidget);

      // And it must ask about exactly what the plugin declared — nothing pre-granted.
      expect(boxFor(tester, '文件').value, isTrue);
      expect(boxFor(tester, '网络').value, isFalse);
      expect(boxFor(tester, '剪贴板').value, isFalse);

      // The in-process warning is the one thing a native plugin must not hide.
      expect(find.textContaining('同进程运行'), findsOneWidget);
    });

    testWidgets('an unprobed package still offers all three permissions', (tester) async {
      await tester.pumpWidget(
        const MaterialApp(
          home: Scaffold(body: AddPluginDialog(packageName: 'example.wasm')),
        ),
      );

      expect(boxFor(tester, '网络').value, isTrue);
      expect(boxFor(tester, '文件').value, isTrue);
      expect(boxFor(tester, '剪贴板').value, isTrue);
      expect(find.textContaining('同进程运行'), findsNothing);
    });
  });

  group('native import plumbing', () {
    test('the picker accepts shared libraries and refuses everything else', () {
      expect('libexample_native.so'.isPluginPackage, isTrue);
      expect('example_native.dll'.isPluginPackage, isTrue);
      expect('libexample.dylib'.isPluginPackage, isTrue);
      expect('example.wasm'.isPluginPackage, isTrue);
      expect('example.inkext'.isPluginPackage, isTrue);

      expect('libexample_native.so'.isNativeLibrary, isTrue);
      expect('example.wasm'.isNativeLibrary, isFalse);
      expect('notes.pdf'.isPluginPackage, isFalse);
      expect('archive.zip'.isPluginPackage, isFalse);
    });

    test('probing a native library yields its declared permissions', () async {
      final repository = DemoPluginRepository();
      final manifest = await repository.probeNative('/tmp/libexample_native.so');

      expect(manifest.id, 'com.example.native-demo');
      expect(manifest.permissions.filesystem, isTrue);
      expect(
        manifest.permissions.labels,
        ['文件'],
        reason: 'only the declared permission may be offered',
      );

      expect(
        () => repository.probeNative('/tmp/example.wasm'),
        throwsA(isA<StateError>()),
        reason: 'a WASM package carries its manifest beside the code, not inside it',
      );
    });

    test('installing a shared library registers an uninstallable native card', () async {
      final repository = DemoPluginRepository();
      final before = (await repository.cards()).length;

      final installed = await repository.install(
        '/tmp/libexample_native.so',
        const PermissionsDto(network: false, filesystem: true, clipboard: false),
      );
      expect(installed.kind, PluginKindDto.native);
      expect(installed.isUninstallable, isTrue);
      expect((await repository.cards()).length, before + 1);

      await repository.uninstall(installed.id);
      expect((await repository.cards()).length, before);
    });
  });

  group('demo repository', () {
    test('install adds an uninstallable extension; uninstall removes it', () async {
      final repository = DemoPluginRepository();
      final before = (await repository.cards()).length;

      final installed = await repository.install(
        '/tmp/example.wasm',
        const PermissionsDto(network: true, filesystem: false, clipboard: false),
      );
      expect(installed.kind, PluginKindDto.wasm);
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