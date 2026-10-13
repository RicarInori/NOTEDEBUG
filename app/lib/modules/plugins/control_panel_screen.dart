import 'dart:async';

import 'package:file_picker/file_picker.dart';
import 'package:flutter/material.dart';

import '../../bridge/api.dart';
import '../../data/plugin_extensions.dart';
import '../../data/plugin_repository.dart';
import 'add_plugin_dialog.dart';
import 'contribution_registry.dart';
import 'plugin_card_view.dart';

/// The unified control panel (§8.3) — the single place plugins are managed.
///
/// It renders exactly what the registry reports: enabling, disabling and uninstalling all go
/// through [PluginRepository], which in production is the Rust `PluginRegistry`. There is no
/// second management surface anywhere in the app.
class ControlPanelScreen extends StatefulWidget {
  const ControlPanelScreen({super.key, required this.repository});

  final PluginRepository repository;

  @override
  State<ControlPanelScreen> createState() => _ControlPanelScreenState();
}

class _ControlPanelScreenState extends State<ControlPanelScreen> {
  List<PluginCardDto> _cards = const [];
  List<TaskSummary> _tasks = const [];
  MemorySummary _memory =
      const MemorySummary(processUsedMb: 0, pressure: 'none', overBudget: []);
  bool _loading = true;
  StreamSubscription<PluginStateChangeDto>? _stateEvents;

  /// States worth re-reading the snapshot for. A single enable walks
  /// `Pending → Loading → Active`, and there is nothing new to show until it lands.
  static const _settledStates = {
    PluginStateDto.active,
    PluginStateDto.failed,
    PluginStateDto.installed,
    PluginStateDto.disposed,
  };

  @override
  void initState() {
    super.initState();
    _refresh();

    // Mirror kernel transitions instead of polling: a plugin can change state without this screen
    // asking it to (a dependency arriving, a fiber being torn down by the host).
    _stateEvents = widget.repository.stateChanges().listen((change) {
      if (!mounted) return;
      if (_settledStates.contains(change.state)) _refresh();
    });
  }

  @override
  void dispose() {
    _stateEvents?.cancel();
    super.dispose();
  }

  Future<void> _refresh() async {
    setState(() => _loading = true);
    final cards = await widget.repository.cards();
    final memory = await widget.repository.memory();
    final tasks = await widget.repository.tasks();
    if (!mounted) return;
    setState(() {
      _cards = cards;
      _memory = memory;
      _tasks = tasks;
      _loading = false;
    });
  }

  Future<void> _toggle(PluginCardDto card, bool enabled) async {
    await widget.repository.setEnabled(card.id, enabled);
    await _refresh();
    _notify('${card.name} 已${enabled ? '启用' : '停用'}');
  }

  Future<void> _uninstall(PluginCardDto card) async {
    final confirmed = await showDialog<bool>(
      context: context,
      builder: (context) => AlertDialog(
        title: Text('卸载 ${card.name}'),
        content: const Text('将停止该插件、取消其后台任务并删除其私有数据。此操作不可撤销。'),
        actions: [
          TextButton(
            onPressed: () => Navigator.of(context).pop(false),
            child: const Text('取消'),
          ),
          FilledButton(
            onPressed: () => Navigator.of(context).pop(true),
            child: const Text('卸载'),
          ),
        ],
      ),
    );
    if (confirmed != true) return;
    try {
      await widget.repository.uninstall(card.id);
      await _refresh();
      _notify('${card.name} 已卸载');
    } catch (error) {
      _notify('卸载失败：$error');
    }
  }

  Future<void> _showLog(PluginCardDto card) async {
    final lines = await widget.repository.log(card.id);
    if (!mounted) return;
    await showDialog<void>(
      context: context,
      builder: (context) => AlertDialog(
        title: Text('${card.name} · 运行日志'),
        content: SizedBox(
          width: double.maxFinite,
          child: SingleChildScrollView(
            child: Text(
              lines.isEmpty ? '暂无日志' : lines.join('\n'),
              style: Theme.of(context).textTheme.bodySmall,
            ),
          ),
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.of(context).pop(),
            child: const Text('关闭'),
          ),
        ],
      ),
    );
  }

  /// The "add plugin" entry: pick a package, confirm permissions, install, hot-load.
  ///
  /// A native library is probed first so the dialog can show the plugin's own name and permission
  /// request; a WASM package carries its manifest beside the code and is validated by the host.
  Future<void> _addPlugin() async {
    // file_picker 13 exposes a static `pickFiles` returning a list; empty means cancelled. The type
    // filter is `any` on purpose: Android's custom-type picker maps extensions to MIME types
    // unreliably, and a rejected pick here beats an empty picker the user cannot get past.
    final picked = await FilePicker.pickFiles(
      dialogTitle: '选择插件包（.wasm / .inkext / .so / .dll）',
      type: FileType.any,
    );
    if (picked.isEmpty || !mounted) return;

    final file = picked.first;
    final path = file.path;
    if (path == null) {
      _notify('该平台未提供文件路径，无法安装');
      return;
    }
    if (!file.name.isPluginPackage) {
      _notify('无法识别的插件包类型：${file.name}');
      return;
    }

    NativeManifestDto? manifest;
    if (file.name.isNativeLibrary) {
      try {
        manifest = await widget.repository.probeNative(path);
      } catch (error) {
        _notify('无法读取插件清单：$error');
        return;
      }
      if (!mounted) return;
    }

    final accepted = await showDialog<PermissionsDto>(
      context: context,
      builder: (context) =>
          AddPluginDialog(packageName: file.name, manifest: manifest),
    );
    if (accepted == null) return;

    try {
      final installed = await widget.repository.install(path, accepted);
      await _refresh();
      _notify('已安装并热加载：${installed.name}');
    } catch (error) {
      _notify('安装失败：$error');
    }
  }

  /// Open a contribution the plugin declared. The screen comes from the Dart-side registry — a
