//! Group commit for the hot execution writes. Every execution inserts its
//! row when it starts and updates (or deletes) it when it ends; under load
//! one transaction per write makes SQLite's single writer the bottleneck.
//! Writes queue here instead and each batch commits in one transaction.
//! A write's caller is answered only after its batch has committed.

use sqlx::SqlitePool;
use tokio::sync::{mpsc, oneshot};

pub enum Bind {
    Text(Option<String>),
    Int(Option<i64>),
}

pub struct Op {
    pub sql: &'static str,
    pub binds: Vec<Bind>,
    /// `(rows affected, last insert rowid)`.
    pub reply: oneshot::Sender<Result<(u64, i64), String>>,
}

const MAX_BATCH: usize = 256;

pub fn spawn(writer: SqlitePool) -> mpsc::UnboundedSender<Op> {
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
                    let mut q = sqlx::query(op.sql);
                    for b in &op.binds {
                        q = match b {
                            Bind::Text(v) => q.bind(v.clone()),
                            Bind::Int(v) => q.bind(*v),
                        };
                    }
                    results.push(q.execute(&mut *t).await.map(|r| (r.rows_affected(), r.last_insert_rowid())).map_err(|e| e.to_string()));
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
