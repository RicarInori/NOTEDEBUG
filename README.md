# Inkweave

A Cordis-style, plugin-based, cross-platform handwriting notebook.
**Flutter** for everything that is not handwriting, a **Rust kernel** for everything that is, and a
native surface for the pen itself.

> Design creed: **everything is a plugin · registration is reversible · no privileged core · zero
> compromise on latency.**

The authoritative specification is [`docs/inkweave-dev-guide.md`](docs/inkweave-dev-guide.md)
(v2.1) with the research behind it in [`research/`](research/).

---

## Status: M0 skeleton — Rust and Flutter both verified

Both halves build and are green on Windows:

```text
cargo test --workspace                                  30 passed, 0 failed
cargo clippy --workspace --all-targets -- -D warnings    clean
cargo fmt --all -- --check                               clean
cd app && flutter analyze                                no issues found
cd app && flutter test                                   6 passed
cd app && flutter build apk --debug                      builds, with libinkweave_bridge.so inside
```

### The Flutter app runs the real host

`flutter_rust_bridge_codegen generate` produced `app/rust/src/frb_generated.rs` and
`app/lib/bridge/*.dart`, and those are **committed** — they are the Dart projection of the contract,
so a fresh clone can `flutter analyze`, `flutter test` and `cargo build` without installing the
codegen tool. Regenerate after any change to `app/rust/src/api`, with the frb version pinned exactly
on both sides.

The app has **no demo fallback**: if the native library cannot be loaded it shows a "宿主未就绪"
screen naming the build command, rather than quietly running on fake data. `DemoPluginRepository`
exists only for widget tests.

One structural decision worth knowing: the bridge types from `bridge/api.dart` are used **directly**
by the UI, with UI-only conveniences added as extensions (`plugin_extensions.dart`). There is no
second model to keep in sync — that duplication is exactly what the generator removes.

Two frb details that cost time and are easy to get wrong:

- **`mirror` is not a v2 config key.** A type from another crate can only be brought in with the
  `#[frb(mirror(...))]` attribute, which re-lists every field. The API module therefore declares its
  own wire DTOs and converts — zero config, and the wire shape cannot drift with an internal type.
- **`StreamSink` is generated into your crate, not exported by the library.** It comes from
  `frb_generated_stream_sink!` inside `frb_generated.rs`, so the import is `crate::frb_generated::StreamSink`.
  And frb 2.13 does not recognise an `impl Stream` *return* type — it silently treats the item type
  as unused, which is a confusing failure mode.

Five real defects surfaced during the first build, which is the point of having a build:

| What broke | Why |
| --- | --- |
| `FnPlugin` not reachable at the crate root | Declared in `plugin.rs` but never re-exported from `lib.rs` — four crates failed to compile |
| `Context::child` could not build a parent handle | Took `&self`, but the child needs an `Arc` to its parent; now `self: &Arc<Self>` |
| `thiserror` rejected `Apply { source: String }` | A field named `source` is treated as the error chain and must implement `Error`; renamed to `message` |
| Background work could never be cancelled | `sync-webdav` registered under `plugin:com.inkwell.preset.webdav` while `plugin-manager` cancelled `plugin:sync-webdav`. Both sides now derive the namespace from `inkweave_api::runtime::task_owner` |
| `Context::plugin` was unusable from a trait method | The registry verbs live on `PluginRegistry`, which was not in scope at three call sites |

The namespace mismatch is the reason to build early: in review it would have looked like a working
feature, and would have leaked background sync forever in production.

The control panel's own test caught a subtler thing worth knowing: **a `ListView` never builds
offscreen children**, so an assertion that a disabled plugin's card *moves to the "已停用" section*
cannot be written against the widget tree. The test asserts the registry state and the header count
instead.

