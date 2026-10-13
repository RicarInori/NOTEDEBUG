//! End-to-end test of the *import* flow, not just the loader: the user picks a file, the host
//! probes it, copies it into its private storage, mounts it, and can disable and uninstall it again.
//!
//! The discriminator throughout is a file the **plugin** wrote into the directory the **host**
//! handed it. That only appears if the picked `.so`/`.dll` was really loaded, handed a host table,
//! and driven through its lifecycle — so nothing here can pass by mocking the loader.

use cordis_rs::Context;
use inkweave_api::registry::{
    InstallRequest, Permissions, PluginInfo, PluginKind, PluginRegistry, PluginRuntimeState,
    RegistryError,
};
use plugin_manager::PluginManager;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

const PLUGIN_ID: &str = "com.inkweave.example.native";

fn artifact_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "example_native.dll"
    } else if cfg!(target_os = "macos") {
        "libexample_native.dylib"
    } else {
        "libexample_native.so"
    }
}

/// The example plugin is a workspace member, so a workspace-wide test run has already built it.
fn example_plugin() -> PathBuf {
    let exe = std::env::current_exe().expect("test executable path");
    let debug_dir = exe
        .parent()
        .and_then(Path::parent)
        .expect("the test binary lives in <target>/debug/deps");
    let artifact = debug_dir.join(artifact_name());

    if !artifact.exists() {
        let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
        let status = Command::new(cargo)
            .args(["build", "-p", "example-native"])
            .status()
            .expect("run cargo build for the example plugin");
        assert!(status.success(), "building example-native failed");
    }
    assert!(
        artifact.exists(),
        "the example cdylib should be at {}",
        artifact.display()
    );
    artifact
}

struct Fixture {
    manager: Arc<PluginManager>,
    extensions_dir: PathBuf,
    picked: PathBuf,
}

impl Fixture {
    fn new(tag: &str) -> Self {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default();
        let extensions_dir = std::env::temp_dir().join(format!("inkweave-ext-{tag}-{stamp}"));
        std::fs::create_dir_all(&extensions_dir).expect("create extensions dir");

        let manager = PluginManager::new(Context::root(), extensions_dir.clone());
        Self {
            manager,
            extensions_dir,
            picked: example_plugin(),
        }
    }

    fn plugin_dir(&self) -> PathBuf {
        self.extensions_dir.join(PLUGIN_ID)
    }

    /// Where the plugin writes its markers: the `files_dir` the host handed it through the ABI.
    fn marker(&self) -> PathBuf {
        self.plugin_dir().join("files").join("lifecycle.log")
    }

    fn card(&self) -> PluginInfo {
        self.manager
            .snapshot()
            .into_iter()
            .find(|card| card.id == PLUGIN_ID)
            .expect("the plugin is registered")
    }

    fn request(&self, accepted: Permissions) -> InstallRequest {
        InstallRequest {
            package_path: self.picked.to_string_lossy().to_string(),
            accepted,
        }
    }
}

fn all_permissions() -> Permissions {
    Permissions {
        network: true,
        filesystem: true,
        clipboard: true,
    }
}

