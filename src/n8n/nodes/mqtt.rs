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
use crate::n8n::types::{Item, NodeOutput};
use rumqttc::{AsyncClient, Event, Incoming, MqttOptions, Outgoing, QoS, Transport};
use serde_json::Value;
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
