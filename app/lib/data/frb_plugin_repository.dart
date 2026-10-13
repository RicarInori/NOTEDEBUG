import 'package:path_provider/path_provider.dart';

import '../bridge/api.dart';
import '../bridge/frb_generated.dart' show RustLib;
import 'plugin_repository.dart';

/// The real [PluginRepository]: a thin delegate over the Rust host.
///
/// Everything the control panel can do goes through these functions, and each one lands on the same
/// `PluginRegistry` the kernel uses internally — the UI is not a second, parallel management model.
class FrbPluginRepository implements PluginRepository {
  @override
  Future<void> bootstrap() async {
    // Loads the native library and starts the Dart-side bridge. Must happen before any other call.
    await RustLib.init();

    // The host owns its subdirectories (plugin registry state, per-plugin private data, and later
    // the extension packages) beneath this directory.
    final supportDir = await getApplicationSupportDirectory();
    await bootstrapHost(dataDir: supportDir.path);
  }

  @override
  Future<List<PluginCardDto>> cards() => pluginCards();

  @override
  Future<void> setEnabled(String id, bool enabled) => setPluginEnabled(id: id, enabled: enabled);

  @override
  Future<PluginCardDto> install(String packagePath, PermissionsDto accepted) => installExtension(
        packagePath: packagePath,
        network: accepted.network,
        filesystem: accepted.filesystem,
        clipboard: accepted.clipboard,
      );

  @override
  Future<void> uninstall(String id) => uninstallExtension(id: id);

  @override
  Future<List<String>> log(String id, {int lines = 40}) => pluginLog(id: id, lines: lines);

  @override
  Future<MemorySummary> memory() => memorySummary();

  @override
  Future<List<TaskSummary>> tasks() => schedulerTasks();
}