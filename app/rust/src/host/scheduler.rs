use async_trait::async_trait;
use inkweave_api::runtime::{BackgroundScheduler, ScheduleError, TaskId, WorkConstraints};
use parking_lot::Mutex;
use std::collections::HashMap;

/// One scheduled unit of work, as declared by a plugin.
#[derive(Clone, Debug)]
pub struct ScheduledTask {
    pub task: TaskId,
    pub periodic: bool,
    pub interval_minutes: u32,
    pub delay_seconds: u32,
    pub expedited: bool,
    pub constraints: WorkConstraints,
}

/// Host implementation of [`BackgroundScheduler`] (§8.5).
///
/// This records intent and enforces ownership/cancellation. The platform hop — `WorkManager` on
/// Android, `BGTaskScheduler` on iOS — is wired through a `flutter_rust_bridge` stream, which is
/// part of the M4 sync milestone. Keeping the bookkeeping here means the control panel's
/// "background tasks" counter is live, and `cancel_owner` is genuinely exercised on unload.
#[derive(Default)]
pub struct HostBackgroundScheduler {
    owners: Mutex<HashMap<String, Vec<ScheduledTask>>>,
}

impl HostBackgroundScheduler {
    pub fn new() -> Self {
        Self::default()
    }

    /// Every task currently registered, grouped by owner. Used by the panel's diagnostics view.
    pub fn snapshot(&self) -> HashMap<String, Vec<ScheduledTask>> {
        self.owners.lock().clone()
    }

    fn upsert(&self, owner: &str, task: ScheduledTask) {
        let mut owners = self.owners.lock();
        let entry = owners.entry(owner.to_string()).or_default();
        match entry.iter_mut().find(|existing| existing.task == task.task) {
            Some(existing) => *existing = task,
            None => entry.push(task),
        }
    }
}

#[async_trait]
impl BackgroundScheduler for HostBackgroundScheduler {
    async fn enqueue_periodic(
        &self,
        owner: &str,
        task: TaskId,
        interval_minutes: u32,
        constraints: WorkConstraints,
    ) -> Result<(), ScheduleError> {
        if interval_minutes == 0 {
            return Err(ScheduleError::Rejected(
                "periodic work needs a non-zero interval".to_string(),
            ));
        }
        self.upsert(
            owner,
            ScheduledTask {
                task,
                periodic: true,
                interval_minutes,
                delay_seconds: 0,
                expedited: false,
                constraints,
            },
        );
        Ok(())
    }

    async fn enqueue_one_shot(
        &self,
        owner: &str,
        task: TaskId,
        delay_seconds: u32,
        expedited: bool,
        constraints: WorkConstraints,
    ) -> Result<(), ScheduleError> {
        self.upsert(
            owner,
            ScheduledTask {
                task,
                periodic: false,
                interval_minutes: 0,
                delay_seconds,
                expedited,
                constraints,
            },
        );
        Ok(())
    }

    async fn cancel(&self, owner: &str, task: TaskId) -> Result<(), ScheduleError> {
        let mut owners = self.owners.lock();
        if let Some(list) = owners.get_mut(owner) {
            list.retain(|scheduled| scheduled.task != task);
        }
        Ok(())
    }

    async fn cancel_owner(&self, owner: &str) {
        self.owners.lock().remove(owner);
    }

    fn unfinished(&self, owner: &str) -> u32 {
        self.owners
            .lock()
            .get(owner)
            .map(|list| list.len() as u32)
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn tasks_are_namespaced_and_cancelled_per_owner() {
        let scheduler = HostBackgroundScheduler::new();
        scheduler
            .enqueue_periodic(
                "plugin:webdav",
                "sync".to_string(),
                15,
                WorkConstraints::UNMETERED,
            )
            .await
            .expect("periodic work is accepted");
        scheduler
            .enqueue_one_shot(
                "plugin:webdav",
                "flush".to_string(),
                5,
                true,
                WorkConstraints::METERED_OK,
            )
            .await
            .expect("one-shot work is accepted");
        scheduler
            .enqueue_periodic(
                "plugin:other",
                "tick".to_string(),
                60,
                WorkConstraints::default(),
            )
            .await
            .expect("second owner");

        assert_eq!(scheduler.unfinished("plugin:webdav"), 2);
        assert_eq!(scheduler.unfinished("plugin:other"), 1);

        // Unloading a plugin must not disturb anyone else's work.
        scheduler.cancel_owner("plugin:webdav").await;
        assert_eq!(scheduler.unfinished("plugin:webdav"), 0);
        assert_eq!(scheduler.unfinished("plugin:other"), 1);
    }

    #[tokio::test]
    async fn a_zero_interval_is_rejected() {
        let scheduler = HostBackgroundScheduler::new();
        let error = scheduler
            .enqueue_periodic(
                "plugin:x",
                "loop".to_string(),
                0,
                WorkConstraints::default(),
            )
            .await
            .expect_err("must reject");
        assert!(matches!(error, ScheduleError::Rejected(_)));
    }
}
