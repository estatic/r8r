//! RabbitMQ node (plan task 1.13): `sendMessage` (`mode: queue | exchange`)
//! against the `rabbitmq` credential. Faithful to n8n's `RabbitMQ.node.js`
//! (typeVersions 1, 1.1, 1.2) and `GenericFunctions.js`. Crate: `lapin`
//! (tokio executor, rustls for `amqps://`).
//!
//! `deleteMessage` only makes sense for a message delivered by a RabbitMQ
//! Trigger node (`this.sendResponse(...)`, acking/nacking the trigger's
//! delivery) -- trigger nodes wait for leader election (plan task 4.1) and
//! are out of scope here, so it surfaces a clear "not supported" error
//! instead of silently doing nothing.
//!
//! Queues and exchanges: like the reference action node, r8r only checks
//! that they exist (`checkQueue`/`checkExchange`, a passive declare), so a
//! missing one fails the node with NOT_FOUND and an existing one is used as
//! declared, whatever its arguments. `durable`/`exclusive`/`autoDelete`
//! take effect only with `options.assertQueue`/`assertExchange` (Trigger
//! node options, never set by the action node's editor form).

use crate::n8n::node::{ExecCtx, NodeError, NodeResult, NodeType};
use crate::n8n::server::triggers::{fire, fire_and_wait, resolve_credential, LongLivedTrigger};
use crate::n8n::server::N8n;
use crate::n8n::types::{Item, NodeOutput};
use crate::n8n::workflow::Node;
use base64::Engine;
use futures_util::StreamExt;
use lapin::message::Delivery;
use lapin::options::{BasicAckOptions, BasicConsumeOptions, BasicNackOptions, BasicPublishOptions, BasicQosOptions, ExchangeDeclareOptions, QueueBindOptions, QueueDeclareOptions};
use lapin::types::{AMQPValue, FieldTable, LongString, ShortString};
use lapin::{BasicProperties, Channel, Connection, ConnectionProperties, ExchangeKind};
use serde_json::{json, Map, Value};
use std::sync::Arc;

pub struct RabbitMq;

// ---- credential -> AMQP URI -------------------------------------------------------

/// Percent-encodes everything outside of AMQP URI "unreserved" characters,
/// matching the escaping `amq-protocol-uri` expects in the userinfo/vhost
/// segments (e.g. `/` in a vhost must become `%2F`).
fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn cred_str<'a>(cred: &'a Value, key: &str) -> Option<&'a str> {
    cred.get(key).and_then(Value::as_str).filter(|s| !s.is_empty())
}

fn cred_bool(cred: &Value, key: &str, default: bool) -> bool {
    cred.get(key).and_then(|v| v.as_bool().or_else(|| v.as_str().map(|s| s == "true"))).unwrap_or(default)
}

/// Builds the `amqp(s)://user:pass@host:port/vhost` URI
/// `rabbitmqConnect`/amqplib expects. SSL client-certificate ("passwordless"
/// / SASL EXTERNAL) options are not supported in this build (no TLS client
/// auth wiring here): a credential with `ssl: true` and `passwordless: true`
/// still connects over `amqps://` with normal username/password SASL, which
/// fails clearly against a broker that requires certificate auth rather
/// than silently misbehaving.
fn amqp_uri(cred: &Value) -> String {
    let scheme = if cred_bool(cred, "ssl", false) { "amqps" } else { "amqp" };
    let host = cred_str(cred, "hostname").unwrap_or("localhost");
    let port = cred.get("port").and_then(|v| v.as_u64().or_else(|| v.as_str().and_then(|s| s.parse().ok()))).unwrap_or(5672);
    let userinfo = match (cred_str(cred, "username"), cred_str(cred, "password")) {
        (Some(u), Some(p)) => format!("{}:{}@", percent_encode(u), percent_encode(p)),
        (Some(u), None) => format!("{}@", percent_encode(u)),
        _ => String::new(),
    };
    let vhost = cred_str(cred, "vhost").unwrap_or("/");
    format!("{scheme}://{userinfo}{host}:{port}/{}", percent_encode(vhost))
}

