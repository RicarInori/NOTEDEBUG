//! Persisted host state.
//!
//! Deliberately minimal: the only thing the host remembers across restarts today is which preset
//! plugins the user turned off. Keeping it as a plain newline-delimited file means no dependency and
//! no schema to migrate; if this grows past a handful of keys it should move to the same store the
//! real settings live in.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Records the plugins the user disabled, so `bootstrap` does not silently re-enable them.
pub struct HostStateStore {
    path: PathBuf,
}

impl HostStateStore {
    /// `data_dir` is the application support directory the platform layer hands to `bootstrap`.
    pub fn new(data_dir: &Path) -> Self {
        Self {
            path: data_dir.join("disabled-plugins.txt"),
        }
    }

    /// Plugin ids the user turned off. A missing or unreadable file means "nothing disabled".
    pub fn disabled_plugins(&self) -> BTreeSet<String> {
        let Ok(contents) = std::fs::read_to_string(&self.path) else {
            return BTreeSet::new();
        };
        contents
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
            .map(str::to_string)
            .collect()
    }

    /// Record (or clear) a plugin's disabled state. Best effort: a user who turned a plugin off
    /// should not see the app fail because the state file could not be written.
    pub fn set_disabled(&self, id: &str, disabled: bool) {
        let mut ids = self.disabled_plugins();
        let changed = if disabled {
            ids.insert(id.to_string())
        } else {
            ids.remove(id)
        };
        if !changed {
            return;
        }
        if let Some(parent) = self.path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let body: String = ids
            .iter()
            .map(|id| format!("{id}\n"))
            .collect::<Vec<_>>()
            .join("");
        let _ = std::fs::write(&self.path, body);
    }
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
    fn disable_then_enable_round_trips() {
        let dir = temp_dir("roundtrip");
        let store = HostStateStore::new(&dir);

        store.set_disabled("stylus-generic", true);
        store.set_disabled("sync-webdav", true);
        assert_eq!(
            store.disabled_plugins(),
            BTreeSet::from(["stylus-generic".to_string(), "sync-webdav".to_string()])
        );

        // A fresh store over the same directory must see the same state — that is the whole point.
        let reopened = HostStateStore::new(&dir);
        assert!(reopened.disabled_plugins().contains("sync-webdav"));

        store.set_disabled("sync-webdav", false);
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
