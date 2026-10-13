//! Persisted host state.
//!
//! Deliberately minimal: the only thing the host remembers across restarts today is which preset
//! plugins the user turned off. Keeping it as a plain newline-delimited file means no dependency and
//! no schema to migrate; if this grows past a handful of keys it should move to the same store the
//! real settings live in.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// What the host remembers about plugin enablement across restarts.
///
/// **Both directions are recorded, and that is the point.** The presets are enabled at bootstrap, so
/// remembering only the ones the user turned *off* was enough for them — but a plugin that is not a
/// preset (the note engine, an imported extension) is off at bootstrap, and if only refusals were
/// remembered then a user who turned it on would find it off again at every launch. The choice the
/// control panel offers has to survive, or it is not a choice.
pub struct HostStateStore {
    disabled_path: PathBuf,
    enabled_path: PathBuf,
}

impl HostStateStore {
    /// `data_dir` is the application support directory the platform layer hands to `bootstrap`.
    pub fn new(data_dir: &Path) -> Self {
        Self {
            disabled_path: data_dir.join("disabled-plugins.txt"),
            enabled_path: data_dir.join("enabled-plugins.txt"),
        }
    }

    /// Plugin ids the user turned off. A missing or unreadable file means "nothing disabled".
    pub fn disabled_plugins(&self) -> BTreeSet<String> {
        read_ids(&self.disabled_path)
    }

    /// Plugin ids the user turned on beyond the presets.
    pub fn enabled_plugins(&self) -> BTreeSet<String> {
        read_ids(&self.enabled_path)
    }

    /// Record a choice. Turning a plugin on also clears any earlier refusal, and vice versa: the two
    /// files describe one state, and a plugin could otherwise be in both.
    pub fn set_remembered(&self, id: &str, enabled: bool) {
        let (add_to, remove_from) = if enabled {
            (&self.enabled_path, &self.disabled_path)
        } else {
            (&self.disabled_path, &self.enabled_path)
        };
        let mut ids = read_ids(add_to);
        let added = ids.insert(id.to_string());
        write_ids(remove_from, &{
            let mut others = read_ids(remove_from);
            others.remove(id);
            others
        });
        if added {
            write_ids(add_to, &ids);
        }
    }

    /// Forget a plugin entirely — used when it is uninstalled, so a stale id cannot make the next
    /// bootstrap try to enable something that is no longer there.
    pub fn forget(&self, id: &str) {
        for path in [&self.disabled_path, &self.enabled_path] {
            let mut ids = read_ids(path);
            if ids.remove(id) {
                write_ids(path, &ids);
            }
        }
    }
}

/// Ids from a state file. Best effort: an unreadable file means "nothing remembered" rather than a
/// failed launch.
fn read_ids(path: &Path) -> BTreeSet<String> {
    let Ok(contents) = std::fs::read_to_string(path) else {
        return BTreeSet::new();
    };
    contents
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(str::to_string)
        .collect()
}

/// Best effort for the same reason: a user who made a choice should not see the app fail because the
/// note could not be written.
fn write_ids(path: &Path, ids: &BTreeSet<String>) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let body: String = ids.iter().map(|id| format!("{id}\n")).collect();
    let _ = std::fs::write(path, body);
}

/// Restart-surviving state, also used by the bridge to answer "is this plugin one the user
/// deliberately turned off?".
#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(tag: &str) -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or_default();
        let dir = std::env::temp_dir().join(format!("inkweave-state-{tag}-{stamp}"));
        std::fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    #[test]
    fn a_missing_file_means_nothing_is_disabled() {
        let store = HostStateStore::new(&temp_dir("missing"));
        assert!(store.disabled_plugins().is_empty());
    }

    #[test]
    fn turning_a_plugin_on_is_remembered_and_survives_a_reopen() {
        let dir = temp_dir("enabled");
        let store = HostStateStore::new(&dir);

        store.set_remembered("ink-engine", true);
        let reopened = HostStateStore::new(&dir);
        assert!(
            reopened.enabled_plugins().contains("ink-engine"),
            "a plugin the user turned on must still be on next launch"
        );
        assert!(reopened.disabled_plugins().is_empty());
    }

    #[test]
    fn a_plugin_is_never_in_both_lists() {
        let dir = temp_dir("exclusive");
        let store = HostStateStore::new(&dir);

        store.set_remembered("sync-webdav", true);
        store.set_remembered("sync-webdav", false);
        assert!(!store.enabled_plugins().contains("sync-webdav"));
        assert!(store.disabled_plugins().contains("sync-webdav"));

        store.set_remembered("sync-webdav", true);
        assert!(!store.disabled_plugins().contains("sync-webdav"));
        assert!(store.enabled_plugins().contains("sync-webdav"));
    }

    #[test]
    fn forgetting_clears_both_directions() {
        let dir = temp_dir("forget");
        let store = HostStateStore::new(&dir);
        store.set_remembered("com.example.ext", true);
        store.set_remembered("stylus-generic", false);

        store.forget("com.example.ext");
        store.forget("stylus-generic");
        assert!(store.enabled_plugins().is_empty());
        assert!(store.disabled_plugins().is_empty());
    }

    #[test]
    fn disable_then_enable_round_trips() {
        let dir = temp_dir("roundtrip");
        let store = HostStateStore::new(&dir);

        store.set_remembered("stylus-generic", false);
        store.set_remembered("sync-webdav", false);
        assert_eq!(
            store.disabled_plugins(),
            BTreeSet::from(["stylus-generic".to_string(), "sync-webdav".to_string()])
        );

        // A fresh store over the same directory must see the same state — that is the whole point.
        let reopened = HostStateStore::new(&dir);
        assert!(reopened.disabled_plugins().contains("sync-webdav"));

        store.set_remembered("sync-webdav", true);
        assert_eq!(
            store.disabled_plugins(),
            BTreeSet::from(["stylus-generic".to_string()])
        );
    }

    #[test]
    fn comment_and_blank_lines_are_ignored() {
        let dir = temp_dir("comments");
        std::fs::write(
            dir.join("disabled-plugins.txt"),
            "# a note\n\n  stylus-generic  \n",
        )
        .expect("seed file");
        let store = HostStateStore::new(&dir);
        assert_eq!(
            store.disabled_plugins(),
            BTreeSet::from(["stylus-generic".to_string()])
        );
    }
}
