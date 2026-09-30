//! GitHub node (spec §6.6), v1/1.1, faithful to n8n's `nodes/Github/*`.
//!
//! Implements the most-used resources/operations (`pullRequest`, `workflow`
//! and the GitHub App / webhook-trigger side of the real node are out of
//! scope, per spec):
//!
//! - `file`: create, delete, edit, get (incl. `asBinaryProperty`), list
//! - `issue`: create, createComment, edit, get, lock
//! - `release`: create, delete, get, getAll, update
//! - `repository`: get, getIssues, getLicense, getProfile, getPullRequests,
//!   listPopularPaths, listReferrers
//! - `review`: create, get, getAll, update
//! - `user`: getRepositories, invite, getUserIssues (the "Get Issues" menu
//!   entry; its wire operation value is `getUserIssues`)
//! - `organization`: getRepositories
//!
//! Anything else (organization:getMembers, the whole `pullRequest` and
//! `workflow` resources) returns a clear "not supported natively yet" error.
//!
//! Faithful quirks kept from n8n's `Github.node.js`/`GenericFunctions.js`:
//! - `file`/`issue`/`release`/`repository`/`review`/`user`/`organization`
//!   operations that "overwrite" data replace the node's output items with
//!   one item per response (array responses explode into one item per
//!   element, exactly as `returnJsonArray` does elsewhere in r8r).
//!   `issue:lock` is *not* one of these operations in n8n (a genuine
//!   omission in the original node): its GitHub response is discarded and
//!   the input items pass through unchanged.
//! - `release:delete`'s output is always `{"success": true}`, not GitHub's
//!   (empty) response body.
//! - File paths are percent-encoded two different ways depending on code
//!   path: `file:create/delete/edit/get/list` encode with
//!   `encodeURIComponent` semantics (slashes become `%2F`), but the
//!   internal "fetch the file's current SHA" request (used by edit/delete)
//!   encodes with `encodeURI` semantics (slashes are left alone).
//! - `review:create`'s `additionalFields.commitId` is copied verbatim into
//!   the request body as `commitId` (GitHub's API expects `commit_id`, so
//!   this field is effectively a no-op in real n8n too).
//! - `returnAll` pagination (`githubApiRequestAllItems`) walks `page`
//!   1, 2, 3... at `per_page=100` until the response's `Link` header stops
//!   containing the substring `"next"` (a loose substring check, not a
//!   parsed `rel="next"` match, matching the original).
//!
//! Known simplifications vs real n8n: `githubAppApi` authentication is not
//! implemented (only `accessToken` and `oAuth2`, as scoped), and
//! `githubOAuth2Api` expects the credential's `oauthTokenData.access_token`
//! to already be populated (no interactive 3-legged OAuth dance).

use crate::n8n::node::{ExecCtx, NodeError, NodeResult, NodeType};
use crate::n8n::types::{Item, NodeOutput};
use base64::Engine as _;
use serde_json::{json, Map, Value};

pub struct Github;

// ---- small value helpers ---------------------------------------------------

fn value_to_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        Value::Bool(b) => b.to_string(),
        other => other.to_string(),
    }
}

/// A `resourceLocator` value's `.value`, or the value itself when it is
/// already a plain string (r8r workflows may supply either shape).
fn locator_str(v: &Value) -> Option<String> {
    match v {
        Value::Object(o) => o.get("value").map(value_to_string),
        Value::String(s) => Some(s.clone()),
        _ => None,
    }
}

fn owner_str(ctx: &ExecCtx<'_>, i: usize) -> NodeResult<String> {
    let v = ctx.param("owner", i)?;
    Ok(locator_str(&v).unwrap_or_default())
}

fn repository_str(ctx: &ExecCtx<'_>, i: usize) -> NodeResult<String> {
    let v = ctx.param("repository", i)?;
    Ok(locator_str(&v).unwrap_or_default())
}

fn unsupported(resource: &str, operation: &str, i: usize) -> NodeError {
    NodeError::new(format!("GitHub \"{resource}\" / \"{operation}\" is not supported natively yet")).at(i)
}

/// n8n's `returnJsonArray`: an array response becomes one item per element
/// (non-object elements wrapped under `data`), anything else one item.
fn to_items(value: Value, i: usize) -> Vec<Item> {
    match value {
        Value::Array(a) => a.into_iter().map(|v| Item::from_value(v).paired(i)).collect(),
        other => vec![Item::from_value(other).paired(i)],
    }
}

