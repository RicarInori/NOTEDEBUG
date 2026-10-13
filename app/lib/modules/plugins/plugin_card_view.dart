import 'package:flutter/material.dart';

import '../../bridge/api.dart';
import '../../data/plugin_extensions.dart';

/// One plugin row in the control panel (§8.3).
///
/// Everything shown comes from the registry snapshot; the card never infers state, and it never
/// offers an action the host would reject — uninstall is hidden for internal modules.
class PluginCardView extends StatelessWidget {
  const PluginCardView({
    super.key,
    required this.card,
    required this.onToggle,
    required this.onUninstall,
    required this.onShowLog,
    this.onOpenContribution,
  });

  final PluginCardDto card;
  final ValueChanged<bool> onToggle;
  final VoidCallback onUninstall;
  final VoidCallback onShowLog;
  final ValueChanged<ContributionDto>? onOpenContribution;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final scheme = theme.colorScheme;

    return Card(
      elevation: 0,
      margin: EdgeInsets.zero,
      shape: RoundedRectangleBorder(
        borderRadius: BorderRadius.circular(16),
        side: BorderSide(color: scheme.outlineVariant),
      ),
      child: Padding(
        padding: const EdgeInsets.fromLTRB(16, 12, 8, 12),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Row(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Icon(_icon(card), size: 28, color: scheme.primary),
                const SizedBox(width: 12),
                Expanded(
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      Text(card.name, style: theme.textTheme.titleMedium),
                      const SizedBox(height: 2),
                      Text(
                        'v${card.version} · ${_channel(card.kind)}',
                        style: theme.textTheme.bodySmall
                            ?.copyWith(color: scheme.onSurfaceVariant),
                      ),
                    ],
                  ),
                ),
                Switch(
                  value: card.state.isActive,
                  onChanged: onToggle,
                ),
              ],
            ),
            if (card.description.isNotEmpty) ...[
              const SizedBox(height: 8),
              Text(card.description, style: theme.textTheme.bodyMedium),
            ],
            const SizedBox(height: 10),
            Wrap(
              spacing: 8,
              runSpacing: 4,
              children: [
                _Badge(label: card.state.label, emphasised: card.state.isActive),
                if (card.backgroundTasks > 0)
                  _Badge(label: '后台任务 ${card.backgroundTasks}'),
                if (card.memoryBudgetMb > 0)
                  _Badge(label: '内存 ${card.memoryUsedMb}/${card.memoryBudgetMb} MB'),
                for (final permission in card.permissions.labels)
                  _Badge(label: '权限：$permission'),
                for (final capability in card.capabilities.labels)
                  _Badge(label: capability),
              ],
            ),
            if (card.lastError != null) ...[
              const SizedBox(height: 8),
              Text(
                card.lastError!,
                style: theme.textTheme.bodySmall?.copyWith(color: scheme.error),
                maxLines: 3,
                overflow: TextOverflow.ellipsis,
              ),
            ],
            if (card.contributions.isNotEmpty) ...[
              const Divider(height: 20),
              Wrap(
                spacing: 8,
                children: [
                  for (final contribution in card.contributions)
                    TextButton(
                      onPressed: onOpenContribution == null
                          ? null
                          : () => onOpenContribution!(contribution),
                      child: Text('打开「${contribution.title}」'),
                    ),
                ],
              ),
            ],
            Row(
              children: [
                const Spacer(),
                TextButton(onPressed: onShowLog, child: const Text('查看日志')),
                if (card.isUninstallable)
                  IconButton(
                    onPressed: onUninstall,
                    icon: const Icon(Icons.delete_outline),
                    tooltip: '卸载 ${card.name}',
                  ),
              ],
            ),
          ],
        ),
      ),
    );
  }

  /// Which channel the plugin arrived through. Worth showing: a native extension runs in-process
  /// without a sandbox, so "third-party" alone would hide the distinction that matters (§8.4).
  static String _channel(PluginKindDto kind) => switch (kind) {
        PluginKindDto.internal => '预置',
        PluginKindDto.wasm => 'WASM 扩展',
        PluginKindDto.native => '原生扩展',
      };

  static IconData _icon(PluginCardDto card) {
    if (card.id.contains('stylus')) return Icons.draw_outlined;
    if (card.id.contains('sync')) return Icons.cloud_outlined;
    if (card.id.contains('manager')) return Icons.tune;
    if (card.id.contains('pdf')) return Icons.picture_as_pdf_outlined;
    if (card.id.contains('storage') || card.id.contains('nbk')) {
      return Icons.folder_outlined;
    }
    return Icons.extension_outlined;
  }
}

class _Badge extends StatelessWidget {
  const _Badge({required this.label, this.emphasised = false});

  final String label;
  final bool emphasised;

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    return Container(
      padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 4),
      decoration: BoxDecoration(
        color: emphasised ? scheme.secondaryContainer : scheme.surfaceContainerHighest,
        borderRadius: BorderRadius.circular(8),
      ),
      child: Text(label, style: Theme.of(context).textTheme.labelMedium),
    );
  }
}