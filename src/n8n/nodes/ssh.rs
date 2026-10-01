//! SSH node (spec §6.6), faithful to n8n's `Ssh.node.js` (typeVersion 1):
//! authentication `password` (`sshPassword` credential) or `privateKey`
//! (`sshPrivateKey` credential, with an optional passphrase); resource
//! `command` (operation `execute`: `command` + `cwd` ->
//! `{code, signal, stdout, stderr}`) and resource `file` (`upload`: binary
//! data to `path`/`options.fileName`; `download`: a remote file to binary
//! under `binaryPropertyName`).
//!
//! Built on `russh` (command execution) and `russh-sftp` (file transfer --
//! the reference `node-ssh` library also transfers files over an SFTP
//! sub-channel of the same SSH connection, not SCP), both pure-Rust/tokio;
//! see `ssh_common.rs` for the shared connect/auth plumbing.
//!
//! Two-phase split (see `mysql.rs`'s module doc): phase 1 resolves every
//! item's parameters from `ExecCtx<'_>` synchronously (no `.await`, so
//! nothing borrowed from `ctx` needs to be `'static`); phase 2, run inside
//! `tokio::spawn`, owns only `String`/`Vec<u8>` data and does the actual
//! SSH/SFTP work.
//!
//! Deviations from n8n: `resolveHomeDir` (a leading `~/` in a path expands
//! to `$HOME`) is implemented for the `command` resource's `cwd` only, not
//! for `file` resource paths (an edge case; `file` paths are expected to be
//! absolute, as in every reference example). Exact wording of connection
//! errors differs from `ssh2`'s (a different underlying library).

use super::ssh_common::{self, shell_quote, SshAuth};
use crate::n8n::node::{ExecCtx, NodeError, NodeResult, NodeType};
use crate::n8n::types::{Item, NodeOutput};
use base64::Engine;
use russh::ChannelMsg;
use serde_json::{json, Map, Value};
use std::time::Duration;

pub fn all() -> Vec<Box<dyn NodeType>> {
    vec![Box::new(Ssh)]
}

struct Ssh;

#[async_trait::async_trait]
impl NodeType for Ssh {
    fn type_name(&self) -> &'static str {
        "n8n-nodes-base.ssh"
    }

    async fn execute(&self, ctx: &mut ExecCtx<'_>) -> NodeResult<NodeOutput> {
        let input_len = if ctx.input().is_empty() { 1 } else { ctx.input().len() };
        let authentication = ctx.param_str("authentication", 0, "password")?;
        let resource = ctx.param_str("resource", 0, "command")?;
        let continue_on_fail = ctx.continue_on_fail();

        let auth = resolve_auth(ctx, &authentication).await?;

        match resource.as_str() {
            "command" => run_command_resource(ctx, auth, input_len, continue_on_fail).await,
            "file" => run_file_resource(ctx, auth, input_len, continue_on_fail).await,
            other => Err(NodeError::new(format!("The resource \"{other}\" is not known!"))),
        }
    }
}

async fn resolve_auth(ctx: &ExecCtx<'_>, authentication: &str) -> NodeResult<SshAuth> {
    if authentication == "privateKey" {
        let (_, cred) = ctx.credentials("sshPrivateKey").await?;
        Ok(SshAuth::PrivateKey {
            host: cred_str(&cred, "host"),
            port: cred_u16(&cred, "port", 22),
            username: cred_str(&cred, "username"),
            private_key: cred_str(&cred, "privateKey"),
            passphrase: cred.get("passphrase").and_then(Value::as_str).filter(|s| !s.is_empty()).map(String::from),
        })
    } else {
        let (_, cred) = ctx.credentials("sshPassword").await?;
        Ok(SshAuth::Password { host: cred_str(&cred, "host"), port: cred_u16(&cred, "port", 22), username: cred_str(&cred, "username"), password: cred_str(&cred, "password") })
    }
}

fn cred_str(cred: &Value, key: &str) -> String {
    cred.get(key).and_then(Value::as_str).unwrap_or_default().to_string()
}

fn cred_u16(cred: &Value, key: &str, default: u16) -> u16 {
    cred.get(key).and_then(|v| v.as_u64().or_else(|| v.as_str().and_then(|s| s.parse().ok()))).map(|n| n as u16).unwrap_or(default)
}

const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);

// ---- resource: command -------------------------------------------------

