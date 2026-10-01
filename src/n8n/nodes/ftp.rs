//! FTP node (spec §6.6), faithful to n8n's `Ftp.node.js` (typeVersion
//! 1/1.1): protocol `ftp` (credential `ftp`) or `sftp` (credential `sftp`,
//! password or private key + optional passphrase); operations `delete`
//! (file or, with `options.folder`, a folder -- optionally
//! `options.recursive`), `download` (-> binary), `list`
//! (`recursive` option), `rename` (`oldPath`/`newPath`, optionally
//! `options.createDirectories`), `upload` (binary or `fileContent` text).
//!
//! Built on `suppaftp` (protocol `ftp`, tokio feature) and `russh` +
//! `russh-sftp` (protocol `sftp`, reusing `ssh_common.rs`'s connect/auth),
//! both pure-Rust/tokio.
//!
//! Faithful quirks kept from the reference:
//! - `upload`'s output item is the *input* item unchanged (json + any
//!   pre-existing binary), not a `{success: true}` marker -- unlike `list`/
//!   `delete`/`rename`, which replace the item's json with the operation's
//!   own result, and unlike `download`, which attaches a new binary
//!   property onto a copy of the input item.
//! - Listed entries are normalised to `{type, name, size, modifyTime,
//!   path}` (sftp also carries `accessTime`), mirroring
//!   `normalizeFtpItem`/`normalizeSFtpItem`; `path` is the full path to the
//!   entry (parent + name for a non-recursive list, the already-absolute
//!   recursive-walk path otherwise).
//!
//! Deviations from n8n: `upload` always ensures its destination directory
//! exists first (matching the reference's *unconditional* sftp behaviour,
//! `recursivelyCreateSftpDirs`), rather than the reference ftp protocol's
//! reactive "retry after mkdir on a directory-missing error code" --
//! `options.createDirectories` isn't actually an upload option in the
//! reference (only `rename` has it); r8r's always-create behaviour makes
//! that option moot for uploads on both protocols. `rights`/`owner`/`group`
//! are not included in list output (numeric uid/gid vs. resolved names
//! differs too much across server configurations to assert reliably).
//! Exact wording of connection/auth errors differs from the reference's
//! underlying libraries.

use super::ssh_common::{self, SshAuth};
use crate::n8n::node::{ExecCtx, NodeError, NodeResult, NodeType};
use crate::n8n::types::{Item, NodeOutput};
use base64::Engine;
use chrono::{DateTime, SecondsFormat, Utc};
use russh_sftp::client::SftpSession;
use russh_sftp::protocol::FileType as SftpFileType;
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::str::FromStr;
use std::time::{Duration, SystemTime};
use suppaftp::list::File as FtpListFile;
use suppaftp::tokio::AsyncFtpStream;
use suppaftp::types::FileType as FtpFileType;

pub fn all() -> Vec<Box<dyn NodeType>> {
    vec![Box::new(Ftp)]
}

struct Ftp;

