//! Kafka node (plan task 1.13): `send` against the `kafka` credential.
//! Faithful to n8n's `KafkaV1.node.js` (typeVersion 1 -- the `Kafka`
//! `VersionedNodeType`'s `defaultVersion`, i.e. what the editor creates for
//! a brand-new node).
//!
//! Crate: `rskafka` (pure Rust, no C/`librdkafka` build dependency -- the
//! brief's stated preference). It speaks the Kafka wire protocol directly
//! and exposes per-partition clients rather than a topic-level producer
//! with automatic partitioning/batching: this node always produces to
//! **partition 0** of a topic (documented gap -- no key-based partitioner;
//! fine for the single-partition topics `auto.create.topics.enable`
//! creates, which is what the BDD broker uses, but not a faithful
//! reproduction of kafkajs' default partitioner for a multi-partition
//! topic).
//!
//! Output cardinality mirrors kafkajs' `producer.sendBatch`: **one item
//! per distinct topic actually written to** (kafkajs returns one
//! `RecordMetadata` per topic-partition in the batch, not one per input
//! item), falling back to a single `{success: true}` item if nothing was
//! produced (empty input). `useSchemaRegistry` is not implemented (no
//! registry client in this build): it surfaces a clear "not supported"
//! error rather than silently skipping encoding.

use crate::n8n::node::{ExecCtx, NodeError, NodeResult, NodeType};
use crate::n8n::types::{Item, NodeOutput};
use rskafka::client::partition::{Compression, OffsetAt, PartitionClient, UnknownTopicHandling};
use rskafka::client::{Client, ClientBuilder, Credentials, SaslConfig};
use rskafka::record::Record;
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::sync::Arc;

pub struct Kafka;

fn cred_str<'a>(cred: &'a Value, key: &str) -> Option<&'a str> {
    cred.get(key).and_then(Value::as_str).filter(|s| !s.is_empty())
}

async fn connect(cred: &Value) -> NodeResult<Client> {
    let brokers: Vec<String> = cred_str(cred, "brokers").unwrap_or("").split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
    if brokers.is_empty() {
        return Err(NodeError::new("No Kafka brokers configured"));
    }
    let client_id = cred_str(cred, "clientId").unwrap_or("").to_string();
    let mut builder = ClientBuilder::new(brokers);
    if !client_id.is_empty() {
        builder = builder.client_id(client_id);
    }

    let authentication = cred.get("authentication").and_then(Value::as_bool).unwrap_or(false);
    if authentication {
        let (Some(username), Some(password)) = (cred_str(cred, "username"), cred_str(cred, "password")) else {
            return Err(NodeError::new("Username and password are required for authentication"));
        };
        let credentials = Credentials::new(username.to_string(), password.to_string());
        let sasl = match cred_str(cred, "saslMechanism").unwrap_or("plain") {
            "scram-sha-256" => SaslConfig::ScramSha256(credentials),
            "scram-sha-512" => SaslConfig::ScramSha512(credentials),
            _ => SaslConfig::Plain(credentials),
        };
        builder = builder.sasl_config(sasl);
    }

    // `ssl: true` with no CA/client-cert fields is the credential's own
    // default; r8r only wires up custom TLS trust/client-auth when a CA is
    // actually given -- a plain `ssl: true` (default, no certs) connects
    // without TLS rather than silently failing or trusting any cert.
    // Documented gap: TLS is untested here (`tests/bdd/services.sh`'s Kafka
    // broker is PLAINTEXT-only).
    if let Some(ca) = cred_str(cred, "ca") {
        let mut root_store = rustls::RootCertStore::empty();
        let mut reader = std::io::BufReader::new(ca.as_bytes());
        for cert in rustls_pemfile::certs(&mut reader) {
            let cert = cert.map_err(|e| NodeError::new(format!("Invalid Kafka CA certificate: {e}")))?;
            root_store.add(cert).map_err(|e| NodeError::new(format!("Invalid Kafka CA certificate: {e}")))?;
        }
        let tls_config = rustls::ClientConfig::builder().with_root_certificates(root_store).with_no_client_auth();
        builder = builder.tls_config(Arc::new(tls_config));
    }

    // `ClientBuilder::build` retries broker connections with its own
    // (effectively unbounded) backoff and never gives up on its own; wrap
    // it so an unreachable broker fails the node instead of hanging.
    match tokio::time::timeout(std::time::Duration::from_secs(8), builder.build()).await {
        Ok(Ok(client)) => Ok(client),
        Ok(Err(e)) => Err(NodeError::new(e.to_string())),
        Err(_) => Err(NodeError::new("Timed out connecting to the Kafka broker")),
    }
}

fn headers_from_ui(options: &Value) -> BTreeMap<String, Vec<u8>> {
    let mut out = BTreeMap::new();
    if let Some(rows) = options.get("headerValues").and_then(Value::as_array) {
        for row in rows {
            let (Some(k), Some(v)) = (row.get("key").and_then(Value::as_str), row.get("value")) else { continue };
            let v = match v {
                Value::String(s) => s.clone(),
                Value::Null => String::new(),
                other => other.to_string(),
            };
            out.insert(k.to_string(), v.into_bytes());
        }
    }
    out
}

struct ResolvedMessage {
    topic: String,
    key: Option<Vec<u8>>,
    value: Vec<u8>,
    headers: BTreeMap<String, Vec<u8>>,
}

