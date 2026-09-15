use tokio_cron_scheduler::{Job, JobScheduler};

/// Thin wrapper around `tokio_cron_scheduler::JobScheduler`.
///
/// Decouples r8r's own `register`/`unregister` API (used by
/// `triggers::activate_workflow_triggers` / `TriggerRegistry`) from the
/// underlying crate's actual API surface, which this module is the only
/// place allowed to depend on directly.
pub struct Scheduler {
    inner: JobScheduler,
}

impl Scheduler {
    /// Construct and start the underlying job scheduler.
    pub async fn new() -> anyhow::Result<Self> {
        let inner = JobScheduler::new().await?;
        inner.start().await?;
        Ok(Self { inner })
    }

    /// Register a recurring job on `cron_expr` (standard 6-field
    /// `sec min hour day month weekday` cron syntax). `job` is called on
    /// each firing; returns an opaque id usable later with `unregister`.
    pub async fn register<F, Fut>(&self, cron_expr: &str, job: F) -> anyhow::Result<uuid::Uuid>
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = ()> + Send + 'static,
    {
        // tokio_cron_scheduler::Job::new_async expects a `FnMut(Uuid,
        // JobScheduler) -> Pin<Box<dyn Future<Output = ()> + Send>>`. Our own
        // public signature only requires `Fn() -> Fut`, so wrap it: ignore
        // the job id / scheduler handle the crate passes in (r8r's caller
        // owns everything the closure needs already, per the plan's Global
        // Constraints), and box the returned future.
        let cron_job = Job::new_async(cron_expr, move |_job_id, _scheduler| {
            let fut = job();
            Box::pin(fut) as std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>
        })?;
        let job_id = self.inner.add(cron_job).await?;
        Ok(job_id)
    }

    /// Remove a previously-registered job. Unknown / already-removed ids are
    /// treated as a benign no-op (`Ok(())`), not an error — see the module's
    /// caller (`TriggerRegistry::take_cron_job`), which only calls this when
    /// it believes the job still exists.
    pub async fn unregister(&self, job_id: uuid::Uuid) -> anyhow::Result<()> {
        match self.inner.remove(&job_id).await {
            Ok(()) => Ok(()),
            Err(e) => {
                // tokio-cron-scheduler 0.10's `JobSchedulerError` has no
                // dedicated "not found" variant, and its default
                // (`SimpleMetadataStore`) backing store's `delete` is a
                // HashMap removal that never errors on a missing key — so in
                // practice this arm is unreachable for "unknown id", but any
                // other underlying failure is treated the same way per the
                // Interfaces contract above.
                tracing::debug!(error = %e, "scheduler.unregister: job already gone");
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    #[tokio::test]
    async fn registered_job_fires_on_its_schedule() {
        let scheduler = Scheduler::new().await.unwrap();
        let fire_count = Arc::new(AtomicUsize::new(0));
        let counted = fire_count.clone();

        // Every second — the tightest interval a standard 6-field cron
        // expression (sec min hour day month weekday) can express, used here
        // purely to keep the test fast; production cron expressions come from
        // user-authored core.schedule node parameters.
        let job_id = scheduler
            .register("* * * * * *", move || {
                let counted = counted.clone();
                async move {
                    counted.fetch_add(1, Ordering::SeqCst);
                }
            })
            .await
            .unwrap();

        tokio::time::sleep(std::time::Duration::from_millis(2200)).await;
        assert!(fire_count.load(Ordering::SeqCst) >= 1, "job should have fired at least once in 2.2s");

        scheduler.unregister(job_id).await.unwrap();
        let count_after_unregister = fire_count.load(Ordering::SeqCst);
        tokio::time::sleep(std::time::Duration::from_millis(1200)).await;
        assert_eq!(
            fire_count.load(Ordering::SeqCst),
            count_after_unregister,
            "job should not fire again after unregister"
        );
    }

    #[tokio::test]
    async fn unregister_on_unknown_job_id_does_not_error() {
        let scheduler = Scheduler::new().await.unwrap();
        let result = scheduler.unregister(uuid::Uuid::new_v4()).await;
        assert!(result.is_ok());
    }
}
