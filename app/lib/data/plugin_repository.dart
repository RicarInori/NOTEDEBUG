import 'models.dart';

/// The single management seam the UI talks to.
///
/// Two implementations exist: [DemoPluginRepository] (in-memory, so the UI runs before the Rust
/// bridge is generated) and `FrbPluginRepository` (the real one, over `flutter_rust_bridge`). The
/// UI is identical against both, which is what keeps the control panel honest: it can only render
/// what the registry exposes.
abstract class PluginRepository {
  Future<void> bootstrap();

  Future<List<PluginCard>> cards();

  Future<void> setEnabled(String id, bool enabled);

  /// Install a WASM extension after the user accepted [accepted] (§8.4).
  Future<PluginCard> install(String packagePath, Permissions accepted);

  Future<void> uninstall(String id);

  Future<List<String>> log(String id, {int lines = 40});

  Future<MemorySummary> memory();

  Future<List<TaskSummary>> tasks();
}

/// In-memory stand-in that mirrors the preset plugin set registered by the Rust host.
///
/// Used until `flutter_rust_bridge_codegen generate` has produced the bindings; see README.md.
class DemoPluginRepository implements PluginRepository {
  DemoPluginRepository() {
    _cards = [
      _card('plugin-manager', '统一控制面板', PluginKind.internal, PluginRuntimeState.active,
          description: '插件启停、装入、卸载与运行观测的唯一入口',
          contributions: const [
            Contribution(
                id: 'panel.plugins',
                title: '控制面板',
                kind: ContributionKind.settingsView),
          ]),
      _card('stylus-generic', '手写笔系统', PluginKind.internal, PluginRuntimeState.active,
          description: '低延迟手写输入、压感与掌拒；厂商适配器接入后按能力位图替换本 provider',
          budgetMb: 64,
          capabilities: const StylusCapabilities(
              pressure: true, tilt: true, eraser: true),
          contributions: const [
            Contribution(
                id: 'stylus.canvas',
                title: '手写板',
                kind: ContributionKind.settingsView),
          ]),
      _card('sync-webdav', 'WebDAV 同步', PluginKind.internal, PluginRuntimeState.active,
          description: '六态同步状态机、etag 双校验与 LWW 冲突解决',
          budgetMb: 48,
          permissions: const Permissions(network: true, filesystem: true),
          backgroundTasks: 1,
          contributions: const [
            Contribution(
                id: 'sync.settings',
                title: 'WebDAV 同步',
                kind: ContributionKind.settingsView),
          ]),
      _card('ink-engine', '手写引擎', PluginKind.internal, PluginRuntimeState.active,
          description: '笔画管线、工具与画布渲染（M1）', budgetMb: 96),
      _card('pdfium-engine', 'PDF 底座', PluginKind.internal, PluginRuntimeState.active,
          description: '句柄池、tile 渲染、批注与导出（M3）', budgetMb: 128),
      _card('storage-sqlite', '文档存储', PluginKind.internal, PluginRuntimeState.active,
          description: 'SQLite + FTS5、.nbk 容器与 undo 日志（M2）'),
      _card('nbk-migrate', '格式迁移', PluginKind.internal, PluginRuntimeState.active,
          description: '.nbk 版本迁移链（M2）'),
      _card('ext-loader', '扩展沙箱', PluginKind.internal, PluginRuntimeState.active,
          description: 'wasmtime 沙箱与 inkweave ABI（M6）'),
    ];
  }

  late List<PluginCard> _cards;

  static PluginCard _card(
    String id,
    String name,
    PluginKind kind,
    PluginRuntimeState state, {
    String description = '',
    Permissions permissions = Permissions.none,
    StylusCapabilities capabilities = const StylusCapabilities(),
    int budgetMb = 32,
    int backgroundTasks = 0,
    List<Contribution> contributions = const [],
  }) =>
      PluginCard(
        id: id,
        name: name,
        version: '0.1.0',
        kind: kind,
        state: state,
        author: 'Inkweave',
        description: description,
        permissions: permissions,
        capabilities: capabilities,
        memoryBudgetMb: budgetMb,
        backgroundTasks: backgroundTasks,
        contributions: contributions,
      );

  @override
  Future<void> bootstrap() async {}

  @override
  Future<List<PluginCard>> cards() async => List.unmodifiable(_cards);

  @override
  Future<void> setEnabled(String id, bool enabled) async {
    _cards = [
      for (final card in _cards)
        if (card.id == id)
          card.copyWith(
              state: enabled ? PluginRuntimeState.active : PluginRuntimeState.installed)
        else
          card,
    ];
  }

  @override
  Future<PluginCard> install(String packagePath, Permissions accepted) async {
    final id = 'com.example.${packagePath.split(RegExp(r'[/\\]')).last.replaceAll('.wasm', '')}';
    final card = _card(
      id,
      '第三方扩展',
      PluginKind.wasm,
      PluginRuntimeState.active,
      description: '来自 $packagePath',
      permissions: accepted,
    );
    _cards = [..._cards, card];
    return card;
  }

  @override
  Future<void> uninstall(String id) async {
    _cards = _cards.where((card) => card.id != id).toList();
  }

  @override
  Future<List<String>> log(String id, {int lines = 40}) async => [
        '[$id] 演示模式：真实日志由 Rust 侧的插件日志缓冲提供',
      ];

  @override
  Future<MemorySummary> memory() async => const MemorySummary(
        processUsedMb: 42,
        pressure: 'none',
      );

  @override
  Future<List<TaskSummary>> tasks() async => const [
        TaskSummary(
            owner: 'plugin:sync-webdav',
            task: 'sync',
            periodic: true,
            intervalMinutes: 15,
            requiresUnmetered: true),
      ];
}