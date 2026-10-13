//! `nbk-migrate` — `.nbk` format version migration (DEV guide §5.3).
//!
//! The container's `manifest.version` increases monotonically. Migrations are registered as
//! `from → to` hops and executed as a chain, so `v1 → v3` runs `v1 → v2` then `v2 → v3`.
//!
//! Two hard rules from the guide are enforced here rather than left to callers:
//!
//! * **Never open a newer format with older code.** A version above [`CURRENT_FORMAT_VERSION`]
//!   returns [`MigrateError::TooNew`], which the caller surfaces as read-only mode plus an upgrade
//!   prompt.
//! * **Back up before touching the file.** Backup creation itself lives in `storage-sqlite`
//!   (the only crate that touches the filesystem); this crate is pure and fully testable.

use std::collections::BTreeMap;

/// Highest format version this build understands.
pub const CURRENT_FORMAT_VERSION: u32 = 3;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum MigrateError {
    #[error("the file is version {found}, but this build only understands up to {supported}; open it read-only")]
    TooNew { found: u32, supported: u32 },

    #[error("no migration registered for the hop {from} → {to}")]
    MissingHop { from: u32, to: u32 },

    #[error("migration {from} → {to} was registered more than once")]
    DuplicateHop { from: u32, to: u32 },
}

/// Mutable view of a document handed to each migration hop.
///
/// The real container exposes typed access to `manifest.json` and the page index; the skeleton
/// keeps a string map so the chain logic can be exercised without a file format.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct MigrationContext {
    pub version: u32,
    pub fields: BTreeMap<String, String>,
}

impl MigrationContext {
    pub fn new(version: u32) -> Self {
        Self {
            version,
            fields: BTreeMap::new(),
        }
    }

    pub fn set(&mut self, key: impl Into<String>, value: impl Into<String>) {
        self.fields.insert(key.into(), value.into());
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.fields.get(key).map(String::as_str)
    }
}

/// One version hop. `apply` rewrites the context in place and bumps `version`.
#[derive(Clone)]
pub struct Migration {
    pub from: u32,
    pub to: u32,
    pub description: &'static str,
    pub apply: fn(&mut MigrationContext),
}

/// Ordered registry of migration hops.
#[derive(Default)]
pub struct MigrationChain {
    migrations: Vec<Migration>,
}

impl MigrationChain {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a hop. Hops are kept sorted by `from`, so registration order does not matter.
    pub fn register(&mut self, migration: Migration) -> Result<(), MigrateError> {
        if self
            .migrations
            .iter()
            .any(|existing| existing.from == migration.from && existing.to == migration.to)
        {
            return Err(MigrateError::DuplicateHop {
                from: migration.from,
                to: migration.to,
            });
        }
        self.migrations.push(migration);
        self.migrations.sort_by_key(|migration| migration.from);
        Ok(())
    }

    pub fn migrations(&self) -> &[Migration] {
        &self.migrations
    }

    /// Compute the ordered list of versions a document passes through.
    ///
    /// Migrating downwards is not supported: an older build must not rewrite a newer file.
    pub fn path(&self, from: u32, to: u32) -> Result<Vec<u32>, MigrateError> {
        if from > to {
            return Err(MigrateError::TooNew {
                found: from,
                supported: to,
            });
        }
        let mut path = vec![from];
        let mut current = from;
        while current < to {
            let hop = self
                .migrations
                .iter()
                .find(|migration| migration.from == current)
                .ok_or(MigrateError::MissingHop { from: current, to })?;
            if hop.to <= current {
                return Err(MigrateError::MissingHop { from: current, to });
            }
            current = hop.to;
            path.push(current);
        }
        Ok(path)
    }

    /// Run the chain in place and return the resulting version.
    pub fn migrate(
        &self,
        context: &mut MigrationContext,
        target: u32,
    ) -> Result<u32, MigrateError> {
        let path = self.path(context.version, target)?;
        for window in path.windows(2) {
            let (from, to) = (window[0], window[1]);
            let hop = self
                .migrations
                .iter()
                .find(|migration| migration.from == from && migration.to == to)
                .ok_or(MigrateError::MissingHop { from, to })?;
            (hop.apply)(context);
            context.version = to;
        }
        Ok(context.version)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v1_to_v2(context: &mut MigrationContext) {
        context.set("layer_model", "introduced");
    }

    fn v2_to_v3(context: &mut MigrationContext) {
        context.set("undo_log", "checkpointed");
    }

    fn chain() -> MigrationChain {
        let mut chain = MigrationChain::new();
        chain
            .register(Migration {
                from: 1,
                to: 2,
                description: "add the layer model",
                apply: v1_to_v2,
            })
            .expect("register v1->v2");
        chain
            .register(Migration {
                from: 2,
                to: 3,
                description: "add the undo checkpoint",
                apply: v2_to_v3,
            })
            .expect("register v2->v3");
        chain
    }

    #[test]
    fn chain_runs_every_hop_in_order() {
        let mut context = MigrationContext::new(1);
        let result = chain()
            .migrate(&mut context, CURRENT_FORMAT_VERSION)
            .expect("migrates");

        assert_eq!(result, 3);
        assert_eq!(context.version, 3);
        assert_eq!(context.get("layer_model"), Some("introduced"));
        assert_eq!(context.get("undo_log"), Some("checkpointed"));
    }

    #[test]
    fn path_reports_the_hops_taken() {
        let path = chain().path(1, 3).expect("path exists");
        assert_eq!(path, vec![1, 2, 3]);
    }

    #[test]
    fn a_newer_file_is_refused_rather_than_migrated() {
        let mut context = MigrationContext::new(9);
        let error = chain()
            .migrate(&mut context, CURRENT_FORMAT_VERSION)
            .expect_err("must refuse");
        assert_eq!(
            error,
            MigrateError::TooNew {
                found: 9,
                supported: 3
            }
        );
    }

    #[test]
    fn a_missing_hop_is_reported_not_skipped() {
        let mut chain = MigrationChain::new();
        chain
            .register(Migration {
                from: 1,
                to: 2,
                description: "only hop",
                apply: v1_to_v2,
            })
            .expect("register");

        let error = chain.path(1, 3).expect_err("no hop to 3");
        assert_eq!(error, MigrateError::MissingHop { from: 2, to: 3 });
    }

    #[test]
    fn duplicate_hops_are_rejected_at_registration() {
        let mut chain = chain();
        let error = chain
            .register(Migration {
                from: 1,
                to: 2,
                description: "duplicate",
                apply: v1_to_v2,
            })
            .expect_err("must reject");
        assert_eq!(error, MigrateError::DuplicateHop { from: 1, to: 2 });
    }
}
