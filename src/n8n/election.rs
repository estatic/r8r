//! Multi-main leader election (plan task 4.1, spec §7.3 follow-on).
//!
//! When two or more `r8r start` processes share a queue-mode deployment
//! (`EXECUTIONS_MODE=queue`), only one of them should run schedules,
//! pollers and other long-lived triggers (RabbitMQ/Kafka/MQTT/IMAP, once
//! those land) -- running the same interval timer on every main would fire
//! each tick once per main. Webhooks and forms are stateless HTTP handlers
//! and stay registered on every main, exactly as n8n does.
//!
//! This mirrors n8n's `MultiMainSetup` / `LeaderElectionClient`
//! (scaling/multi-main-setup.ee.js, scaling/leader-election-client.js):
//! a Redis key `{prefix}:main_instance_leader` is claimed with
//! `SET key hostId NX EX ttl`, and renewed by the holder with a
//! compare-and-expire Lua script so a main that lost the key (e.g. after a
//! long GC pause) steps down instead of assuming it is still leader.
//!
//! Env (names match n8n):
//! - `N8N_MULTI_MAIN_SETUP_ENABLED` (default false) -- single-main, the
//!   default, never runs this: [`LeaderGate::always_leader`] is always true
//!   and existing behaviour is unchanged.
//! - `N8N_MULTI_MAIN_SETUP_KEY_TTL` seconds, default 10.
//! - `N8N_MULTI_MAIN_SETUP_CHECK_INTERVAL` seconds, default 3.

use std::sync::Arc;
use std::time::Duration;
use tokio::sync::watch;

/// Read-only, cheaply-clonable handle to the instance's current
/// leadership. Schedules, pollers and (in future) long-lived trigger nodes
/// such as RabbitMQ/Kafka/MQTT/IMAP should hold one of these and only run
/// their background work while `is_leader()` is true, re-checking via
/// `changed()`/`subscribe()` so a takeover or stepdown is picked up without
/// polling.
#[derive(Clone)]
pub struct LeaderGate(watch::Receiver<bool>);

impl LeaderGate {
    fn new(rx: watch::Receiver<bool>) -> Self {
        Self(rx)
    }

    /// Single-main (the default): always leader, forever. Used when
    /// `N8N_MULTI_MAIN_SETUP_ENABLED` is not set, so existing behaviour is
    /// unchanged.
    pub fn always_leader() -> Self {
        let (_tx, rx) = watch::channel(true);
        Self(rx)
    }

    pub fn is_leader(&self) -> bool {
        *self.0.borrow()
    }

    /// A clone of the underlying receiver, for a task that wants to await
    /// `changed()` on its own without borrowing this gate.
    pub fn subscribe(&self) -> watch::Receiver<bool> {
        self.0.clone()
    }
}

/// What an election backend must provide: acquire-if-vacant and
/// renew-if-still-ours, both compare-and-swap so two mains can never both
/// believe they hold the lock.
#[async_trait::async_trait]
pub trait ElectionBackend: Send + Sync {
    /// `SET key hostId NX EX ttl`: true if the lock is now held by us
    /// (either we just set it, or nobody else did).
    async fn try_acquire(&self) -> anyhow::Result<bool>;
    /// Renews the TTL only if the key still holds our id. `Ok(false)`
    /// means the key expired or another host holds it -- we are no longer
    /// leader and must go through `try_acquire` again from scratch.
    async fn renew(&self) -> anyhow::Result<bool>;
    /// Clears the key, but only if we still hold it (graceful shutdown, so
    /// the next main doesn't wait out the TTL).
    async fn release(&self);
    fn describe(&self) -> String;
    /// How long the lock outlives its last renewal (the key's TTL).
    fn lease(&self) -> Duration;
}

/// Drives an [`ElectionBackend`] on an interval, exposing the result via a
/// [`LeaderGate`]. Mirrors n8n's `MultiMainSetup.checkLeader`.
pub struct Election {
    gate_tx: watch::Sender<bool>,
    pub gate: LeaderGate,
    backend: Arc<dyn ElectionBackend>,
    checker: std::sync::Mutex<Option<tokio::task::JoinHandle<()>>>,
    /// When the lock was last acquired or renewed. A leader that can't reach
    /// Redis keeps leading only until the lock it last set has expired;
    /// after that another main may hold it, and leading on would mean two
    /// mains firing every schedule.
    last_held: std::sync::Mutex<std::time::Instant>,
}