#[async_trait::async_trait]
impl NodeType for Ftp {
    fn type_name(&self) -> &'static str {
        "n8n-nodes-base.ftp"
    }

    async fn execute(&self, ctx: &mut ExecCtx<'_>) -> NodeResult<NodeOutput> {
        let input_len = if ctx.input().is_empty() { 1 } else { ctx.input().len() };
        let protocol = ctx.param_str("protocol", 0, "ftp")?;
        let operation = ctx.param_str("operation", 0, "download")?;
        let continue_on_fail = ctx.continue_on_fail();
        let timeout_ms = ctx.param_f64("options.timeout", 0, 10000.0)?.max(1.0) as u64;

        let (_, cred) = ctx.credentials_typed(&protocol).await?;

        // Phase 1 (sync, ctx-only): resolve every item's plan.
        let mut resolved: Vec<(usize, FtpPlan, Item)> = Vec::with_capacity(input_len);
        for i in 0..input_len {
            let item = ctx.input().get(i).cloned().unwrap_or_default();
            match resolve_plan(ctx, i, &operation, &item) {
                Ok(plan) => resolved.push((i, plan, item)),
                Err(e) if continue_on_fail => ctx.push_error_item(&e, i),
                Err(e) => return Err(e),
            }
        }
        let originals: HashMap<usize, Item> = resolved.iter().map(|(i, _, item)| (*i, item.clone())).collect();
        let plans: Vec<(usize, FtpPlan)> = resolved.into_iter().map(|(i, p, _)| (i, p)).collect();
        let timeout = Duration::from_millis(timeout_ms);

        // Phase 2 (owned/'static): connect and run every plan in its own
        // task (see `mysql.rs`'s module doc for why this split exists).
        let batch = if protocol == "sftp" {
            let auth = sftp_auth(&cred);
            tokio::spawn(run_sftp(auth, timeout, plans)).await
        } else {
            let host = cred_str(&cred, "host");
            let port = cred_u16(&cred, "port", 21);
            let user = cred_str(&cred, "username");
            let pass = cred_str(&cred, "password");
            tokio::spawn(run_ftp(host, port, user, pass, timeout, plans)).await
        };
        let results = batch.map_err(|e| NodeError::new(format!("The FTP task panicked: {e}")))?;

        let results = match results {
            Ok(r) => r,
            Err(e) => {
                if continue_on_fail {
                    let mut json = Map::new();
                    json.insert("error".into(), json!(e.message));
                    return Ok(vec![vec![Item::new(json)]]);
                }
                return Err(e);
            }
        };

        let mut out = Vec::new();
        for (idx, result) in results {
            match result {
                Ok(outcome) => out.extend(apply_outcome(originals.get(&idx).cloned().unwrap_or_default(), outcome, idx)),
                Err(e) => {
                    let e = e.at(idx);
                    if continue_on_fail {
                        ctx.push_error_item(&e, idx);
                    } else {
                        return Err(e);
                    }
                }
            }
        }
        Ok(vec![out])
    }
}

// n8n's credential name differs from the parameter value only for "ftp"
// (parameter "ftp" -> credential "ftp") and "sftp" (parameter "sftp" ->
// credential "sftp"); both already match, so this is just a thin wrapper
// used for a clearer call site above.
trait CredentialsTyped {
    async fn credentials_typed(&self, protocol: &str) -> NodeResult<(String, Value)>;
}
impl CredentialsTyped for ExecCtx<'_> {
    async fn credentials_typed(&self, protocol: &str) -> NodeResult<(String, Value)> {
        self.credentials(if protocol == "sftp" { "sftp" } else { "ftp" }).await
    }
}

fn cred_str(cred: &Value, key: &str) -> String {
    cred.get(key).and_then(Value::as_str).unwrap_or_default().to_string()
}

fn cred_u16(cred: &Value, key: &str, default: u16) -> u16 {
    cred.get(key).and_then(|v| v.as_u64().or_else(|| v.as_str().and_then(|s| s.parse().ok()))).map(|n| n as u16).unwrap_or(default)
}

fn sftp_auth(cred: &Value) -> SshAuth {
    let host = cred_str(cred, "host");
    let port = cred_u16(cred, "port", 22);
    let username = cred_str(cred, "username");
    let private_key = cred_str(cred, "privateKey");
    if !private_key.is_empty() {
        let passphrase = cred.get("passphrase").and_then(Value::as_str).filter(|s| !s.is_empty()).map(String::from);
        SshAuth::PrivateKey { host, port, username, private_key, passphrase }
    } else {
        SshAuth::Password { host, port, username, password: cred_str(cred, "password") }
    }
}

// ---- plans (resolved from ctx, no connection access) ---------------------

enum FtpPlan {
    List { path: String, recursive: bool },
    Delete { path: String, folder: bool, recursive: bool },
    Rename { old_path: String, new_path: String, create_dirs: bool },
    Upload { path: String, bytes: Vec<u8> },
    Download { path: String, binary_prop: String },
}