/// n8n's `removeTrailingSlash`.
fn remove_trailing_slash(s: &str) -> &str {
    s.strip_suffix('/').unwrap_or(s)
}

/// `encodeURIComponent` semantics: everything but unreserved chars is
/// percent-encoded, including `/`. Used for the file content endpoints.
fn encode_uri_component(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'!' | b'~' | b'*' | b'\'' | b'(' | b')' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// `encodeURI` semantics: reserved characters (incl. `/`) are left alone.
/// Used only by the internal "get file SHA" lookup.
fn encode_uri(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9'
            | b'-' | b'_' | b'.' | b'!' | b'~' | b'*' | b'\'' | b'('
            | b')' | b';' | b',' | b'/' | b'?' | b':' | b'@' | b'&' | b'=' | b'+' | b'$' | b'#' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// n8n's `isBase64`.
fn is_base64(s: &str) -> bool {
    if s.is_empty() {
        return true;
    }
    let is_b64 = |c: u8| c.is_ascii_alphanumeric() || c == b'+' || c == b'/';
    let (core, pad) = if let Some(c) = s.strip_suffix("==") {
        (c, 2)
    } else if let Some(c) = s.strip_suffix('=') {
        (c, 1)
    } else {
        (s, 0)
    };
    if core.is_empty() && pad > 0 {
        return false;
    }
    if !core.bytes().all(is_b64) {
        return false;
    }
    match pad {
        0 => core.len() % 4 == 0,
        1 => core.len() % 4 == 3,
        2 => core.len() % 4 == 2,
        _ => false,
    }
}

/// n8n's `snakeCase(...).toUpperCase()` for the review `event` field.
fn snake_upper(s: &str) -> String {
    let mut out = String::new();
    for (idx, c) in s.chars().enumerate() {
        if c.is_uppercase() && idx > 0 {
            out.push('_');
        }
        out.extend(c.to_uppercase());
    }
    out
}

fn mime_for_ext(ext: &str) -> &'static str {
    match ext.to_ascii_lowercase().as_str() {
        "txt" | "md" => "text/plain",
        "json" => "application/json",
        "csv" => "text/csv",
        "html" | "htm" => "text/html",
        "xml" => "application/xml",
        "css" => "text/css",
        "js" => "text/javascript",
        "pdf" => "application/pdf",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "zip" => "application/zip",
        "gz" | "gzip" => "application/gzip",
        "yml" | "yaml" => "application/yaml",
        _ => "application/octet-stream",
    }
}

/// n8n's binary data shape: base64 `data`, `mimeType`, `fileExtension`,
/// `fileSize` and an optional `fileName`.
fn binary_entry(data: &[u8], file_name: &str) -> Value {
    let ext = file_name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    let mime = mime_for_ext(&ext);
    json!({
        "data": base64::engine::general_purpose::STANDARD.encode(data),
        "mimeType": mime,
        "fileExtension": ext,
        "fileSize": format!("{} B", data.len()),
        "fileName": file_name,
    })
}

fn binary_bytes(item: &Item, name: &str, i: usize) -> NodeResult<Vec<u8>> {
    let entry = item.binary.as_ref().and_then(|b| b.get(name)).ok_or_else(|| NodeError::new(format!("Item has no binary field '{name}'")).at(i))?;
    let data = entry.get("data").and_then(Value::as_str).ok_or_else(|| NodeError::new(format!("Item has no binary field '{name}'")).at(i))?;
    base64::engine::general_purpose::STANDARD.decode(data.trim()).map_err(|e| NodeError::new(format!("The binary field '{name}' does not contain valid base64 data: {e}")).at(i))
}

/// A JSON object's entries as query-string pairs (skips null/empty-string
/// values, the way an unset `collection` field is simply absent in n8n).
fn obj_to_query(v: &Value) -> Vec<(String, String)> {
    v.as_object()
        .into_iter()
        .flatten()
        .filter_map(|(k, val)| match val {
            Value::Null => None,
            Value::String(s) if s.is_empty() => None,
            other => Some((k.clone(), value_to_string(other))),
        })
        .collect()
}

// ---- authentication & the underlying HTTP call -----------------------------

struct Auth {
    header: String,
    base_url: String,
}

async fn resolve_auth(ctx: &ExecCtx<'_>) -> NodeResult<Auth> {
    let method = ctx.param_str("authentication", 0, "accessToken")?;
    if method == "oAuth2" {
        let (_, cred) = ctx.credentials("githubOAuth2Api").await?;
        let token = cred
            .pointer("/oauthTokenData/access_token")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| NodeError::new("The GitHub OAuth2 credential is not connected").describe("Complete the OAuth2 authorization for this credential before using it."))?
            .to_string();
        let base_url = cred["server"].as_str().filter(|s| !s.is_empty()).unwrap_or("https://api.github.com").trim_end_matches('/').to_string();
        Ok(Auth { header: format!("Bearer {token}"), base_url })
    } else {
        let (_, cred) = ctx.credentials("githubApi").await?;
        let token = cred["accessToken"].as_str().unwrap_or("").to_string();
        if token.is_empty() {
            return Err(NodeError::new("GitHub credentials are not set").describe("Add an Access Token to the GitHub API credential."));
        }
        let base_url = cred["server"].as_str().filter(|s| !s.is_empty()).unwrap_or("https://api.github.com").trim_end_matches('/').to_string();
        Ok(Auth { header: format!("token {token}"), base_url })
    }
}

