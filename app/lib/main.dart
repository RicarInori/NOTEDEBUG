import 'package:flutter/material.dart';

import 'data/plugin_repository.dart';
import 'modules/library/library_screen.dart';
import 'modules/plugins/control_panel_screen.dart';
import 'modules/settings/settings_screen.dart';
import 'theme/app_theme.dart';

void main() {
  WidgetsFlutterBinding.ensureInitialized();

  // Until `flutter_rust_bridge_codegen generate` has run, the UI is driven by the in-memory
  // repository so it can be developed and reviewed without the native host. Swap in
  // `FrbPluginRepository()` (see data/frb_plugin_repository.dart) to run against the Rust kernel:
  //
  //   final repository = FrbPluginRepository();
  //   await repository.bootstrap();
  //
  final PluginRepository repository = DemoPluginRepository();
  repository.bootstrap();

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