//! Direct MQTT access for the `@requires-mqtt` scenarios
//! (`04-nodes/mqtt.feature`): a background subscriber (mirroring
//! `steps/push.rs`'s push-channel listener) is needed because MQTT is
//! pub/sub -- a message published before anyone subscribes is gone, so the
//! subscription must be live *before* the workflow executes and publishes.

use crate::steps::{docstring, eventually};
use crate::world::R8rWorld;
use cucumber::gherkin::Step;
use cucumber::{given, then, when};
use rumqttc::{AsyncClient, Event, Incoming, MqttOptions, Outgoing, QoS};
use serde_json::Value;
use std::time::Duration;

fn mqtt_host_port() -> (String, u16) {
    let host = std::env::var("R8R_BDD_MQTT_HOST").unwrap_or_else(|_| "127.0.0.1".into());
    let port: u16 = std::env::var("R8R_BDD_MQTT_PORT").unwrap_or_else(|_| "1883".into()).parse().unwrap_or(1883);
    (host, port)
}

/// Connects a dedicated test client, subscribes to `topic`, and spawns a
/// background task collecting every `Publish` the broker delivers into
/// `w.mqtt_messages`. Waits for the SubAck so the subscription is
/// guaranteed live once this step returns.
#[given(expr = "I am subscribed to the MQTT topic {string}")]
async fn subscribe(w: &mut R8rWorld, topic: String) {
    let (host, port) = mqtt_host_port();
    let client_id = format!("bdd-{}", uuid::Uuid::new_v4().simple());
    let opts = MqttOptions::new(client_id, host, port);
    let (client, mut eventloop) = AsyncClient::new(opts, 50);
    client.subscribe(&topic, QoS::AtLeastOnce).await.expect("subscribe to bdd mqtt topic");

    let messages = w.mqtt_messages.clone();
    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
    w.mqtt_task = Some(tokio::spawn(async move {
        let mut ready_tx = Some(ready_tx);
        loop {
            match eventloop.poll().await {
                Ok(Event::Incoming(Incoming::SubAck(_))) => {
                    if let Some(tx) = ready_tx.take() {
                        let _ = tx.send(());
                    }
                }
                Ok(Event::Incoming(Incoming::Publish(p))) => {
                    messages.lock().unwrap().push((p.topic.clone(), p.payload.to_vec()));
                }
                Ok(_) => {}
                Err(_) => break,
            }
        }
    }));
    tokio::time::timeout(Duration::from_secs(5), ready_rx).await.expect("mqtt suback timeout").ok();
}

/// Publishes directly to `topic` (QoS 0, no retain) for the MQTT Trigger's
/// `@requires-mqtt` scenarios. Waits for the publish to actually leave the
/// socket before returning (mirrors `mqtt.rs`'s own node code).
#[when(expr = "I publish to the MQTT topic {string}:")]
async fn publish(w: &mut R8rWorld, topic: String, step: &Step) {
    let body = w.expand(docstring(step).trim());
    let (host, port) = mqtt_host_port();
    let client_id = format!("bdd-pub-{}", uuid::Uuid::new_v4().simple());
    let opts = MqttOptions::new(client_id, host, port);
    let (client, mut eventloop) = AsyncClient::new(opts, 10);
    client.publish(topic, QoS::AtMostOnce, false, body.into_bytes()).await.expect("publish to the bdd mqtt broker");
    loop {
        match tokio::time::timeout(Duration::from_secs(5), eventloop.poll()).await {
            Ok(Ok(Event::Outgoing(Outgoing::Publish(_)))) => break,
            Ok(Ok(_)) => continue,
            Ok(Err(e)) => panic!("mqtt publish failed: {e}"),
            Err(_) => panic!("timed out waiting for the mqtt publish to leave the socket"),
        }
    }
    let _ = client.disconnect().await;
}

fn parse_body(payload: &[u8]) -> Value {
    serde_json::from_slice(payload).unwrap_or_else(|_| Value::String(String::from_utf8_lossy(payload).into_owned()))
}

fn assert_subset(expected: &Value, actual: &Value) {
    match (expected, actual) {
        (Value::Object(exp), Value::Object(act)) => {
            for (k, v) in exp {
                let a = act.get(k).unwrap_or_else(|| panic!("missing key {k:?}; actual = {actual}"));
                assert_subset(v, a);
            }
        }
        (e, a) => assert_eq!(e, a, "mismatch"),
    }
}

/// `the MQTT topic "<t>" receives a message matching:` -- the docstring is
/// the expected body, checked as a *subset* (object keys) or exact value
/// (string/other).
#[then(expr = "the MQTT topic {string} receives a message matching:")]
async fn receives(w: &mut R8rWorld, topic: String, step: &Step) {
    let expected: Value = serde_json::from_str(docstring(step)).expect("json docstring");
    let found = eventually(Duration::from_secs(5), || {
        let messages = w.mqtt_messages.clone();
        let topic = topic.clone();
        async move { messages.lock().unwrap().iter().find(|(t, _)| *t == topic).map(|(_, p)| parse_body(p)) }
    })
    .await;
    let body = found.unwrap_or_else(|| panic!("no MQTT message arrived on topic {topic:?} within 5s"));
    assert_subset(&expected, &body);
}

#[then(expr = "the MQTT topic {string} received no message")]
async fn received_none(w: &mut R8rWorld, topic: String) {
    tokio::time::sleep(Duration::from_millis(500)).await;
    let any = w.mqtt_messages.lock().unwrap().iter().any(|(t, _)| *t == topic);
    assert!(!any, "expected no message on topic {topic:?}, but one arrived");
}
