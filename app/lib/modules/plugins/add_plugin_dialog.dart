import 'package:flutter/material.dart';

import '../../data/models.dart';

/// Permission confirmation for an extension install (§8.4).
///
/// Modelled on Obsidian's progressive permissions: nothing is granted silently, and the dialog
/// lists exactly what the extension asked for. Returns the accepted set, or null if cancelled.
class AddPluginDialog extends StatefulWidget {
  const AddPluginDialog({super.key, required this.packageName});

  /// File name being installed, shown so the user knows what they picked.
  final String packageName;

  @override
  State<AddPluginDialog> createState() => _AddPluginDialogState();
}

class _AddPluginDialogState extends State<AddPluginDialog> {
  bool _network = true;
  bool _filesystem = true;
  bool _clipboard = true;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return AlertDialog(
      title: const Text('添加插件'),
      content: SingleChildScrollView(
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          mainAxisSize: MainAxisSize.min,
          children: [
            Text(widget.packageName, style: theme.textTheme.bodyMedium),
            const SizedBox(height: 16),
            Text('该扩展申请以下权限，请逐项确认：', style: theme.textTheme.bodyMedium),
            CheckboxListTile(
              value: _network,
              onChanged: (value) => setState(() => _network = value ?? false),
              title: const Text('网络'),
              subtitle: const Text('访问其配置的服务器'),
              controlAffinity: ListTileControlAffinity.leading,
              contentPadding: EdgeInsets.zero,
            ),
            CheckboxListTile(
              value: _filesystem,
              onChanged: (value) => setState(() => _filesystem = value ?? false),
              title: const Text('文件'),
              subtitle: const Text('读取你选择交给它的文档'),
              controlAffinity: ListTileControlAffinity.leading,
              contentPadding: EdgeInsets.zero,
            ),
            CheckboxListTile(
              value: _clipboard,
              onChanged: (value) => setState(() => _clipboard = value ?? false),
              title: const Text('剪贴板'),
              subtitle: const Text('读取或写入剪贴板'),
              controlAffinity: ListTileControlAffinity.leading,
              contentPadding: EdgeInsets.zero,
            ),
            const SizedBox(height: 8),
            Text(
              '未勾选的权限会被拒绝，扩展在申请受限能力时将收到错误而不是越权。',
              style: theme.textTheme.bodySmall
                  ?.copyWith(color: theme.colorScheme.onSurfaceVariant),
            ),
          ],
        ),
      ),
      actions: [
        TextButton(
          onPressed: () => Navigator.of(context).pop(),
          child: const Text('取消'),
        ),
        FilledButton(
          onPressed: () => Navigator.of(context).pop(
            Permissions(
              network: _network,
              filesystem: _filesystem,
              clipboard: _clipboard,
            ),
          ),
          child: const Text('确认并安装'),
        ),
      ],
    );
  }
}