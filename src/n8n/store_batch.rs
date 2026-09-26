//! Group commit for the hot execution writes. Every execution inserts its
//! row when it starts and updates (or deletes) it when it ends; under load
//! one transaction per write makes SQLite's single writer the bottleneck.
//! Writes queue here instead and each batch commits in one transaction.
//! A write's caller is answered only after its batch has committed.

use sqlx::AnyPool;
use sqlx::Row;
use tokio::sync::{mpsc, oneshot};

pub enum Bind {
    Text(Option<String>),
    Int(Option<i64>),
}

pub struct Op {
    /// Already in the database's placeholder style.
    pub sql: String,
    /// An INSERT whose new row id is wanted.
    pub returning: bool,
    pub binds: Vec<Bind>,
    /// `(rows affected, returned id)`.
    pub reply: oneshot::Sender<Result<(u64, i64), String>>,
}

const MAX_BATCH: usize = 256;

pub fn spawn(writer: AnyPool, postgres: bool) -> mpsc::UnboundedSender<Op> {
    let (tx, mut rx) = mpsc::unbounded_channel::<Op>();
    tokio::spawn(async move {
        while let Some(first) = rx.recv().await {
            let mut ops = vec![first];
            while ops.len() < MAX_BATCH {
                match rx.try_recv() {
                    Ok(op) => ops.push(op),
                    Err(_) => break,
                }
            }
            let mut results: Vec<Result<(u64, i64), String>> = Vec::with_capacity(ops.len());
            let committed = async {
                let mut t = writer.begin().await?;
                for op in &ops {
                    let mut q = sqlx::query(&op.sql);
                    for b in &op.binds {
                        q = match b {
                            Bind::Text(v) => q.bind(v.clone()),
                            Bind::Int(v) => q.bind(*v),
                        };
                    }
                    let r = match q.execute(&mut *t).await {
                        Ok(_) if op.returning => new_id(&mut t, postgres).await.map(|id| (1, id)),
                        Ok(done) => Ok((done.rows_affected(), 0)),
                        Err(e) => Err(e),
                    };
                    results.push(r.map_err(|e| e.to_string()));
                }
                t.commit().await
            }
            .await;
            match committed {
                Ok(()) => {
                    for (op, r) in ops.into_iter().zip(results) {
                        let _ = op.reply.send(r);
                    }
                }
                Err(e) => {
                    for op in ops {
                        let _ = op.reply.send(Err(format!("could not commit: {e}")));
                    }
                }
            }
        }
    });
    tx
}

/// The id of the row just inserted on this connection. (sqlx's `Any` driver
/// returns no rows for `INSERT ... RETURNING` on PostgreSQL and no
/// `last_insert_id` on SQLite, so each database is asked directly.)
pub async fn new_id(conn: &mut sqlx::AnyConnection, postgres: bool) -> Result<i64, sqlx::Error> {
    let sql = if postgres { "SELECT lastval() AS id" } else { "SELECT last_insert_rowid() AS id" };
    let rows = sqlx::query(sql).fetch_all(conn).await?;
    rows.first().map(|r| r.get::<i64, _>("id")).ok_or(sqlx::Error::RowNotFound)
}
