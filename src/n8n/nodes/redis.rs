//! Redis node (spec §6.6): get/set/delete/incr/keys/pop/publish/push/info
//! against the `redis` credential.
//!
//! Faithful to n8n's `Redis.node.js` / `utils.js`, including some real
//! quirks:
//! - `operation` is read once, from item 0 (it is `noDataExpression` in the
//!   editor), so it cannot vary per item.
//! - `info` runs exactly once regardless of the number of input items (it
//!   does not consult `getInputData()` at all).
//! - Connecting to Redis happens once, before any per-item work, and is
//!   *not* covered by `continueOnFail`/`onError`: a connection failure
//!   always fails the whole node.
//! - `set`/`incr`'s "automatic" type detection and "list"/"sets" `set`
//!   variants mirror the real node's JS-value semantics (a bare string
//!   behaves like a character array for `keyType: "list"`, etc).
//! - `List Length` (`llen`) is in n8n's operation dropdown but out of scope
//!   here (not asked for): selecting it fails with a clear error rather
//!   than being silently mishandled.
//!
//! SSL/TLS Redis connections are not supported in this build (the `redis`
//! crate dependency has no TLS feature enabled here): a credential with
//! `ssl: true` fails clearly instead of silently connecting in the clear or
//! panicking.

use super::set_path;
use crate::n8n::node::{ExecCtx, NodeError, NodeResult, NodeType};
use crate::n8n::types::{Item, NodeOutput};
use redis::aio::MultiplexedConnection;
use redis::{ConnectionAddr, ConnectionInfo, RedisConnectionInfo};
use serde_json::{Map, Value};
use std::time::Duration;

pub struct Redis;

// ---- connection ------------------------------------------------------------------

pub(super) struct Creds {
    host: String,
    port: u16,
    database: i64,
    user: Option<String>,
    password: Option<String>,
    ssl: bool,
}

pub(super) fn read_creds(cred: &Value) -> Creds {
    Creds {
        host: cred["host"].as_str().filter(|s| !s.is_empty()).unwrap_or("localhost").to_string(),
        port: cred["port"].as_u64().unwrap_or(6379) as u16,
        database: cred["database"].as_i64().unwrap_or(0),
        user: cred["user"].as_str().filter(|s| !s.is_empty()).map(String::from),
        password: cred["password"].as_str().filter(|s| !s.is_empty()).map(String::from),
        ssl: cred["ssl"].as_bool().unwrap_or(false),
    }
}

/// Connects and pings, the way n8n's `execute()` does before touching any
/// operation. Never covered by `continueOnFail`: a connection failure fails
/// the whole node. The error text never includes the password: it is never
/// interpolated into a connection URL (a struct is passed straight to the
/// client) and `redis::RedisError`'s own message never carries connection
/// info.
pub(super) async fn connect(creds: &Creds) -> NodeResult<MultiplexedConnection> {
    if creds.ssl {
        return Err(NodeError::new("SSL/TLS Redis connections are not supported in this build"));
    }
    let info = ConnectionInfo {
        addr: ConnectionAddr::Tcp(creds.host.clone(), creds.port),
        redis: RedisConnectionInfo { db: creds.database, username: creds.user.clone(), password: creds.password.clone(), ..Default::default() },
    };
    let client = redis::Client::open(info).map_err(|e| NodeError::new(format!("Invalid Redis connection settings: {e}")))?;
    let mut conn = match tokio::time::timeout(Duration::from_secs(10), client.get_multiplexed_async_connection()).await {
        Ok(Ok(conn)) => conn,
        Ok(Err(e)) => return Err(NodeError::new(format!("Could not connect to Redis at {}:{}: {e}", creds.host, creds.port))),
        Err(_) => return Err(NodeError::new(format!("Connection timeout: unable to connect to Redis at {}:{}", creds.host, creds.port))),
    };
    redis::cmd("PING")
        .query_async::<String>(&mut conn)
        .await
        .map_err(|e| NodeError::new(format!("Could not connect to Redis at {}:{}: {e}", creds.host, creds.port)))?;
    Ok(conn)
}

fn redis_err(e: redis::RedisError) -> NodeError {
    NodeError::new(e.to_string())
}

// ---- value <-> JS-ish string conversions -----------------------------------------

/// n8n's `value.toString()` for the JS values a "string" field or an
/// expression can produce.
fn js_to_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Null => String::new(),
        Value::Array(a) => a.iter().map(js_to_string).collect::<Vec<_>>().join(","),
        Value::Object(_) => "[object Object]".to_string(),
    }
}

