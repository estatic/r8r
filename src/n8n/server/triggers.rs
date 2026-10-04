//! Long-lived trigger activation (plan task 1.15 / 4.1 follow-on): triggers
//! that hold a connection open for their whole active lifetime (the Email
//! Trigger (IMAP) node; future RabbitMQ/Kafka/MQTT triggers) rather than
//! firing on a timer like Schedule Trigger.
//!
//! Leader-only, exactly like schedules (see `activation.rs`'s module doc):
//! `activation::register` spawns one task per node when this instance is
//! the leader; `activation::unregister` aborts them; multi-main takeover/
//! stepdown add/remove them for already-active workflows via
//! `activate_long_lived_for_all`/`deactivate_long_lived_for_all`, called
//! alongside the schedule equivalents in `server/mod.rs`.
//!
//! A new trigger type plugs in by implementing [`LongLivedTrigger`] and
//! adding one line to [`for_node_type`] -- no other file needs to change.

use super::N8n;
use crate::n8n::node::Mode;
use crate::n8n::server::runner::{self, RunRequest};
use crate::n8n::types::Item;
use crate::n8n::workflow::Node;
use serde_json::Value;
use std::sync::Arc;

/// What a long-lived trigger node type must implement to plug into this
/// path. One listener task is spawned per node per workflow while this
/// instance is the leader.
#[async_trait::async_trait]
pub trait LongLivedTrigger: Send + Sync {
    /// Connects (and disconnects) once, synchronously, during activation --
    /// on every main, regardless of leadership, exactly like
    /// `activation::register` validates a Schedule Trigger's cron
    /// expression before confirming activation. A connection or
    /// authentication failure here fails the activation request the same
    /// way n8n's `trigger()` throwing during `workflowActivate.add()`
    /// surfaces as an activation error.
    async fn validate(&self, n8n: &Arc<N8n>, node: &Node) -> Result<(), String>;

    /// Runs until the task is aborted (deactivation, stepdown, or process
    /// shutdown). Implementations should loop: wait for the next message
    /// from the external source, then call [`fire`] with the trigger's
    /// output items, mirroring n8n's `this.emit([items])`. A transient
    /// connection error should be logged and retried with backoff rather
    /// than returned -- the task must not exit just because the source was
    /// briefly unreachable.
    async fn run(&self, n8n: Arc<N8n>, workflow_id: String, node: Node);
}

/// `node_type` -> constructor, so `activation.rs` can spawn the right
/// trigger without a match statement that grows with every node. The
/// RabbitMQ/Kafka/MQTT triggers (plan task 1.13) each add one line here.
pub fn for_node_type(node_type: &str) -> Option<Arc<dyn LongLivedTrigger>> {
    match node_type {
        "n8n-nodes-base.emailReadImap" => Some(Arc::new(crate::n8n::nodes::email_imap::ImapTrigger) as Arc<dyn LongLivedTrigger>),
        "n8n-nodes-base.rabbitmqTrigger" => Some(Arc::new(crate::n8n::nodes::rabbitmq::RabbitMqTrigger) as Arc<dyn LongLivedTrigger>),
        "n8n-nodes-base.kafkaTrigger" => Some(Arc::new(crate::n8n::nodes::kafka::KafkaTrigger) as Arc<dyn LongLivedTrigger>),
        "n8n-nodes-base.mqttTrigger" => Some(Arc::new(crate::n8n::nodes::mqtt::MqttTrigger) as Arc<dyn LongLivedTrigger>),
        _ => None,
    }
}

/// Starts an execution with `items` as the given node's output -- a
/// long-lived trigger's analogue of a schedule tick or a webhook call
/// (spec: "each received message starts an execution with the trigger's
/// output items, like n8n's `emit`"). Fire-and-forget: callers that need
/// the finished execution's outcome (e.g. a RabbitMQ Trigger's
/// `executionFinishes`/`executionFinishesSuccessfully` ack modes, which
/// must ack/nack based on whether the run errored) should call
/// [`fire_and_wait`] instead.
pub async fn fire(n8n: &Arc<N8n>, workflow_id: &str, node: &str, items: Vec<Item>) {
    fire_and_wait(n8n, workflow_id, node, items).await;
}

