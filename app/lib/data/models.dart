/// View models for the control panel.
///
/// These mirror the Rust `inkweave_api::registry` types one-to-one. Once
/// `flutter_rust_bridge_codegen generate` has run, the generated types replace the hand-written
/// ones and this file's classes are only used by [DemoPluginRepository].
library;

/// How a plugin is distributed; decides which of the four verbs apply (ADR-007).
enum PluginKind { internal, wasm }

/// Mirror of the kernel's fiber state plus `installed` for plugins not yet mounted this run.
enum PluginRuntimeState {
  installed,
  pending,
  loading,
  active,
  failed,
  unloading,
  disposed,
}

extension PluginRuntimeStateLabels on PluginRuntimeState {
  String get label => switch (this) {
        PluginRuntimeState.installed => '未加载',
        PluginRuntimeState.pending => '等待依赖',
        PluginRuntimeState.loading => '加载中',
        PluginRuntimeState.active => '已启用',
        PluginRuntimeState.failed => '错误',
        PluginRuntimeState.unloading => '卸载中',
        PluginRuntimeState.disposed => '已停用',
      };

  bool get isActive => this == PluginRuntimeState.active;
}

/// Declared capabilities, rendered as a permission list and confirmed before install (§8.4).
class Permissions {
  const Permissions({
    this.network = false,
    this.filesystem = false,
    this.clipboard = false,
  });

  static const none = Permissions();

  final bool network;
  final bool filesystem;
  final bool clipboard;

  List<String> get labels => [
        if (network) '网络',
        if (filesystem) '文件',
        if (clipboard) '剪贴板',
      ];
}

enum ContributionKind { command, settingsView, menuItem, brush, template }

class Contribution {
  const Contribution({required this.id, required this.title, required this.kind});

  final String id;
  final String title;
  final ContributionKind kind;
}

/// Stylus capability bitmap. The UI shows or hides features from this and never branches on
/// vendor (DEV guide §6).
class StylusCapabilities {
  const StylusCapabilities({
    this.pressure = false,
    this.tilt = false,
    this.hover = false,
    this.eraser = false,
    this.predictStroke = false,
    this.recognizeShape = false,
    this.recognizeHandwriting = false,
  });

  final bool pressure;
  final bool tilt;
  final bool hover;
  final bool eraser;
  final bool predictStroke;
  final bool recognizeShape;
  final bool recognizeHandwriting;

  List<String> get labels => [
        if (pressure) '压感',
        if (tilt) '倾角',
        if (hover) '悬停',
        if (eraser) '笔尾橡皮',
        if (predictStroke) '笔迹预测',
        if (recognizeShape) '一笔成形',
        if (recognizeHandwriting) '手写识别',
      ];
}

/// Everything the panel renders for one plugin.
class PluginCard {
  const PluginCard({
    required this.id,
    required this.name,
    required this.version,
    required this.kind,
    required this.state,
    this.author = '',
    this.description = '',
    this.permissions = Permissions.none,
    this.capabilities = const StylusCapabilities(),
    this.memoryBudgetMb = 0,
    this.memoryUsedMb = 0,
    this.backgroundTasks = 0,
    this.contributions = const [],
    this.lastError,
  });

  final String id;
  final String name;
  final String version;
  final PluginKind kind;
  final PluginRuntimeState state;
  final String author;
  final String description;
  final Permissions permissions;
  final StylusCapabilities capabilities;
  final int memoryBudgetMb;
  final int memoryUsedMb;
  final int backgroundTasks;
  final List<Contribution> contributions;
  final String? lastError;

  /// Uninstall only applies to WASM extensions; internal modules are compiled in (ADR-003).
  bool get isUninstallable => kind == PluginKind.wasm;

  PluginCard copyWith({PluginRuntimeState? state, String? lastError}) => PluginCard(
        id: id,
        name: name,
        version: version,
        kind: kind,
        state: state ?? this.state,
        author: author,
        description: description,
        permissions: permissions,
        capabilities: capabilities,
        memoryBudgetMb: memoryBudgetMb,
        memoryUsedMb: memoryUsedMb,
        backgroundTasks: backgroundTasks,
        contributions: contributions,
        lastError: lastError ?? this.lastError,
      );
}

class MemorySummary {
  const MemorySummary({
    this.processUsedMb = 0,
    this.pressure = 'none',
    this.overBudget = const [],
  });

  final int processUsedMb;
  final String pressure;
  final List<String> overBudget;
}

class TaskSummary {
  const TaskSummary({
    required this.owner,
    required this.task,
    this.periodic = false,
    this.intervalMinutes = 0,
    this.requiresUnmetered = false,
  });

  final String owner;
  final String task;
  final bool periodic;
  final int intervalMinutes;
  final bool requiresUnmetered;
}