import 'package:flutter/material.dart';

import '../../bridge/api.dart';

/// Permission confirmation for an extension install (§8.4).
///
/// Modelled on Obsidian's progressive permissions: nothing is granted silently, and the dialog
/// lists exactly what the extension asked for. Returns the accepted set, or null if cancelled.
///
/// [manifest] is what a *native* plugin declares inside its own binary. When it is available the
/// dialog shows the plugin's identity and prefills the boxes from its real request, so the user
/// answers the question the plugin actually asked. A WASM package carries its manifest beside the
/// code and is not probed by this flow, so there the boxes start checked and the host refuses the
/// install if the manifest asked for more than was accepted.
class AddPluginDialog extends StatefulWidget {
  const AddPluginDialog({super.key, required this.packageName, this.manifest});

  /// File name being installed, shown so the user knows what they picked.
  final String packageName;

  /// The plugin's own declared permissions, when the host could read them before install.
  final NativeManifestDto? manifest;

  @override
  State<AddPluginDialog> createState() => _AddPluginDialogState();
}

class _AddPluginDialogState extends State<AddPluginDialog> {
  late bool _network;
  late bool _filesystem;
  late bool _clipboard;

  @override
  void initState() {
    super.initState();
    final declared = widget.manifest?.permissions;
    _network = declared?.network ?? true;
    _filesystem = declared?.filesystem ?? true;
    _clipboard = declared?.clipboard ?? true;
  }

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final manifest = widget.manifest;
    final declared = manifest?.permissions;
    final asksForNothing = declared != null &&
        !declared.network &&
        !declared.filesystem &&
        !declared.clipboard;

    return AlertDialog(
      title: const Text('添加插件'),
      content: SingleChildScrollView(
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          mainAxisSize: MainAxisSize.min,
          children: [
            Text(
              manifest?.name ?? widget.packageName,
              style: theme.textTheme.titleSmall,
            ),
            if (manifest != null) ...[
              const SizedBox(height: 2),
              Text(
                '${widget.packageName} · v${manifest.version}'
                '${manifest.author.isEmpty ? '' : ' · ${manifest.author}'}',
                style: theme.textTheme.bodySmall
                    ?.copyWith(color: theme.colorScheme.onSurfaceVariant),
              ),
              if (manifest.description.isNotEmpty) ...[
                const SizedBox(height: 8),
                Text(manifest.description, style: theme.textTheme.bodyMedium),
              ],
            ] else ...[
              const SizedBox(height: 2),
              Text(widget.packageName, style: theme.textTheme.bodySmall),
            ],
            const SizedBox(height: 16),
            Text(
              asksForNothing ? '该扩展不申请任何权限。' : '该扩展申请以下权限，请逐项确认：',
              style: theme.textTheme.bodyMedium,
            ),
            if (!asksForNothing) ...[
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
            ],
            const SizedBox(height: 8),
            Text(
              '未勾选的权限会被拒绝，扩展在申请受限能力时将收到错误而不是越权。',
              style: theme.textTheme.bodySmall
                  ?.copyWith(color: theme.colorScheme.onSurfaceVariant),
            ),
            if (manifest != null) ...[
              const SizedBox(height: 8),
              Text(
                '原生插件与宿主同进程运行，权限清单是告知而非隔离边界，请只从可信来源导入。',
                style: theme.textTheme.bodySmall
                    ?.copyWith(color: theme.colorScheme.error),
              ),
            ],
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
            PermissionsDto(
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