| Area | State |
| --- | --- |
| `cordis-rs` kernel — Context tree, `ArcSwap` service table, Fiber six-state machine, LIFO effect recovery, reactive injection | Implemented, with tests |
| `inkweave-api` — Definition layer (`PluginRegistry`, `StylusDriver`, `SyncProvider`, scheduler, memory governor) | Implemented |
| `plugin-manager` — control-panel backend, enable/disable/install/uninstall, log buffers | Implemented |
| `stylus-drivers` — generic stylus provider, capability bitmap, pressure curve | Implemented, with tests |
| `sync-webdav` — six-state sync machine, LWW w/ clock-rollback guard, injectable transport | State machine implemented; **HTTP transport is M4** |
| `nbk-migrate` — `.nbk` version migration chain | Implemented, with tests |
| `ink-engine`, `pdfium-engine`, `storage-sqlite`, `ext-loader` | Placeholder crates (documented scope, no service provided yet) |
| Flutter app — Material 3 control panel, add-plugin entry, settings, library | Implemented, driven by the Rust host |
| `flutter_rust_bridge` lifecycle bridge | **Wired** — codegen 2.13.0, `FrbPluginRepository` calls the host, no demo fallback in the app |
| Rust host inside the APK | **Yes** — `cargo ndk` cross-compiles `libinkweave_bridge.so` for `arm64-v8a` and `x86_64`; the Gradle `buildRustHost` task runs before `preBuild` |
| Plugin enable state | **Persisted** — `HostStateStore` writes `disabled-plugins.txt` under the app support directory and `bootstrap` honours it. Verified on the emulator: disable → force-stop → cold launch still shows the plugin disabled, and deleting the file restores the defaults |
| Contribution entry points | **Wired** — `ContributionRegistry` maps a declared contribution id to a Flutter screen, since rendering is Flutter's job (ADR-001). A contribution with no registered screen is reported plainly instead of showing a dead button |
| Lifecycle state stream (§2.4) | **Wired** — the kernel's ledger notifies a state observer on *every* transition, including the dependency-driven `Pending → Active` and the final `Disposed` after the context cycle is broken; `plugin-manager` broadcasts it and the bridge exposes a Dart `Stream` the panel listens to |
| Third-party contribution UI | **Open question** — the contract declares `{id, title, kind}` and nothing more. §8.3 wants the host to render all management UI, which needs a declarative UI schema in the contract; until then only first-party plugins can contribute a screen |
| Spike-0 latency proof (ADR-002 gate) | **Not started — the gate before M1** |

## Layout

```
inkweave/
├── crates/
│   ├── cordis-rs/        microkernel (Context / Fiber / Effect / events)
│   ├── inkweave-api/     Definition layer: every service trait and data type
│   ├── plugin-manager/   the control panel's backend
│   ├── ink-engine/       stroke pipeline + tools + wgpu canvas      (M1)
│   ├── pdfium-engine/    handle pool + tiles + export               (M3)
│   ├── storage-sqlite/   SQLite + FTS5 + .nbk + undo log            (M2)
│   ├── sync-webdav/      WebDAV provider                            (M4)
│   ├── stylus-drivers/   generic provider + vendor adapter seam
│   ├── ext-loader/       wasmtime sandbox + inkweave ABI            (M6)
│   └── nbk-migrate/      format migration chain                     (M2)
├── app/                  Flutter UI + the frb bridge crate
├── extensions/           first-party WASM extension examples
├── research/             the research pack the guide summarises
└── docs/                 the development guide + ADR index
```

## Building

### Toolchain

Rust is pinned by `rust-toolchain.toml`. The setup details worth knowing on this machine:

| Need | What it is | Why |
| --- | --- | --- |
| `rustup` + Rust **1.99.0** | `rustup-init`, then `rustup toolchain install stable` | The dependency graph resolves to crates requiring `edition2024`, which needs Rust ≥ 1.85; 1.80 fails to even parse `indexmap` |
| An MSVC linker | VS Build Tools 2022 with `VCTools` + a Windows SDK | `rustc`'s default host is `x86_64-pc-windows-msvc`; proc-macro crates must be linked even for `cargo check` |
| crates.io mirror | `~/.cargo/config.toml` → USTC | `index.crates.io` resolves, but `static.crates.io` (the tarballs) does not on this network |
| Flutter **3.47.7** | SDK zip from the **Tencent** mirror | Google's CDN (`storage.googleapis.com`) delivers ~150 KB/s here; the Tencent mirror does ~15 MB/s — 1.9 GB in two minutes instead of two hours |
| JDK **17** | `winget install Microsoft.OpenJDK.17` → `C:\Program Files\Microsoft\jdk-17.0.20.101-hotspot` | Required by the Android Gradle build; `JAVA_HOME` must be a Windows-style path |
| Android SDK | cmdline-tools to `~/Android/Sdk`, then `sdkmanager "platform-tools" "platforms;android-36" "build-tools;36.0.0"` | Flutter 3.47 targets compileSdk/targetSdk **36**; the versions come from `FlutterExtension.kt`, not from this repo |
| `cargo-ndk` + the Rust Android targets | `cargo install cargo-ndk`, `rustup target add aarch64-linux-android x86_64-linux-android` | Cross-compiles the host for the APK; the Gradle task passes the NDK path from `android.ndkDirectory`, so nothing machine-specific is committed |

