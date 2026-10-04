//! Activating (publishing) workflows: validation, webhook and form
//! registration with conflict detection, and schedules (spec §6.3).

use super::runner::{self, RunRequest};
use super::webhooks;
use super::{ApiError, N8n};
use crate::n8n::engine::check_node_types;
use crate::n8n::node::Mode;
use crate::n8n::store_ext::WorkflowRow;
use crate::n8n::types::Item;
use crate::n8n::workflow::Workflow;
use chrono::TimeZone;
use serde_json::{json, Map, Value};
use std::sync::Arc;
use std::time::Duration;

enum Rule {
    Every(Duration),
    Cron(Box<croner::Cron>),
}

fn num(v: &Value, key: &str, default: u64) -> u64 {
    v[key].as_u64().or_else(|| v[key].as_f64().map(|f| f as u64)).or_else(|| v[key].as_str().and_then(|s| s.parse().ok())).unwrap_or(default)
}

fn cron(expr: &str) -> Result<Box<croner::Cron>, String> {
    croner::Cron::new(expr.trim()).with_seconds_optional().parse().map(Box::new).map_err(|e| format!("Invalid cron expression \"{expr}\": {e}"))
}

/// Schedule Trigger rules (`rule.interval[]`) as timers.
fn schedule_rules(parameters: &Value) -> Result<Vec<Rule>, String> {
    let intervals = parameters.pointer("/rule/interval").and_then(Value::as_array).cloned().unwrap_or_else(|| vec![json!({"field": "days"})]);
    let mut rules = Vec::new();
    for i in &intervals {
        let minute = num(i, "triggerAtMinute", 0);
        let hour = num(i, "triggerAtHour", 0);
        let rule = match i["field"].as_str().unwrap_or("days") {
            "seconds" => Rule::Every(Duration::from_secs(num(i, "secondsInterval", 30).max(1))),
            "minutes" => Rule::Every(Duration::from_secs(60 * num(i, "minutesInterval", 5).max(1))),
            "hours" => Rule::Cron(cron(&format!("0 {minute} */{} * * *", num(i, "hoursInterval", 1).max(1)))?),
            "days" => Rule::Cron(cron(&format!("0 {minute} {hour} */{} * *", num(i, "daysInterval", 1).max(1)))?),
            "weeks" => {
                let days: Vec<String> = match &i["triggerAtDay"] {
                    Value::Array(a) if !a.is_empty() => a.iter().map(|d| d.as_u64().map(|n| n.to_string()).unwrap_or_else(|| d.as_str().unwrap_or("0").to_string())).collect(),
                    _ => vec!["0".into()],
                };
                Rule::Cron(cron(&format!("0 {minute} {hour} * * {}", days.join(",")))?)
            }
            "months" => Rule::Cron(cron(&format!("0 {minute} {hour} {} */{} *", num(i, "triggerAtDayOfMonth", 1), num(i, "monthsInterval", 1).max(1)))?),
            "cronExpression" => Rule::Cron(cron(i["expression"].as_str().unwrap_or(""))?),
            other => return Err(format!("Unknown schedule interval \"{other}\"")),
        };
        rules.push(rule);
    }
    Ok(rules)
}

fn workflow_tz(n8n: &N8n, workflow: &Value) -> chrono_tz::Tz {
    workflow
        .pointer("/settings/timezone")
        .and_then(Value::as_str)
        .filter(|t| *t != "DEFAULT")
        .unwrap_or(&n8n.config.timezone)
        .parse()
        .unwrap_or(chrono_tz::UTC)
}

fn ordinal(n: u32) -> String {
    let suffix = match (n % 10, n % 100) {
        (_, 11..=13) => "th",
        (1, _) => "st",
        (2, _) => "nd",
        (3, _) => "rd",
        _ => "th",
    };
    format!("{n}{suffix}")
}