/// n8n's `convertInfoToObject`: parses the Redis `INFO` reply into an
/// object, with `key=value,...` sections nested and numeric-looking values
/// parsed as numbers.
fn convert_info_to_object(text: &str) -> Map<String, Value> {
    fn parsed(v: &str) -> Value {
        if !v.is_empty() && v.chars().all(|c| c.is_ascii_digit() || c == '.') {
            if let Ok(f) = v.parse::<f64>() {
                if let Some(n) = serde_json::Number::from_f64(f) {
                    return Value::Number(n);
                }
            }
        }
        Value::String(v.to_string())
    }
    let mut out = Map::new();
    for line in text.split('\n') {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut parts = line.split(':');
        let Some(key) = parts.next() else { continue };
        let Some(value) = parts.next() else { continue };
        let value = value.trim();
        if value.contains('=') {
            let mut sub = Map::new();
            for kv in value.split(',') {
                if let Some((k2, v2)) = kv.split_once('=') {
                    sub.insert(k2.to_string(), parsed(v2));
                }
            }
            out.insert(key.to_string(), Value::Object(sub));
        } else {
            out.insert(key.to_string(), parsed(value));
        }
    }
    out
}

/// n8n's `getValue`: fetches a key with automatic type detection (via
/// `TYPE`) or an explicit `keyType`. A missing key ("none") or an unknown
/// type returns `null`, matching `(await getValue(...)) ?? null`.
async fn get_value(conn: &mut MultiplexedConnection, key: &str, key_type: &str) -> NodeResult<Value> {
    let actual = if key_type.is_empty() || key_type == "automatic" {
        redis::cmd("TYPE").arg(key).query_async::<String>(conn).await.map_err(redis_err)?
    } else {
        key_type.to_string()
    };
    Ok(match actual.as_str() {
        "string" => {
            let v: Option<String> = redis::cmd("GET").arg(key).query_async(conn).await.map_err(redis_err)?;
            v.map(Value::String).unwrap_or(Value::Null)
        }
        "hash" => {
            let flat: Vec<String> = redis::cmd("HGETALL").arg(key).query_async(conn).await.map_err(redis_err)?;
            let mut map = Map::new();
            for pair in flat.chunks(2) {
                if let [k, v] = pair {
                    map.insert(k.clone(), Value::String(v.clone()));
                }
            }
            Value::Object(map)
        }
        "list" => {
            let v: Vec<String> = redis::cmd("LRANGE").arg(key).arg(0).arg(-1).query_async(conn).await.map_err(redis_err)?;
            Value::Array(v.into_iter().map(Value::String).collect())
        }
        "sets" => {
            let v: Vec<String> = redis::cmd("SMEMBERS").arg(key).query_async(conn).await.map_err(redis_err)?;
            Value::Array(v.into_iter().map(Value::String).collect())
        }
        _ => Value::Null,
    })
}

/// n8n's `setValue`. `keyType: "automatic"` looks at the JS type of `value`
/// (a JSON object/array from an expression, otherwise a plain string).
async fn set_value(conn: &mut MultiplexedConnection, key: &str, value: &Value, expire: bool, ttl: i64, key_type: &str, value_is_json: bool) -> NodeResult<()> {
    let resolved: String = if key_type.is_empty() || key_type == "automatic" {
        match value {
            Value::String(_) => "string".to_string(),
            Value::Array(_) => "list".to_string(),
            Value::Object(_) => "hash".to_string(),
            _ => return Err(NodeError::new("Could not identify the type to set. Please set it manually!")),
        }
    } else {
        key_type.to_string()
    };
    match resolved.as_str() {
        "string" => {
            redis::cmd("SET").arg(key).arg(js_to_string(value)).query_async::<()>(conn).await.map_err(redis_err)?;
        }
        "hash" => set_hash(conn, key, value, value_is_json).await?,
        "list" => set_list(conn, key, value).await?,
        "sets" => set_set(conn, key, value).await?,
        other => return Err(NodeError::new(format!("The key type \"{other}\" is not supported"))),
    }
    if expire {
        redis::cmd("EXPIRE").arg(key).arg(ttl).query_async::<()>(conn).await.map_err(redis_err)?;
    }
    Ok(())
}

