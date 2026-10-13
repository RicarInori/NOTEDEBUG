import 'package:flutter/material.dart';

/// Material 3 theme for Inkweave (ADR-006).
///
/// Every screen — library, settings, toolbar, the unified control panel and any plugin-contributed
/// view — takes its colours and shapes from here. Plugins never ship their own styling; their
/// contributions are rendered by the host with these components.
class AppTheme {
  const AppTheme._();

  /// Seed colour. Replace with `ColorScheme.fromSeed(seedColor: ..., dynamicSchemeVariant: ...)`
  /// plus the `dynamic_color` package once Android 12+ system colours are wired up.
  static const Color seed = Color(0xFF3A5BA0);

  static ThemeData light() => _base(Brightness.light);

  static ThemeData dark() => _base(Brightness.dark);

  static ThemeData _base(Brightness brightness) {
    final scheme = ColorScheme.fromSeed(seedColor: seed, brightness: brightness);
    return ThemeData(
      useMaterial3: true,
      colorScheme: scheme,
      scaffoldBackgroundColor: scheme.surface,
      appBarTheme: AppBarTheme(
        backgroundColor: scheme.surface,
        surfaceTintColor: scheme.surfaceTint,
        centerTitle: false,
      ),
      navigationBarTheme: NavigationBarThemeData(
        backgroundColor: scheme.surface,
        indicatorColor: scheme.secondaryContainer,
      ),
      // Touch targets stay at or above 48dp, and the text theme respects the system scale.
      listTileTheme: const ListTileThemeData(minVerticalPadding: 8),
    );
  }
}