async fn run_command_resource(ctx: &mut ExecCtx<'_>, auth: SshAuth, input_len: usize, continue_on_fail: bool) -> NodeResult<NodeOutput> {
    let operation = ctx.param_str("operation", 0, "execute")?;
    if operation != "execute" {
        return Err(NodeError::new(format!("The operation \"{operation}\" is not known!")));
    }

    // Phase 1: resolve every item's command + cwd (sync, ctx-only).
    let mut plans = Vec::with_capacity(input_len);
    for i in 0..input_len {
        match (|| -> NodeResult<(String, String)> { Ok((ctx.param_str("command", i, "")?, ctx.param_str("cwd", i, "/")?)) })() {
            Ok(p) => plans.push((i, p.0, p.1)),
            Err(e) if continue_on_fail => ctx.push_error_item(&e, i),
            Err(e) => return Err(e),
        }
    }

    let handle = tokio::spawn(run_commands(auth, plans, continue_on_fail));
    let (items, errors) = handle.await.map_err(|e| NodeError::new(format!("The SSH task panicked: {e}")))??;
    for (idx, e) in errors {
        ctx.push_error_item(&e, idx);
    }
    Ok(vec![items])
}

async fn run_commands(auth: SshAuth, plans: Vec<(usize, String, String)>, continue_on_fail: bool) -> NodeResult<(Vec<Item>, Vec<(usize, NodeError)>)> {
    let mut session = ssh_common::connect(&auth, CONNECT_TIMEOUT).await?;
    let mut items = Vec::with_capacity(plans.len());
    let mut errors = Vec::new();
    for (idx, command, cwd) in plans {
        match run_one_command(&mut session, &command, &cwd).await {
            Ok(json) => items.push(Item::new(json).paired(idx)),
            Err(e) => {
                let e = e.at(idx);
                if continue_on_fail {
                    errors.push((idx, e));
                } else {
                    let _ = session.disconnect(russh::Disconnect::ByApplication, "", "en").await;
                    return Err(e);
                }
            }
        }
    }
    let _ = session.disconnect(russh::Disconnect::ByApplication, "", "en").await;
    Ok((items, errors))
}

/// `resolveHomeDir`: a `cwd` of exactly `~` is an error (ambiguous: n8n
/// asks the user to replace it with the real home directory or `~/`); a
/// `~/`-prefixed `cwd` is expanded via a throwaway `echo $HOME`.
async fn resolve_cwd(session: &mut russh::client::Handle<ssh_common::ClientHandler>, cwd: &str) -> NodeResult<String> {
    if cwd == "~" {
        return Err(NodeError::new("Invalid path. Replace \"~\" with home directory or \"~/\""));
    }
    if let Some(rest) = cwd.strip_prefix("~/") {
        let (_, _, home, _) = exec_raw(session, "echo $HOME").await?;
        let mut home = home;
        if !home.ends_with('/') {
            home.push('/');
        }
        return Ok(format!("{home}{rest}"));
    }
    Ok(cwd.to_string())
}

async fn run_one_command(session: &mut russh::client::Handle<ssh_common::ClientHandler>, command: &str, cwd: &str) -> NodeResult<Map<String, Value>> {
    let resolved_cwd = resolve_cwd(session, cwd).await?;
    let full = if resolved_cwd.is_empty() { command.to_string() } else { format!("cd {} && {}", shell_quote(&resolved_cwd), command) };
    let (code, signal, stdout, stderr) = exec_raw(session, &full).await?;
    let mut json = Map::new();
    json.insert("code".into(), code.map(|c| json!(c)).unwrap_or(Value::Null));
    json.insert("signal".into(), signal.map(|s| json!(s)).unwrap_or(Value::Null));
    json.insert("stdout".into(), json!(stdout));
    json.insert("stderr".into(), json!(stderr));
    Ok(json)
}

/// Runs `command` on a fresh channel and returns `(exit_code, exit_signal,
/// stdout, stderr)`, trimming one trailing newline from each stream (the
/// reference `node-ssh` returns trimmed output).
async fn exec_raw(session: &mut russh::client::Handle<ssh_common::ClientHandler>, command: &str) -> NodeResult<(Option<i64>, Option<String>, String, String)> {
    let mut channel = session.channel_open_session().await.map_err(|e| NodeError::new(format!("SSH connection failed: {e}")))?;
    channel.exec(true, command).await.map_err(|e| NodeError::new(format!("SSH connection failed: {e}")))?;

    let mut code: Option<i64> = None;
    let mut signal: Option<String> = None;
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    while let Some(msg) = channel.wait().await {
        match msg {
            ChannelMsg::Data { data } => stdout.extend_from_slice(&data),
            ChannelMsg::ExtendedData { data, ext: 1 } => stderr.extend_from_slice(&data),
            ChannelMsg::ExitStatus { exit_status } => code = Some(exit_status as i64),
            ChannelMsg::ExitSignal { signal_name, .. } => signal = Some(format!("{signal_name:?}")),
            _ => {}
        }
    }
    let stdout = String::from_utf8_lossy(&stdout).trim_end_matches('\n').to_string();
    let stderr = String::from_utf8_lossy(&stderr).trim_end_matches('\n').to_string();
    Ok((code, signal, stdout, stderr))
}