async fn set_hash(conn: &mut MultiplexedConnection, key: &str, value: &Value, value_is_json: bool) -> NodeResult<()> {
    if value_is_json {
        let raw = match value {
            Value::String(s) => s.clone(),
            other => js_to_string(other),
        };
        // n8n's literal fallback ("this is how we originally worked and
        // prevents a breaking change"): on JSON-parse failure it treats the
        // raw string itself as the object, so `Object.keys` walks its
        // character indices.
        let parsed: Value = serde_json::from_str(&raw).unwrap_or_else(|_| Value::String(raw.clone()));
        match parsed {
            Value::Object(map) => {
                for (k, v) in map {
                    redis::cmd("HSET").arg(key).arg(&k).arg(js_to_string(&v)).query_async::<()>(conn).await.map_err(redis_err)?;
                }
            }
            Value::String(s) => {
                for (idx, ch) in s.chars().enumerate() {
                    redis::cmd("HSET").arg(key).arg(idx.to_string()).arg(ch.to_string()).query_async::<()>(conn).await.map_err(redis_err)?;
                }
            }
            _ => {}
        }
    } else {
        let raw = match value {
            Value::String(s) => s.clone(),
            other => js_to_string(other),
        };
        let parts: Vec<&str> = raw.split(' ').collect();
        if !parts.len().is_multiple_of(2) {
            return Err(NodeError::new("The hash value must be space-separated \"field value\" pairs"));
        }
        for pair in parts.chunks(2) {
            redis::cmd("HSET").arg(key).arg(pair[0]).arg(pair[1]).query_async::<()>(conn).await.map_err(redis_err)?;
        }
    }
    Ok(())
}

async fn set_list(conn: &mut MultiplexedConnection, key: &str, value: &Value) -> NodeResult<()> {
    // n8n's `setValue` does `value[index].toString()` over `value.length`:
    // a JS array indexes/lengths normally; a bare string (the field's
    // native type) indexes/lengths by character.
    let elements: Vec<String> = match value {
        Value::Array(a) => a.iter().map(js_to_string).collect(),
        Value::String(s) => s.chars().map(String::from).collect(),
        other => vec![js_to_string(other)],
    };
    for (index, el) in elements.iter().enumerate() {
        redis::cmd("LSET").arg(key).arg(index as i64).arg(el).query_async::<()>(conn).await.map_err(redis_err)?;
    }
    Ok(())
}

async fn set_set(conn: &mut MultiplexedConnection, key: &str, value: &Value) -> NodeResult<()> {
    match value {
        Value::Array(a) => {
            if a.is_empty() {
                return Ok(());
            }
            let mut cmd = redis::cmd("SADD");
            cmd.arg(key);
            for m in a {
                cmd.arg(js_to_string(m));
            }
            cmd.query_async::<()>(conn).await.map_err(redis_err)?;
        }
        other => {
            redis::cmd("SADD").arg(key).arg(js_to_string(other)).query_async::<()>(conn).await.map_err(redis_err)?;
        }
    }
    Ok(())
}

// ---- per-item operations -----------------------------------------------------------