async fn connect(cred: &Value) -> NodeResult<(Connection, Channel)> {
    let uri = amqp_uri(cred);
    let conn = Connection::connect(&uri, ConnectionProperties::default()).await.map_err(|e| NodeError::new(e.to_string()))?;
    let channel = conn.create_channel().await.map_err(|e| NodeError::new(e.to_string()))?;
    Ok((conn, channel))
}

async fn close(conn: Connection, channel: Channel) {
    let _ = channel.close(200, ShortString::from("")).await;
    let _ = conn.close(200, ShortString::from("")).await;
}

// ---- options -----------------------------------------------------------------

fn fixed_collection_pairs(options: &Value, collection: &str, inner: &str) -> Vec<(String, String)> {
    let Some(rows) = options.get(collection).and_then(|c| c.get(inner)).and_then(Value::as_array) else { return vec![] };
    rows.iter()
        .filter_map(|row| {
            let key = row.get("key").and_then(Value::as_str)?.to_string();
            let value = row.get("value").map(value_to_string).unwrap_or_default();
            Some((key, value))
        })
        .collect()
}

fn value_to_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

fn headers_field_table(options: &Value) -> FieldTable {
    let mut table = FieldTable::default();
    for (k, v) in fixed_collection_pairs(options, "headers", "header") {
        table.insert(ShortString::from(k), AMQPValue::LongString(LongString::from(v)));
    }
    table
}

/// `parsePublishArguments`: `options.arguments.argument` entries are merged
/// directly into the publish call's options object in the reference
/// implementation (not nested under an "arguments" AMQP property). r8r maps
/// each key to the matching `BasicProperties` field when the name is a
/// recognised AMQP basic property (content-type, expiration, priority, ...);
/// an unrecognised key is dropped (documented gap -- amqplib's object spread
/// has no equivalent fixed-field restriction).
fn apply_arguments(mut props: BasicProperties, options: &Value) -> BasicProperties {
    for (key, value) in fixed_collection_pairs(options, "arguments", "argument") {
        props = match key.as_str() {
            "contentType" => props.with_content_type(ShortString::from(value)),
            "contentEncoding" => props.with_content_encoding(ShortString::from(value)),
            "deliveryMode" => value.parse::<u8>().map(|v| props.clone().with_delivery_mode(v)).unwrap_or(props),
            "priority" => value.parse::<u8>().map(|v| props.clone().with_priority(v)).unwrap_or(props),
            "correlationId" => props.with_correlation_id(ShortString::from(value)),
            "replyTo" => props.with_reply_to(ShortString::from(value)),
            "expiration" => props.with_expiration(ShortString::from(value)),
            "messageId" => props.with_message_id(ShortString::from(value)),
            "type" => props.with_type(ShortString::from(value)),
            "userId" => props.with_user_id(ShortString::from(value)),
            "appId" => props.with_app_id(ShortString::from(value)),
            "clusterId" => props.with_cluster_id(ShortString::from(value)),
            _ => props,
        };
    }
    props
}

// ---- declare helpers -----------------------------------------------------------

/// n8n's `rabbitmqConnectQueue`: `assertQueue` (declare with the options)
/// only when `options.assertQueue` is set -- the action node has no such
/// option, so it always runs `checkQueue`, a passive declare that fails if
/// the queue is missing and never conflicts with how it was declared.
async fn ensure_queue(channel: &Channel, queue: &str, options: &Value) -> NodeResult<()> {
    if !options.get("assertQueue").and_then(Value::as_bool).unwrap_or(false) {
        let passive = QueueDeclareOptions { passive: true, ..Default::default() };
        channel.queue_declare(ShortString::from(queue.to_string()), passive, FieldTable::default()).await.map_err(|e| NodeError::new(e.to_string()))?;
        return Ok(());
    }
    let durable = options.get("durable").and_then(Value::as_bool).unwrap_or(true);
    let auto_delete = options.get("autoDelete").and_then(Value::as_bool).unwrap_or(false);
    let exclusive = options.get("exclusive").and_then(Value::as_bool).unwrap_or(false);
    let decl_options = QueueDeclareOptions { durable, auto_delete, exclusive, ..Default::default() };
    channel.queue_declare(ShortString::from(queue.to_string()), decl_options, FieldTable::default()).await.map_err(|e| NodeError::new(e.to_string()))?;
    Ok(())
}