fn resolve_plan(ctx: &ExecCtx<'_>, i: usize, operation: &str, item: &Item) -> NodeResult<FtpPlan> {
    match operation {
        "list" => Ok(FtpPlan::List { path: ctx.param_str("path", i, "/")?, recursive: ctx.param_bool("recursive", i, false)? }),
        "delete" => Ok(FtpPlan::Delete {
            path: non_empty(ctx.param_str("path", i, "")?, "Path", i)?,
            folder: ctx.param_bool("options.folder", i, false)?,
            recursive: ctx.param_bool("options.recursive", i, false)?,
        }),
        "rename" => Ok(FtpPlan::Rename {
            old_path: non_empty(ctx.param_str("oldPath", i, "")?, "Old Path", i)?,
            new_path: non_empty(ctx.param_str("newPath", i, "")?, "New Path", i)?,
            create_dirs: ctx.param_bool("options.createDirectories", i, false)?,
        }),
        "upload" => {
            let path = non_empty(ctx.param_str("path", i, "")?, "Path", i)?;
            let binary_data = ctx.param_bool("binaryData", i, true)?;
            let bytes = if binary_data {
                let binary_prop = ctx.param_str("binaryPropertyName", i, "data")?;
                let entry = binary_entry(item, &binary_prop, i)?;
                binary_bytes(entry, i)?
            } else {
                ctx.param_str("fileContent", i, "")?.into_bytes()
            };
            Ok(FtpPlan::Upload { path, bytes })
        }
        "download" => Ok(FtpPlan::Download { path: non_empty(ctx.param_str("path", i, "")?, "Path", i)?, binary_prop: ctx.param_str("binaryPropertyName", i, "data")? }),
        other => Err(NodeError::new(format!("The operation \"{other}\" is not known!")).at(i)),
    }
}

fn non_empty(s: String, field: &str, i: usize) -> NodeResult<String> {
    if s.is_empty() {
        Err(NodeError::new(format!("{field} is required")).at(i))
    } else {
        Ok(s)
    }
}

// ---- outcomes -> items -----------------------------------------------------

enum FtpOutcome {
    Listed(Vec<Map<String, Value>>),
    Success,
    PassThrough,
    Downloaded { binary_prop: String, bytes: Vec<u8>, file_name: String },
}

fn apply_outcome(original: Item, outcome: FtpOutcome, idx: usize) -> Vec<Item> {
    match outcome {
        FtpOutcome::Listed(rows) => rows.into_iter().map(|json| Item::new(json).paired(idx)).collect(),
        FtpOutcome::Success => {
            let mut json = Map::new();
            json.insert("success".into(), json!(true));
            vec![Item::new(json).paired(idx)]
        }
        FtpOutcome::PassThrough => vec![original.paired(idx)],
        FtpOutcome::Downloaded { binary_prop, bytes, file_name } => {
            let mut binary = original.binary.clone().unwrap_or_default();
            binary.insert(binary_prop, make_binary(&bytes, &file_name));
            vec![Item { json: original.json, binary: Some(binary), paired_item: None }.paired(idx)]
        }
    }
}

// ---- path helpers (shared by both protocols) ------------------------------

fn join_path(parent: &str, name: &str) -> String {
    if parent.ends_with('/') {
        format!("{parent}{name}")
    } else {
        format!("{parent}/{name}")
    }
}

fn parent_dir(path: &str) -> Option<String> {
    match path.rsplit_once('/') {
        Some(("", _)) => Some("/".to_string()),
        Some((dir, _)) if !dir.is_empty() => Some(dir.to_string()),
        _ => None,
    }
}

fn system_time_to_iso(t: SystemTime) -> String {
    let dt: DateTime<Utc> = t.into();
    dt.to_rfc3339_opts(SecondsFormat::Secs, true)
}

async fn resolve_addr(host: &str, port: u16) -> NodeResult<SocketAddr> {
    if let Ok(addr) = format!("{host}:{port}").parse() {
        return Ok(addr);
    }
    tokio::net::lookup_host((host, port))
        .await
        .map_err(|e| NodeError::new(format!("getaddrinfo ENOTFOUND {host}: {e}")))?
        .next()
        .ok_or_else(|| NodeError::new(format!("getaddrinfo ENOTFOUND {host}")))
}

// ---- ftp protocol backend (suppaftp) --------------------------------------

async fn run_ftp(host: String, port: u16, user: String, pass: String, timeout: Duration, plans: Vec<(usize, FtpPlan)>) -> NodeResult<Vec<(usize, NodeResult<FtpOutcome>)>> {
    let addr = resolve_addr(&host, port).await?;
    let mut ftp = AsyncFtpStream::connect_timeout(addr, timeout).await.map_err(|e| NodeError::new(format!("connect to FTP server failed: {e}")))?;
    ftp.login(&user, &pass).await.map_err(|e| NodeError::new(format!("FTP login failed: {e}")))?;
    let _ = ftp.transfer_type(FtpFileType::Binary).await;

    let mut out = Vec::with_capacity(plans.len());
    for (idx, plan) in plans {
        out.push((idx, run_ftp_plan(&mut ftp, plan).await));
    }
    let _ = ftp.quit().await;
    Ok(out)
}