/// Like [`fire`], but waits for the execution to finish and returns its
/// outcome (`None` if the workflow was deactivated/not found before the
/// run could start, or if the run could not be started at all).
pub async fn fire_and_wait(n8n: &Arc<N8n>, workflow_id: &str, node: &str, items: Vec<Item>) -> Option<Arc<runner::RunOutcome>> {
    let Ok(Some(row)) = n8n.store.workflow_row(workflow_id).await else { return None };
    if !row.active {
        return None;
    }
    let mut req = RunRequest::new(row.data.clone(), Mode::Trigger);
    req.start_node = Some(node.to_string());
    req.start_items = Some(items);
    match runner::start(n8n, req).await {
        Ok(handle) => handle.done.await.ok(),
        Err(e) => {
            tracing::error!(workflowId = workflow_id, error = %e, "long-lived trigger execution could not start");
            None
        }
    }
}

/// Decrypted data of the credential `node` has for `cred_type`, resolved
/// directly against the store (outside of any running execution -- used
/// during activation/validation and by the listener task itself, neither
/// of which has an `ExecCtx`). Mirrors `node::ExecCtx::credentials_for`.
pub async fn resolve_credential(n8n: &N8n, node: &Node, cred_type: &str) -> Result<Value, String> {
    let reference = node
        .credentials
        .get(cred_type)
        .ok_or_else(|| format!("Node \"{}\" does not have any credentials of type \"{cred_type}\" set", node.name))?;
    let id = reference["id"].as_str().unwrap_or_default();
    let record = match n8n.store.get_credential(id).await.map_err(|e| e.to_string())? {
        Some(r) => r,
        None => {
            let name = reference["name"].as_str().unwrap_or_default();
            let all = n8n.store.list_credentials().await.map_err(|e| e.to_string())?;
            let mut matching = all.into_iter().filter(|c| c.name == name && c.cred_type == cred_type);
            match (matching.next(), matching.next()) {
                (Some(c), None) => c,
                _ => return Err(format!("Credential with ID \"{id}\" does not exist for type \"{cred_type}\".")),
            }
        }
    };
    n8n.store.decrypt_credential(&record).await.map_err(|e| e.to_string())
}

/// Spawns one listener task per long-lived-trigger node of `workflow`,
/// replacing any it already had. Only called while this instance is the
/// leader (or single-main, which is always leader); see
/// `activate_long_lived_for_all`/`deactivate_long_lived_for_all` for how a
/// multi-main takeover/stepdown adds or removes these for workflows that
/// were already active.
pub fn start(n8n: &Arc<N8n>, id: &str, workflow: &crate::n8n::workflow::Workflow) {
    let handles: Vec<_> = workflow
        .nodes
        .iter()
        .filter(|n| !n.disabled)
        .filter_map(|n| for_node_type(&n.node_type).map(|t| (t, n.clone())))
        .map(|(trigger, node)| {
            let n8n = n8n.clone();
            let workflow_id = id.to_string();
            tokio::spawn(async move { trigger.run(n8n, workflow_id, node).await })
        })
        .collect();
    if let Some(old) = n8n.long_lived_triggers.lock().unwrap().insert(id.to_string(), handles) {
        for h in old {
            h.abort();
        }
    }
}

pub fn stop(n8n: &N8n, workflow_id: &str) {
    if let Some(handles) = n8n.long_lived_triggers.lock().unwrap().remove(workflow_id) {
        for h in handles {
            h.abort();
        }
    }
}

/// Multi-main takeover: adds long-lived trigger listeners for every
/// workflow that is already active, mirroring
/// `activation::activate_schedules_for_all`.
pub async fn activate_for_all(n8n: &Arc<N8n>) {
    let active: Vec<(String, Arc<Value>)> = n8n.active.read().unwrap().iter().map(|(id, w)| (id.clone(), w.clone())).collect();
    let mut n = 0;
    for (id, workflow_json) in active {
        match crate::n8n::workflow::Workflow::from_json(&workflow_json) {
            Ok(workflow) => {
                start(n8n, &id, &workflow);
                n += 1;
            }
            Err(e) => tracing::warn!(workflowId = %id, error = %e, "could not add long-lived triggers for workflow on leader takeover"),
        }
    }
    tracing::info!("[multi-main] leader takeover: added long-lived triggers for {n} active workflow(s)");
}

/// Multi-main stepdown: removes every long-lived trigger listener this
/// instance was running, mirroring `activation::deactivate_schedules_for_all`.
pub fn deactivate_for_all(n8n: &N8n) {
    let ids: Vec<String> = n8n.long_lived_triggers.lock().unwrap().keys().cloned().collect();
    let n = ids.len();
    for id in ids {
        stop(n8n, &id);
    }
    tracing::info!("[multi-main] stepped down: removed long-lived triggers for {n} workflow(s)");
}