/// n8n's `rabbitmqConnectExchange`: like [`ensure_queue`], `checkExchange`
/// unless `options.assertExchange` is set.
async fn ensure_exchange(channel: &Channel, exchange: &str, exchange_type: &str, options: &Value) -> NodeResult<()> {
    if !options.get("assertExchange").and_then(Value::as_bool).unwrap_or(false) {
        let passive = ExchangeDeclareOptions { passive: true, ..Default::default() };
        channel
            .exchange_declare(ShortString::from(exchange.to_string()), ExchangeKind::Direct, passive, FieldTable::default())
            .await
            .map_err(|e| NodeError::new(e.to_string()))?;
        return Ok(());
    }
    let kind = match exchange_type {
        "direct" => ExchangeKind::Direct,
        "topic" => ExchangeKind::Topic,
        "headers" => ExchangeKind::Headers,
        "fanout" => ExchangeKind::Fanout,
        other => ExchangeKind::Custom(other.to_string()),
    };
    let durable = options.get("durable").and_then(Value::as_bool).unwrap_or(true);
    let auto_delete = options.get("autoDelete").and_then(Value::as_bool).unwrap_or(false);
    let decl_options = ExchangeDeclareOptions { durable, auto_delete, ..Default::default() };
    let mut arguments = FieldTable::default();
    if let Some(alt) = options.get("alternateExchange").and_then(Value::as_str).filter(|s| !s.is_empty()) {
        arguments.insert(ShortString::from("alternate-exchange"), AMQPValue::LongString(LongString::from(alt.to_string())));
    }
    channel.exchange_declare(ShortString::from(exchange.to_string()), kind, decl_options, arguments).await.map_err(|e| NodeError::new(e.to_string()))?;
    Ok(())
}

// ---- node -----------------------------------------------------------------

#[async_trait::async_trait]
impl NodeType for RabbitMq {
    fn type_name(&self) -> &'static str {
        "n8n-nodes-base.rabbitmq"
    }

    async fn execute(&self, ctx: &mut ExecCtx<'_>) -> NodeResult<NodeOutput> {
        let operation = ctx.param_str("operation", 0, "sendMessage")?;
        if operation != "sendMessage" {
            // `deleteMessage` requires the delivery context a RabbitMQ
            // Trigger node provides (ack/nack on `this.sendResponse`); there
            // is no such context outside a trigger-originated execution.
            return Err(NodeError::new(format!("RabbitMQ \"{operation}\" is not supported natively yet")));
        }

        let (_, cred) = ctx.credentials("rabbitmq").await?;
        let mode = ctx.param_str("mode", 0, "queue")?;
        let send_input_data = ctx.param_bool("sendInputData", 0, true)?;
        let options = ctx.param("options", 0)?;
        let node_version = ctx.node.type_version;

        let (conn, channel) = connect(&cred).await?;

        let input = if ctx.input().is_empty() { vec![Item::default()] } else { ctx.input().to_vec() };
        let mut out = Vec::with_capacity(input.len());

        let setup: NodeResult<()> = async {
            match mode.as_str() {
                "queue" => {
                    let queue = ctx.param_str("queue", 0, "")?;
                    ensure_queue(&channel, &queue, &options).await
                }
                "exchange" => {
                    let exchange = ctx.param_str("exchange", 0, "")?;
                    let exchange_type = ctx.param_str("exchangeType", 0, "fanout")?;
                    ensure_exchange(&channel, &exchange, &exchange_type, &options).await
                }
                other => Err(NodeError::new(format!("The operation \"{other}\" is not known!"))),
            }
        }
        .await;
        if let Err(e) = setup {
            close(conn, channel).await;
            return Err(e);
        }

        for (i, item) in input.iter().enumerate() {
            let result = publish_one(ctx, &channel, &mode, send_input_data, node_version, i, item, &options).await;
            match result {
                Ok(success_item) => out.push(success_item),
                Err(e) if ctx.continue_on_fail() => ctx.push_error_item(&e, i),
                Err(e) => {
                    close(conn, channel).await;
                    return Err(e.at(i));
                }
            }
        }

        close(conn, channel).await;
        Ok(vec![out])
    }
}

