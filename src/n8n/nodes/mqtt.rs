//! MQTT node (plan task 1.13): `publish` against the `mqtt` credential.
//! Faithful to n8n's `Mqtt.node.js` (typeVersion 1) and `GenericFunctions.js`
//! (`createClient`). Crate: `rumqttc` (tokio, rustls).
//!
//! The reference node has no per-item `try`/`catch`: `Promise.all` over
//! every item's `publishAsync`, so a publish failure fails the whole node
//! as one unit (not per item) -- r8r mirrors that: on failure the node
//! either throws, or (continueOnFail) returns a single `{error}` item, just
//! like `redis.rs`'s `info` operation.
//!
//! Output matches the reference exactly: `return [items]` -- the node
//! passes its *input* items through unchanged (not a `{success: true}`
//! marker, despite how similar broker-publish nodes in this codebase look).
//!
//! TLS (`mqtts`): supported only when a CA certificate is configured
//! (`rumqttc`'s `TlsConfiguration::Simple` has no "use the system/public
//! root store" fallback); `ssl: true` with no CA fails clearly rather than
//! silently skipping certificate verification. `ws`/`wss` transport is not
//! supported in this build (the `websocket` crate feature is not enabled).
//! Untested against a live TLS broker: `tests/bdd/services.sh`'s mosquitto
//! is plaintext/anonymous only.

use crate::n8n::node::{ExecCtx, NodeError, NodeResult, NodeType};
use crate::n8n::server::triggers::{fire, resolve_credential, LongLivedTrigger};
use crate::n8n::server::N8n;
use crate::n8n::types::{Item, NodeOutput};
use crate::n8n::workflow::Node;
use rumqttc::{AsyncClient, Event, EventLoop, Incoming, MqttOptions, Outgoing, QoS, Transport};
use serde_json::{json, Map, Value};
use std::sync::Arc;
use std::time::Duration;

pub struct Mqtt;

fn cred_str<'a>(cred: &'a Value, key: &str) -> Option<&'a str> {
    cred.get(key).and_then(Value::as_str).filter(|s| !s.is_empty())
}

fn qos_from(n: f64) -> QoS {
    match n as i64 {
        1 => QoS::AtLeastOnce,
        2 => QoS::ExactlyOnce,
        _ => QoS::AtMostOnce,
    }
}

fn random_suffix() -> String {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    (0..8).map(|_| std::iter::once(rng.gen_range(b'a'..=b'z') as char).next().unwrap()).collect()
}

fn mqtt_options(cred: &Value) -> NodeResult<MqttOptions> {
    let protocol = cred_str(cred, "protocol").unwrap_or("mqtt");
    if protocol == "ws" {
        return Err(NodeError::new("MQTT \"ws\" protocol is not supported natively yet"));
    }
    let host = cred_str(cred, "host").unwrap_or("localhost");
    let port = cred.get("port").and_then(|v| v.as_u64().or_else(|| v.as_str().and_then(|s| s.parse().ok()))).unwrap_or(1883) as u16;
    let client_id = cred_str(cred, "clientId").map(String::from).unwrap_or_else(|| format!("r8r_{}", random_suffix()));

    let mut opts = MqttOptions::new(client_id, host, port);
    opts.set_clean_session(cred.get("clean").and_then(Value::as_bool).unwrap_or(true));
    if let (Some(u), Some(p)) = (cred_str(cred, "username"), cred_str(cred, "password")) {
        opts.set_credentials(u, p);
    }

    let ssl = cred.get("ssl").and_then(Value::as_bool).unwrap_or(false);
    if protocol == "mqtts" || ssl {
        let ca = cred_str(cred, "ca").ok_or_else(|| NodeError::new("MQTT SSL without a CA certificate is not supported in this build"))?;
        let client_auth = match (cred_str(cred, "cert"), cred_str(cred, "key")) {
            (Some(cert), Some(key)) => Some((cert.as_bytes().to_vec(), key.as_bytes().to_vec())),
            _ => None,
        };
        opts.set_transport(Transport::tls(ca.as_bytes().to_vec(), client_auth, None));
    }
    Ok(opts)
}