async fn run_ftp_plan(ftp: &mut AsyncFtpStream, plan: FtpPlan) -> NodeResult<FtpOutcome> {
    match plan {
        FtpPlan::List { path, recursive } => {
            let rows = if recursive { ftp_list_recursive(ftp, &path).await? } else { ftp_list_dir(ftp, &path).await? };
            Ok(FtpOutcome::Listed(rows))
        }
        FtpPlan::Delete { path, folder, recursive } => {
            if folder {
                if recursive {
                    ftp_delete_recursive(ftp, &path).await?;
                } else {
                    ftp.rmdir(&path).await.map_err(|e| NodeError::new(e.to_string()))?;
                }
            } else {
                ftp.rm(&path).await.map_err(|e| NodeError::new(e.to_string()))?;
            }
            Ok(FtpOutcome::Success)
        }
        FtpPlan::Rename { old_path, new_path, create_dirs } => {
            if create_dirs {
                if let Some(dir) = parent_dir(&new_path) {
                    ftp_mkdir_p(ftp, &dir).await;
                }
            }
            ftp.rename(&old_path, &new_path).await.map_err(|e| NodeError::new(e.to_string()))?;
            Ok(FtpOutcome::Success)
        }
        FtpPlan::Upload { path, bytes } => {
            if let Some(dir) = parent_dir(&path) {
                ftp_mkdir_p(ftp, &dir).await;
            }
            use tokio::io::AsyncWriteExt;
            let mut stream = ftp.put_with_stream(&path).await.map_err(|e| NodeError::new(e.to_string()))?;
            stream.write_all(&bytes).await.map_err(|e| NodeError::new(format!("writing to FTP stream failed: {e}")))?;
            stream.finish().await.map_err(|e| NodeError::new(e.to_string()))?;
            Ok(FtpOutcome::PassThrough)
        }
        FtpPlan::Download { path, binary_prop } => {
            use tokio::io::AsyncReadExt;
            let mut stream = ftp.retr_as_stream(&path).await.map_err(|e| NodeError::new(e.to_string()))?;
            let mut buf = Vec::new();
            stream.read_to_end(&mut buf).await.map_err(|e| NodeError::new(format!("reading from FTP stream failed: {e}")))?;
            stream.finish().await.map_err(|e| NodeError::new(e.to_string()))?;
            let file_name = path.rsplit('/').next().unwrap_or(&path).to_string();
            Ok(FtpOutcome::Downloaded { binary_prop, bytes: buf, file_name })
        }
    }
}

async fn ftp_mkdir_p(ftp: &mut AsyncFtpStream, dir: &str) {
    let mut cur = String::new();
    for part in dir.split('/').filter(|p| !p.is_empty()) {
        cur.push('/');
        cur.push_str(part);
        let _ = ftp.mkdir(&cur).await;
    }
}

fn ftp_file_json(f: &FtpListFile, parent: &str, path_override: Option<String>) -> Map<String, Value> {
    let ty = if f.is_directory() { "d" } else if f.is_symlink() { "l" } else { "-" };
    let mut m = Map::new();
    m.insert("type".into(), json!(ty));
    m.insert("name".into(), json!(f.name()));
    m.insert("size".into(), json!(f.size()));
    m.insert("modifyTime".into(), json!(system_time_to_iso(f.modified())));
    m.insert("path".into(), json!(path_override.unwrap_or_else(|| join_path(parent, f.name()))));
    m
}

async fn ftp_list_dir(ftp: &mut AsyncFtpStream, path: &str) -> NodeResult<Vec<Map<String, Value>>> {
    let lines = ftp.list(Some(path)).await.map_err(|e| NodeError::new(e.to_string()))?;
    Ok(lines.iter().filter_map(|l| FtpListFile::from_str(l).ok()).filter(|f| f.name() != "." && f.name() != "..").map(|f| ftp_file_json(&f, path, None)).collect())
}

