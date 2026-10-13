import 'package:flutter/material.dart';

import '../../data/plugin_repository.dart';
import '../editor/canvas_host.dart';

/// Document library. A placeholder surface: the real list arrives with `storage-sqlite` in M2,
/// and the search field will be backed by FTS5.
class LibraryScreen extends StatelessWidget {
  const LibraryScreen({super.key, required this.repository});

  final PluginRepository repository;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);

    return Scaffold(
      appBar: AppBar(
        title: const Text('文件库'),
        actions: [
          IconButton(
            onPressed: () => Navigator.of(context).push(
              MaterialPageRoute<void>(builder: (_) => const CanvasHost()),
            ),
            icon: const Icon(Icons.draw_outlined),
            tooltip: '打开手写画布',
          ),
        ],
      ),
      body: ListView(
        padding: const EdgeInsets.fromLTRB(16, 8, 16, 24),
        children: [
          SearchBar(
            hintText: '搜索笔记、标签、手写内容',
            leading: const Icon(Icons.search),
            elevation: const WidgetStatePropertyAll(0),
            onChanged: (_) {},
          ),
          const SizedBox(height: 16),
          Card(
            elevation: 0,
            margin: EdgeInsets.zero,
            shape: RoundedRectangleBorder(
              borderRadius: BorderRadius.circular(16),
              side: BorderSide(color: theme.colorScheme.outlineVariant),
            ),
            child: const ListTile(
              leading: Icon(Icons.info_outline),
              title: Text('文档列表待接入'),
              subtitle: Text('storage-sqlite 落地后，此处显示文件夹、标签与最近文档（M2）'),
            ),
          ),
          const SizedBox(height: 12),
          FilledButton.tonalIcon(
            onPressed: () => Navigator.of(context).push(
              MaterialPageRoute<void>(builder: (_) => const CanvasHost()),
            ),
            icon: const Icon(Icons.brush_outlined),
            label: const Text('新建手写页'),
          ),
        ],
      ),
    );
  }
}