#[allow(clippy::too_many_arguments)]
async fn publish_one(
    ctx: &ExecCtx<'_>,
    channel: &Channel,
    mode: &str,
    send_input_data: bool,
    node_version: f64,
    i: usize,
    item: &Item,
    node_options: &Value,
) -> NodeResult<Item> {
    let message = if send_input_data { serde_json::to_string(&item.json).unwrap_or_default() } else { ctx.param_str("message", i, "")? };
    // Per-item options (headers can be expression-driven per item, matching
    // the reference's `this.getNodeParameter('options', i, {})` inside the
    // publish loop).
    let item_options = ctx.param("options", i)?;
    let headers = headers_field_table(&item_options);
    let props = BasicProperties::default().with_headers(headers);
    let props = apply_arguments(props, node_options);

    let (exchange, routing_key) = match mode {
        "queue" => {
            let queue = ctx.param_str("queue", 0, "")?;
            (String::new(), queue)
        }
        _ => {
            let exchange = ctx.param_str("exchange", 0, "")?;
            // typeVersion >= 1.2 reads `routingKey` per item; earlier
            // versions read it once from item 0.
            let routing_key = ctx.param_str("routingKey", if node_version >= 1.2 { i } else { 0 }, "")?;
            (exchange, routing_key)
        }
    };

    channel
        .basic_publish(ShortString::from(exchange), ShortString::from(routing_key), BasicPublishOptions::default(), message.as_bytes(), props)
        .await
        .map_err(|e| NodeError::new(e.to_string()))?;

    let mut json = Map::new();
    json.insert("success".into(), Value::Bool(true));
    Ok(Item::new(json).paired(i))
}

// ---- RabbitMQ Trigger (plan task 1.13) -------------------------------------------------
//
// Faithful to n8n's `RabbitMQTrigger.node.js` (typeVersion 1) +
// `GenericFunctions.js`'s `handleMessage`/`rabbitmqConnectQueue`. Unlike the
// action node (which only ever passively `checkQueue`s), the Trigger's own
// `options` collection spreads in `rabbitDefaultOptions`
// (`DefaultOptions.js`), whose `assertQueue` field defaults to `true`: a
// freshly-activated Trigger node declares the queue (durable by default)
// rather than merely checking it exists, so [`trigger_ensure_queue`] below
// has the opposite default from [`ensure_queue`].
//
// `laterMessageNode` ("Specified Later in Workflow", acked by a later
// RabbitMQ node's `deleteMessage` operation) needs a response hook plumbed
// from a *later* node back to this trigger's delivery, which r8r's executor
// has no path for; it is rejected at activation with a clear error instead
// of silently behaving like `immediately`.
//
// Concurrency: `parallelMessages` becomes the channel's `prefetch`
// (`basic_qos`); AMQP flow control (the broker won't push more unacked
// deliveries than the prefetch count) is what bounds how many executions
// run at once for `executionFinishes`/`executionFinishesSuccessfully` --
// each delivery is handled in its own task that acks/nacks only once that
// delivery's execution finishes, exactly mirroring the reference's
// `parallelMessages` + prefetch semantics without needing a separate
// semaphore.
//
// n8n's `MessageTracker`-based graceful close (give in-flight deliveries up
// to 5 minutes to finish before closing the channel) is not implemented:
// deactivation aborts the listener task immediately, like every other
// long-lived trigger in r8r.

pub struct RabbitMqTrigger;