#[async_trait::async_trait]
impl NodeType for Mqtt {
    fn type_name(&self) -> &'static str {
        "n8n-nodes-base.mqtt"
    }

    async fn execute(&self, ctx: &mut ExecCtx<'_>) -> NodeResult<NodeOutput> {
        let (_, cred) = ctx.credentials("mqtt").await?;
        let input = if ctx.input().is_empty() { vec![Item::default()] } else { ctx.input().to_vec() };

        let mut messages = Vec::with_capacity(input.len());
        for (i, item) in input.iter().enumerate() {
            let topic = ctx.param_str("topic", i, "")?;
            let send_input_data = ctx.param_bool("sendInputData", i, true)?;
            let message = if send_input_data { serde_json::to_string(&item.json).unwrap_or_default() } else { ctx.param_str("message", i, "")? };
            let qos = qos_from(ctx.param_f64("options.qos", i, 0.0)?);
            let retain = ctx.param_bool("options.retain", i, false)?;
            messages.push((topic, qos, retain, message.into_bytes()));
        }

        match publish_all(&cred, messages).await {
            Ok(()) => Ok(vec![input]),
            Err(e) if ctx.continue_on_fail() => {
                ctx.push_error_item(&e, 0);
                Ok(vec![vec![]])
            }
            Err(e) => Err(e),
        }
    }
}

/// Waits (up to 10s) for the next event matching `pred`, skipping anything
/// else that the event loop forwards in the meantime.
async fn wait_for_event(rx: &mut tokio::sync::mpsc::UnboundedReceiver<Result<Event, String>>, pred: impl Fn(&Event) -> bool) -> NodeResult<()> {
    loop {
        match tokio::time::timeout(Duration::from_secs(10), rx.recv()).await {
            Ok(Some(Ok(event))) if pred(&event) => return Ok(()),
            Ok(Some(Ok(_))) => continue,
            Ok(Some(Err(e))) => return Err(NodeError::new(e)),
            Ok(None) => return Err(NodeError::new("MQTT connection closed unexpectedly")),
            Err(_) => return Err(NodeError::new("Timed out waiting for the MQTT broker")),
        }
    }
}

async fn publish_all(cred: &Value, messages: Vec<(String, QoS, bool, Vec<u8>)>) -> NodeResult<()> {
    let opts = mqtt_options(cred)?;
    let (client, mut eventloop) = AsyncClient::new(opts, 50);

    // Drive the event loop in the background so queued publishes actually
    // reach the socket; forward events so the caller can wait for ConnAck
    // and for every QoS>0 publish to be acked (mirrors mqtt.js's
    // `endAsync()`, which the reference calls to let in-flight QoS 1/2
    // messages finish before disconnecting).
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let poll_handle = tokio::spawn(async move {
        loop {
            match eventloop.poll().await {
                Ok(event) => {
                    if tx.send(Ok(event)).is_err() {
                        break;
                    }
                }
                Err(e) => {
                    let _ = tx.send(Err(e.to_string()));
                    break;
                }
            }
        }
    });

    // Wait for the connection handshake before publishing.
    if let Err(e) = wait_for_event(&mut rx, |e| matches!(e, Event::Incoming(Incoming::ConnAck(_)))).await {
        poll_handle.abort();
        return Err(e);
    }

    let total = messages.len();
    let needs_ack = messages.iter().filter(|(_, qos, _, _)| *qos != QoS::AtMostOnce).count();
    for (topic, qos, retain, payload) in messages {
        if let Err(e) = client.publish(topic, qos, retain, payload).await {
            poll_handle.abort();
            return Err(NodeError::new(e.to_string()));
        }
    }

    // Wait for every publish to actually leave the socket (`Outgoing::Publish`
    // fires only once the write syscall succeeds) before even considering
    // disconnecting -- `client.publish().await` only *enqueues* the request
    // to the event loop task, it does not wait for the network write.
    for _ in 0..total {
        if let Err(e) = wait_for_event(&mut rx, |e| matches!(e, Event::Outgoing(Outgoing::Publish(_)))).await {
            poll_handle.abort();
            return Err(e);
        }
    }

    // For QoS 1/2, additionally wait for the broker's ack (mirrors
    // mqtt.js's `endAsync()`, which the reference calls precisely to let
    // in-flight QoS 1/2 messages finish before disconnecting).
    for _ in 0..needs_ack {
        let is_ack = |e: &Event| matches!(e, Event::Incoming(Incoming::PubAck(_)) | Event::Incoming(Incoming::PubComp(_)));
        if let Err(e) = wait_for_event(&mut rx, is_ack).await {
            poll_handle.abort();
            return Err(e);
        }
    }

    let _ = client.disconnect().await;
    poll_handle.abort();
    Ok(())
}

