//! Importing an n8n database (`r8r migrate-from-n8n`, spec goal G3). The
//! n8n side is a fixture dumped from a real n8n 2.35.7 SQLite database.

use crate::world::R8rWorld;
use cucumber::given;
use sqlx::{Connection, Executor, Row};

fn fixture(name: &str) -> String {
    let path = format!("{}/tests/bdd/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("fixture {path}: {e}"))
}

async fn open(w: &R8rWorld, file: &str) -> sqlx::SqliteConnection {
    let path = w.dir.path().join(file);
    let url = format!("sqlite:{}?mode=rwc", path.display());
    sqlx::SqliteConnection::connect(&url).await.unwrap_or_else(|e| panic!("cannot open {url}: {e}"))
}

/// Builds an n8n SQLite database in the scenario folder from a SQL dump.
#[given(expr = "the n8n database {string} created from the fixture {string}")]
async fn n8n_database(w: &mut R8rWorld, file: String, name: String) {
    let mut conn = open(w, &file).await;
    conn.execute(fixture(&name).as_str()).await.unwrap_or_else(|e| panic!("loading fixture {name}: {e}"));
    conn.close().await.unwrap();
}

/// The public API key the n8n instance issued (stored as issued in n8n's
/// `user_api_keys`), used as `user`'s key.
#[given(expr = "the API key stored in the n8n database {string} is the key of {string}")]
async fn n8n_api_key(w: &mut R8rWorld, file: String, user: String) {
    let mut conn = open(w, &file).await;
    let row = sqlx::query("SELECT apiKey FROM user_api_keys WHERE audience = 'public-api' ORDER BY createdAt LIMIT 1")
        .fetch_one(&mut conn)
        .await
        .unwrap_or_else(|e| panic!("no API key in {file}: {e}"));
    w.api_keys.insert(user, row.get::<String, _>("apiKey"));
}

#[given(expr = "the execution {string} is remembered")]
async fn remember_execution(w: &mut R8rWorld, id: String) {
    w.vars.insert("EXECUTION_ID".into(), id);
}