/// Descriptive messages for common HTTP status codes, mirroring n8n-workflow's
/// `NodeApiError` `STATUS_CODE_MESSAGES` table.
fn status_code_message(code: u16) -> String {
    match code {
        400 => "Bad request - please check your parameters".into(),
        401 => "Authorization failed - please check your credentials".into(),
        402 => "Payment required - perhaps check your payment details?".into(),
        403 => "Forbidden - perhaps check your credentials?".into(),
        404 => "The resource you are requesting could not be found".into(),
        405 => "Method not allowed - please check you are using the right HTTP method".into(),
        429 => "The service is receiving too many requests from you".into(),
        500 => "The service was not able to process your request".into(),
        502 => "Bad gateway - the service failed to handle your request".into(),
        503 => "Service unavailable - try again later or consider setting this node to retry automatically (in the node settings)".into(),
        504 => "Gateway timed out - perhaps try again later?".into(),
        c if (400..500).contains(&c) => "Your request is invalid or could not be processed by the service".into(),
        c if (500..600).contains(&c) => "The service failed to process your request".into(),
        c => format!("Request failed with status code {c}"),
    }
}

fn github_error(status: u16, body: &Value) -> NodeError {
    let message = status_code_message(status);
    let description = body.get("message").and_then(Value::as_str).map(String::from);
    NodeError::api(message, Some(status), description)
}

/// Performs one GitHub API request. Returns the parsed JSON body (`Value::Null`
/// for an empty body) and the response's `Link` header, if any.
async fn github_request_full(ctx: &ExecCtx<'_>, auth: &Auth, i: usize, method: &str, path: &str, body: Option<&Value>, query: &[(String, String)]) -> NodeResult<(Value, Option<String>)> {
    let url_str = format!("{}{}", auth.base_url, path);
    let mut url = reqwest::Url::parse(&url_str).map_err(|_| NodeError::new(format!("Invalid GitHub API URL: {url_str}")).at(i))?;
    super::check_ssrf(&url, ctx.config()).await.map_err(|m| NodeError::new(m).at(i))?;
    if !query.is_empty() {
        let mut pairs = url.query_pairs_mut();
        for (k, v) in query {
            pairs.append_pair(k, v);
        }
    }
    let method = reqwest::Method::from_bytes(method.as_bytes()).map_err(|_| NodeError::new(format!("Invalid HTTP method \"{method}\"")).at(i))?;
    let mut req = ctx.services.http.request(method, url).header("Authorization", &auth.header).header("User-Agent", "r8r");
    if let Some(b) = body {
        req = req.json(b);
    }
    let resp = req.send().await.map_err(|e| NodeError::api(format!("The request to GitHub failed: {e}"), None, None).at(i))?;
    let status = resp.status().as_u16();
    let link = resp.headers().get("link").and_then(|v| v.to_str().ok()).map(String::from);
    let bytes = resp.bytes().await.unwrap_or_default();
    if status >= 400 {
        let parsed: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        return Err(github_error(status, &parsed).at(i));
    }
    if bytes.is_empty() {
        return Ok((Value::Null, link));
    }
    let value: Value = serde_json::from_slice(&bytes).map_err(|e| NodeError::new(format!("GitHub returned invalid JSON: {e}")).at(i))?;
    Ok((value, link))
}

async fn github_request(ctx: &ExecCtx<'_>, auth: &Auth, i: usize, method: &str, path: &str, body: Option<&Value>, query: &[(String, String)]) -> NodeResult<Value> {
    Ok(github_request_full(ctx, auth, i, method, path, body, query).await?.0)
}