// ---- MQTT Trigger (plan task 1.13) -----------------------------------------------------
//
// Faithful to n8n's `MqttTrigger.node.js` (typeVersion 1) + `GenericFunctions.js`'s
// `createClient`: subscribes to `topics` (comma-separated, each optionally
// suffixed `:qos`, default QoS 0; an out-of-range QoS silently falls back
// to 0, matching the reference's `if (qos<0||qos>2) qos = 0`) and emits
// `{message, topic}` per message (`onlyMessage` emits just the message
// value, via [`Item::from_value`] for a non-object value -- this
// codebase's existing convention for that shape, also used by the Kafka
// Trigger's `onlyMessage`).
//
// `parallelProcessing` (default `true`): when `false`, the reference waits
// for the triggered execution to finish before processing the next MQTT
// event (a `donePromise` it awaits inside the `on('message', ...)`
// handler, which **blocks the client's own event loop task** while
// waiting -- r8r instead awaits inline in the listener's own poll loop,
// which has the same effect (no new message is read off the socket until
// the previous execution's result comes back) without needing a
// `Delivery`-style response hook.

pub struct MqttTrigger;

const MQTT_RECONNECT_BACKOFF: Duration = Duration::from_secs(5);
const MQTT_CONNECT_TIMEOUT: Duration = Duration::from_secs(20);

struct TopicQos {
    topic: String,
    qos: QoS,
}

fn qos_from_i64(n: i64) -> QoS {
    match n {
        1 => QoS::AtLeastOnce,
        2 => QoS::ExactlyOnce,
        _ => QoS::AtMostOnce,
    }
}

/// n8n's `topics.split(',')` + per-entry `topic:qos` parsing.
fn parse_topics(raw: &str) -> Result<Vec<TopicQos>, String> {
    let topics: Vec<TopicQos> = raw
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|part| {
            let mut split = part.splitn(2, ':');
            let topic = split.next().unwrap_or("").trim().to_string();
            let qos = match split.next().map(str::trim) {
                Some(s) if !s.is_empty() => s.parse::<i64>().map(qos_from_i64).unwrap_or(QoS::AtMostOnce),
                _ => QoS::AtMostOnce,
            };
            TopicQos { topic, qos }
        })
        .collect();
    if topics.is_empty() || topics.iter().any(|t| t.topic.is_empty()) {
        return Err("Topics are mandatory!".to_string());
    }
    Ok(topics)
}

/// Drives `eventloop` until the broker's `ConnAck`, an error, or the
/// timeout.
async fn wait_connack(eventloop: &mut EventLoop) -> Result<(), String> {
    tokio::time::timeout(MQTT_CONNECT_TIMEOUT, async {
        loop {
            match eventloop.poll().await {
                Ok(Event::Incoming(Incoming::ConnAck(_))) => return Ok(()),
                Ok(_) => continue,
                Err(e) => return Err(e.to_string()),
            }
        }
    })
    .await
    .unwrap_or_else(|_| Err("Timed out connecting to the MQTT broker".to_string()))
}

/// n8n's `parsePayload`: `jsonParseBody` (silently falls back to the raw
/// string on a parse failure) then `onlyMessage`.
fn build_item(topic: &str, payload: &[u8], json_parse_body: bool, only_message: bool) -> Item {
    let raw = String::from_utf8_lossy(payload).into_owned();
    let message: Value = if json_parse_body { serde_json::from_str(&raw).unwrap_or_else(|_| json!(raw)) } else { json!(raw) };
    if only_message {
        return Item::from_value(message);
    }
    let mut json = Map::new();
    json.insert("message".into(), message);
    json.insert("topic".into(), json!(topic));
    Item::new(json)
}

