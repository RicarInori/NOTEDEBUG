import '../bridge/api.dart';

/// Presentation helpers on the generated bridge types.
///
/// The bridge types are the single definition of each concept — the UI does not keep a parallel
/// model of its own, so these extensions are where UI-only convenience belongs. Making them an
/// extension rather than a wrapper is deliberate: a wrapper would reintroduce the duplication the
/// generated types exist to remove.

extension PluginStateDtoX on PluginStateDto {
  /// Whether the plugin is mounted and working (ADR-007's `Active`).
  bool get isActive => this == PluginStateDto.active;

  String get label => switch (this) {
        PluginStateDto.installed => '未加载',
        PluginStateDto.pending => '等待依赖',
        PluginStateDto.loading => '加载中',
        PluginStateDto.active => '已启用',
        PluginStateDto.failed => '错误',
        PluginStateDto.unloading => '卸载中',
        PluginStateDto.disposed => '已停用',
      };
}

extension PluginCardDtoX on PluginCardDto {
  /// Uninstall applies to anything that arrived from outside the binary — WASM or native.
  /// Internal modules are compiled in and have nothing on disk to remove (ADR-003).
  bool get isUninstallable => kind != PluginKindDto.internal;
}

/// Classifies a picked file the way the host does, so the picker can reject the wrong kind
/// immediately instead of after a round trip. The host remains the authority.
extension PluginPackageNameX on String {
  /// A shared library the native loader can mount (ADR-003 v2.3).
  bool get isNativeLibrary =>
      endsWith('.so') || endsWith('.dll') || endsWith('.dylib');

  /// A WASM package, whose manifest sits beside the code rather than inside it.
  bool get isWasmPackage => endsWith('.wasm') || endsWith('.inkext');

  bool get isPluginPackage => isNativeLibrary || isWasmPackage;
}

extension PermissionsDtoX on PermissionsDto {
  List<String> get labels => [
        if (network) '网络',
        if (filesystem) '文件',
        if (clipboard) '剪贴板',
      ];
}

extension StylusCapabilitiesDtoX on StylusCapabilitiesDto {
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