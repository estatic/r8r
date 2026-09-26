//! n8n-compatible core (spec §6): workflow model, execution engine,
//! expressions, native nodes, credentials and storage.
//!
//! The server (`server`) and the CLI (`crate::cli`) run on it. The original
//! `r8r` engine (`crate::engine`, `crate::nodes`) still serves the legacy
//! editor API under `/rest/r8r`.

pub mod cipher;
pub mod config;
pub mod credential_types;
pub mod engine;
pub mod expr;
pub mod node;
pub mod node_types;
pub mod nodes;
pub mod server;
pub mod store;
pub mod store_ext;
pub mod types;
pub mod vm;
pub mod workflow;
