//! Shared SSH session setup -- connect, then password or public-key
//! (private key + optional passphrase) authentication -- used by `ssh.rs`
//! (command execution and SFTP-based file transfer) and `ftp.rs`'s `sftp`
//! protocol. Built on `russh` (pure-Rust SSH client, tokio) and
//! `russh-sftp` (pure-Rust SFTP client, run as an SSH subsystem channel on
//! top of a `russh` channel), matching the task brief's "prefer pure-Rust
//! over libssh2 bindings".
//!
//! n8n's reference nodes (`node-ssh`/`ssh2-sftp-client`, both wrapping
//! `ssh2`) don't verify host keys either (no `hostVerifier` is configured),
//! so `check_server_key` here always accepts, matching that behaviour.

use crate::n8n::node::NodeError;
use russh::client::{self, Handle};
use russh::keys::{decode_secret_key, PrivateKeyWithHashAlg, PublicKeyOrCertificate};
use russh_sftp::client::SftpSession;
use std::sync::Arc;
use std::time::Duration;

/// Owned (`'static`) connection parameters, resolved from the node's
/// credential before crossing into the `tokio::spawn`ed task (see
/// `mysql.rs`'s module doc for why that split exists: `ExecCtx<'_>` borrows
/// the running node/workflow and cannot itself move into a spawned task).
#[derive(Clone)]
pub enum SshAuth {
    Password { host: String, port: u16, username: String, password: String },
    PrivateKey { host: String, port: u16, username: String, private_key: String, passphrase: Option<String> },
}

impl SshAuth {
    pub fn host(&self) -> &str {
        match self {
            SshAuth::Password { host, .. } | SshAuth::PrivateKey { host, .. } => host,
        }
    }

    pub fn port(&self) -> u16 {
        match self {
            SshAuth::Password { port, .. } | SshAuth::PrivateKey { port, .. } => *port,
        }
    }
}

pub struct ClientHandler;

impl client::Handler for ClientHandler {
    type Error = russh::Error;

    async fn check_server_key(&mut self, _server_public_key: &PublicKeyOrCertificate) -> Result<bool, Self::Error> {
        Ok(true)
    }
}

/// `n8n_workflow`'s `NodeOperationError`-style prefix n8n's own SSH/SFTP
/// credential tests use ("SSH connection failed: <reason>" /
/// "sftp connection failed: ..."); kept as a stable, recognisable prefix
/// across every connect-time failure here.
fn conn_error(detail: impl std::fmt::Display) -> NodeError {
    NodeError::new(format!("SSH connection failed: {detail}"))
}

/// Connects and authenticates, returning the live session handle.
/// `connect_timeout` bounds the TCP connect only (matching n8n's own
/// connect-timeout-only behaviour; there's no separate auth timeout).
pub async fn connect(auth: &SshAuth, connect_timeout: Duration) -> Result<Handle<ClientHandler>, NodeError> {
    let config = Arc::new(client::Config::default());
    let addr = (auth.host().to_string(), auth.port());

    let mut session = tokio::time::timeout(connect_timeout, client::connect(config, addr, ClientHandler))
        .await
        .map_err(|_| conn_error(format!("connect ETIMEDOUT {}:{}", auth.host(), auth.port())))?
        .map_err(conn_error)?;

    let auth_ok = match auth {
        SshAuth::Password { username, password, .. } => session
            .authenticate_password(username.clone(), password.clone())
            .await
            .map_err(conn_error)?
            .success(),
        SshAuth::PrivateKey { username, private_key, passphrase, .. } => {
            let key = decode_secret_key(private_key, passphrase.as_deref()).map_err(conn_error)?;
            let hash_alg = session.best_supported_rsa_hash().await.map_err(conn_error)?.flatten();
            session
                .authenticate_publickey(username.clone(), PrivateKeyWithHashAlg::new(Arc::new(key), hash_alg))
                .await
                .map_err(conn_error)?
                .success()
        }
    };
    if !auth_ok {
        return Err(conn_error("All configured authentication methods failed"));
    }
    Ok(session)
}

/// Opens an SFTP session over a fresh channel of an already-authenticated
/// SSH session (the `sftp` subsystem, per RFC 4254 §6.5 / the SFTP draft).
pub async fn open_sftp(session: &mut Handle<ClientHandler>) -> Result<SftpSession, NodeError> {
    let channel = session.channel_open_session().await.map_err(conn_error)?;
    channel.request_subsystem(true, "sftp").await.map_err(conn_error)?;
    SftpSession::new(channel.into_stream()).await.map_err(|e| NodeError::new(format!("SFTP session failed: {e}")))
}

/// A short, byte-exact shell quote: wraps in single quotes, escaping any
/// embedded `'` as `'\''` (POSIX sh, works on every target shell these test
/// servers run).
pub fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}