/// The item a Schedule Trigger emits, as n8n builds it.
pub fn schedule_item(now: chrono::DateTime<chrono_tz::Tz>, tz: &chrono_tz::Tz) -> Item {
    use chrono::{Datelike, Timelike};
    let offset = now.format("%:z").to_string();
    let hour12 = match now.hour() % 12 {
        0 => 12,
        h => h,
    };
    let ampm = if now.hour() < 12 { "am" } else { "pm" };
    let time = format!("{hour12}:{:02}:{:02} {ampm}", now.minute(), now.second());
    let mut json = Map::new();
    json.insert("timestamp".into(), json!(now.format("%Y-%m-%dT%H:%M:%S%.3f%:z").to_string()));
    json.insert("Readable date".into(), json!(format!("{} {} {}, {time}", now.format("%B"), ordinal(now.day()), now.year())));
    json.insert("Readable time".into(), json!(time));
    json.insert("Day of week".into(), json!(now.format("%A").to_string()));
    json.insert("Year".into(), json!(now.format("%Y").to_string()));
    json.insert("Month".into(), json!(now.format("%B").to_string()));
    json.insert("Day of month".into(), json!(now.format("%d").to_string()));
    json.insert("Hour".into(), json!(now.format("%H").to_string()));
    json.insert("Minute".into(), json!(now.format("%M").to_string()));
    json.insert("Second".into(), json!(now.format("%S").to_string()));
    json.insert("Timezone".into(), json!(format!("{} (UTC{offset})", tz.name())));
    Item::new(json).paired(0)
}

async fn fire_schedule(n8n: &Arc<N8n>, workflow_id: &str, node: &str) {
    let Ok(Some(row)) = n8n.store.workflow_row(workflow_id).await else { return };
    if !row.active {
        return;
    }
    let tz = workflow_tz(n8n, &row.data);
    let now = chrono::Utc::now().with_timezone(&tz);
    let mut req = RunRequest::new(row.data.clone(), Mode::Trigger);
    req.start_node = Some(node.to_string());
    req.start_items = Some(vec![schedule_item(now, &tz)]);
    if let Err(e) = runner::start(n8n, req).await {
        tracing::error!(workflowId = workflow_id, error = %e, "scheduled execution could not start");
    }
}

fn spawn_rule(n8n: &Arc<N8n>, workflow_id: String, node: String, tz: chrono_tz::Tz, rule: Rule) -> tokio::task::JoinHandle<()> {
    let weak = Arc::downgrade(n8n);
    tokio::spawn(async move {
        loop {
            let wait = match &rule {
                Rule::Every(d) => *d,
                Rule::Cron(c) => {
                    let now = chrono::Utc::now().with_timezone(&tz);
                    match c.find_next_occurrence(&now, false) {
                        Ok(next) => (next.with_timezone(&chrono::Utc) - chrono::Utc::now()).to_std().unwrap_or(Duration::from_millis(10)),
                        Err(_) => return,
                    }
                }
            };
            tokio::time::sleep(wait).await;
            let Some(n8n) = weak.upgrade() else { return };
            fire_schedule(&n8n, &workflow_id, &node).await;
        }
    })
}

fn trigger_types() -> &'static [&'static str] {
    &[
        "n8n-nodes-base.webhook",
        "n8n-nodes-base.formTrigger",
        "n8n-nodes-base.scheduleTrigger",
        "n8n-nodes-base.errorTrigger",
        "n8n-nodes-base.executeWorkflowTrigger",
        "n8n-nodes-base.emailReadImap",
        "n8n-nodes-base.rabbitmqTrigger",
        "n8n-nodes-base.kafkaTrigger",
        "n8n-nodes-base.mqttTrigger",
    ]
}