async fn ftp_list_recursive(ftp: &mut AsyncFtpStream, root: &str) -> NodeResult<Vec<Map<String, Value>>> {
    let mut dirs = vec![root.to_string()];
    let mut out = Vec::new();
    let mut idx = 0;
    while idx < dirs.len() {
        let cur = dirs[idx].clone();
        let lines = ftp.list(Some(&cur)).await.map_err(|e| NodeError::new(e.to_string()))?;
        for line in &lines {
            let Ok(f) = FtpListFile::from_str(line) else { continue };
            if f.name() == "." || f.name() == ".." {
                continue;
            }
            let full = join_path(&cur, f.name());
            if f.is_directory() {
                dirs.push(full.clone());
            }
            out.push(ftp_file_json(&f, &cur, Some(full)));
        }
        idx += 1;
    }
    Ok(out)
}

async fn ftp_delete_recursive(ftp: &mut AsyncFtpStream, root: &str) -> NodeResult<()> {
    let mut dirs = vec![root.to_string()];
    let mut all_dirs = vec![root.to_string()];
    let mut files = Vec::new();
    let mut idx = 0;
    while idx < dirs.len() {
        let cur = dirs[idx].clone();
        let lines = ftp.list(Some(&cur)).await.map_err(|e| NodeError::new(e.to_string()))?;
        for line in &lines {
            let Ok(f) = FtpListFile::from_str(line) else { continue };
            if f.name() == "." || f.name() == ".." {
                continue;
            }
            let full = join_path(&cur, f.name());
            if f.is_directory() {
                dirs.push(full.clone());
                all_dirs.push(full);
            } else {
                files.push(full);
            }
        }
        idx += 1;
    }
    for file in &files {
        ftp.rm(file).await.map_err(|e| NodeError::new(e.to_string()))?;
    }
    for dir in all_dirs.iter().rev() {
        ftp.rmdir(dir).await.map_err(|e| NodeError::new(e.to_string()))?;
    }
    Ok(())
}

// ---- sftp protocol backend (russh + russh-sftp) ----------------------------

async fn run_sftp(auth: SshAuth, timeout: Duration, plans: Vec<(usize, FtpPlan)>) -> NodeResult<Vec<(usize, NodeResult<FtpOutcome>)>> {
    let mut session = ssh_common::connect(&auth, timeout).await?;
    let sftp = ssh_common::open_sftp(&mut session).await?;

    let mut out = Vec::with_capacity(plans.len());
    for (idx, plan) in plans {
        out.push((idx, run_sftp_plan(&sftp, plan).await));
    }
    let _ = sftp.close().await;
    let _ = session.disconnect(russh::Disconnect::ByApplication, "", "en").await;
    Ok(out)
}

async fn run_sftp_plan(sftp: &SftpSession, plan: FtpPlan) -> NodeResult<FtpOutcome> {
    match plan {
        FtpPlan::List { path, recursive } => {
            let rows = if recursive { sftp_list_recursive(sftp, &path).await? } else { sftp_list_dir(sftp, &path).await? };
            Ok(FtpOutcome::Listed(rows))
        }
        FtpPlan::Delete { path, folder, recursive } => {
            if folder {
                if recursive {
                    sftp_delete_recursive(sftp, &path).await?;
                } else {
                    sftp.remove_dir(path.clone()).await.map_err(|e| NodeError::new(e.to_string()))?;
                }
            } else {
                sftp.remove_file(path.clone()).await.map_err(|e| NodeError::new(e.to_string()))?;
            }
            Ok(FtpOutcome::Success)
        }
        FtpPlan::Rename { old_path, new_path, create_dirs } => {
            if create_dirs {
                if let Some(dir) = parent_dir(&new_path) {
                    sftp_mkdir_p(sftp, &dir).await;
                }
            }
            sftp.rename(old_path.clone(), new_path.clone()).await.map_err(|e| NodeError::new(e.to_string()))?;
            Ok(FtpOutcome::Success)
        }
        FtpPlan::Upload { path, bytes } => {
            if let Some(dir) = parent_dir(&path) {
                sftp_mkdir_p(sftp, &dir).await;
            }
            ssh_common::sftp_write(sftp, &path, &bytes).await?;
            Ok(FtpOutcome::PassThrough)
        }
        FtpPlan::Download { path, binary_prop } => {
            let bytes = sftp.read(path.clone()).await.map_err(|e| NodeError::new(e.to_string()))?;
            let file_name = path.rsplit('/').next().unwrap_or(&path).to_string();
            Ok(FtpOutcome::Downloaded { binary_prop, bytes, file_name })
        }
    }
}