const RECONNECT_BACKOFF: std::time::Duration = std::time::Duration::from_secs(5);

fn trigger_queue(node: &Node) -> String {
    node.parameters.get("queue").and_then(Value::as_str).unwrap_or("").to_string()
}

fn trigger_options(node: &Node) -> Value {
    node.parameters.get("options").cloned().unwrap_or_else(|| json!({}))
}

/// `channel.prefetch -1 (default: -1): no number, or 0, or < -1 -- rejects
/// activation (n8n's `trigger()` throws a `NodeOperationError` for the same
/// inputs).
fn validate_parallel_messages(options: &Value) -> Result<(), String> {
    let Some(v) = options.get("parallelMessages") else { return Ok(()) };
    let n = v.as_f64().unwrap_or(f64::NAN);
    if n.is_nan() || n == 0.0 || n < -1.0 {
        return Err("Parallel message processing limit must be a number greater than zero (or -1 for no limit)".to_string());
    }
    Ok(())
}

/// n8n's `rabbitmqConnectQueue`: `assertQueue` (declare, default options
/// durable/autoDelete/exclusive) when `options.assertQueue` is set --
/// defaulting to `true` for the Trigger (see module doc) -- else
/// `checkQueue` (passive). Bindings (`options.binding.bindings`) are
/// applied either way.
async fn trigger_ensure_queue(channel: &Channel, queue: &str, options: &Value) -> NodeResult<()> {
    if !options.get("assertQueue").and_then(Value::as_bool).unwrap_or(true) {
        let passive = QueueDeclareOptions { passive: true, ..Default::default() };
        channel.queue_declare(ShortString::from(queue.to_string()), passive, FieldTable::default()).await.map_err(|e| NodeError::new(e.to_string()))?;
    } else {
        let durable = options.get("durable").and_then(Value::as_bool).unwrap_or(true);
        let auto_delete = options.get("autoDelete").and_then(Value::as_bool).unwrap_or(false);
        let exclusive = options.get("exclusive").and_then(Value::as_bool).unwrap_or(false);
        let decl_options = QueueDeclareOptions { durable, auto_delete, exclusive, ..Default::default() };
        channel.queue_declare(ShortString::from(queue.to_string()), decl_options, FieldTable::default()).await.map_err(|e| NodeError::new(e.to_string()))?;
    }
    if let Some(bindings) = options.pointer("/binding/bindings").and_then(Value::as_array) {
        for binding in bindings {
            let exchange = binding.get("exchange").and_then(Value::as_str).unwrap_or("");
            if exchange.is_empty() {
                continue;
            }
            let routing_key = binding.get("routingKey").and_then(Value::as_str).unwrap_or("");
            channel
                .queue_bind(ShortString::from(queue.to_string()), ShortString::from(exchange.to_string()), ShortString::from(routing_key.to_string()), QueueBindOptions::default(), FieldTable::default())
                .await
                .map_err(|e| NodeError::new(e.to_string()))?;
        }
    }
    Ok(())
}

fn amqp_value_to_json(v: &AMQPValue) -> Value {
    use AMQPValue::*;
    match v {
        LongString(s) => json!(s.to_string()),
        ShortString(s) => json!(s.to_string()),
        Boolean(b) => json!(*b),
        LongInt(n) => json!(*n),
        LongLongInt(n) => json!(*n),
        ShortInt(n) => json!(*n),
        ShortShortInt(n) => json!(*n),
        LongUInt(n) => json!(*n),
        ShortUInt(n) => json!(*n),
        ShortShortUInt(n) => json!(*n),
        Double(n) => json!(*n),
        Float(n) => json!(*n),
        Void => Value::Null,
        other => json!(format!("{other:?}")),
    }
}

fn field_table_to_json(table: &FieldTable) -> Value {
    let mut map = Map::new();
    for (k, v) in table.inner() {
        map.insert(k.to_string(), amqp_value_to_json(v));
    }
    Value::Object(map)
}

