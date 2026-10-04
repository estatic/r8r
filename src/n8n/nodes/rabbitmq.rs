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
//! Deviation from the reference: the real action node always calls
//! `channel.checkQueue`/`checkExchange` and never `assertQueue`/
//! `assertExchange` (those options exist only on the Trigger node's
//! `options`, so `options.assertQueue` is always `undefined` here) -- so
//! `durable`/`exclusive`/`autoDelete` are silently inert in the upstream
//! action node; a queue/exchange that does not already exist makes the
//! whole node fail with a NOT_FOUND channel error. r8r instead *declares*
//! (asserts) the queue/exchange with those options, which is what they are
//! documented to do and is the only way to make them observable/testable.
//! Documented here and in the task's final report.

use crate::n8n::node::{ExecCtx, NodeError, NodeResult, NodeType};
use crate::n8n::types::{Item, NodeOutput};
use lapin::options::{BasicPublishOptions, ExchangeDeclareOptions, QueueDeclareOptions};
use lapin::types::{AMQPValue, FieldTable, LongString, ShortString};
use lapin::{BasicProperties, Channel, Connection, ConnectionProperties, ExchangeKind};
use serde_json::{Map, Value};

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

async fn ensure_queue(channel: &Channel, queue: &str, options: &Value) -> NodeResult<()> {
    let durable = options.get("durable").and_then(Value::as_bool).unwrap_or(true);
    let auto_delete = options.get("autoDelete").and_then(Value::as_bool).unwrap_or(false);
    let exclusive = options.get("exclusive").and_then(Value::as_bool).unwrap_or(false);
    let decl_options = QueueDeclareOptions { durable, auto_delete, exclusive, ..Default::default() };
    channel.queue_declare(ShortString::from(queue.to_string()), decl_options, FieldTable::default()).await.map_err(|e| NodeError::new(e.to_string()))?;
    Ok(())
}

async fn ensure_exchange(channel: &Channel, exchange: &str, exchange_type: &str, options: &Value) -> NodeResult<()> {
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