/// contribution with no registered screen is reported plainly rather than failing silently.
  Future<void> _openContribution(PluginCardDto card, ContributionDto contribution) async {
    final builder = ContributionRegistry.standard.lookup(contribution.id);
    if (builder == null) {
      _notify('「${contribution.title}」由 ${card.name} 声明，但尚未提供可用的界面');
      return;
    }
    await Navigator.of(context).push(MaterialPageRoute<void>(builder: builder));
  }

  void _notify(String message) {
    if (!mounted) return;
    ScaffoldMessenger.of(context).showSnackBar(SnackBar(content: Text(message)));
  }

  @override
  Widget build(BuildContext context) {
    final sections = _sections();

    return Scaffold(
      appBar: AppBar(
        title: const Text('控制面板'),
        actions: [
          IconButton(
            onPressed: _refresh,
            icon: const Icon(Icons.refresh),
            tooltip: '刷新',
          ),
        ],
      ),
      floatingActionButton: FloatingActionButton.extended(
        onPressed: _addPlugin,
        icon: const Icon(Icons.add),
        label: const Text('添加插件'),
      ),
      body: _loading
          ? const Center(child: CircularProgressIndicator())
          : RefreshIndicator(
              onRefresh: _refresh,
              child: ListView(
                padding: const EdgeInsets.fromLTRB(16, 16, 16, 96),
                children: [
                  _MemoryHeader(
                    memory: _memory,
                    cardCount: _cards.length,
                    activeCount: _cards.where((card) => card.state.isActive).length,
                    taskCount: _tasks.length,
                  ),
                  for (final section in sections) ...[
                    const SizedBox(height: 20),
                    Text(section.title, style: Theme.of(context).textTheme.titleMedium),
                    const SizedBox(height: 8),
                    for (final card in section.cards) ...[
                      PluginCardView(
                        card: card,
                        onToggle: (enabled) => _toggle(card, enabled),
                        onUninstall: () => _uninstall(card),
                        onShowLog: () => _showLog(card),
                        onOpenContribution: (contribution) =>
                            _openContribution(card, contribution),
                      ),
                      const SizedBox(height: 12),
                    ],
                  ],
                ],
              ),
            ),
    );
  }

  List<_Section> _sections() {
    final activeInternal = _cards
        .where((card) => card.kind == PluginKindDto.internal && card.state.isActive)
        .toList();
    final activeExtensions = _cards
        .where((card) => card.isUninstallable && card.state.isActive)
        .toList();
    final inactive = _cards.where((card) => !card.state.isActive).toList();

    return [
      _Section('预置插件', activeInternal),
      _Section('已装入的扩展', activeExtensions),
      _Section('已停用', inactive),
    ].where((section) => section.cards.isNotEmpty).toList();
  }
}

class _Section {
  const _Section(this.title, this.cards);

  final String title;
  final List<PluginCardDto> cards;
}

class _MemoryHeader extends StatelessWidget {
  const _MemoryHeader({
    required this.memory,
    required this.cardCount,
    required this.activeCount,
    required this.taskCount,
  });

  final MemorySummary memory;
  final int cardCount;
  final int activeCount;
  final int taskCount;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return Card(
      elevation: 0,
      margin: EdgeInsets.zero,
      shape: RoundedRectangleBorder(
        borderRadius: BorderRadius.circular(16),
        side: BorderSide(color: theme.colorScheme.outlineVariant),
      ),
      child: Padding(
        padding: const EdgeInsets.all(16),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Text('已安装 $cardCount 个插件 · 启用 $activeCount 个',
                style: theme.textTheme.titleMedium),
            const SizedBox(height: 4),
            Text(
              '宿主堆占用约 ${memory.processUsedMb} MB · 后台任务 $taskCount 个 · 内存压力 ${memory.pressure}',
              style: theme.textTheme.bodyMedium
                  ?.copyWith(color: theme.colorScheme.onSurfaceVariant),
            ),
            const SizedBox(height: 4),
            Text(
              '插件后台任务统一由宿主调度；停用或卸载时会被自动取消。',
              style: theme.textTheme.bodySmall
                  ?.copyWith(color: theme.colorScheme.onSurfaceVariant),
            ),
          ],
        ),
      ),
    );
  }
}