// ---- resource: file (SFTP-based) ---------------------------------------

enum FilePlan {
    Upload { target_dir: String, file_name: String, bytes: Vec<u8> },
    Download { remote_path: String, binary_prop: String, file_name_override: Option<String> },
}

async fn run_file_resource(ctx: &mut ExecCtx<'_>, auth: SshAuth, input_len: usize, continue_on_fail: bool) -> NodeResult<NodeOutput> {
    let operation = ctx.param_str("operation", 0, "upload")?;

    // Phase 1 (sync): resolve every item's plan, keeping the original item
    // (json + binary) for `download`, which attaches the new binary
    // property onto a shallow copy of the existing item (matching the
    // reference `items[i] = newItem` behaviour).
    let mut plans: Vec<(usize, FilePlan, Item)> = Vec::with_capacity(input_len);
    for i in 0..input_len {
        let item = ctx.input().get(i).cloned().unwrap_or_default();
        let plan = match operation.as_str() {
            "upload" => resolve_upload(ctx, i, &item),
            "download" => resolve_download(ctx, i),
            other => Err(NodeError::new(format!("The operation \"{other}\" is not known!")).at(i)),
        };
        match plan {
            Ok(p) => plans.push((i, p, item)),
            Err(e) if continue_on_fail => ctx.push_error_item(&e, i),
            Err(e) => return Err(e),
        }
    }

    let items_only: Vec<(usize, FilePlan)> = plans.iter().map(|(i, p, _)| (*i, clone_plan(p))).collect();
    let originals: std::collections::HashMap<usize, Item> = plans.into_iter().map(|(i, _, item)| (i, item)).collect();

    let handle = tokio::spawn(run_file_plans(auth, items_only));
    let (results, errors) = handle.await.map_err(|e| NodeError::new(format!("The SSH task panicked: {e}")))??;

    let mut out = Vec::with_capacity(results.len());
    let mut failed = Vec::new();
    for (idx, result) in results {
        match result {
            Ok(outcome) => out.push(apply_outcome(originals.get(&idx).cloned().unwrap_or_default(), outcome, idx)),
            Err(e) => failed.push((idx, e.at(idx))),
        }
    }
    for (idx, e) in errors.into_iter().chain(failed) {
        ctx.push_error_item(&e, idx);
    }
    Ok(vec![out])
}

fn clone_plan(p: &FilePlan) -> FilePlan {
    match p {
        FilePlan::Upload { target_dir, file_name, bytes } => FilePlan::Upload { target_dir: target_dir.clone(), file_name: file_name.clone(), bytes: bytes.clone() },
        FilePlan::Download { remote_path, binary_prop, file_name_override } => {
            FilePlan::Download { remote_path: remote_path.clone(), binary_prop: binary_prop.clone(), file_name_override: file_name_override.clone() }
        }
    }
}

enum FileOutcome {
    Uploaded,
    Downloaded { binary_prop: String, bytes: Vec<u8>, file_name: String },
}

fn apply_outcome(original: Item, outcome: FileOutcome, idx: usize) -> Item {
    match outcome {
        FileOutcome::Uploaded => {
            let mut json = Map::new();
            json.insert("success".into(), json!(true));
            Item::new(json).paired(idx)
        }
        FileOutcome::Downloaded { binary_prop, bytes, file_name } => {
            let mut binary = original.binary.clone().unwrap_or_default();
            binary.insert(binary_prop, make_binary(&bytes, &file_name));
            Item { json: original.json, binary: Some(binary), paired_item: None }.paired(idx)
        }
    }
}

fn resolve_upload(ctx: &ExecCtx<'_>, i: usize, item: &Item) -> NodeResult<FilePlan> {
    let binary_prop = ctx.param_str("binaryPropertyName", i, "data")?;
    let target_dir = ctx.param_str("path", i, "")?;
    if target_dir.is_empty() {
        return Err(NodeError::new("Target Directory is required").at(i));
    }
    let entry = binary_entry(item, &binary_prop, i)?;
    let bytes = binary_bytes(entry, i)?;
    let file_name_override = ctx.param_str("options.fileName", i, "")?;
    let original_name = entry.get("fileName").and_then(Value::as_str).unwrap_or("file");
    let raw_name = if file_name_override.is_empty() { original_name } else { &file_name_override };
    Ok(FilePlan::Upload { target_dir, file_name: sanitize_filename(raw_name), bytes })
}