/// Validates a workflow for activation and registers its triggers.
pub async fn register(n8n: &Arc<N8n>, workflow_json: &Value) -> Result<(), ApiError> {
    let workflow = Workflow::from_json(workflow_json).map_err(|e| ApiError::bad_request(e.to_string()))?;
    let id = workflow.id.clone().ok_or_else(|| ApiError::bad_request("The workflow has no id"))?;
    check_node_types(&workflow, &n8n.registry, &n8n.services).map_err(|e| ApiError::bad_request(e.0))?;
    let triggers: Vec<_> = workflow.nodes.iter().filter(|n| !n.disabled && trigger_types().contains(&n.node_type.as_str())).collect();
    if triggers.is_empty() {
        return Err(ApiError::bad_request(
            "Workflow has no node to start the workflow - at least one trigger, poller or webhook node is required",
        ));
    }
    runner::check_credential_access(n8n, workflow_json).await.map_err(ApiError::bad_request)?;
    let registrations = webhooks::registrations_for(&workflow);
    {
        let existing = n8n.webhooks.read().unwrap();
        for r in &registrations {
            if let Some(other) = existing.iter().find(|e| e.workflow_id != id && e.conflicts_with(r)) {
                return Err(ApiError::bad_request(format!(
                    "There is a conflict with one of the webhooks. The webhook \"{} {}\" is already used by the workflow \"{}\".",
                    r.method, r.path, other.workflow_id
                )));
            }
        }
    }
    for node in triggers.iter().filter(|n| n.node_type == "n8n-nodes-base.scheduleTrigger") {
        // Validate every rule now, even if we won't spawn timers for it
        // (not the leader): activation must fail the same way on every
        // main (spec: "A cron expression with an invalid format is
        // rejected on activation").
        schedule_rules(&node.parameters).map_err(ApiError::bad_request)?;
    }
    for node in triggers.iter().copied() {
        // Long-lived triggers (Email Trigger (IMAP), and future RabbitMQ/
        // Kafka/MQTT) connect once now, on every main regardless of
        // leadership, so a bad host/credentials fails activation the same
        // way n8n's trigger() throwing during add() does, rather than only
        // showing up later in the leader's logs.
        if let Some(trigger) = super::triggers::for_node_type(&node.node_type) {
            trigger.validate(n8n, node).await.map_err(ApiError::bad_request)?;
        }
    }
    unregister(n8n, &id);
    n8n.webhooks.write().unwrap().extend(registrations);
    let mut cached = workflow_json.clone();
    cached["active"] = json!(true);
    n8n.active.write().unwrap().insert(id.clone(), Arc::new(cached));
    if n8n.schedules_enabled.load(std::sync::atomic::Ordering::SeqCst) && n8n.leader.is_leader() {
        start_schedules(n8n, &id, &workflow);
        super::triggers::start(n8n, &id, &workflow);
    }
    tracing::info!(workflowId = %id, "Activated workflow \"{}\"", workflow.name);
    Ok(())
}

/// Spawns schedule timers for `workflow`'s Schedule Trigger nodes,
/// replacing any it already had. Only called while this instance is the
/// leader (or single-main, which is always leader); see
/// [`activate_schedules_for_all`]/[`deactivate_schedules_for_all`] for how a
/// multi-main takeover/stepdown adds or removes these for workflows that
/// were already active.
fn start_schedules(n8n: &Arc<N8n>, id: &str, workflow: &Workflow) {
    let raw = Value::Object(workflow.raw.clone());
    let tz = workflow_tz(n8n, &raw);
    let mut schedules = Vec::new();
    for node in workflow.nodes.iter().filter(|n| !n.disabled && n.node_type == "n8n-nodes-base.scheduleTrigger") {
        let Ok(rules) = schedule_rules(&node.parameters) else { continue };
        for rule in rules {
            schedules.push((node.name.clone(), rule));
        }
    }
    let handles: Vec<_> = schedules.into_iter().map(|(node, rule)| spawn_rule(n8n, id.to_string(), node, tz, rule)).collect();
    if let Some(old) = n8n.schedules.lock().unwrap().insert(id.to_string(), handles) {
        for h in old {
            h.abort();
        }
    }
}

fn stop_schedules(n8n: &N8n, workflow_id: &str) {
    if let Some(handles) = n8n.schedules.lock().unwrap().remove(workflow_id) {
        for h in handles {
            h.abort();
        }
    }
}

pub fn unregister(n8n: &N8n, workflow_id: &str) {
    n8n.webhooks.write().unwrap().retain(|r| r.workflow_id != workflow_id);
    n8n.active.write().unwrap().remove(workflow_id);
    stop_schedules(n8n, workflow_id);
    super::triggers::stop(n8n, workflow_id);
}

/// Multi-main takeover (plan task 4.1): adds schedule timers for every
/// workflow that is already active (registered for its webhooks/forms by
/// every main regardless of leadership), mirroring n8n's
/// `addAllNonWebhookTriggerWorkflows`.
pub async fn activate_schedules_for_all(n8n: &Arc<N8n>) {
    if !n8n.schedules_enabled.load(std::sync::atomic::Ordering::SeqCst) {
        return;
    }
    let active: Vec<(String, Arc<Value>)> = n8n.active.read().unwrap().iter().map(|(id, w)| (id.clone(), w.clone())).collect();
    let mut n = 0;
    for (id, workflow_json) in active {
        match Workflow::from_json(&workflow_json) {
            Ok(workflow) => {
                start_schedules(n8n, &id, &workflow);
                n += 1;
            }
            Err(e) => tracing::warn!(workflowId = %id, error = %e, "could not add schedules for workflow on leader takeover"),
        }
    }
    tracing::info!("[multi-main] leader takeover: added schedules for {n} active workflow(s)");
}