#[tokio::test]
async fn importing_a_native_plugin_copies_it_mounts_it_and_uninstalls_cleanly() {
    let fixture = Fixture::new("roundtrip");

    // 1. The probe the dialog is fed by must agree with what install will mount.
    let probed = fixture
        .manager
        .probe_native(&fixture.picked)
        .expect("probe");
    assert_eq!(probed.id, PLUGIN_ID);
    assert!(probed.permissions.filesystem);
    assert!(!fixture.plugin_dir().exists(), "probing must not write");

    // 2. Install: copy into private storage, validate the copy, mount.
    let info = fixture
        .manager
        .install(fixture.request(all_permissions()))
        .await
        .expect("install the plugin");
    assert_eq!(info.kind, PluginKind::Native);
    assert_eq!(info.state, PluginRuntimeState::Installed);
    assert!(info.is_uninstallable(), "an imported plugin can be removed");

    let copied = fixture
        .plugin_dir()
        .join(format!("plugin.{}", library_suffix()));
    assert!(
        copied.exists(),
        "the package must be copied into private storage"
    );
    assert_ne!(
        copied, fixture.picked,
        "the mounted artifact must be the host's copy, not the picker's file"
    );
    assert_eq!(
        fixture.card().state,
        PluginRuntimeState::Active,
        "install ends with the plugin hot-loaded"
    );

    // The plugin wrote this by reading `files_dir` out of the host table it was handed.
    let enabled = std::fs::read_to_string(fixture.marker())
        .unwrap_or_else(|error| panic!("the plugin wrote no marker: {error}"));
    assert!(
        enabled.contains("enabled"),
        "`create`+`enable` ran: {enabled:?}"
    );

    // 3. Disable, then enable again — the switch in the control panel, over a native library.
    fixture.manager.disable(PLUGIN_ID).await.expect("disable");
    assert_eq!(fixture.card().state, PluginRuntimeState::Installed);
    fixture.manager.enable(PLUGIN_ID).await.expect("re-enable");
    assert_eq!(fixture.card().state, PluginRuntimeState::Active);

    let cycled = std::fs::read_to_string(fixture.marker()).expect("marker");
    assert_eq!(
        cycled.matches("enabled").count(),
        2,
        "a disable/enable cycle re-opens the library: {cycled:?}"
    );
    assert!(
        cycled.contains("disabled"),
        "`disable` ran on unload: {cycled:?}"
    );

    // 4. Uninstall: stop it and delete both the package and its private data.
    fixture
        .manager
        .uninstall(PLUGIN_ID)
        .await
        .expect("uninstall");
    assert!(
        !fixture.plugin_dir().exists(),
        "uninstall must remove the plugin's directory"
    );
    assert!(
        fixture
            .manager
            .snapshot()
            .iter()
            .all(|card| card.id != PLUGIN_ID),
        "no card may survive the uninstall"
    );
}

#[tokio::test]
async fn a_withheld_permission_refuses_the_install_and_leaves_nothing_behind() {
    let fixture = Fixture::new("denied");

    // The example declares `filesystem`; accepting nothing must refuse rather than run it narrowed.
    let error = fixture
        .manager
        .install(fixture.request(Permissions::NONE))
        .await
        .expect_err("must refuse a permission the user withheld");
    assert!(
        matches!(error, RegistryError::PermissionDenied(_)),
        "expected a permission error, got {error:?}"
    );
    assert!(
        !fixture.plugin_dir().exists(),
        "a refused install must not leave a half-copied package on disk"
    );
}

#[tokio::test]
async fn installing_a_file_that_is_not_a_plugin_reports_a_bad_package() {
    let fixture = Fixture::new("decoy");
    let decoy = fixture.extensions_dir.join("decoy.so");
    std::fs::write(&decoy, b"not a shared library").expect("write the decoy");

    let error = fixture
        .manager
        .install(InstallRequest {
            package_path: decoy.to_string_lossy().to_string(),
            accepted: all_permissions(),
        })
        .await
        .expect_err("must refuse");
    assert!(
        matches!(error, RegistryError::InvalidPackage(_)),
        "expected a bad-package error, got {error:?}"
    );
}

#[tokio::test]
async fn a_file_type_the_host_does_not_recognise_is_refused_by_name() {
    let fixture = Fixture::new("unknown");
    let error = fixture
        .manager
        .install(InstallRequest {
            package_path: fixture
                .extensions_dir
                .join("plugin.zip")
                .to_string_lossy()
                .to_string(),
            accepted: all_permissions(),
        })
        .await
        .expect_err("must refuse");
    assert!(
        matches!(error, RegistryError::InvalidPackage(_)),
        "expected a bad-package error, got {error:?}"
    );
}

fn library_suffix() -> &'static str {
    if cfg!(target_os = "windows") {
        "dll"
    } else if cfg!(target_os = "macos") {
        "dylib"
    } else {
        "so"
    }
}
