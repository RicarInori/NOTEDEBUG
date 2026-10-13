import '../bridge/api.dart';

/// The single management seam the UI talks to.
///
/// [FrbPluginRepository] is the real implementation, over the Rust host. [DemoPluginRepository]
/// mirrors the same preset set in memory so the UI can be built, analysed and widget-tested without
/// building the native library — tests use it, the app does not.
///
/// Types come straight from the generated bridge (`bridge/api.dart`); there is no second model to
/// keep in sync.
abstract class PluginRepository {
  Future<void> bootstrap();

  Future<List<PluginCardDto>> cards();

  Future<void> setEnabled(String id, bool enabled);

  /// Install a WASM extension after the user accepted [accepted] (§8.4).
  Future<PluginCardDto> install(String packagePath, PermissionsDto accepted);

  Future<void> uninstall(String id);

  Future<List<String>> log(String id, {int lines = 40});

  Future<MemorySummary> memory();

  Future<List<TaskSummary>> tasks();

  /// Kernel lifecycle transitions, so the control panel mirrors state instead of polling (§2.4).
  ///
  /// Comes from the kernel's ledger observer, so it also carries transitions with no API call
  /// behind them — a dependency appearing, a fiber being torn down.
  Stream<PluginStateChangeDto> stateChanges();
}

/// In-memory stand-in mirroring the preset plugins the Rust host registers at bootstrap.
///
/// Keep this list in step with `app/rust/src/host/mod.rs`; it is what the widget tests assert on.
class DemoPluginRepository implements PluginRepository {
  DemoPluginRepository() {
    _cards = [
      _card('plugin-manager', '统一控制面板', PluginKindDto.internal, PluginStateDto.active,
          description: '插件启停、装入、卸载与运行观测的唯一入口',
          contributions: const [
            ContributionDto(
                id: 'panel.plugins',
                title: '控制面板',
                kind: ContributionKindDto.settingsView),
          ]),
      _card('stylus-generic', '手写笔系统', PluginKindDto.internal, PluginStateDto.active,
          description: '低延迟手写输入、压感与掌拒；厂商适配器接入后按能力位图替换本 provider',
          budgetMb: 64,
          capabilities: const StylusCapabilitiesDto(
              pressure: true,
              tilt: true,
              hover: false,
              eraser: true,
              predictStroke: false,
              recognizeShape: false,
              recognizeHandwriting: false),
          contributions: const [
            ContributionDto(
                id: 'stylus.canvas',
                title: '手写板',
                kind: ContributionKindDto.settingsView),
          ]),
      _card('sync-webdav', 'WebDAV 同步', PluginKindDto.internal, PluginStateDto.active,
          description: '六态同步状态机、etag 双校验与 LWW 冲突解决',
          budgetMb: 48,
          permissions: const PermissionsDto(network: true, filesystem: true, clipboard: false),
          backgroundTasks: 1,
          contributions: const [
            ContributionDto(
                id: 'sync.settings',
                title: 'WebDAV 同步',
                kind: ContributionKindDto.settingsView),
          ]),
      _card('ink-engine', '手写引擎', PluginKindDto.internal, PluginStateDto.active,
          description: '笔画管线、工具与画布渲染（M1）', budgetMb: 96),
      _card('pdfium-engine', 'PDF 底座', PluginKindDto.internal, PluginStateDto.active,
          description: '句柄池、tile 渲染、批注与导出（M3）', budgetMb: 128),
      _card('storage-sqlite', '文档存储', PluginKindDto.internal, PluginStateDto.active,
          description: 'SQLite + FTS5、.nbk 容器与 undo 日志（M2）'),
      _card('nbk-migrate', '格式迁移', PluginKindDto.internal, PluginStateDto.active,
          description: '.nbk 版本迁移链（M2）'),
      _card('ext-loader', '扩展沙箱', PluginKindDto.internal, PluginStateDto.active,
          description: 'wasmtime 沙箱与 inkweave ABI（M6）'),
    ];
  }

  late List<PluginCardDto> _cards;

  static PluginCardDto _card(
    String id,
    String name,
    PluginKindDto kind,
    PluginStateDto state, {
    String description = '',
    PermissionsDto permissions =
        const PermissionsDto(network: false, filesystem: false, clipboard: false),
    StylusCapabilitiesDto capabilities = const StylusCapabilitiesDto(
        pressure: false,
        tilt: false,
        hover: false,
        eraser: false,
        predictStroke: false,
        recognizeShape: false,
        recognizeHandwriting: false),
    int budgetMb = 32,
    int backgroundTasks = 0,
    List<ContributionDto> contributions = const [],
  }) =>
      PluginCardDto(
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
        memoryUsedMb: 0,
        backgroundTasks: backgroundTasks,
        contributions: contributions,
      );

  @override
  Future<void> bootstrap() async {}

  @override
  Future<List<PluginCardDto>> cards() async => List.unmodifiable(_cards);

  @override
  Future<void> setEnabled(String id, bool enabled) async {
    _cards = [
      for (final card in _cards)
        if (card.id == id)
          _replaceState(card, enabled ? PluginStateDto.active : PluginStateDto.installed)
        else
          card,
    ];
  }

  static PluginCardDto _replaceState(PluginCardDto card, PluginStateDto state) => PluginCardDto(
        id: card.id,
        name: card.name,
        version: card.version,
        kind: card.kind,
        state: state,
        author: card.author,
        description: card.description,
        permissions: card.permissions,
        capabilities: card.capabilities,
        memoryBudgetMb: card.memoryBudgetMb,
        memoryUsedMb: card.memoryUsedMb,
        backgroundTasks: card.backgroundTasks,
        contributions: card.contributions,
        lastError: card.lastError,
      );

  @override
  Future<PluginCardDto> install(String packagePath, PermissionsDto accepted) async {
    final leaf = packagePath.split(RegExp(r'[/\\]')).last.replaceAll('.wasm', '');
    final card = _card(
      'com.example.$leaf',
      '第三方扩展',
      PluginKindDto.wasm,
      PluginStateDto.active,
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
        overBudget: [],
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

  /// Nothing changes state on its own here — every transition follows one of the calls above.
  @override
  Stream<PluginStateChangeDto> stateChanges() => const Stream.empty();
}