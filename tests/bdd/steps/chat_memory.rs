//! Read-back of the AI chat memories' stores (`10-ai/chat_memory`): the
//! rows/list entries must have the shape n8n's LangChain histories write,
//! so memory written by one engine can be read by the other.

use super::docstring;
use crate::support::json::{assert_matches, parse_strict, Mode};
use crate::world::{pretty, R8rWorld};
use cucumber::gherkin::Step;
use cucumber::{given, then};
use serde_json::Value;
use sqlx::{Connection, Executor, Row};

async fn pg() -> sqlx::PgConnection {
    let url = std::env::var("R8R_BDD_POSTGRES_URL").unwrap_or_else(|_| "postgres://postgres:postgres@127.0.0.1:5432/postgres".into());
    sqlx::PgConnection::connect(&url).await.unwrap_or_else(|e| panic!("cannot connect to {url}: {e}"))
}

#[given(expr = "the Postgres table {string} does not exist")]
#[then(expr = "the Postgres table {string} is dropped")]
async fn drop_table(_w: &mut R8rWorld, table: String) {
    let mut c = pg().await;
    c.execute(format!("DROP TABLE IF EXISTS \"{}\"", table.replace('"', "")).as_str()).await.expect("drop table");
}

/// Doc string: the `message` JSONB of the session's rows, in id order,
/// matched as a subset.
#[then(expr = "the Postgres table {string} holds for the session {string} the messages:")]
async fn pg_messages(_w: &mut R8rWorld, table: String, session: String, step: &Step) {
    let mut c = pg().await;
    let rows = sqlx::query(&format!("SELECT message::text AS m FROM \"{}\" WHERE session_id = $1 ORDER BY id", table.replace('"', "")))
        .bind(&session)
        .fetch_all(&mut c)
        .await
        .expect("select messages");
    let actual = Value::Array(rows.iter().map(|r| serde_json::from_str(&r.get::<String, _>("m")).unwrap()).collect());
    let expected = parse_strict(docstring(step), "expected messages");
    assert_matches(&expected, &actual, Mode::Subset).unwrap_or_else(|e| panic!("{e}\nrows: {}", pretty(&actual)));
    assert_eq!(expected.as_array().map(Vec::len), actual.as_array().map(Vec::len), "rows: {}", pretty(&actual));
}

async fn redis() -> redis::aio::MultiplexedConnection {
    let host = std::env::var("R8R_BDD_REDIS_HOST").unwrap_or_else(|_| "127.0.0.1".into());
    let port = std::env::var("R8R_BDD_REDIS_PORT").unwrap_or_else(|_| "6379".into());
    redis::Client::open(format!("redis://{host}:{port}")).unwrap().get_multiplexed_async_connection().await.expect("connect to the bdd redis instance")
}

/// Doc string: the list's entries (JSON) from first to last (LRANGE 0 -1,
/// i.e. newest first, as LPUSH leaves them), matched as a subset.
#[then(expr = "the redis list {string} holds the entries:")]
async fn redis_list(_w: &mut R8rWorld, key: String, step: &Step) {
    let mut c = redis().await;
    let raw: Vec<String> = redis::cmd("LRANGE").arg(&key).arg(0).arg(-1).query_async(&mut c).await.expect("LRANGE");
    let actual = Value::Array(raw.iter().map(|s| serde_json::from_str(s).unwrap_or(Value::String(s.clone()))).collect());
    let expected = parse_strict(docstring(step), "expected entries");
    assert_matches(&expected, &actual, Mode::Subset).unwrap_or_else(|e| panic!("{e}\nlist: {}", pretty(&actual)));
    assert_eq!(expected.as_array().map(Vec::len), actual.as_array().map(Vec::len), "list: {}", pretty(&actual));
}

/// Seeds a history the way n8n's LangChain store writes it (creating the
/// table as `PostgresChatMessageHistory` does).
#[given(expr = "the Postgres table {string} has for the session {string} the messages:")]
async fn pg_seed(_w: &mut R8rWorld, table: String, session: String, step: &Step) {
    let table = table.replace('"', "");
    let mut c = pg().await;
    c.execute(format!("CREATE TABLE IF NOT EXISTS \"{table}\" (id SERIAL PRIMARY KEY, session_id VARCHAR(255) NOT NULL, message JSONB NOT NULL)").as_str()).await.expect("create table");
    for m in parse_strict(docstring(step), "messages").as_array().expect("array of messages") {
        sqlx::query(&format!("INSERT INTO \"{table}\" (session_id, message) VALUES ($1, $2::jsonb)")).bind(&session).bind(m.to_string()).execute(&mut c).await.expect("insert");
    }
}

/// Seeds a Redis history: the doc string lists entries oldest first; each
/// is LPUSHed, as `RedisChatMessageHistory.addMessage` does.
#[given(expr = "the redis list {string} has the entries pushed in order:")]
async fn redis_seed(_w: &mut R8rWorld, key: String, step: &Step) {
    let mut c = redis().await;
    for m in parse_strict(docstring(step), "entries").as_array().expect("array of entries") {
        let _: i64 = redis::cmd("LPUSH").arg(&key).arg(m.to_string()).query_async(&mut c).await.expect("LPUSH");
    }
}