impl Election {
    /// Makes an initial bid for leadership, then spawns a background task
    /// that renews (if leader) or tries to take over (if follower) every
    /// `interval`.
    pub async fn start(backend: Arc<dyn ElectionBackend>, interval: Duration) -> Arc<Self> {
        let initial = match backend.try_acquire().await {
            Ok(got) => got,
            Err(e) => {
                tracing::warn!(error = %e, "[multi-main] could not reach the leader lock at start-up; starting as a follower");
                false
            }
        };
        let (gate_tx, rx) = watch::channel(initial);
        if initial {
            tracing::info!("[multi-main] leader is now this instance");
        } else {
            tracing::info!("[multi-main] this instance starts as a follower");
        }
        let election = Arc::new(Self {
            gate_tx,
            gate: LeaderGate::new(rx),
            backend,
            checker: std::sync::Mutex::new(None),
            last_held: std::sync::Mutex::new(std::time::Instant::now()),
        });
        let weak = Arc::downgrade(&election);
        let handle = tokio::spawn(async move {
            let mut tick = tokio::time::interval(interval);
            tick.tick().await; // the first tick is immediate; we already bid above
            loop {
                tick.tick().await;
                let Some(election) = weak.upgrade() else { return };
                election.check().await;
            }
        });
        *election.checker.lock().unwrap() = Some(handle);
        election
    }

    async fn check(&self) {
        let was_leader = self.gate.is_leader();
        let now_leader = if was_leader {
            match self.backend.renew().await {
                Ok(true) => {
                    *self.last_held.lock().unwrap() = std::time::Instant::now();
                    true
                }
                Ok(false) => {
                    tracing::warn!("[multi-main] lost the leader lock; stepping down");
                    false
                }
                Err(e) if self.last_held.lock().unwrap().elapsed() < self.backend.lease() => {
                    tracing::warn!(error = %e, "[multi-main] could not renew the leader lock; holding on until it expires");
                    true
                }
                Err(e) => {
                    tracing::warn!(error = %e, "[multi-main] could not renew the leader lock and it has expired; stepping down");
                    false
                }
            }
        } else {
            match self.backend.try_acquire().await {
                Ok(got) => {
                    if got {
                        *self.last_held.lock().unwrap() = std::time::Instant::now();
                    }
                    got
                }
                Err(e) => {
                    tracing::warn!(error = %e, "[multi-main] could not check the leader lock");
                    false
                }
            }
        };
        if now_leader != was_leader {
            if now_leader {
                tracing::info!("[multi-main] leader is now this instance");
            } else {
                tracing::info!("[multi-main] this is now a follower instance");
            }
            let _ = self.gate_tx.send(now_leader);
        }
    }

    /// Graceful shutdown: stop checking and give up the lock if we hold it,
    /// so the next main doesn't wait out the full TTL.
    pub async fn shutdown(&self) {
        if let Some(h) = self.checker.lock().unwrap().take() {
            h.abort();
        }
        if self.gate.is_leader() {
            self.backend.release().await;
        }
    }

    pub fn describe(&self) -> String {
        self.backend.describe()
    }
}

// ---- Redis backend ------------------------------------------------------

/// `GET key == hostId ? EXPIRE key ttl : 0` -- renews only if we still
/// hold the lock (n8n's `INCREASE_TTL_IF_LEADER`).
const RENEW_SCRIPT: &str = r"
local v = redis.call('GET', KEYS[1])
if not v then return 0 end
if v ~= ARGV[1] then return 0 end
return redis.call('EXPIRE', KEYS[1], ARGV[2])
";

/// `GET key == hostId ? DEL key : 0` -- compare-and-delete, so releasing a
/// lock we no longer hold (e.g. raced by another main) is a no-op.
const RELEASE_SCRIPT: &str = r"
local v = redis.call('GET', KEYS[1])
if v == ARGV[1] then return redis.call('DEL', KEYS[1]) end
return 0
";

pub struct RedisElectionBackend {
    conn: redis::aio::MultiplexedConnection,
    key: String,
    host_id: String,
    ttl_secs: u64,
    url: String,
}