/// Picks up webhook/form registrations made by *another* process sharing
/// this database: used by `r8r webhook` (always) and, since multi-main
/// setup means more than one `r8r start` main shares a database, by every
/// main when `N8N_MULTI_MAIN_SETUP_ENABLED=true`. A workflow activated
/// through main A's REST API is otherwise invisible to main B until this
/// runs; schedules/pollers are unaffected here (they come from
/// leader election, not from this poll -- `register` only starts them
/// when this instance is already the leader).
pub async fn sync_registrations(n8n: &Arc<N8n>) {
    let Ok(rows) = n8n.store.workflow_rows().await else { return };
    let mut active_ids = Vec::new();
    for row in rows.into_iter().filter(|r| r.active) {
        let id = row.data["id"].as_str().unwrap_or_default().to_string();
        let mut current = row.data.clone();
        current["active"] = json!(true);
        let unchanged = n8n.active.read().unwrap().get(&id).is_some_and(|w| **w == current);
        if !unchanged {
            if let Err(e) = register(n8n, &row.data).await {
                tracing::warn!(workflowId = %id, error = %e.1, "could not register workflow webhooks");
            }
        }
        active_ids.push(id);
    }
    let stale: Vec<String> = n8n.active.read().unwrap().keys().filter(|k| !active_ids.contains(k)).cloned().collect();
    for id in stale {
        unregister(n8n, &id);
    }
}

/// Multi-main stepdown (plan task 4.1): removes every schedule timer this
/// instance was running, leaving webhooks/forms (served by every main)
/// untouched. Mirrors n8n's `removeAllNonWebhookTriggerWorkflows`.
pub fn deactivate_schedules_for_all(n8n: &N8n) {
    let ids: Vec<String> = n8n.schedules.lock().unwrap().keys().cloned().collect();
    let n = ids.len();
    for id in ids {
        stop_schedules(n8n, &id);
    }
    tracing::info!("[multi-main] stepped down: removed schedules for {n} workflow(s)");
}

async fn set_active(n8n: &N8n, row: &WorkflowRow, active: bool) -> Result<WorkflowRow, ApiError> {
    let mut data = row.data.clone();
    data["active"] = json!(active);
    n8n.store.save_workflow(&data).await?;
    let id = data["id"].as_str().unwrap_or_default();
    Ok(n8n.store.workflow_row(id).await?.expect("just saved"))
}

pub async fn activate(n8n: &Arc<N8n>, row: &WorkflowRow) -> Result<WorkflowRow, ApiError> {
    register(n8n, &row.data).await?;
    set_active(n8n, row, true).await
}

pub async fn deactivate(n8n: &Arc<N8n>, row: &WorkflowRow) -> Result<WorkflowRow, ApiError> {
    unregister(n8n, row.data["id"].as_str().unwrap_or_default());
    set_active(n8n, row, false).await
}

/// Used by tests of the schedule item shape.
pub fn schedule_item_at(ts: i64, tz: &str) -> Item {
    let tz: chrono_tz::Tz = tz.parse().unwrap_or(chrono_tz::UTC);
    schedule_item(tz.timestamp_millis_opt(ts).unwrap(), &tz)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schedule_items_carry_the_zone_offset() {
        let item = schedule_item_at(1_700_000_000_000, "Europe/Paris");
        assert_eq!(item.json["timestamp"], "2023-11-14T23:13:20.000+01:00");
        assert_eq!(item.json["Timezone"], "Europe/Paris (UTC+01:00)");
        assert_eq!(item.json["Readable date"], "November 14th 2023, 11:13:20 pm");
    }

    #[test]
    fn invalid_cron_is_rejected() {
        assert!(schedule_rules(&json!({"rule": {"interval": [{"field": "cronExpression", "expression": "not a cron"}]}})).is_err());
        assert!(schedule_rules(&json!({"rule": {"interval": [{"field": "cronExpression", "expression": "*/5 * * * *"}]}})).is_ok());
    }
}