/// n8n's `githubApiRequestAllItems`: pages at `per_page=100` while the
/// response's `Link` header contains the substring `"next"`.
async fn paginate(ctx: &ExecCtx<'_>, auth: &Auth, i: usize, method: &str, path: &str, body: Option<&Value>, mut query: Vec<(String, String)>) -> NodeResult<Vec<Value>> {
    let mut out = Vec::new();
    query.retain(|(k, _)| k != "per_page" && k != "page");
    query.push(("per_page".into(), "100".into()));
    let mut page = 1u32;
    loop {
        let mut q = query.clone();
        q.push(("page".into(), page.to_string()));
        let (value, link) = github_request_full(ctx, auth, i, method, path, body, &q).await?;
        out.extend(value.as_array().cloned().unwrap_or_default());
        page += 1;
        if !link.map(|l| l.contains("next")).unwrap_or(false) {
            break;
        }
    }
    Ok(out)
}

/// Fetches the SHA of a file (used before edit/delete), n8n's `getFileSha`.
async fn get_file_sha(ctx: &ExecCtx<'_>, auth: &Auth, i: usize, owner: &str, repository: &str, file_path: &str, branch: Option<&str>) -> NodeResult<String> {
    let mut query = Vec::new();
    if let Some(b) = branch {
        if !b.is_empty() {
            query.push(("ref".into(), b.to_string()));
        }
    }
    let endpoint = format!("/repos/{owner}/{repository}/contents/{}", encode_uri(file_path));
    let resp = github_request(ctx, auth, i, "GET", &endpoint, Some(&json!({})), &query).await?;
    resp.get("sha")
        .and_then(Value::as_str)
        .map(String::from)
        .ok_or_else(|| NodeError::new("Could not get the SHA of the file.").at(i))
}

// ---- additionalParameters (author/committer/branch) ------------------------

fn apply_additional_parameters(body: &mut Map<String, Value>, additional: &Value) -> Option<String> {
    if let Some(author) = additional.get("author").filter(|v| !v.is_null()) {
        body.insert("author".into(), author.clone());
    }
    if let Some(committer) = additional.get("committer").filter(|v| !v.is_null()) {
        body.insert("committer".into(), committer.clone());
    }
    let branch = additional.pointer("/branch/branch").and_then(Value::as_str).filter(|s| !s.is_empty());
    if let Some(b) = branch {
        body.insert("branch".into(), json!(b));
    }
    branch.map(String::from)
}

// ---- per-resource dispatch --------------------------------------------------

/// Operations whose GitHub response replaces the node's output items (n8n's
/// `overwriteDataOperations`/`overwriteDataOperationsArray`, which are
/// handled identically in the real node). Anything else -- within our
/// scope, only `issue:lock` -- passes the input item through unchanged.
const RESPONSE_OPERATIONS: &[&str] = &[
    "file:create",
    "file:delete",
    "file:edit",
    "file:get",
    "file:list",
    "issue:create",
    "issue:createComment",
    "issue:edit",
    "issue:get",
    "release:create",
    "release:delete",
    "release:get",
    "release:getAll",
    "release:update",
    "repository:get",
    "repository:getIssues",
    "repository:getLicense",
    "repository:getProfile",
    "repository:getPullRequests",
    "repository:listPopularPaths",
    "repository:listReferrers",
    "review:create",
    "review:get",
    "review:getAll",
    "review:update",
    "user:invite",
    "user:getRepositories",
    "user:getUserIssues",
    "organization:getRepositories",
];