#[async_trait::async_trait]
impl LongLivedTrigger for MqttTrigger {
    async fn validate(&self, n8n: &Arc<N8n>, node: &Node) -> Result<(), String> {
        let cred = resolve_credential(n8n, node, "mqtt").await?;
        let topics_raw = node.parameters.get("topics").and_then(Value::as_str).unwrap_or("").to_string();
        parse_topics(&topics_raw)?;
        let opts = mqtt_options(&cred).map_err(|e| e.message)?;
        let (client, mut eventloop) = AsyncClient::new(opts, 10);
        let res = wait_connack(&mut eventloop).await;
        let _ = client.disconnect().await;
        res
    }

    async fn run(&self, n8n: Arc<N8n>, workflow_id: String, node: Node) {
        let topics_raw = node.parameters.get("topics").and_then(Value::as_str).unwrap_or("").to_string();
        let topics = match parse_topics(&topics_raw) {
            Ok(t) => t,
            Err(e) => {
                tracing::error!(workflowId = %workflow_id, node = %node.name, error = %e, "MQTT Trigger: invalid topics; the listener will not start");
                return;
            }
        };
        let options = node.parameters.get("options").cloned().unwrap_or(json!({}));
        let json_parse_body = options.get("jsonParseBody").and_then(Value::as_bool).unwrap_or(false);
        let only_message = options.get("onlyMessage").and_then(Value::as_bool).unwrap_or(false);
        let parallel_processing = options.get("parallelProcessing").and_then(Value::as_bool).unwrap_or(true);

        loop {
            let cred = match resolve_credential(&n8n, &node, "mqtt").await {
                Ok(v) => v,
                Err(e) => {
                    tracing::warn!(workflowId = %workflow_id, node = %node.name, error = %e, "MQTT Trigger: could not read credentials; retrying");
                    tokio::time::sleep(MQTT_RECONNECT_BACKOFF).await;
                    continue;
                }
            };
            let opts = match mqtt_options(&cred) {
                Ok(o) => o,
                Err(e) => {
                    tracing::warn!(workflowId = %workflow_id, node = %node.name, error = %e.message, "MQTT Trigger: invalid credential; retrying");
                    tokio::time::sleep(MQTT_RECONNECT_BACKOFF).await;
                    continue;
                }
            };
            let (client, mut eventloop) = AsyncClient::new(opts, 50);
            if let Err(e) = wait_connack(&mut eventloop).await {
                tracing::warn!(workflowId = %workflow_id, node = %node.name, error = %e, "MQTT Trigger: connection failed; retrying");
                tokio::time::sleep(MQTT_RECONNECT_BACKOFF).await;
                continue;
            }
            let mut subscribe_ok = true;
            for t in &topics {
                if let Err(e) = client.subscribe(t.topic.clone(), t.qos).await {
                    tracing::warn!(workflowId = %workflow_id, node = %node.name, error = %e, "MQTT Trigger: could not subscribe; reconnecting");
                    subscribe_ok = false;
                    break;
                }
            }
            if !subscribe_ok {
                let _ = client.disconnect().await;
                tokio::time::sleep(MQTT_RECONNECT_BACKOFF).await;
                continue;
            }

            loop {
                match eventloop.poll().await {
                    Ok(Event::Incoming(Incoming::Publish(p))) => {
                        let item = build_item(&p.topic, &p.payload, json_parse_body, only_message);
                        if parallel_processing {
                            let n8n = n8n.clone();
                            let workflow_id = workflow_id.clone();
                            let node_name = node.name.clone();
                            tokio::spawn(async move { fire(&n8n, &workflow_id, &node_name, vec![item]).await });
                        } else {
                            fire(&n8n, &workflow_id, &node.name, vec![item]).await;
                        }
                    }
                    Ok(_) => {}
                    Err(e) => {
                        tracing::warn!(workflowId = %workflow_id, node = %node.name, error = %e, "MQTT Trigger: connection error; reconnecting");
                        break;
                    }
                }
            }
            tokio::time::sleep(MQTT_RECONNECT_BACKOFF).await;
        }
    }
}