/// n8n's `parseMessage`: `contentIsBinary` returns `{binary: {data}, json:
/// message}` with `message.content` cleared; otherwise `message.content`
/// becomes a string (optionally `JSON.parse`d), and `onlyContent` returns
/// just that content as the item's `json` (a non-object value when the
/// content isn't valid JSON -- mapped through [`Item::from_value`], this
/// codebase's existing convention for that n8n quirk, same as the Kafka and
/// MQTT triggers' `onlyMessage`).
fn build_item(delivery: &Delivery, options: &Value) -> Item {
    let mut fields = Map::new();
    fields.insert("deliveryTag".into(), json!(delivery.delivery_tag));
    fields.insert("redelivered".into(), json!(delivery.redelivered));
    fields.insert("exchange".into(), json!(delivery.exchange.to_string()));
    fields.insert("routingKey".into(), json!(delivery.routing_key.to_string()));

    let mut properties = Map::new();
    if let Some(headers) = delivery.properties.headers() {
        properties.insert("headers".into(), field_table_to_json(headers));
    }
    if let Some(ct) = delivery.properties.content_type() {
        properties.insert("contentType".into(), json!(ct.to_string()));
    }

    let content_is_binary = options.get("contentIsBinary").and_then(Value::as_bool).unwrap_or(false);
    if content_is_binary {
        let mut json = Map::new();
        json.insert("fields".into(), Value::Object(fields));
        json.insert("properties".into(), Value::Object(properties));
        let mut binary = Map::new();
        binary.insert("data".into(), json!({"data": base64::engine::general_purpose::STANDARD.encode(&delivery.data), "mimeType": "application/octet-stream"}));
        return Item { json, binary: Some(binary), paired_item: None };
    }

    let raw = String::from_utf8_lossy(&delivery.data).into_owned();
    let json_parse_body = options.get("jsonParseBody").and_then(Value::as_bool).unwrap_or(false);
    let content: Value = if json_parse_body { serde_json::from_str(&raw).unwrap_or_else(|_| json!(raw)) } else { json!(raw) };

    let only_content = options.get("onlyContent").and_then(Value::as_bool).unwrap_or(false);
    if only_content {
        return Item::from_value(content);
    }

    let mut json = Map::new();
    json.insert("content".into(), content);
    json.insert("fields".into(), Value::Object(fields));
    json.insert("properties".into(), Value::Object(properties));
    Item::new(json)
}

async fn handle_delivery(n8n: Arc<N8n>, workflow_id: String, node_name: String, delivery: Delivery, acknowledge_mode: String, options: Value) {
    let item = build_item(&delivery, &options);
    if acknowledge_mode == "immediately" {
        let _ = delivery.acker.ack(BasicAckOptions::default()).await;
        fire(&n8n, &workflow_id, &node_name, vec![item]).await;
        return;
    }
    let outcome = fire_and_wait(&n8n, &workflow_id, &node_name, vec![item]).await;
    let failed = outcome.as_ref().is_some_and(|o| o.error().is_some());
    if acknowledge_mode == "executionFinishesSuccessfully" && failed {
        let _ = delivery.acker.nack(BasicNackOptions { requeue: true, ..Default::default() }).await;
    } else {
        let _ = delivery.acker.ack(BasicAckOptions::default()).await;
    }
}

#[async_trait::async_trait]
impl LongLivedTrigger for RabbitMqTrigger {
    async fn validate(&self, n8n: &Arc<N8n>, node: &Node) -> Result<(), String> {
        let cred = resolve_credential(n8n, node, "rabbitmq").await?;
        let options = trigger_options(node);
        if options.get("acknowledge").and_then(Value::as_str) == Some("laterMessageNode") {
            return Err(
                "RabbitMQ Trigger: the \"Specified Later in Workflow\" acknowledge mode is not supported natively yet -- use \"Immediately\", \"Execution Finishes\" or \"Execution Finishes Successfully\" instead".to_string(),
            );
        }
        validate_parallel_messages(&options)?;
        let queue = trigger_queue(node);
        let (conn, channel) = connect(&cred).await.map_err(|e| e.message)?;
        let res = trigger_ensure_queue(&channel, &queue, &options).await;
        close(conn, channel).await;
        res.map_err(|e| e.message)
    }

