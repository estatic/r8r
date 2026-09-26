//! The job queue for queue mode (spec §7.3): main (and webhook processes)
//! enqueue executions, workers lease and run them. A lease is kept alive
//! by heartbeats; a job whose worker dies is handed out again once its
//! lease expires. Backends: Redis (`QUEUE_BULL_REDIS_*`) or PostgreSQL
//! (`R8R_QUEUE_BACKEND=postgres`, `R8R_DATABASE_URL`,
//! `DB_POSTGRESDB_SCHEMA`).

use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;

/// A leased job: `lease` identifies it for heartbeats and ack.
pub struct Leased {
    pub lease: String,
    pub payload: Value,
}

#[async_trait::async_trait]
pub trait Queue: Send + Sync {
    async fn push(&self, payload: &Value) -> anyhow::Result<()>;
    /// Waits up to about a second for a job.
    async fn pop(&self) -> anyhow::Result<Option<Leased>>;
    async fn heartbeat(&self, lease: &str) -> anyhow::Result<()>;
    async fn ack(&self, lease: &str) -> anyhow::Result<()>;
    /// Puts jobs whose lease expired back in the queue.
    async fn requeue_expired(&self) -> anyhow::Result<u64>;
    fn lease_duration(&self) -> Duration;
    fn describe(&self) -> String;
}

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.trim().is_empty())
}

/// `QUEUE_WORKER_LOCK_DURATION` (ms), 10 s by default.
fn lease_duration() -> Duration {
    Duration::from_millis(env("QUEUE_WORKER_LOCK_DURATION").and_then(|v| v.parse().ok()).unwrap_or(10_000).max(1000))
}

pub async fn connect() -> anyhow::Result<Arc<dyn Queue>> {
    match env("R8R_QUEUE_BACKEND").as_deref().unwrap_or("redis") {
        "postgres" | "postgresdb" => Ok(Arc::new(PgQueue::connect().await?)),
        "redis" => Ok(Arc::new(RedisQueue::connect().await?)),
        other => anyhow::bail!("invalid configuration: R8R_QUEUE_BACKEND: expected redis or postgres, got \"{other}\""),
    }
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

// ---- Redis ------------------------------------------------------------------------

pub struct RedisQueue {
    conn: redis::aio::MultiplexedConnection,
    /// A separate connection for blocking pops, so they don't stall others.
    blocking: redis::aio::MultiplexedConnection,
    prefix: String,
    lease: Duration,
    url: String,
}

impl RedisQueue {
    async fn connect() -> anyhow::Result<Self> {
        let host = env("QUEUE_BULL_REDIS_HOST").unwrap_or_else(|| "localhost".into());
        let port = env("QUEUE_BULL_REDIS_PORT").unwrap_or_else(|| "6379".into());
        let db = env("QUEUE_BULL_REDIS_DB").unwrap_or_else(|| "0".into());
        let auth = match (env("QUEUE_BULL_REDIS_USERNAME"), env("QUEUE_BULL_REDIS_PASSWORD")) {
            (Some(u), Some(p)) => format!("{u}:{p}@"),
            (None, Some(p)) => format!(":{p}@"),
            _ => String::new(),
        };
        let scheme = if env("QUEUE_BULL_REDIS_TLS").as_deref() == Some("true") { "rediss" } else { "redis" };
        let url = format!("{scheme}://{auth}{host}:{port}/{db}");
        let client = redis::Client::open(url.as_str())?;
        let conn = client
            .get_multiplexed_async_connection()
            .await
            .map_err(|e| anyhow::anyhow!("cannot connect to Redis at {host}:{port} (QUEUE_BULL_REDIS_HOST/PORT): {e}"))?;
        let blocking = client.get_multiplexed_async_connection().await?;
        let prefix = format!("{}:r8r", env("QUEUE_BULL_PREFIX").unwrap_or_else(|| "bull".into()));
        Ok(Self { conn, blocking, prefix, lease: lease_duration(), url: format!("redis {host}:{port}/{db}") })
    }

    fn key(&self, name: &str) -> String {
        format!("{}:{name}", self.prefix)
    }
}

#[async_trait::async_trait]
impl Queue for RedisQueue {
    async fn push(&self, payload: &Value) -> anyhow::Result<()> {
        let id = uuid::Uuid::new_v4().to_string();
        let mut c = self.conn.clone();
        redis::pipe()
            .atomic()
            .set(self.key(&format!("job:{id}")), payload.to_string())
            .lpush(self.key("wait"), &id)
            .query_async::<()>(&mut c)
            .await?;
        Ok(())
    }

    async fn pop(&self) -> anyhow::Result<Option<Leased>> {
        let mut b = self.blocking.clone();
        let id: Option<String> = redis::cmd("BLMOVE")
            .arg(self.key("wait"))
            .arg(self.key("active"))
            .arg("RIGHT")
            .arg("LEFT")
            .arg(1)
            .query_async(&mut b)
            .await?;
        let Some(id) = id else { return Ok(None) };
        let mut c = self.conn.clone();
        let _: () = redis::cmd("ZADD").arg(self.key("leases")).arg(now_ms() + self.lease.as_millis() as i64).arg(&id).query_async(&mut c).await?;
        let payload: Option<String> = redis::cmd("GET").arg(self.key(&format!("job:{id}"))).query_async(&mut c).await?;
        let Some(payload) = payload else {
            // The job's data is gone (acked elsewhere): drop the entry.
            self.ack(&id).await?;
            return Ok(None);
        };
        Ok(Some(Leased { lease: id, payload: serde_json::from_str(&payload)? }))
    }

    async fn heartbeat(&self, lease: &str) -> anyhow::Result<()> {
        let mut c = self.conn.clone();
        let _: () = redis::cmd("ZADD").arg(self.key("leases")).arg("XX").arg(now_ms() + self.lease.as_millis() as i64).arg(lease).query_async(&mut c).await?;
        Ok(())
    }

    async fn ack(&self, lease: &str) -> anyhow::Result<()> {
        let mut c = self.conn.clone();
        redis::pipe()
            .atomic()
            .lrem(self.key("active"), 1, lease)
            .zrem(self.key("leases"), lease)
            .del(self.key(&format!("job:{lease}")))
            .query_async::<()>(&mut c)
            .await?;
        Ok(())
    }

    async fn requeue_expired(&self) -> anyhow::Result<u64> {
        let mut c = self.conn.clone();
        // Active jobs without a lease (a worker died between taking the job
        // and leasing it) get one now, so they expire like the others.
        let active: Vec<String> = redis::cmd("LRANGE").arg(self.key("active")).arg(0).arg(-1).query_async(&mut c).await?;
        for id in &active {
            let _: () = redis::cmd("ZADD").arg(self.key("leases")).arg("NX").arg(now_ms() + self.lease.as_millis() as i64).arg(id).query_async(&mut c).await?;
        }
        let expired: Vec<String> = redis::cmd("ZRANGEBYSCORE").arg(self.key("leases")).arg("-inf").arg(now_ms()).query_async(&mut c).await?;
        let mut n = 0;
        for id in expired {
            // Only the process that removes the lease requeues the job.
            let removed: i64 = redis::cmd("ZREM").arg(self.key("leases")).arg(&id).query_async(&mut c).await?;
            if removed == 1 {
                redis::pipe().atomic().lrem(self.key("active"), 1, &id).rpush(self.key("wait"), &id).query_async::<()>(&mut c).await?;
                n += 1;
            }
        }
        Ok(n)
    }

    fn lease_duration(&self) -> Duration {
        self.lease
    }

    fn describe(&self) -> String {
        self.url.clone()
    }
}

// ---- PostgreSQL -------------------------------------------------------------------

pub struct PgQueue {
    pool: sqlx::PgPool,
    table: String,
    lease: Duration,
}

impl PgQueue {
    async fn connect() -> anyhow::Result<Self> {
        let url = env("R8R_DATABASE_URL").ok_or_else(|| anyhow::anyhow!("invalid configuration: R8R_DATABASE_URL: required for the PostgreSQL queue"))?;
        let schema = env("DB_POSTGRESDB_SCHEMA").unwrap_or_else(|| "public".into());
        if !schema.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            anyhow::bail!("invalid configuration: DB_POSTGRESDB_SCHEMA: only letters, digits and _ are allowed");
        }
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(5)
            .acquire_timeout(Duration::from_secs(10))
            .connect(&url)
            .await
            .map_err(|e| anyhow::anyhow!("cannot connect to PostgreSQL (R8R_DATABASE_URL): {e}"))?;
        sqlx::query(&format!("CREATE SCHEMA IF NOT EXISTS \"{schema}\"")).execute(&pool).await?;
        let table = format!("\"{schema}\".r8r_queue");
        sqlx::query(&format!(
            "CREATE TABLE IF NOT EXISTS {table} (id BIGSERIAL PRIMARY KEY, payload TEXT NOT NULL, leased_until TIMESTAMPTZ, created_at TIMESTAMPTZ NOT NULL DEFAULT now())"
        ))
        .execute(&pool)
        .await?;
        Ok(Self { pool, table, lease: lease_duration() })
    }
}