impl Github {
    #[allow(clippy::too_many_lines, clippy::too_many_arguments)]
    async fn dispatch(&self, ctx: &ExecCtx<'_>, auth: &Auth, resource: &str, operation: &str, full_operation: &str, owner: &str, repository: &str, i: usize, item: &Item) -> NodeResult<Value> {
        match (resource, operation) {
            // ---- file ------------------------------------------------------
            ("file", "create") | ("file", "edit") => {
                let file_path = remove_trailing_slash(&ctx.param_str("filePath", i, "")?).to_string();
                let additional = ctx.param("additionalParameters", i)?;
                let mut body = Map::new();
                let branch = apply_additional_parameters(&mut body, &additional);
                if operation == "edit" {
                    let sha = get_file_sha(ctx, auth, i, owner, repository, &file_path, branch.as_deref()).await?;
                    body.insert("sha".into(), json!(sha));
                }
                body.insert("message".into(), json!(ctx.param_str("commitMessage", i, "")?));
                if ctx.param_bool("binaryData", i, false)? {
                    let binary_prop = ctx.param_str("binaryPropertyName", i, "data")?;
                    let bytes = binary_bytes(item, &binary_prop, i)?;
                    body.insert("content".into(), json!(base64::engine::general_purpose::STANDARD.encode(&bytes)));
                } else {
                    let content = ctx.param_str("fileContent", i, "")?;
                    let encoded = if is_base64(&content) { content } else { base64::engine::general_purpose::STANDARD.encode(content.as_bytes()) };
                    body.insert("content".into(), json!(encoded));
                }
                let endpoint = format!("/repos/{owner}/{repository}/contents/{}", encode_uri_component(&file_path));
                github_request(ctx, auth, i, "PUT", &endpoint, Some(&Value::Object(body)), &[]).await
            }
            ("file", "delete") => {
                let additional = ctx.param("additionalParameters", i)?;
                let mut body = Map::new();
                let branch = apply_additional_parameters(&mut body, &additional);
                let file_path = remove_trailing_slash(&ctx.param_str("filePath", i, "")?).to_string();
                body.insert("message".into(), json!(ctx.param_str("commitMessage", i, "")?));
                let sha = get_file_sha(ctx, auth, i, owner, repository, &file_path, branch.as_deref()).await?;
                body.insert("sha".into(), json!(sha));
                let endpoint = format!("/repos/{owner}/{repository}/contents/{}", encode_uri_component(&file_path));
                github_request(ctx, auth, i, "DELETE", &endpoint, Some(&Value::Object(body)), &[]).await
            }
            ("file", "get") => {
                let file_path = remove_trailing_slash(&ctx.param_str("filePath", i, "")?).to_string();
                let additional = ctx.param("additionalParameters", i)?;
                let mut query = Vec::new();
                if let Some(r) = additional.get("reference").and_then(Value::as_str).filter(|s| !s.is_empty()) {
                    query.push(("ref".into(), r.to_string()));
                }
                let endpoint = format!("/repos/{owner}/{repository}/contents/{}", encode_uri_component(&file_path));
                github_request(ctx, auth, i, "GET", &endpoint, None, &query).await
            }
            ("file", "list") => {
                let file_path = remove_trailing_slash(&ctx.param_str("filePath", i, "")?).to_string();
                let endpoint = format!("/repos/{owner}/{repository}/contents/{}", encode_uri_component(&file_path));
                github_request(ctx, auth, i, "GET", &endpoint, None, &[]).await
            }
            // ---- issue -------------------------------------------------------
            ("issue", "create") => {
                let labels: Vec<String> = ctx.param("labels", i)?.as_array().into_iter().flatten().filter_map(|o| o.get("label").and_then(Value::as_str)).map(String::from).collect();
                let assignees: Vec<String> = ctx.param("assignees", i)?.as_array().into_iter().flatten().filter_map(|o| o.get("assignee").and_then(Value::as_str)).map(String::from).collect();
                let body = json!({
                    "title": ctx.param_str("title", i, "")?,
                    "body": ctx.param_str("body", i, "")?,
                    "labels": labels,
                    "assignees": assignees,
                });
                let endpoint = format!("/repos/{owner}/{repository}/issues");
                github_request(ctx, auth, i, "POST", &endpoint, Some(&body), &[]).await
            }
            ("issue", "createComment") => {
                let issue_number = ctx.param_f64("issueNumber", i, 0.0)? as i64;
                let body = json!({"body": ctx.param_str("body", i, "")?});
                let endpoint = format!("/repos/{owner}/{repository}/issues/{issue_number}/comments");
                github_request(ctx, auth, i, "POST", &endpoint, Some(&body), &[]).await
            }
            ("issue", "edit") => {
                let issue_number = ctx.param_f64("issueNumber", i, 0.0)? as i64;
                let mut body = ctx.param("editFields", i)?.as_object().cloned().unwrap_or_default();
                if let Some(labels) = body.remove("labels") {
                    let mapped: Vec<Value> = labels.as_array().into_iter().flatten().filter_map(|o| o.get("label").cloned()).collect();
                    body.insert("labels".into(), Value::Array(mapped));
                }
                if let Some(assignees) = body.remove("assignees") {
                    let mapped: Vec<Value> = assignees.as_array().into_iter().flatten().filter_map(|o| o.get("assignee").cloned()).collect();
                    body.insert("assignees".into(), Value::Array(mapped));
                }
                let endpoint = format!("/repos/{owner}/{repository}/issues/{issue_number}");
                github_request(ctx, auth, i, "PATCH", &endpoint, Some(&Value::Object(body)), &[]).await
            }
            ("issue", "get") => {
                let issue_number = ctx.param_f64("issueNumber", i, 0.0)? as i64;
                let endpoint = format!("/repos/{owner}/{repository}/issues/{issue_number}");
                github_request(ctx, auth, i, "GET", &endpoint, None, &[]).await
            }
            ("issue", "lock") => {
                let issue_number = ctx.param_f64("issueNumber", i, 0.0)? as i64;
                let lock_reason = ctx.param_str("lockReason", i, "resolved")?;
                let endpoint = format!("/repos/{owner}/{repository}/issues/{issue_number}/lock");
                github_request(ctx, auth, i, "PUT", &endpoint, Some(&json!({})), &[("lock_reason".into(), lock_reason)]).await
            }
            // ---- release -------------------------------------------------------
            ("release", "create") => {
                let mut body = ctx.param("additionalFields", i)?.as_object().cloned().unwrap_or_default();
                body.insert("tag_name".into(), json!(ctx.param_str("releaseTag", i, "")?));
                let endpoint = format!("/repos/{owner}/{repository}/releases");
                github_request(ctx, auth, i, "POST", &endpoint, Some(&Value::Object(body)), &[]).await
            }
            ("release", "delete") => {
                let release_id = ctx.param_str("release_id", i, "")?;
                let endpoint = format!("/repos/{owner}/{repository}/releases/{release_id}");
                github_request(ctx, auth, i, "DELETE", &endpoint, None, &[]).await
            }
            ("release", "get") => {
                let release_id = ctx.param_str("release_id", i, "")?;
                let endpoint = format!("/repos/{owner}/{repository}/releases/{release_id}");
                github_request(ctx, auth, i, "GET", &endpoint, None, &[]).await
            }
            ("release", "getAll") => {
                let return_all = ctx.param_bool("returnAll", 0, false)?;
                let endpoint = format!("/repos/{owner}/{repository}/releases");
                if return_all {
                    Ok(Value::Array(paginate(ctx, auth, i, "GET", &endpoint, None, vec![]).await?))
                } else {
                    let limit = ctx.param_f64("limit", 0, 50.0)? as i64;
                    github_request(ctx, auth, i, "GET", &endpoint, None, &[("per_page".into(), limit.to_string())]).await
                }
            }
            ("release", "update") => {
                let release_id = ctx.param_str("release_id", i, "")?;
                let body = ctx.param("additionalFields", i)?;
                let endpoint = format!("/repos/{owner}/{repository}/releases/{release_id}");
                github_request(ctx, auth, i, "PATCH", &endpoint, Some(&body), &[]).await
            }
            // ---- repository -------------------------------------------------------
            ("repository", "listPopularPaths") => {
                let endpoint = format!("/repos/{owner}/{repository}/traffic/popular/paths");
                github_request(ctx, auth, i, "GET", &endpoint, None, &[]).await
            }
            ("repository", "listReferrers") => {
                let endpoint = format!("/repos/{owner}/{repository}/traffic/popular/referrers");
                github_request(ctx, auth, i, "GET", &endpoint, None, &[]).await
            }
            ("repository", "get") => {
                let endpoint = format!("/repos/{owner}/{repository}");
                github_request(ctx, auth, i, "GET", &endpoint, None, &[]).await
            }
            ("repository", "getLicense") => {
                let endpoint = format!("/repos/{owner}/{repository}/license");
                github_request(ctx, auth, i, "GET", &endpoint, None, &[]).await
            }
            ("repository", "getProfile") => {
                let endpoint = format!("/repos/{owner}/{repository}/community/profile");
                github_request(ctx, auth, i, "GET", &endpoint, None, &[]).await
            }
            ("repository", "getIssues") => {
                let filters = ctx.param("getRepositoryIssuesFilters", i)?;
                let endpoint = format!("/repos/{owner}/{repository}/issues");
                let return_all = ctx.param_bool("returnAll", 0, false)?;
                let mut query = obj_to_query(&filters);
                if return_all {
                    Ok(Value::Array(paginate(ctx, auth, i, "GET", &endpoint, None, query).await?))
                } else {
                    let limit = ctx.param_f64("limit", 0, 50.0)? as i64;
                    query.push(("per_page".into(), limit.to_string()));
                    github_request(ctx, auth, i, "GET", &endpoint, None, &query).await
                }
            }
            ("repository", "getPullRequests") => {
                let filters = ctx.param("getRepositoryPullRequestsFilters", i)?;
                let endpoint = format!("/repos/{owner}/{repository}/pulls");
                let return_all = ctx.param_bool("returnAll", 0, false)?;
                let mut query = obj_to_query(&filters);
                if return_all {
                    Ok(Value::Array(paginate(ctx, auth, i, "GET", &endpoint, None, query).await?))
                } else {
                    let limit = ctx.param_f64("limit", 0, 50.0)? as i64;
                    query.push(("per_page".into(), limit.to_string()));
                    github_request(ctx, auth, i, "GET", &endpoint, None, &query).await
                }
            }
            // ---- review -------------------------------------------------------
            ("review", "get") => {
                let pr = ctx.param_f64("pullRequestNumber", i, 0.0)? as i64;
                let review_id = ctx.param_str("reviewId", i, "")?;
                let endpoint = format!("/repos/{owner}/{repository}/pulls/{pr}/reviews/{review_id}");
                github_request(ctx, auth, i, "GET", &endpoint, None, &[]).await
            }
            ("review", "getAll") => {
                let pr = ctx.param_f64("pullRequestNumber", i, 0.0)? as i64;
                let return_all = ctx.param_bool("returnAll", 0, false)?;
                let endpoint = format!("/repos/{owner}/{repository}/pulls/{pr}/reviews");
                if return_all {
                    Ok(Value::Array(paginate(ctx, auth, i, "GET", &endpoint, None, vec![]).await?))
                } else {
                    let limit = ctx.param_f64("limit", 0, 50.0)? as i64;
                    github_request(ctx, auth, i, "GET", &endpoint, None, &[("per_page".into(), limit.to_string())]).await
                }
            }
            ("review", "create") => {
                let pr = ctx.param_f64("pullRequestNumber", i, 0.0)? as i64;
                let mut body = ctx.param("additionalFields", i)?.as_object().cloned().unwrap_or_default();
                let event = snake_upper(&ctx.param_str("event", i, "approve")?);
                if event == "REQUEST_CHANGES" || event == "COMMENT" {
                    body.insert("body".into(), json!(ctx.param_str("body", i, "")?));
                }
                body.insert("event".into(), json!(event));
                let endpoint = format!("/repos/{owner}/{repository}/pulls/{pr}/reviews");
                github_request(ctx, auth, i, "POST", &endpoint, Some(&Value::Object(body)), &[]).await
            }
            ("review", "update") => {
                let pr = ctx.param_f64("pullRequestNumber", i, 0.0)? as i64;
                let review_id = ctx.param_str("reviewId", i, "")?;
                let body = json!({"body": ctx.param_str("body", i, "")?});
                let endpoint = format!("/repos/{owner}/{repository}/pulls/{pr}/reviews/{review_id}");
                github_request(ctx, auth, i, "PUT", &endpoint, Some(&body), &[]).await
            }
            // ---- user / organization -------------------------------------------------------
            ("user", "getRepositories") => {
                let endpoint = format!("/users/{owner}/repos");
                let return_all = ctx.param_bool("returnAll", 0, false)?;
                if return_all {
                    Ok(Value::Array(paginate(ctx, auth, i, "GET", &endpoint, None, vec![]).await?))
                } else {
                    let limit = ctx.param_f64("limit", 0, 50.0)? as i64;
                    github_request(ctx, auth, i, "GET", &endpoint, None, &[("per_page".into(), limit.to_string())]).await
                }
            }
            ("user", "getUserIssues") => {
                let filters = ctx.param("getUserIssuesFilters", i)?;
                let return_all = ctx.param_bool("returnAll", 0, false)?;
                let mut query = obj_to_query(&filters);
                if return_all {
                    Ok(Value::Array(paginate(ctx, auth, i, "GET", "/issues", None, query).await?))
                } else {
                    let limit = ctx.param_f64("limit", 0, 50.0)? as i64;
                    query.push(("per_page".into(), limit.to_string()));
                    github_request(ctx, auth, i, "GET", "/issues", None, &query).await
                }
            }
            ("user", "invite") => {
                let org = ctx.param_str("organization", i, "")?;
                let body = json!({"email": ctx.param_str("email", i, "")?});
                let endpoint = format!("/orgs/{org}/invitations");
                github_request(ctx, auth, i, "POST", &endpoint, Some(&body), &[]).await
            }
            ("organization", "getRepositories") => {
                let endpoint = format!("/orgs/{owner}/repos");
                let return_all = ctx.param_bool("returnAll", 0, false)?;
                if return_all {
                    Ok(Value::Array(paginate(ctx, auth, i, "GET", &endpoint, None, vec![]).await?))
                } else {
                    let limit = ctx.param_f64("limit", 0, 50.0)? as i64;
                    github_request(ctx, auth, i, "GET", &endpoint, None, &[("per_page".into(), limit.to_string())]).await
                }
            }
            (r, o) => {
                let _ = full_operation;
                Err(unsupported(r, o, i))
            }
        }
    }

