//! n8n-compatible core (spec §6): workflow model, execution engine,
//! expressions, native nodes, credentials and storage.
//!
//! This grows alongside the original `r8r` engine (`crate::engine`,
//! `crate::nodes`) until the server moves over to it (Phase 2). The CLI
//! (`crate::cli`) already runs on it.

pub mod cipher;
pub mod config;
pub mod engine;
pub mod expr;
pub mod node;
pub mod nodes;
pub mod store;
pub mod types;
pub mod vm;
pub mod workflow;