    async fn run(&self, n8n: Arc<N8n>, workflow_id: String, node: Node) {
        let queue = trigger_queue(&node);
        let options = trigger_options(&node);
        let mut acknowledge_mode = options.get("acknowledge").and_then(Value::as_str).unwrap_or("immediately").to_string();
        let parallel_messages = options.get("parallelMessages").and_then(Value::as_f64).unwrap_or(-1.0) as i64;
        if parallel_messages != -1 && acknowledge_mode == "immediately" {
            // Mirrors the reference: a prefetch limit with no explicit
            // acknowledge mode forces "executionFinishes" since
            // "immediately" would defeat the point of limiting in-flight
            // executions.
            acknowledge_mode = "executionFinishes".to_string();
        }

        loop {
            let cred = match resolve_credential(&n8n, &node, "rabbitmq").await {
                Ok(v) => v,
                Err(e) => {
                    tracing::warn!(workflowId = %workflow_id, node = %node.name, error = %e, "RabbitMQ Trigger: could not read credentials; retrying");
                    tokio::time::sleep(RECONNECT_BACKOFF).await;
                    continue;
                }
            };
            let (conn, channel) = match connect(&cred).await {
                Ok(v) => v,
                Err(e) => {
                    tracing::warn!(workflowId = %workflow_id, node = %node.name, error = %e.message, "RabbitMQ Trigger: connection failed; retrying");
                    tokio::time::sleep(RECONNECT_BACKOFF).await;
                    continue;
                }
            };
            if let Err(e) = trigger_ensure_queue(&channel, &queue, &options).await {
                tracing::warn!(workflowId = %workflow_id, node = %node.name, error = %e.message, "RabbitMQ Trigger: could not declare/check the queue; retrying");
                close(conn, channel).await;
                tokio::time::sleep(RECONNECT_BACKOFF).await;
                continue;
            }
            if parallel_messages != -1 {
                if let Err(e) = channel.basic_qos(parallel_messages.clamp(1, u16::MAX as i64) as u16, BasicQosOptions::default()).await {
                    tracing::warn!(workflowId = %workflow_id, node = %node.name, error = %e, "RabbitMQ Trigger: could not set prefetch; retrying");
                    close(conn, channel).await;
                    tokio::time::sleep(RECONNECT_BACKOFF).await;
                    continue;
                }
            }
            let mut consumer = match channel.basic_consume(ShortString::from(queue.clone()), ShortString::from(""), BasicConsumeOptions::default(), FieldTable::default()).await {
                Ok(c) => c,
                Err(e) => {
                    tracing::warn!(workflowId = %workflow_id, node = %node.name, error = %e, "RabbitMQ Trigger: could not consume the queue; retrying");
                    close(conn, channel).await;
                    tokio::time::sleep(RECONNECT_BACKOFF).await;
                    continue;
                }
            };

            loop {
                match consumer.next().await {
                    Some(Ok(delivery)) => {
                        let n8n = n8n.clone();
                        let workflow_id = workflow_id.clone();
                        let node_name = node.name.clone();
                        let ack_mode = acknowledge_mode.clone();
                        let options = options.clone();
                        tokio::spawn(async move { handle_delivery(n8n, workflow_id, node_name, delivery, ack_mode, options).await });
                    }
                    Some(Err(e)) => {
                        tracing::warn!(workflowId = %workflow_id, node = %node.name, error = %e, "RabbitMQ Trigger: consumer error; reconnecting");
                        break;
                    }
                    None => {
                        tracing::warn!(workflowId = %workflow_id, node = %node.name, "RabbitMQ Trigger: consumer was cancelled; reconnecting");
                        break;
                    }
                }
            }
            close(conn, channel).await;
            tokio::time::sleep(RECONNECT_BACKOFF).await;
        }
    }
}
