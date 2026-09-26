//! Queue-mode processes (spec §7.3): `r8r worker` runs executions from the
//! queue, `r8r webhook` answers production webhooks and enqueues their
//! executions. Both share the database with the main process.

use super::{activation, runner, webhooks, N8n};
use crate::n8n::config::Config;
use axum::routing::get;
use axum::{Json, Router};
use serde_json::json;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

fn grace() -> Duration {
    Duration::from_secs(std::env::var("N8N_GRACEFUL_SHUTDOWN_TIMEOUT").ok().and_then(|v| v.parse().ok()).unwrap_or(30))
}

async fn queue_state(config: Config, process: &str) -> anyhow::Result<Arc<N8n>> {
    if config.executions_mode != "queue" {
        anyhow::bail!("r8r {process} needs queue mode: set EXECUTIONS_MODE=queue (and QUEUE_BULL_REDIS_HOST, or R8R_QUEUE_BACKEND=postgres)");
    }
    N8n::new(config).await
}

async fn serve_health(port: u16) -> anyhow::Result<()> {
    let app = Router::new()
        .route("/healthz", get(|| async { Json(json!({"status": "ok"})) }))
        .route("/healthz/readiness", get(|| async { Json(json!({"status": "ok"})) }));
    let listener = tokio::net::TcpListener::bind(("0.0.0.0", port))
        .await
        .map_err(|e| anyhow::anyhow!("cannot listen on QUEUE_HEALTH_CHECK_PORT {port}: {e}"))?;
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    Ok(())
}

/// `r8r worker --concurrency=N`: leases jobs, keeps their leases alive
/// while running them, and acknowledges them when done.
pub async fn run_worker(concurrency: usize) -> anyhow::Result<()> {
    let config = Config::load()?;
    let n8n = queue_state(config, "worker").await?;
    let queue = n8n.queue.clone().expect("queue mode");
    if std::env::var("QUEUE_HEALTH_CHECK_ACTIVE").is_ok_and(|v| v == "true") {
        let port = std::env::var("QUEUE_HEALTH_CHECK_PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(5678);
        serve_health(port).await?;
    }
    n8n.spawn_requeuer();
    let concurrency = concurrency.max(1);
    tracing::info!("r8r worker ready (concurrency {concurrency}, queue {})", queue.describe());
    let slots = Arc::new(tokio::sync::Semaphore::new(concurrency));
    let shutdown = super::shutdown_signal();
    tokio::pin!(shutdown);
    loop {
        let permit = tokio::select! {
            _ = &mut shutdown => break,
            p = slots.clone().acquire_owned() => p.expect("semaphore open"),
        };
        let job = tokio::select! {
            _ = &mut shutdown => break,
            j = queue.pop() => j,
        };
        let leased = match job {
            Ok(Some(l)) => l,
            Ok(None) => continue,
            Err(e) => {
                tracing::warn!(error = %e, "could not take a job from the queue");
                tokio::time::sleep(Duration::from_secs(1)).await;
                continue;
            }
        };
        let (n8n, queue) = (n8n.clone(), queue.clone());
        tokio::spawn(async move {
            let _permit = permit;
            let execution_id = leased.payload["executionId"].clone();
            tracing::info!(executionId = %execution_id, "worker took job");
            let beat = {
                let (queue, lease) = (queue.clone(), leased.lease.clone());
                tokio::spawn(async move {
                    let mut tick = tokio::time::interval(queue.lease_duration() / 3);
                    tick.tick().await;
                    loop {
                        tick.tick().await;
                        if let Err(e) = queue.heartbeat(&lease).await {
                            tracing::warn!(error = %e, "could not renew a job lease");
                        }
                    }
                })
            };
            if let Err(e) = runner::run_job(&n8n, &leased.payload).await {
                tracing::error!(executionId = %execution_id, error = %e, "job failed to run");
            }
            beat.abort();
            if let Err(e) = queue.ack(&leased.lease).await {
                tracing::warn!(error = %e, "could not acknowledge a finished job");
            }
        });
    }
    tracing::info!("worker stopping: finishing running jobs");
    let _ = tokio::time::timeout(grace(), slots.acquire_many(concurrency as u32)).await;
    Ok(())
}

/// `r8r webhook`: production webhooks and forms only; executions go to the
/// queue. Registrations follow the database (activations made through the
/// main process show up within a couple of seconds).
pub async fn run_webhook_process() -> anyhow::Result<()> {
    let config = Config::load()?;
    let port = config.port;
    let address = config.listen_address.clone();
    let n8n = queue_state(config, "webhook").await?;
    n8n.schedules_enabled.store(false, Ordering::SeqCst);
    let me = n8n.clone();
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(2));
        loop {
            tick.tick().await;
            sync_registrations(&me).await;
        }
    });
    let app = Router::new()
        .route("/healthz", get(|| async { Json(json!({"status": "ok"})) }))
        .route("/healthz/readiness", get(|| async { Json(json!({"status": "ok"})) }))
        .merge(webhooks::router())
        .with_state(n8n.clone());
    let listener = tokio::net::TcpListener::bind(format!("{address}:{port}")).await.map_err(|e| anyhow::anyhow!("cannot listen on port {port}: {e}"))?;
    tracing::info!("r8r webhook process ready on port {port}");
    axum::serve(listener, app).with_graceful_shutdown(super::shutdown_signal()).await?;
    n8n.drain(grace()).await;
    Ok(())
}

async fn sync_registrations(n8n: &Arc<N8n>) {
    let Ok(rows) = n8n.store.workflow_rows().await else { return };
    let mut active_ids = Vec::new();
    for row in rows.into_iter().filter(|r| r.active) {
        let id = row.data["id"].as_str().unwrap_or_default().to_string();
        let mut current = row.data.clone();
        current["active"] = json!(true);
        let unchanged = n8n.active.read().unwrap().get(&id).is_some_and(|w| **w == current);
        if !unchanged {
            if let Err(e) = activation::register(n8n, &row.data).await {
                tracing::warn!(workflowId = %id, error = %e.1, "could not register workflow webhooks");
            }
        }
        active_ids.push(id);
    }
    let stale: Vec<String> = n8n.active.read().unwrap().keys().filter(|k| !active_ids.contains(k)).cloned().collect();
    for id in stale {
        activation::unregister(n8n, &id);
    }
}