impl RedisElectionBackend {
    /// Connects using the same `QUEUE_BULL_REDIS_*` variables as the job
    /// queue (`src/n8n/queue.rs`) -- multi-main setup in n8n always shares
    /// its Redis connection with the queue, and r8r only offers a Redis
    /// backend for leader election (see module docs for why a PostgreSQL
    /// advisory-lock variant is not included).
    pub async fn connect(ttl_secs: u64) -> anyhow::Result<Self> {
        fn env(name: &str) -> Option<String> {
            std::env::var(name).ok().filter(|v| !v.trim().is_empty())
        }
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
            .map_err(|e| anyhow::anyhow!("cannot connect to Redis at {host}:{port} for leader election (QUEUE_BULL_REDIS_HOST/PORT): {e}"))?;
        // n8n's leader key uses the same top-level prefix as the queue
        // (`redis.prefix`), not the `bull`-suffixed job key prefix.
        let prefix = env("QUEUE_BULL_PREFIX").unwrap_or_else(|| "bull".into());
        let host_id = env("R8R_INSTANCE_ID").unwrap_or_else(|| format!("{}-{}", hostname(), std::process::id()));
        Ok(Self { conn, key: format!("{prefix}:main_instance_leader"), host_id, ttl_secs: ttl_secs.max(1), url: format!("redis {host}:{port}/{db}") })
    }
}

fn hostname() -> String {
    std::env::var("HOSTNAME").ok().or_else(|| std::env::var("COMPUTERNAME").ok()).unwrap_or_else(|| "r8r".into())
}

#[async_trait::async_trait]
impl ElectionBackend for RedisElectionBackend {
    async fn try_acquire(&self) -> anyhow::Result<bool> {
        let mut c = self.conn.clone();
        let result: Option<String> = redis::cmd("SET").arg(&self.key).arg(&self.host_id).arg("NX").arg("EX").arg(self.ttl_secs).query_async(&mut c).await?;
        Ok(result.is_some())
    }

    async fn renew(&self) -> anyhow::Result<bool> {
        let mut c = self.conn.clone();
        let result: i64 = redis::Script::new(RENEW_SCRIPT).key(&self.key).arg(&self.host_id).arg(self.ttl_secs).invoke_async(&mut c).await?;
        Ok(result == 1)
    }

    async fn release(&self) {
        let mut c = self.conn.clone();
        let _: Result<i64, _> = redis::Script::new(RELEASE_SCRIPT).key(&self.key).arg(&self.host_id).invoke_async(&mut c).await;
    }

    fn describe(&self) -> String {
        self.url.clone()
    }

    /// 80% of the key's TTL: the key expires TTL after Redis applied the
    /// last renewal, which was before we observed it, so stepping down at
    /// the full TTL could overlap a new leader by a round trip.
    fn lease(&self) -> Duration {
        Duration::from_secs(self.ttl_secs).mul_f64(0.8)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn always_leader_never_changes() {
        let gate = LeaderGate::always_leader();
        assert!(gate.is_leader());
    }

    /// Acquires once, then every renewal fails as if Redis were unreachable.
    struct Unreachable {
        lease: Duration,
    }

    #[async_trait::async_trait]
    impl ElectionBackend for Unreachable {
        async fn try_acquire(&self) -> anyhow::Result<bool> {
            Ok(true)
        }
        async fn renew(&self) -> anyhow::Result<bool> {
            anyhow::bail!("connection refused")
        }
        async fn release(&self) {}
        fn describe(&self) -> String {
            "unreachable".into()
        }
        fn lease(&self) -> Duration {
            self.lease
        }
    }

    #[tokio::test]
    async fn a_leader_that_cannot_renew_steps_down_once_its_lease_is_over() {
        // A long interval so only the explicit checks below run.
        let election = Election::start(Arc::new(Unreachable { lease: Duration::from_millis(200) }), Duration::from_secs(3600)).await;
        assert!(election.gate.is_leader());

        election.check().await;
        assert!(election.gate.is_leader(), "a failed renewal within the lease keeps leading");

        tokio::time::sleep(Duration::from_millis(250)).await;
        election.check().await;
        assert!(!election.gate.is_leader(), "past the lease another main may hold the lock");
    }
}
