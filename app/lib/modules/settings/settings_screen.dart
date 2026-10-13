import 'package:flutter/foundation.dart' show kDebugMode;
import 'package:flutter/material.dart';

import '../../bridge/api.dart';
import '../../data/plugin_repository.dart';
import '../debug/latency_probe.dart';

/// Settings, including the read-outs that make the background and memory design observable
/// (§8.5, §8.6). Plugin configuration itself lives in each plugin's contributed view, reachable
/// from the control panel — this screen is the host's own surface.
class SettingsScreen extends StatefulWidget {
  const SettingsScreen({super.key, required this.repository});

  final PluginRepository repository;

  @override
  State<SettingsScreen> createState() => _SettingsScreenState();
}

class _SettingsScreenState extends State<SettingsScreen> {
  MemorySummary _memory =
      const MemorySummary(processUsedMb: 0, pressure: 'none', overBudget: []);
  List<TaskSummary> _tasks = const [];

  @override
  void initState() {
    super.initState();
    _load();
  }

  Future<void> _load() async {
    final memory = await widget.repository.memory();
    final tasks = await widget.repository.tasks();
    if (!mounted) return;
    setState(() {
      _memory = memory;
      _tasks = tasks;
    });
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      appBar: AppBar(title: const Text('设置')),
      body: ListView(
        padding: const EdgeInsets.fromLTRB(16, 8, 16, 24),
        children: [
          const _Section(
            title: '插件运行时',
            children: [
              ListTile(
                leading: Icon(Icons.hub_outlined),
                title: Text('契约版本'),
                subtitle: Text('inkweave-api 0.1.0 · inkweave_abi_v1'),
              ),
              ListTile(
                leading: Icon(Icons.bolt_outlined),
                title: Text('装载语义'),
                subtitle: Text('热加载/关闭 = 运行时装、卸与启停；不做代码热替换（ADR-007）'),
              ),
              Divider(height: 1),
            ],
          ),
          _Section(
            title: '后台机制',
            children: [
              ListTile(
                leading: const Icon(Icons.schedule_outlined),
                title: const Text('已注册后台任务'),
                subtitle: Text(_tasks.isEmpty
                    ? '暂无'
                    : _tasks
                        .map((task) =>
                            '${task.owner} · ${task.task}${task.periodic ? '（每 ${task.intervalMinutes} 分钟${task.requiresUnmetered ? '，仅 Wi-Fi' : ''}）' : ''}')
                        .join('\n')),
              ),
              const ListTile(
                leading: Icon(Icons.power_settings_new_outlined),
                title: Text('取消策略'),
                subtitle: Text('插件停用或卸载时，宿主自动取消其名下全部任务'),
              ),
              const Divider(height: 1),
            ],
          ),
          _Section(
            title: '内存管理',
            children: [
              ListTile(
                leading: const Icon(Icons.memory_outlined),
                title: const Text('宿主堆占用'),
                subtitle: Text('约 ${_memory.processUsedMb} MB · 当前压力 ${_memory.pressure}'),
              ),
              const ListTile(
                leading: Icon(Icons.compress_outlined),
                title: Text('压力响应'),
                subtitle: Text('系统内存告警时广播给插件释放缓存；极度紧张时卸载空闲插件'),
              ),
              const Divider(height: 1),
            ],
          ),
          if (kDebugMode)
            _Section(
              title: '诊断（仅调试构建）',
              children: [
                ListTile(
                  leading: const Icon(Icons.speed_outlined),
                  title: const Text('手写延迟探针'),
                  subtitle: const Text('测帧内管线成本（vsync→栅格完成）；输入延迟须走 Perfetto，见 ADR-002 补充'),
                  onTap: () => Navigator.of(context).push(
                    MaterialPageRoute<void>(builder: (_) => const LatencyProbeScreen()),
                  ),
                ),
                const Divider(height: 1),
              ],
            ),
          const _Section(
            title: '外观',
            children: [
              ListTile(
                leading: Icon(Icons.palette_outlined),
                title: Text('设计语言'),
                subtitle: Text('Material Design 3（ADR-006），暗色模式随系统'),
              ),
            ],
          ),
        ],
      ),
    );
  }
}

class _Section extends StatelessWidget {
  const _Section({required this.title, required this.children});

  final String title;
  final List<Widget> children;

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.only(bottom: 20),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Padding(
            padding: const EdgeInsets.only(left: 4, bottom: 4),
            child: Text(title, style: Theme.of(context).textTheme.titleSmall),
          ),
          Card(
            elevation: 0,
            margin: EdgeInsets.zero,
            shape: RoundedRectangleBorder(
              borderRadius: BorderRadius.circular(16),
              side: BorderSide(color: Theme.of(context).colorScheme.outlineVariant),
            ),
            child: Column(children: children),
          ),
        ],
      ),
    );
  }
}