async fn sftp_mkdir_p(sftp: &SftpSession, dir: &str) {
    let mut cur = String::new();
    for part in dir.split('/').filter(|p| !p.is_empty()) {
        cur.push('/');
        cur.push_str(part);
        let _ = sftp.create_dir(cur.clone()).await;
    }
}

fn sftp_type_char(ty: SftpFileType) -> &'static str {
    match ty {
        SftpFileType::Dir => "d",
        SftpFileType::Symlink => "l",
        _ => "-",
    }
}

async fn sftp_list_dir(sftp: &SftpSession, path: &str) -> NodeResult<Vec<Map<String, Value>>> {
    let entries = sftp.read_dir(path.to_string()).await.map_err(|e| NodeError::new(e.to_string()))?;
    let mut out = Vec::new();
    for entry in entries {
        let name = entry.file_name();
        if name == "." || name == ".." {
            continue;
        }
        out.push(sftp_entry_json(&entry, path, None));
    }
    Ok(out)
}

fn sftp_entry_json(entry: &russh_sftp::client::fs::DirEntry, parent: &str, path_override: Option<String>) -> Map<String, Value> {
    let meta = entry.metadata();
    let name = entry.file_name();
    let mut m = Map::new();
    m.insert("type".into(), json!(sftp_type_char(entry.file_type())));
    m.insert("name".into(), json!(name));
    m.insert("size".into(), json!(meta.len()));
    if let Ok(t) = meta.modified() {
        m.insert("modifyTime".into(), json!(system_time_to_iso(t)));
    }
    if let Ok(t) = meta.accessed() {
        m.insert("accessTime".into(), json!(system_time_to_iso(t)));
    }
    m.insert("path".into(), json!(path_override.unwrap_or_else(|| join_path(parent, &name))));
    m
}

async fn sftp_list_recursive(sftp: &SftpSession, root: &str) -> NodeResult<Vec<Map<String, Value>>> {
    let mut dirs = vec![root.to_string()];
    let mut out = Vec::new();
    let mut idx = 0;
    while idx < dirs.len() {
        let cur = dirs[idx].clone();
        let entries = sftp.read_dir(cur.clone()).await.map_err(|e| NodeError::new(e.to_string()))?;
        for entry in entries {
            let name = entry.file_name();
            if name == "." || name == ".." {
                continue;
            }
            let full = join_path(&cur, &name);
            if entry.file_type().is_dir() {
                dirs.push(full.clone());
            }
            out.push(sftp_entry_json(&entry, &cur, Some(full)));
        }
        idx += 1;
    }
    Ok(out)
}

async fn sftp_delete_recursive(sftp: &SftpSession, root: &str) -> NodeResult<()> {
    let mut dirs = vec![root.to_string()];
    let mut all_dirs = vec![root.to_string()];
    let mut files = Vec::new();
    let mut idx = 0;
    while idx < dirs.len() {
        let cur = dirs[idx].clone();
        let entries = sftp.read_dir(cur.clone()).await.map_err(|e| NodeError::new(e.to_string()))?;
        for entry in entries {
            let name = entry.file_name();
            if name == "." || name == ".." {
                continue;
            }
            let full = join_path(&cur, &name);
            if entry.file_type().is_dir() {
                dirs.push(full.clone());
                all_dirs.push(full);
            } else {
                files.push(full);
            }
        }
        idx += 1;
    }
    for file in &files {
        sftp.remove_file(file.clone()).await.map_err(|e| NodeError::new(e.to_string()))?;
    }
    for dir in all_dirs.iter().rev() {
        sftp.remove_dir(dir.clone()).await.map_err(|e| NodeError::new(e.to_string()))?;
    }
    Ok(())
}

// ---- binary helpers ---------------------------------------------------------

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

fn guess_mime_from_extension(name: &str) -> &'static str {
    match name.rsplit('.').next().unwrap_or("").to_ascii_lowercase().as_str() {
        "txt" => "text/plain",
        "json" => "application/json",
        "csv" => "text/csv",
        "html" | "htm" => "text/html",
        _ => "application/octet-stream",
    }
}

fn extension_for_mime(mime: &str) -> &str {
    match mime {
        "text/plain" => "txt",
        "application/json" => "json",
        "application/octet-stream" => "bin",
        other => other.split('/').nth(1).unwrap_or("bin"),
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