fn resolve_download(ctx: &ExecCtx<'_>, i: usize) -> NodeResult<FilePlan> {
    let remote_path = ctx.param_str("path", i, "")?;
    if remote_path.is_empty() {
        return Err(NodeError::new("Path is required").at(i));
    }
    let binary_prop = ctx.param_str("binaryPropertyName", i, "data")?;
    let file_name_override = ctx.param_str("options.fileName", i, "")?;
    Ok(FilePlan::Download { remote_path, binary_prop, file_name_override: if file_name_override.is_empty() { None } else { Some(file_name_override) } })
}

async fn run_file_plans(auth: SshAuth, plans: Vec<(usize, FilePlan)>) -> NodeResult<(Vec<(usize, NodeResult<FileOutcome>)>, Vec<(usize, NodeError)>)> {
    let mut session = ssh_common::connect(&auth, CONNECT_TIMEOUT).await?;
    let sftp = ssh_common::open_sftp(&mut session).await?;
    let mut out = Vec::with_capacity(plans.len());
    for (idx, plan) in plans {
        let result = run_one_file_plan(&sftp, plan).await;
        out.push((idx, result));
    }
    let _ = sftp.close().await;
    let _ = session.disconnect(russh::Disconnect::ByApplication, "", "en").await;
    Ok((out, Vec::new()))
}

async fn run_one_file_plan(sftp: &russh_sftp::client::SftpSession, plan: FilePlan) -> NodeResult<FileOutcome> {
    match plan {
        FilePlan::Upload { target_dir, file_name, bytes } => {
            let remote_path = if target_dir.ends_with('/') { format!("{target_dir}{file_name}") } else { format!("{target_dir}/{file_name}") };
            ssh_common::sftp_write(sftp, &remote_path, &bytes).await?;
            Ok(FileOutcome::Uploaded)
        }
        FilePlan::Download { remote_path, binary_prop, file_name_override } => {
            let bytes = sftp.read(remote_path.clone()).await.map_err(|e| NodeError::new(format!("SFTP download failed: {e}")))?;
            let file_name = file_name_override.unwrap_or_else(|| remote_path.rsplit('/').next().unwrap_or(&remote_path).to_string());
            Ok(FileOutcome::Downloaded { binary_prop, bytes, file_name })
        }
    }
}

fn sanitize_filename(name: &str) -> String {
    let base = name.rsplit('/').next().unwrap_or(name);
    base.replace(['/', '\\'], "_")
}

fn extension_for_mime(mime: &str) -> &str {
    match mime {
        "text/plain" => "txt",
        "application/json" => "json",
        "application/octet-stream" => "bin",
        other => other.split('/').nth(1).unwrap_or("bin"),
    }
}

fn guess_mime_from_extension(name: &str) -> &'static str {
    match name.rsplit('.').next().unwrap_or("").to_ascii_lowercase().as_str() {
        "txt" => "text/plain",
        "json" => "application/json",
        "csv" => "text/csv",
        "html" | "htm" => "text/html",
        _ => "application/octet-stream",
    }
}

fn make_binary(data: &[u8], file_name: &str) -> Value {
    let mime = guess_mime_from_extension(file_name);
    json!({
        "data": base64::engine::general_purpose::STANDARD.encode(data),
        "mimeType": mime,
        "fileExtension": extension_for_mime(mime),
        "fileSize": format!("{} B", data.len()),
        "fileName": file_name,
    })
}

fn binary_entry<'a>(item: &'a Item, prop: &str, idx: usize) -> NodeResult<&'a Value> {
    let binary = item
        .binary
        .as_ref()
        .ok_or_else(|| NodeError::new(format!("This operation expects the node's input data to contain a binary file '{prop}', but none was found [item {idx}]")).at(idx))?;
    binary.get(prop).ok_or_else(|| NodeError::new(format!("The item has no binary field '{prop}' [item {idx}]")).at(idx))
}

fn binary_bytes(entry: &Value, idx: usize) -> NodeResult<Vec<u8>> {
    let data = entry.get("data").and_then(Value::as_str).ok_or_else(|| NodeError::new("Binary data has no \"data\" payload").at(idx))?;
    base64::engine::general_purpose::STANDARD.decode(data.trim()).map_err(|e| NodeError::new(format!("Binary data is not valid base64: {e}")).at(idx))
}
