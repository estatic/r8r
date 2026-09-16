use std::collections::HashMap;
use std::sync::Mutex;
use uuid::Uuid;

#[derive(Default)]
pub struct TriggerRegistry {
    cron_jobs: Mutex<HashMap<Uuid, Uuid>>,
    telegram_polls: Mutex<HashMap<Uuid, tokio::task::AbortHandle>>,
}

impl TriggerRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record_cron_job(&self, workflow_id: Uuid, job_id: Uuid) {
        self.cron_jobs.lock().unwrap().insert(workflow_id, job_id);
    }

    pub fn take_cron_job(&self, workflow_id: Uuid) -> Option<Uuid> {
        self.cron_jobs.lock().unwrap().remove(&workflow_id)
    }

    /// Records the `AbortHandle` for a workflow's spawned Telegram
    /// long-polling task, so `deactivate_workflow_triggers` can cancel it
    /// later without the caller needing to track the handle itself.
    pub fn record_telegram_poll(&self, workflow_id: Uuid, handle: tokio::task::AbortHandle) {
        self.telegram_polls.lock().unwrap().insert(workflow_id, handle);
    }

    /// Removes and returns a previously-recorded poll task's `AbortHandle`.
    /// The caller is responsible for calling `.abort()` on it — this method
    /// only manages the registry's bookkeeping, mirroring `take_cron_job`'s
    /// division of responsibility with `Scheduler::unregister`.
    pub fn take_telegram_poll(&self, workflow_id: Uuid) -> Option<tokio::task::AbortHandle> {
        self.telegram_polls.lock().unwrap().remove(&workflow_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_and_takes_a_cron_job() {
        let registry = TriggerRegistry::new();
        let workflow_id = Uuid::new_v4();
        let job_id = Uuid::new_v4();
        registry.record_cron_job(workflow_id, job_id);
        assert_eq!(registry.take_cron_job(workflow_id), Some(job_id));
    }

    #[test]
    fn take_is_idempotent_after_the_first_call() {
        let registry = TriggerRegistry::new();
        let workflow_id = Uuid::new_v4();
        registry.record_cron_job(workflow_id, Uuid::new_v4());
        registry.take_cron_job(workflow_id);
        assert_eq!(registry.take_cron_job(workflow_id), None);
    }

    #[test]
    fn take_on_unknown_workflow_returns_none() {
        let registry = TriggerRegistry::new();
        assert_eq!(registry.take_cron_job(Uuid::new_v4()), None);
    }

    #[test]
    fn recording_again_overwrites_the_previous_job_id() {
        let registry = TriggerRegistry::new();
        let workflow_id = Uuid::new_v4();
        registry.record_cron_job(workflow_id, Uuid::new_v4());
        let second_job = Uuid::new_v4();
        registry.record_cron_job(workflow_id, second_job);
        assert_eq!(registry.take_cron_job(workflow_id), Some(second_job));
    }

    fn dummy_abort_handle() -> tokio::task::AbortHandle {
        tokio::spawn(async {
            std::future::pending::<()>().await;
        })
        .abort_handle()
    }

    #[tokio::test]
    async fn records_and_takes_a_telegram_poll_handle() {
        let registry = TriggerRegistry::new();
        let workflow_id = Uuid::new_v4();
        let handle = dummy_abort_handle();
        registry.record_telegram_poll(workflow_id, handle.clone());
        let taken = registry.take_telegram_poll(workflow_id);
        assert!(taken.is_some());
        taken.unwrap().abort();
    }

    #[tokio::test]
    async fn take_telegram_poll_is_idempotent_after_the_first_call() {
        let registry = TriggerRegistry::new();
        let workflow_id = Uuid::new_v4();
        registry.record_telegram_poll(workflow_id, dummy_abort_handle());
        registry.take_telegram_poll(workflow_id).unwrap().abort();
        assert!(registry.take_telegram_poll(workflow_id).is_none());
    }

    #[tokio::test]
    async fn take_telegram_poll_on_unknown_workflow_returns_none() {
        let registry = TriggerRegistry::new();
        assert!(registry.take_telegram_poll(Uuid::new_v4()).is_none());
    }

    #[tokio::test]
    async fn cron_jobs_and_telegram_polls_are_tracked_independently() {
        let registry = TriggerRegistry::new();
        let workflow_id = Uuid::new_v4();
        registry.record_cron_job(workflow_id, Uuid::new_v4());
        registry.record_telegram_poll(workflow_id, dummy_abort_handle());
        assert!(registry.take_cron_job(workflow_id).is_some());
        assert!(registry.take_telegram_poll(workflow_id).unwrap().abort() == ());
    }
}
