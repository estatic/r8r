//! Direct Redis access for the `@requires-redis` scenarios (`04-nodes`,
//! `13-scaling`): key cleanup between scenarios and a raw TTL check the
//! Redis node's own output can't otherwise show.

use crate::world::R8rWorld;
use cucumber::{given, then};

fn redis_url() -> String {
    let host = std::env::var("R8R_BDD_REDIS_HOST").unwrap_or_else(|_| "127.0.0.1".into());
    let port = std::env::var("R8R_BDD_REDIS_PORT").unwrap_or_else(|_| "6379".into());
    format!("redis://{host}:{port}")
}

async fn connect() -> redis::aio::MultiplexedConnection {
    let client = redis::Client::open(redis_url()).expect("bdd redis url");
    client.get_multiplexed_async_connection().await.expect("connect to the bdd redis instance")
}

/// Deletes every key matching `pattern` (a `KEYS` glob). Used both to make
/// sure a scenario starts from a clean slate and to clean up after it, so
/// scenarios don't leak state into each other.
#[given(expr = "redis has no keys matching {string}")]
#[then(expr = "redis has no keys matching {string}")]
async fn redis_clear(_w: &mut R8rWorld, pattern: String) {
    let mut conn = connect().await;
    let keys: Vec<String> = redis::cmd("KEYS").arg(&pattern).query_async(&mut conn).await.expect("KEYS");
    if !keys.is_empty() {
        let _: () = redis::cmd("DEL").arg(&keys).query_async(&mut conn).await.expect("DEL");
    }
}

#[then(expr = "the redis key {string} has a ttl greater than {int} seconds")]
async fn redis_ttl_gt(_w: &mut R8rWorld, key: String, min: i64) {
    let mut conn = connect().await;
    let ttl: i64 = redis::cmd("TTL").arg(&key).query_async(&mut conn).await.expect("TTL");
    assert!(ttl > min, "expected TTL(\"{key}\") > {min}, got {ttl}");
}
