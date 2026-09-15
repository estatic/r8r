use std::collections::HashMap;
use std::sync::Mutex;
use uuid::Uuid;

#[derive(Default)]
pub struct TriggerRegistry {
    cron_jobs: Mutex<HashMap<Uuid, Uuid>>,
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
}
