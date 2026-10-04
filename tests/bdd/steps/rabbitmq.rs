//! Direct RabbitMQ access for the `@requires-rabbitmq` scenarios
//! (`04-nodes/rabbitmq.feature`): queue cleanup and reading back what the
//! node actually published, the way the node's own JSON output can't show
//! (headers, routing).

use crate::steps::{docstring, eventually};
use crate::world::R8rWorld;
use cucumber::gherkin::Step;
use cucumber::{given, then};
use lapin::options::{BasicGetOptions, ExchangeDeclareOptions, ExchangeDeleteOptions, QueueBindOptions, QueueDeclareOptions, QueueDeleteOptions};
use lapin::types::FieldTable;
use lapin::{Channel, Connection, ConnectionProperties, ExchangeKind};
use serde_json::Value;
use std::time::Duration;

fn amqp_url() -> String {
    let host = std::env::var("R8R_BDD_RABBITMQ_HOST").unwrap_or_else(|_| "127.0.0.1".into());
    let port = std::env::var("R8R_BDD_RABBITMQ_PORT").unwrap_or_else(|_| "5672".into());
    format!("amqp://guest:guest@{host}:{port}/%2f")
}

async fn channel() -> Channel {
    let conn = Connection::connect(&amqp_url(), ConnectionProperties::default()).await.expect("connect to the bdd rabbitmq instance");
    conn.create_channel().await.expect("create channel")
}

/// Recreates the queue empty, durable and with a non-default argument, as a
/// deployment would have declared it: the node only checks that it exists
/// (n8n's `checkQueue`), so it must work against whatever arguments the
/// queue was declared with.
#[given(expr = "the RabbitMQ queue {string} is empty")]
#[then(expr = "the RabbitMQ queue {string} is empty")]
async fn queue_empty(_w: &mut R8rWorld, queue: String) {
    let ch = channel().await;
    let _ = ch.queue_delete(queue.clone().into(), QueueDeleteOptions::default()).await;
    let mut args = FieldTable::default();
    args.insert("x-max-length".into(), lapin::types::AMQPValue::LongInt(1000));
    ch.queue_declare(queue.into(), QueueDeclareOptions { durable: true, ..Default::default() }, args).await.expect("declare queue");
}

#[given(expr = "the RabbitMQ queue {string} does not exist")]
async fn queue_absent(_w: &mut R8rWorld, queue: String) {
    let ch = channel().await;
    let _ = ch.queue_delete(queue.into(), QueueDeleteOptions::default()).await;
}

/// A `type` exchange with `queue` bound to it (routing key `#` for topic
/// exchanges, empty otherwise).
#[given(expr = "the RabbitMQ {word} exchange {string} routes to the queue {string}")]
async fn exchange_bound(_w: &mut R8rWorld, kind: String, exchange: String, queue: String) {
    let ch = channel().await;
    let exchange_kind = match kind.as_str() {
        "direct" => ExchangeKind::Direct,
        "topic" => ExchangeKind::Topic,
        "headers" => ExchangeKind::Headers,
        _ => ExchangeKind::Fanout,
    };
    let _ = ch.exchange_delete(exchange.clone().into(), ExchangeDeleteOptions::default()).await;
    ch.exchange_declare(exchange.clone().into(), exchange_kind, ExchangeDeclareOptions::default(), FieldTable::default())
        .await
        .expect("declare exchange");
    let key = if kind == "topic" { "#" } else { "" };
    ch.queue_bind(queue.into(), exchange.into(), key.into(), QueueBindOptions::default(), FieldTable::default()).await.expect("bind queue");
}

fn field_table_to_json(table: &FieldTable) -> Value {
    let mut map = serde_json::Map::new();
    for (k, v) in table.inner() {
        map.insert(k.to_string(), lapin_value_to_json(v));
    }
    Value::Object(map)
}

fn lapin_value_to_json(v: &lapin::types::AMQPValue) -> Value {
    use lapin::types::AMQPValue::*;
    match v {
        LongString(s) => Value::String(s.to_string()),
        ShortString(s) => Value::String(s.to_string()),
        Boolean(b) => Value::Bool(*b),
        LongInt(n) => Value::Number((*n).into()),
        LongLongInt(n) => Value::Number((*n).into()),
        ShortInt(n) => Value::Number((*n).into()),
        ShortShortInt(n) => Value::Number((*n).into()),
        other => Value::String(format!("{other:?}")),
    }
}

/// `the RabbitMQ queue "<q>" receives a message matching:` takes a JSON
/// docstring with optional top-level `body` / `headers` keys, each checked
/// as a *subset* of the actually-received message (extra keys/headers in
/// the real message are ignored, matching the repo's "outputs items
/// matching" convention). Polls for up to 5s since the node's publish may
/// still be in flight when the `Then` step runs.
#[then(expr = "the RabbitMQ queue {string} receives a message matching:")]
async fn queue_receives(_w: &mut R8rWorld, queue: String, step: &Step) {
    let expected: Value = serde_json::from_str(docstring(step)).expect("json docstring");
    let ch = channel().await;
    let msg = eventually(Duration::from_secs(5), || {
        let ch = &ch;
        let queue = queue.clone();
        async move { ch.basic_get(queue.into(), BasicGetOptions { no_ack: true }).await.ok().flatten() }
    })
    .await
    .unwrap_or_else(|| panic!("no message arrived on RabbitMQ queue {queue:?} within 5s"));

    let data = msg.delivery.data.clone();
    let body: Value = serde_json::from_slice(&data).unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&data).into_owned()));
    let headers = msg.delivery.properties.headers().as_ref().map(field_table_to_json).unwrap_or(Value::Object(Default::default()));

    if let Some(expected_body) = expected.get("body") {
        assert_subset(expected_body, &body, "body");
    }
    if let Some(expected_headers) = expected.get("headers") {
        assert_subset(expected_headers, &headers, "headers");
    }
    if expected.get("body").is_none() && expected.get("headers").is_none() {
        assert_subset(&expected, &body, "body");
    }
}

fn assert_subset(expected: &Value, actual: &Value, what: &str) {
    match (expected, actual) {
        (Value::Object(exp), Value::Object(act)) => {
            for (k, v) in exp {
                let a = act.get(k).unwrap_or_else(|| panic!("missing {what} key {k:?}; actual {what} = {actual}"));
                assert_subset(v, a, what);
            }
        }
        (e, a) => assert_eq!(e, a, "{what} mismatch"),
    }
}