    async fn run_item(&self, ctx: &ExecCtx<'_>, auth: &Auth, resource: &str, operation: &str, i: usize, item: &Item) -> NodeResult<Vec<Item>> {
        let full_operation = format!("{resource}:{operation}");
        let owner = if full_operation != "user:invite" && full_operation != "user:getUserIssues" { owner_str(ctx, i)? } else { String::new() };
        let repository = if !matches!(full_operation.as_str(), "user:getRepositories" | "user:getUserIssues" | "user:invite" | "organization:getRepositories") {
            repository_str(ctx, i)?
        } else {
            String::new()
        };

        // file:get with the binary-property flag is handled entirely outside
        // the generic response-item conversion (it merges into the input
        // item's own binary data instead of replacing json), matching n8n.
        if full_operation == "file:get" && ctx.param_bool("asBinaryProperty", i, true)? {
            let response = self.dispatch(ctx, auth, resource, operation, &full_operation, &owner, &repository, i, item).await?;
            if let Some(arr) = response.as_array() {
                if arr.len() > 1 {
                    return Err(NodeError::new("File Path is a folder, not a file.").at(i));
                }
            }
            let binary_prop = ctx.param_str("binaryPropertyName", i, "data")?;
            let content = response.get("content").and_then(Value::as_str).ok_or_else(|| NodeError::new("GitHub did not return file content").at(i))?;
            let path = response.get("path").and_then(Value::as_str).unwrap_or("file");
            let cleaned: String = content.chars().filter(|c| !c.is_whitespace()).collect();
            let bytes = base64::engine::general_purpose::STANDARD.decode(&cleaned).map_err(|e| NodeError::new(format!("GitHub returned invalid base64 file content: {e}")).at(i))?;
            let file_name = path.rsplit('/').next().unwrap_or(path);
            let mut binary = item.binary.clone().unwrap_or_default();
            binary.insert(binary_prop, binary_entry(&bytes, file_name));
            return Ok(vec![Item { json: item.json.clone(), binary: Some(binary), paired_item: Some(json!({"item": i})) }]);
        }

        let mut value = self.dispatch(ctx, auth, resource, operation, &full_operation, &owner, &repository, i, item).await?;
        if full_operation == "release:delete" {
            value = json!({"success": true});
        }
        if RESPONSE_OPERATIONS.contains(&full_operation.as_str()) {
            Ok(to_items(value, i))
        } else {
            // issue:lock (in scope) and any other operation n8n doesn't
            // overwrite data for: the input item passes through unchanged.
            Ok(vec![item.clone()])
        }
    }
}