Flutter also needs its own mirrors for pub and engine artifacts:

```bash
export FLUTTER_STORAGE_BASE_URL=https://mirrors.cloud.tencent.com/flutter
export PUB_HOSTED_URL=https://mirrors.cloud.tencent.com/dart-pub
```

Both `~/.cargo/config.toml` and these variables are machine-local, because the addresses are
network-specific rather than project-specific. The cargo one:

```toml
[source.crates-io]
replace-with = "ustc"

[source.ustc]
registry = "sparse+https://mirrors.ustc.edu.cn/crates.io-index/"
```

If your network reaches `static.crates.io` directly, delete that file and nothing else changes.

### Commands

```bash
# Rust: kernel + tests (the M0 acceptance suite lives in crates/cordis-rs/tests/kernel.rs)
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings

# Flutter UI (runs against the in-memory demo repository out of the box)
cd app
flutter pub get
flutter analyze
flutter run
```

### Waking up the real bridge

The Flutter app ships with an in-memory repository so the UI can be developed before the native side
is wired. To run against the Rust kernel:

```bash
cd app
flutter_rust_bridge_codegen generate      # writes lib/bridge/ and rust/src/frb_generated.rs
# then uncomment `mod frb_generated;` in rust/src/lib.rs
# rename lib/bridge/frb_plugin_repository.dart.template -> .dart
# switch main.dart from DemoPluginRepository() to FrbPluginRepository()
```

### Platform folders

`app/android` is generated (via `flutter create --platforms=android --project-name inkweave .`) so an
APK can be produced on this machine. `app/ios` is deliberately absent — it cannot be generated or
built on Windows, and the guide's iOS path needs a Mac.

```bash
cd app && flutter build apk --debug
```

## Where the six requirements live

| Requirement | Implementation | Guide |
| --- | --- | --- |
| In-app "add plugin" entry | `app/lib/modules/plugins/control_panel_screen.dart` → `AddPluginDialog`; backend `PluginManager::install` | §8.4 |
| Hot load **and** shutdown | four verbs `Load / Enable / Disable / Unload` over the Fiber machine; `api::set_plugin_enabled` | §8.2, ADR-007 |
| Unified control panel | `plugin-manager` crate + `control_panel_screen.dart`; one management path | §8.3, ADR-008 |
| Material Design | `app/lib/theme/app_theme.dart`; every surface and plugin contribution renders with M3 components | ADR-006 |
| Stylus + WebDAV as toggleable preset plugins | `stylus-drivers`, `sync-webdav` — mounted and toggled like any third-party plugin | §8.7 |
| Background + memory management | `HostBackgroundScheduler`, `HostMemoryGovernor` in `app/rust/src/host/`; `BackgroundScheduler` / `MemoryGovernor` traits | §8.5, §8.6 |

## Two things worth knowing before you read the code

**"Hot load" does not mean hot-swapping code.** ADR-007 splits *dynamism* — can a plugin be mounted,
unmounted, enabled, disabled at runtime? — from *hot replacement* — can loaded code be swapped
in place? The first is core, the second is explicitly refused. Changing an internal module's version
means shipping the host; changing an extension's version means uninstall then install.

**Unload is only as deterministic as the garbage collector.** Disposing a Fiber revokes every effect
it registered, cancels its background work and drops the plugin instance. What it cannot do is force
the allocator to reclaim memory at that instant; that is why a deterministic-unload requirement would
push a plugin into its own process (roadmap M2).

## Licence

MIT OR Apache-2.0. Note the hard constraint from the research round: no AGPL dependency, which is why
MuPDF was rejected in favour of PDFium.