async fn run_item(ctx: &ExecCtx<'_>, conn: &mut MultiplexedConnection, operation: &str, i: usize, item: &Item) -> NodeResult<Vec<Item>> {
    match operation {
        "delete" => {
            let key = ctx.param_str("key", i, "")?;
            redis::cmd("DEL").arg(&key).query_async::<i64>(conn).await.map_err(redis_err)?;
            Ok(vec![item.clone()])
        }
        "get" => {
            let property_name = ctx.param_str("propertyName", i, "propertyName")?;
            let key = ctx.param_str("key", i, "")?;
            let key_type = ctx.param_str("keyType", i, "automatic")?;
            let value = get_value(conn, &key, &key_type).await?;
            let dot_notation = ctx.param_bool("options.dotNotation", i, true)?;
            let mut json = Map::new();
            if dot_notation {
                set_path(&mut json, &property_name, value);
            } else {
                json.insert(property_name, value);
            }
            Ok(vec![Item::new(json).paired(i)])
        }
        "incr" => {
            let key = ctx.param_str("key", i, "")?;
            let expire = ctx.param_bool("expire", i, false)?;
            let ttl = ctx.param_f64("ttl", i, -1.0)? as i64;
            let value: i64 = redis::cmd("INCR").arg(&key).query_async(conn).await.map_err(redis_err)?;
            if expire && ttl > 0 {
                redis::cmd("EXPIRE").arg(&key).arg(ttl).query_async::<()>(conn).await.map_err(redis_err)?;
            }
            // n8n pushes a fresh object here, without `pairedItem`.
            let mut json = Map::new();
            json.insert(key, Value::from(value));
            Ok(vec![Item::new(json)])
        }
        "keys" => {
            let pattern = ctx.param_str("keyPattern", i, "")?;
            let get_values = ctx.param_bool("getValues", i, true)?;
            let keys: Vec<String> = redis::cmd("KEYS").arg(&pattern).query_async(conn).await.map_err(redis_err)?;
            if !get_values {
                let mut json = Map::new();
                json.insert("keys".into(), Value::Array(keys.into_iter().map(Value::String).collect()));
                // n8n pushes a fresh object here, without `pairedItem`.
                return Ok(vec![Item::new(json)]);
            }
            let mut json = Map::new();
            for key_name in &keys {
                let v = get_value(conn, key_name, "automatic").await?;
                json.insert(key_name.clone(), v);
            }
            Ok(vec![Item::new(json).paired(i)])
        }
        "pop" => {
            let list = ctx.param_str("list", i, "")?;
            let tail = ctx.param_bool("tail", i, false)?;
            let property_name = ctx.param_str("propertyName", i, "propertyName")?;
            let cmd = if tail { "RPOP" } else { "LPOP" };
            let raw: Option<String> = redis::cmd(cmd).arg(&list).query_async(conn).await.map_err(redis_err)?;
            // n8n: `value && JSON.parse(value)`, caught on failure -- a
            // missing key/empty list is `null`, an empty string stays `""`.
            let value = match raw {
                None => Value::Null,
                Some(s) if s.is_empty() => Value::String(s),
                Some(s) => serde_json::from_str::<Value>(&s).unwrap_or(Value::String(s)),
            };
            let dot_notation = ctx.param_bool("options.dotNotation", i, true)?;
            let mut json = Map::new();
            if dot_notation {
                set_path(&mut json, &property_name, value);
            } else {
                json.insert(property_name, value);
            }
            Ok(vec![Item::new(json).paired(i)])
        }
        "publish" => {
            let channel = ctx.param_str("channel", i, "")?;
            let message_data = ctx.param_str("messageData", i, "")?;
            redis::cmd("PUBLISH").arg(&channel).arg(&message_data).query_async::<i64>(conn).await.map_err(redis_err)?;
            Ok(vec![item.clone()])
        }
        "push" => {
            let list = ctx.param_str("list", i, "")?;
            let message_data = ctx.param_str("messageData", i, "")?;
            let tail = ctx.param_bool("tail", i, false)?;
            let cmd = if tail { "RPUSH" } else { "LPUSH" };
            redis::cmd(cmd).arg(&list).arg(&message_data).query_async::<i64>(conn).await.map_err(redis_err)?;
            Ok(vec![item.clone()])
        }
        "set" => {
            let key = ctx.param_str("key", i, "")?;
            let value = ctx.param("value", i)?;
            let key_type = ctx.param_str("keyType", i, "automatic")?;
            let value_is_json = ctx.param_bool("valueIsJSON", i, true)?;
            let expire = ctx.param_bool("expire", i, false)?;
            let ttl = ctx.param_f64("ttl", i, -1.0)? as i64;
            set_value(conn, &key, &value, expire, ttl, &key_type, value_is_json).await?;
            Ok(vec![item.clone()])
        }
        other => Err(NodeError::new(format!("The operation \"{other}\" is not supported"))),
    }
}

async fn run_info(conn: &mut MultiplexedConnection) -> NodeResult<Item> {
    let text: String = redis::cmd("INFO").query_async(conn).await.map_err(redis_err)?;
    Ok(Item::new(convert_info_to_object(&text)))
}

#[async_trait::async_trait]
impl NodeType for Redis {
    fn type_name(&self) -> &'static str {
        "n8n-nodes-base.redis"
    }

    async fn execute(&self, ctx: &mut ExecCtx<'_>) -> NodeResult<NodeOutput> {
        let (_, cred) = ctx.credentials("redis").await?;
        let creds = read_creds(&cred);
        // Not covered by continueOnFail: n8n throws straight out of a
        // failed connect()/ping(), whatever `onError` says.
        let mut conn = connect(&creds).await?;

        let operation = ctx.param_str("operation", 0, "info")?;
        let mut out = Vec::new();

        if operation == "info" {
            match run_info(&mut conn).await {
                Ok(item) => out.push(item),
                Err(e) if ctx.continue_on_fail() => ctx.push_error_item(&e, 0),
                Err(e) => return Err(e),
            }
            return Ok(vec![out]);
        }

        let input = if ctx.input().is_empty() { vec![Item::default()] } else { ctx.input().to_vec() };
        for (i, item) in input.iter().enumerate() {
            match run_item(ctx, &mut conn, &operation, i, item).await {
                Ok(mut items) => out.append(&mut items),
                Err(e) if ctx.continue_on_fail() => ctx.push_error_item(&e.at(i), i),
                Err(e) => return Err(e.at(i)),
            }
        }
        Ok(vec![out])
    }
}