#[async_trait::async_trait]
impl Queue for PgQueue {
    async fn push(&self, payload: &Value) -> anyhow::Result<()> {
        sqlx::query(&format!("INSERT INTO {} (payload) VALUES ($1)", self.table)).bind(payload.to_string()).execute(&self.pool).await?;
        Ok(())
    }

    async fn pop(&self) -> anyhow::Result<Option<Leased>> {
        // Poll for up to a second; SKIP LOCKED lets workers take jobs in parallel.
        let sql = format!(
            "UPDATE {t} SET leased_until = now() + ($1 || ' milliseconds')::interval
             WHERE id = (SELECT id FROM {t} WHERE leased_until IS NULL OR leased_until < now() ORDER BY id FOR UPDATE SKIP LOCKED LIMIT 1)
             RETURNING id, payload",
            t = self.table
        );
        for _ in 0..5 {
            let row: Option<(i64, String)> = sqlx::query_as(&sql).bind(self.lease.as_millis().to_string()).fetch_optional(&self.pool).await?;
            if let Some((id, payload)) = row {
                return Ok(Some(Leased { lease: id.to_string(), payload: serde_json::from_str(&payload)? }));
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        Ok(None)
    }

    async fn heartbeat(&self, lease: &str) -> anyhow::Result<()> {
        sqlx::query(&format!("UPDATE {} SET leased_until = now() + ($1 || ' milliseconds')::interval WHERE id = $2", self.table))
            .bind(self.lease.as_millis().to_string())
            .bind(lease.parse::<i64>()?)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn ack(&self, lease: &str) -> anyhow::Result<()> {
        sqlx::query(&format!("DELETE FROM {} WHERE id = $1", self.table)).bind(lease.parse::<i64>()?).execute(&self.pool).await?;
        Ok(())
    }

    /// Expired leases are simply eligible again in `pop`.
    async fn requeue_expired(&self) -> anyhow::Result<u64> {
        Ok(0)
    }

    fn lease_duration(&self) -> Duration {
        self.lease
    }

    fn describe(&self) -> String {
        format!("postgres {}", self.table)
    }
}