fn resolve_message(ctx: &ExecCtx<'_>, i: usize, item: &Item, send_input_data: bool) -> NodeResult<ResolvedMessage> {
    let topic = ctx.param_str("topic", i, "")?;
    let message = if send_input_data { serde_json::to_string(&item.json).unwrap_or_default() } else { ctx.param_str("message", i, "")? };
    let use_key = ctx.param_bool("useKey", i, false)?;
    let key = if use_key { Some(ctx.param_str("key", i, "")?.into_bytes()) } else { None };

    let json_parameters = ctx.param_bool("jsonParameters", i, false)?;
    let headers = if json_parameters {
        let raw = ctx.param_str("headerParametersJson", i, "")?;
        if raw.is_empty() {
            BTreeMap::new()
        } else {
            let parsed: Value = serde_json::from_str(&raw).map_err(|_| NodeError::new("Headers must be a valid json").at(i))?;
            parsed.as_object().map(|m| m.iter().map(|(k, v)| (k.clone(), value_to_string(v).into_bytes())).collect()).unwrap_or_default()
        }
    } else {
        headers_from_ui(&ctx.param("headersUi", i)?)
    };

    Ok(ResolvedMessage { topic, key, value: message.into_bytes(), headers })
}

fn value_to_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

#[async_trait::async_trait]
impl NodeType for Kafka {
    fn type_name(&self) -> &'static str {
        "n8n-nodes-base.kafka"
    }

    async fn execute(&self, ctx: &mut ExecCtx<'_>) -> NodeResult<NodeOutput> {
        let use_schema_registry = ctx.param_bool("useSchemaRegistry", 0, false)?;
        if use_schema_registry {
            return Err(NodeError::new("Kafka \"useSchemaRegistry\" is not supported natively yet"));
        }

        let (_, cred) = ctx.credentials("kafka").await?;
        let send_input_data = ctx.param_bool("sendInputData", 0, true)?;
        let acks = ctx.param_bool("options.acks", 0, false)?;
        let compression = if ctx.param_bool("options.compression", 0, false)? { Compression::Gzip } else { Compression::NoCompression };
        let _timeout_ms = ctx.param_f64("options.timeout", 0, 30000.0)?;
        let _ = acks; // r8r always waits for the leader's ack (rskafka has no fire-and-forget acks=0 mode); documented below.

        let input = if ctx.input().is_empty() { vec![Item::default()] } else { ctx.input().to_vec() };

        let mut resolved = Vec::with_capacity(input.len());
        for (i, item) in input.iter().enumerate() {
            resolved.push(resolve_message(ctx, i, item, send_input_data)?);
        }

        match produce_all(&cred, resolved, compression).await {
            Ok(items) => Ok(vec![items]),
            Err(e) if ctx.continue_on_fail() => {
                ctx.push_error_item(&e, 0);
                Ok(vec![vec![]])
            }
            Err(e) => Err(e),
        }
    }
}

/// Produces every message, grouped by topic (preserving first-seen topic
/// order), each group as one `produce` batch to that topic's partition 0 --
/// mirroring kafkajs' per-topic `ProducerRecord` batching in `sendBatch`.
/// Returns one output item per topic (kafkajs' per-topic-partition
/// `RecordMetadata`), or a single `{success: true}` item if there was
/// nothing to send.
async fn produce_all(cred: &Value, messages: Vec<ResolvedMessage>, compression: Compression) -> NodeResult<Vec<Item>> {
    if messages.is_empty() {
        let mut m = Map::new();
        m.insert("success".into(), Value::Bool(true));
        return Ok(vec![Item::new(m)]);
    }

    let client = connect(cred).await?;

    let mut order: Vec<String> = Vec::new();
    let mut by_topic: BTreeMap<String, Vec<ResolvedMessage>> = BTreeMap::new();
    for msg in messages {
        if !by_topic.contains_key(&msg.topic) {
            order.push(msg.topic.clone());
        }
        by_topic.entry(msg.topic.clone()).or_default().push(msg);
    }

    let mut out = Vec::with_capacity(order.len());
    for topic in order {
        let group = by_topic.remove(&topic).unwrap_or_default();
        let partition_client = client
            .partition_client(topic.clone(), 0, UnknownTopicHandling::Retry)
            .await
            .map_err(|e| NodeError::new(format!("Verify your Kafka configuration: {e}")))?;
        let item = produce_topic(&partition_client, &topic, group, compression).await?;
        out.push(item);
    }
    Ok(out)
}

async fn produce_topic(partition_client: &PartitionClient, topic: &str, group: Vec<ResolvedMessage>, compression: Compression) -> NodeResult<Item> {
    let now = chrono::Utc::now();
    let records: Vec<Record> = group.into_iter().map(|m| Record { key: m.key, value: Some(m.value), headers: m.headers, timestamp: now }).collect();
    let offsets = partition_client.produce(records, compression).await.map_err(|e| NodeError::new(e.to_string()))?;
    let base_offset = offsets.first().copied().unwrap_or(0);
    let log_start_offset = partition_client.get_offset(OffsetAt::Earliest).await.unwrap_or(0);

    let mut m = Map::new();
    m.insert("topicName".into(), json!(topic));
    m.insert("partition".into(), json!(0));
    m.insert("errorCode".into(), json!(0));
    m.insert("baseOffset".into(), json!(base_offset.to_string()));
    m.insert("logAppendTime".into(), json!("-1"));
    m.insert("logStartOffset".into(), json!(log_start_offset.to_string()));
    Ok(Item::new(m))
}