#[async_trait::async_trait]
impl NodeType for Github {
    fn type_name(&self) -> &'static str {
        "n8n-nodes-base.github"
    }

    async fn execute(&self, ctx: &mut ExecCtx<'_>) -> NodeResult<NodeOutput> {
        let input = if ctx.input().is_empty() { vec![Item::default()] } else { ctx.input().to_vec() };
        let resource = ctx.param_str("resource", 0, "issue")?;
        let operation = ctx.param_str("operation", 0, "create")?;
        let auth = resolve_auth(ctx).await?;
        let mut out = Vec::new();
        for (i, item) in input.iter().enumerate() {
            match self.run_item(ctx, &auth, &resource, &operation, i, item).await {
                Ok(items) => out.extend(items),
                Err(e) if ctx.continue_on_fail() => {
                    let full_operation = format!("{resource}:{operation}");
                    if RESPONSE_OPERATIONS.contains(&full_operation.as_str()) {
                        // n8n: `returnData.push({ json: { error: error.message } })`.
                        ctx.push_error_item(&e, i);
                    } else {
                        // n8n: `items[i].json = { error: error.message }` -- the
                        // passing-through item's `.json` is replaced wholesale
                        // (not merged), but its binary data is untouched.
                        let mut json = Map::new();
                        json.insert("error".into(), json!(e.message));
                        ctx.error_items.push(Item { json, binary: item.binary.clone(), paired_item: Some(json!({"item": i})) });
                    }
                }
                Err(e) => return Err(e),
            }
        }
        Ok(vec![out])
    }
}
