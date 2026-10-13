//! End-to-end test of the native channel: build the example `cdylib`, open it through the loader,
//! mount it, and check that the host table reached the plugin and its lifecycle ran.
//!
//! This is the test that would catch a layout mistake in the ABI: nothing here inspects Rust types
//! shared between host and plugin — the evidence is a file the *plugin* wrote into the directory the
//! *host* handed it, which only works if the table survived the boundary intact.

use cordis_rs::{Context, FiberState};
use ext_loader::native::NativeLibrary;
use std::path::PathBuf;
use std::process::Command;

fn artifact_path() -> PathBuf {
    // <target>/debug/deps/native_load-<hash>(.exe) -> <target>/debug
    let exe = std::env::current_exe().expect("test executable path");
    let debug_dir = exe
        .parent()
        .and_then(|deps| deps.parent())
        .expect("the test binary lives in <target>/debug/deps");

    let name = if cfg!(target_os = "windows") {
        "example_native.dll"
    } else if cfg!(target_os = "macos") {
        "libexample_native.dylib"
    } else {
        "libexample_native.so"
    };
    debug_dir.join(name)
}

/// The example plugin is a workspace member, so a workspace-wide test run has already built it. A
/// single-crate run has not, and then it is built here.
fn example_plugin() -> PathBuf {
    let artifact = artifact_path();
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

fn scratch_dir(tag: &str) -> PathBuf {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    let dir = std::env::temp_dir().join(format!("inkweave-native-{tag}-{stamp}"));
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

#[tokio::test]
async fn the_host_table_survives_the_boundary_and_the_lifecycle_runs() {
    let artifact = example_plugin();
    let storage = scratch_dir("lifecycle");

    let library = NativeLibrary::open(&artifact).expect("open the plugin");
    assert_eq!(library.manifest().id, "com.inkweave.example.native");
    assert_eq!(library.manifest().version, "0.1.0");
    assert_eq!(
        library.manifest().permissions,
        inkweave_api::registry::Permissions {
            network: false,
            filesystem: true,
            clipboard: false,
        },
        "the confirmation dialog prefills its checkboxes from these booleans"
    );
    assert_eq!(
        library.manifest().permissions.labels(),
        vec!["文件"],
        "and renders them as labels without a second mapping"
    );

    let plugin = library
        .into_plugin(storage.clone())
        .expect("wrap as a plugin");

    let root = Context::root();
    let fiber = root.plugin(plugin.clone()).await.expect("mount");
    assert_eq!(fiber.state(), FiberState::Active);

    // The plugin wrote this by reading `files_dir` out of the table it was handed.
    let marker = storage.join("lifecycle.log");
    let after_enable = std::fs::read_to_string(&marker)
        .unwrap_or_else(|error| panic!("the plugin did not write {}: {error}", marker.display()));
    assert!(
        after_enable.contains("enabled"),
        "`create` + `enable` must have run, got {after_enable:?}"
    );

    // Unloading disposes the fiber, which releases the plugin the kernel holds; the test holds a
    // second reference, so the adapter is dropped here rather than inside `unload`.
    root.unload(&fiber).await;
    assert_eq!(fiber.state(), FiberState::Disposed);
    drop(plugin);

    let after_drop = std::fs::read_to_string(&marker).expect("marker still readable");
    assert!(
        after_drop.contains("disabled"),
        "`disable` must run when the host releases the plugin, got {after_drop:?}"
    );
    assert!(
        after_drop.contains("destroyed"),
        "`destroy` must run too, and exactly once, got {after_drop:?}"
    );
}

#[test]
fn opening_something_that_is_not_a_library_reports_a_load_error() {
    let path = scratch_dir("garbage").join("not-a-plugin.bin");
    std::fs::write(&path, b"this is not a shared library").expect("write the decoy");

    let error = NativeLibrary::open(&path).expect_err("must refuse");
    assert!(
        matches!(error, ext_loader::native::LoadError::Open { .. }),
        "expected an open failure, got {error:?}"
    );
}

/// The probe is what feeds the confirmation dialog, so it must report exactly the manifest the
/// mount path will use — a divergence here would mean the user consents to one thing and runs
/// another.
#[test]
fn probing_reports_the_same_manifest_the_mount_path_uses() {
    let artifact = example_plugin();

    let probed = NativeLibrary::probe(&artifact).expect("probe the plugin");
    let mounted = NativeLibrary::open(&artifact).expect("open the plugin");
    assert_eq!(
        probed,
        mounted.manifest().clone(),
        "the dialog must show the manifest the host will enforce"
    );

    // A file that is not a library must be refused here too, before the user is asked anything.
    let error = NativeLibrary::probe(&scratch_dir("probe").join("missing.so")).expect_err("refuse");
    assert!(
        matches!(error, ext_loader::native::LoadError::Open { .. }),
        "expected an open failure, got {error:?}"
    );
}
