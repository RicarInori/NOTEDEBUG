import 'package:flutter/material.dart';

import '../editor/canvas_host.dart';

/// Resolves a plugin's declared contribution to the Flutter screen that renders it.
///
/// Rust declares *what* a plugin contributes — a `ContributionDto` carrying an id, a title and a
/// kind. The screen that renders it lives on the Dart side, because rendering is Flutter's job in
/// this architecture (ADR-001); the kernel has no opinion about widgets.
///
/// So a null result from [lookup] is a **normal** outcome, not an error: a third-party WASM
/// extension declares a contribution point but ships no Dart screen. The host says so plainly
/// rather than presenting a button that does nothing.
///
/// The open question this leaves, and which the guide does not yet answer, is how a third-party
/// contribution should be rendered — a declarative UI schema the host could draw generically would
/// be the natural answer, since §8.3 requires the host to own all management UI.
class ContributionRegistry {
  const ContributionRegistry(this._builders);

  final Map<String, WidgetBuilder> _builders;

  WidgetBuilder? lookup(String contributionId) => _builders[contributionId];

  /// Contributions the first-party plugins provide today. Keys match the ids declared in
  /// `app/rust/src/host/mod.rs`.
  static const standard = ContributionRegistry({
    'stylus.canvas': _canvasHost,
  });
}

Widget _canvasHost(BuildContext context) => const CanvasHost();