import 'package:flutter/material.dart';

import 'data/frb_plugin_repository.dart';
import 'data/plugin_repository.dart';
import 'modules/library/library_screen.dart';
import 'modules/plugins/control_panel_screen.dart';
import 'modules/settings/settings_screen.dart';
import 'theme/app_theme.dart';

Future<void> main() async {
  WidgetsFlutterBinding.ensureInitialized();

  // The app talks to the Rust host; there is no demo fallback here on purpose. Silently running on
  // demo data would hide a missing or broken native library, which is exactly the failure worth
  // seeing. Tests inject DemoPluginRepository instead.
  final repository = FrbPluginRepository();
  try {
    await repository.bootstrap();
  } catch (error) {
    runApp(HostUnavailableApp(error: error));
    return;
  }

  runApp(InkweaveApp(repository: repository));
}

class InkweaveApp extends StatelessWidget {
  const InkweaveApp({super.key, required this.repository});

  final PluginRepository repository;

  @override
  Widget build(BuildContext context) {
    return MaterialApp(
      title: 'Inkweave',
      debugShowCheckedModeBanner: false,
      theme: AppTheme.light(),
      darkTheme: AppTheme.dark(),
      home: HomeShell(repository: repository),
    );
  }
}

/// Shown when the native host cannot be loaded — usually because the Rust library has not been
/// built for this platform yet. Says what to run rather than leaving a red screen.
class HostUnavailableApp extends StatelessWidget {
  const HostUnavailableApp({super.key, required this.error});

  final Object error;

  @override
  Widget build(BuildContext context) {
    return MaterialApp(
      title: 'Inkweave',
      debugShowCheckedModeBanner: false,
      theme: AppTheme.light(),
      darkTheme: AppTheme.dark(),
      home: Scaffold(
        body: SafeArea(
          child: Padding(
            padding: const EdgeInsets.all(24),
            child: Center(
              child: Column(
                mainAxisSize: MainAxisSize.min,
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Icon(Icons.extension_off_outlined,
                      size: 48, color: Theme.of(context).colorScheme.error),
                  const SizedBox(height: 16),
                  Text('宿主未就绪', style: Theme.of(context).textTheme.titleLarge),
                  const SizedBox(height: 8),
                  const Text('Flutter 层无法加载 Rust 内核，因此没有任何插件可管理。'),
                  const SizedBox(height: 16),
                  Text('$error', style: Theme.of(context).textTheme.bodySmall),
                  const SizedBox(height: 16),
                  const Text('构建原生库：'),
                  const SelectableText(
                    'cargo build -p inkweave_bridge\n'
                    'cd app && flutter_rust_bridge_codegen generate',
                  ),
                ],
              ),
            ),
          ),
        ),
      ),
    );
  }
}

/// Bottom navigation over the three top-level surfaces. Everything plugin-related is reachable
/// from the control panel — there is no second management surface anywhere.
class HomeShell extends StatefulWidget {
  const HomeShell({super.key, required this.repository});

  final PluginRepository repository;

  @override
  State<HomeShell> createState() => _HomeShellState();
}

class _HomeShellState extends State<HomeShell> {
  int _index = 0;

  @override
  Widget build(BuildContext context) {
    final surfaces = [
      LibraryScreen(repository: widget.repository),
      ControlPanelScreen(repository: widget.repository),
      SettingsScreen(repository: widget.repository),
    ];

    return Scaffold(
      body: SafeArea(child: IndexedStack(index: _index, children: surfaces)),
      bottomNavigationBar: NavigationBar(
        selectedIndex: _index,
        onDestinationSelected: (value) => setState(() => _index = value),
        destinations: const [
          NavigationDestination(
            icon: Icon(Icons.library_books_outlined),
            selectedIcon: Icon(Icons.library_books),
            label: '文件库',
          ),
          NavigationDestination(
            icon: Icon(Icons.extension_outlined),
            selectedIcon: Icon(Icons.extension),
            label: '控制面板',
          ),
          NavigationDestination(
            icon: Icon(Icons.settings_outlined),
            selectedIcon: Icon(Icons.settings),
            label: '设置',
          ),
        ],
      ),
    );
  }
}