//! Direct Kafka access for the `@requires-kafka` scenarios
//! (`04-nodes/kafka.feature`): reading back what the node actually
//! produced (key, headers), since the node's own JSON output is per-topic
//! batch metadata, not the message content.

use crate::steps::{docstring, eventually};
use crate::world::R8rWorld;
use cucumber::gherkin::Step;
use cucumber::{then, when};
use rskafka::client::partition::{Compression, OffsetAt, UnknownTopicHandling};
use rskafka::client::{Client, ClientBuilder};
use rskafka::record::Record;
use serde_json::Value;
use std::time::Duration;

fn brokers() -> Vec<String> {
    let host = std::env::var("R8R_BDD_KAFKA_HOST").unwrap_or_else(|_| "127.0.0.1".into());
    let port = std::env::var("R8R_BDD_KAFKA_PORT").unwrap_or_else(|_| "9092".into());
    vec![format!("{host}:{port}")]
}

async fn client() -> Client {
    ClientBuilder::new(brokers()).build().await.expect("connect to the bdd kafka broker")
}

/// Publishes directly to partition 0 of `topic` for the Kafka Trigger's
/// `@requires-kafka` scenarios -- relies on the broker's own
/// `auto.create.topics.enable` to create the topic on first produce, same
/// as the Kafka action node.
#[when(expr = "I publish to the Kafka topic {string}:")]
async fn publish(w: &mut R8rWorld, topic: String, step: &Step) {
    let body = w.expand(docstring(step).trim());
    let client = client().await;
    let partition_client = client
        .partition_client(topic.clone(), 0, UnknownTopicHandling::Retry)
        .await
        .unwrap_or_else(|e| panic!("cannot open partition client for Kafka topic {topic:?}: {e}"));
    let record = Record { key: None, value: Some(body.into_bytes()), headers: Default::default(), timestamp: chrono::Utc::now() };
    partition_client.produce(vec![record], Compression::NoCompression).await.unwrap_or_else(|e| panic!("could not produce to Kafka topic {topic:?}: {e}"));
}

fn body_matches(expected: &Value, value: &Option<Vec<u8>>) -> bool {
    let Some(bytes) = value else { return expected.is_null() };
    let actual: Value = serde_json::from_slice(bytes).unwrap_or_else(|_| Value::String(String::from_utf8_lossy(bytes).into_owned()));
    subset(expected, &actual)
}

fn subset(expected: &Value, actual: &Value) -> bool {
    match (expected, actual) {
        (Value::Object(exp), Value::Object(act)) => exp.iter().all(|(k, v)| act.get(k).is_some_and(|a| subset(v, a))),
        (e, a) => e == a,
    }
}

/// `the Kafka topic "<t>" receives a message matching:` takes a JSON
/// docstring with optional top-level `value`, `key` and `headers` keys
/// (each matched as a subset / exact string); when none of those keys are
/// present the whole docstring is matched against the record value.
#[then(expr = "the Kafka topic {string} receives a message matching:")]
async fn receives(_w: &mut R8rWorld, topic: String, step: &Step) {
    let expected: Value = serde_json::from_str(docstring(step)).expect("json docstring");
    let client = client().await;
    let partition_client = client
        .partition_client(topic.clone(), 0, UnknownTopicHandling::Retry)
        .await
        .unwrap_or_else(|e| panic!("cannot open partition client for Kafka topic {topic:?}: {e}"));

    let found = eventually(Duration::from_secs(10), || {
        let partition_client = &partition_client;
        let expected = expected.clone();
        async move {
            let earliest = partition_client.get_offset(OffsetAt::Earliest).await.ok()?;
            let (records, _hw) = partition_client.fetch_records(earliest, 1..10_000_000, 1000).await.ok()?;
            records.into_iter().find(|r| record_matches(&expected, r))
        }
    })
    .await;
    assert!(found.is_some(), "no matching message arrived on Kafka topic {topic:?} within 10s");
}

fn record_matches(expected: &Value, record: &rskafka::record::RecordAndOffset) -> bool {
    if let Some(expected_value) = expected.get("value") {
        if !body_matches(expected_value, &record.record.value) {
            return false;
        }
    }
    if let Some(expected_key) = expected.get("key") {
        let actual_key = record.record.key.as_deref().map(|k| String::from_utf8_lossy(k).into_owned());
        if actual_key.as_deref() != expected_key.as_str() {
            return false;
        }
    }
    if let Some(Value::Object(expected_headers)) = expected.get("headers") {
        for (k, v) in expected_headers {
            let Some(actual) = record.record.headers.get(k) else { return false };
            if String::from_utf8_lossy(actual) != v.as_str().unwrap_or_default() {
                return false;
            }
        }
    }
    if expected.get("value").is_none() && expected.get("key").is_none() && expected.get("headers").is_none() {
        return body_matches(expected, &record.record.value);
    }
    true
}
