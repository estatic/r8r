//! Google Drive node (spec §6.6), v2 (typeVersion 3), faithful to n8n's
//! `nodes/Google/Drive/v2/*`. Implements the `file` resource's copy,
//! createFromText (incl. convertToGoogleDocument), deleteFile (incl.
//! deletePermanently), download (incl. Google Workspace export ->
//! binary), move, share, update (content and/or metadata, rename) and
//! upload operations; the `fileFolder` resource's search; the `folder`
//! resource's create, deleteFolder and share; and the `drive` resource's
//! create, deleteDrive, get, list and update (shared drives). The Google
//! Drive Trigger node is out of scope. Anything else returns
//! `Google Drive "<resource>" / "<operation>" is not supported natively
//! yet`.
//!
//! Authentication (OAuth2 with 401 -> refresh -> retry + persist, and
//! service-account RS256 JWT exchange) is shared with the Google Sheets
//! and Gmail nodes via `google_auth.rs`; the default auth method is
//! `serviceAccount`, matching n8n's `googleApiRequest`'s
//! `this.getNodeParameter('authentication', 0, 'serviceAccount')` default
//! (Sheets' default too, unlike Gmail's `oAuth2`).
//!
//! File content (upload/update/createFromText) is always sent as a
//! hand-rolled `multipart/related` body (metadata JSON part + data part),
//! matching n8n's `form-data`-built multipart path. n8n also supports a
//! resumable (chunked `PUT`) upload for streamed binary data (`.id`-backed
//! binary refs, filesystem/S3 binary mode); r8r's binary data is always
//! held inline, so that path never triggers in n8n either once the input
//! is a plain base64 binary property -- multipart covers every case we
//! exercise, so the resumable path is not implemented (documented
//! simplification, not a missing feature in practice).
//!
//! Known simplifications vs real n8n (see the implementation report):
//! `download`'s output mimeType is taken from the file's Drive metadata
//! rather than the response's `Content-Type` header (n8n prefers the
//! header but falls back to the same metadata field); the downloaded
//! binary's `fileExtension`/`fileSize` are derived the way `files.rs`/
//! `openai.rs` do rather than via `mime-types` + `file-type` sniffing;
//! `fileId`/`folderId`/`driveId` "by URL" extraction uses the same regexes
//! as n8n's `GOOGLE_DRIVE_FILE_URL_REGEX`/`GOOGLE_DRIVE_FOLDER_URL_REGEX`
//! but a plain `regex` crate instead of n8n's RLC `extractValue` pipeline;
//! `fileTypesOptions`/`updateCommonOptions.fields` multi-option UI lists
//! are not reproduced as enums (any string the caller sends is passed
//! through); service-account tokens are fetched once per node execution
//! rather than once per HTTP call.

use super::check_ssrf;
use super::google_auth::{self, GoogleAuth};
use crate::n8n::node::{ExecCtx, NodeError, NodeResult, NodeType};
use crate::n8n::types::{Item, NodeOutput};
use base64::Engine as _;
use serde_json::{json, Map, Value};

pub struct GoogleDrive;

const DRIVE_SCOPES: &str = "https://www.googleapis.com/auth/drive https://www.googleapis.com/auth/drive.appdata https://www.googleapis.com/auth/drive.photos.readonly";
const RLC_DRIVE_DEFAULT: &str = "My Drive";
const RLC_FOLDER_DEFAULT: &str = "root";
const FOLDER_MIME: &str = "application/vnd.google-apps.folder";
const DOCUMENT_MIME: &str = "application/vnd.google-apps.document";

fn unsupported(resource: &str, operation: &str) -> NodeError {
    NodeError::new(format!("Google Drive \"{resource}\" / \"{operation}\" is not supported natively yet"))
}

// ---- small value / resource-locator helpers --------------------------------

fn value_to_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        Value::Bool(b) => b.to_string(),
        other => other.to_string(),
    }
}

fn truthy(v: &Value) -> bool {
    match v {
        Value::Bool(b) => *b,
        Value::String(s) => !s.is_empty(),
        Value::Null => false,
        Value::Number(n) => n.as_f64().map(|f| f != 0.0).unwrap_or(false),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

fn locator_mode(v: &Value) -> String {
    match v {
        Value::Object(o) => o.get("mode").and_then(Value::as_str).unwrap_or("id").to_string(),
        _ => "id".to_string(),
    }
}

fn locator_str(v: &Value) -> Option<String> {
    match v {
        Value::Object(o) => o.get("value").map(value_to_string),
        Value::String(s) => Some(s.clone()),
        _ => None,
    }
}

/// n8n's `GOOGLE_DRIVE_FILE_URL_REGEX` / `GOOGLE_DRIVE_FOLDER_URL_REGEX`
/// `extractValue` behavior for a resource locator in `url` mode.
fn extract_url_id(value: &str, kind: &str, i: usize, folder_url: bool) -> NodeResult<String> {
    let pattern = if folder_url { r"https://drive\.google\.com(?:/.*|)/folders/([0-9a-zA-Z\-_]+)(?:/.*|)" } else { r"https://(?:drive|docs)\.google\.com(?:/.*|)/d/([0-9a-zA-Z\-_]+)(?:/.*|)" };
    let re = regex::Regex::new(pattern).expect("valid regex");
    re.captures(value)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string())
        .ok_or_else(|| NodeError::new(format!("Not a valid Google Drive {kind} URL")).at(i))
}

/// Reads a required resource-locator parameter (`fileId`, `folderNoRootId`,
/// `sharedDriveId`, ...), erroring if it's empty.
fn resolve_rlc_required(ctx: &ExecCtx<'_>, path: &str, i: usize, kind: &str, folder_url: bool) -> NodeResult<String> {
    let v = ctx.param(path, i)?;
    let mode = locator_mode(&v);
    let value = locator_str(&v).unwrap_or_default();
    if value.is_empty() {
        return Err(NodeError::new(format!("{kind} must be selected")).at(i));
    }
    if mode == "url" {
        extract_url_id(&value, kind, i, folder_url)
    } else {
        Ok(value)
    }
}

/// Reads an optional resource-locator parameter (`folderId`, `driveId`,
/// which default to `root`/`My Drive`), falling back to `default` when
/// absent or blank.
fn resolve_rlc(ctx: &ExecCtx<'_>, path: &str, i: usize, default: &str, kind: &str, folder_url: bool) -> NodeResult<String> {
    let v = ctx.param(path, i)?;
    if v.is_null() {
        return Ok(default.to_string());
    }
    let mode = locator_mode(&v);
    let value = locator_str(&v).unwrap_or_default();
    if value.is_empty() {
        return Ok(default.to_string());
    }
    if mode == "url" {
        extract_url_id(&value, kind, i, folder_url)
    } else {
        Ok(value)
    }
}

/// n8n's `setParentFolder`.
fn set_parent_folder(folder_id: &str, drive_id: &str) -> String {
    if folder_id != RLC_FOLDER_DEFAULT {
        folder_id.to_string()
    } else if !drive_id.is_empty() && drive_id != RLC_DRIVE_DEFAULT {
        drive_id.to_string()
    } else {
        "root".to_string()
    }
}

/// n8n's `updateDriveScopes`.
fn update_drive_scopes(qs: &mut Map<String, Value>, drive_id: &str) {
    if drive_id.is_empty() {
        return;
    }
    if drive_id == RLC_DRIVE_DEFAULT {
        qs.insert("includeItemsFromAllDrives".into(), json!(false));
        qs.insert("supportsAllDrives".into(), json!(false));
        qs.insert("spaces".into(), json!("appDataFolder, drive"));
        qs.insert("corpora".into(), json!("user"));
    } else {
        qs.insert("driveId".into(), json!(drive_id));
        qs.insert("corpora".into(), json!("drive"));
    }
}

/// n8n's `prepareQueryString`.
fn prepare_query_string(fields: &[String]) -> String {
    if fields.iter().any(|f| f == "*") {
        "*".to_string()
    } else if fields.is_empty() {
        "id, name".to_string()
    } else {
        fields.join(", ")
    }
}

fn string_list(v: &Value) -> Vec<String> {
    v.as_array().into_iter().flatten().filter_map(|x| x.as_str().map(String::from)).collect()
}

/// n8n's `setFileProperties`: `options.propertiesUi.propertyValues` /
/// `options.appPropertiesUi.appPropertyValues` -> `body.properties` /
/// `body.appProperties`.
fn set_file_properties(body: &mut Map<String, Value>, options: &Value) {
    if let Some(vals) = options.pointer("/propertiesUi/propertyValues").and_then(Value::as_array) {
        let mut props = Map::new();
        for v in vals {
            if let Some(k) = v.get("key").and_then(Value::as_str) {
                props.insert(k.to_string(), v.get("value").cloned().unwrap_or(json!("")));
            }
        }
        body.insert("properties".into(), Value::Object(props));
    }
    if let Some(vals) = options.pointer("/appPropertiesUi/appPropertyValues").and_then(Value::as_array) {
        let mut props = Map::new();
        for v in vals {
            if let Some(k) = v.get("key").and_then(Value::as_str) {
                props.insert(k.to_string(), v.get("value").cloned().unwrap_or(json!("")));
            }
        }
        body.insert("appProperties".into(), Value::Object(props));
    }
}

/// n8n's `setUpdateCommonParams`.
fn set_update_common_params(qs: &mut Map<String, Value>, options: &Value) {
    for key in ["keepRevisionForever", "ocrLanguage", "useContentAsIndexableText"] {
        if let Some(v) = options.get(key) {
            if truthy(v) {
                qs.insert(key.to_string(), v.clone());
            }
        }
    }
}

fn qs_to_vec(qs: &Map<String, Value>) -> Vec<(String, String)> {
    qs.iter().filter(|(_, v)| !v.is_null()).map(|(k, v)| (k.clone(), value_to_string(v))).collect()
}

fn default_file_folder_qs() -> Map<String, Value> {
    let mut qs = Map::new();
    qs.insert("includeItemsFromAllDrives".into(), json!(true));
    qs.insert("supportsAllDrives".into(), json!(true));
    qs.insert("spaces".into(), json!("appDataFolder, drive"));
    qs.insert("corpora".into(), json!("allDrives"));
    qs
}

// ---- binary helpers (n8n's `getItemBinaryData`/`prepareBinaryData`) --------

fn binary_bytes<'a>(item: &'a Item, name: &str, i: usize) -> NodeResult<(Vec<u8>, &'a Map<String, Value>)> {
    let entry = item
        .binary
        .as_ref()
        .and_then(|b| b.get(name))
        .ok_or_else(|| NodeError::new("Attachment not found").describe(format!("The input field '{name}' doesn't contain an attachment. Please make sure you specify a field containing binary data")).at(i))?;
    let meta = entry.as_object().ok_or_else(|| NodeError::new(format!("Binary field '{name}' is malformed")).at(i))?;
    let data = meta.get("data").and_then(Value::as_str).ok_or_else(|| NodeError::new(format!("Binary field '{name}' has no data")).at(i))?;
    let bytes = base64::engine::general_purpose::STANDARD.decode(data.trim()).map_err(|e| NodeError::new(format!("Binary field '{name}' is not valid base64: {e}")).at(i))?;
    Ok((bytes, meta))
}

fn binary_entry(data: &[u8], file_name: &str, mime: &str) -> Value {
    let ext = file_name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    json!({
        "data": base64::engine::general_purpose::STANDARD.encode(data),
        "mimeType": mime,
        "fileExtension": ext,
        "fileSize": format!("{} B", data.len()),
        "fileName": file_name,
    })
}

// ---- multipart/related body (n8n's `form-data`-built upload body) ---------

fn multipart_related(parts: &[(&str, &str, Vec<u8>)]) -> (String, Vec<u8>) {
    let boundary = format!("----r8rdriveformboundary{}", uuid::Uuid::new_v4().simple());
    let mut body = Vec::new();
    for (name, ctype, data) in parts {
        body.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"\r\nContent-Type: {ctype}\r\n\r\n").as_bytes());
        body.extend_from_slice(data);
        body.extend_from_slice(b"\r\n");
    }
    body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    (boundary, body)
}

// ---- authentication & the underlying HTTP calls -----------------------------
// (shared with the Google Sheets/Gmail nodes; see `google_auth.rs`.)

async fn resolve_auth(ctx: &ExecCtx<'_>) -> NodeResult<GoogleAuth> {
    google_auth::resolve_auth(ctx, "googleDriveOAuth2Api", "Google Drive", DRIVE_SCOPES, "https://www.googleapis.com", "serviceAccount").await
}

/// Maps a Google Drive API `{error: {message, ...}}` error payload the way
/// n8n's `NodeApiError` construction does, with the same permission/
/// not-found hints the Sheets/Gmail nodes add.
fn drive_error(status: u16, body: &Value) -> NodeError {
    let message = body.pointer("/error/message").and_then(Value::as_str).map(String::from);
    let msg = message.unwrap_or_else(|| format!("Google Drive API request failed with status code {status}"));
    let mut err = NodeError::api(msg, Some(status), Some(body.to_string()));
    if status == 403 {
        err = err.describe("Please check that the account you're using has access to this file, folder or drive.");
    }
    if status == 404 {
        err = err.describe("Double-check the file, folder or drive ID -- it could not be found.");
    }
    err
}

async fn drive_request(ctx: &ExecCtx<'_>, auth: &mut GoogleAuth, method: &str, endpoint: &str, body: Option<Value>, qs: &Map<String, Value>) -> NodeResult<Value> {
    let url = format!("{}{endpoint}", auth.base);
    let query = qs_to_vec(qs);
    google_auth::api_request(ctx, auth, method, &url, body, &query, drive_error).await
}

async fn drive_request_raw(ctx: &ExecCtx<'_>, auth: &mut GoogleAuth, method: &str, endpoint: &str, body: Option<(Vec<u8>, String)>, qs: &Map<String, Value>) -> NodeResult<Vec<u8>> {
    let url = format!("{}{endpoint}", auth.base);
    let query = qs_to_vec(qs);
    google_auth::api_request_raw(ctx, auth, method, &url, body, &query, drive_error).await
}

/// n8n's `googleApiRequestAllItems`: pages through `nextPageToken` with
/// `pageSize=100` per page.
async fn request_all_items(ctx: &ExecCtx<'_>, auth: &mut GoogleAuth, property: &str, endpoint: &str, base_query: &Map<String, Value>) -> NodeResult<Vec<Value>> {
    let mut out = Vec::new();
    let mut page_token: Option<String> = None;
    loop {
        let mut q = base_query.clone();
        q.insert("pageSize".into(), json!(100));
        if let Some(t) = &page_token {
            q.insert("pageToken".into(), json!(t));
        }
        let resp = drive_request(ctx, auth, "GET", endpoint, None, &q).await?;
        if let Some(arr) = resp.get(property).and_then(Value::as_array) {
            out.extend(arr.iter().cloned());
        }
        match resp.get("nextPageToken").and_then(Value::as_str) {
            Some(t) if !t.is_empty() => page_token = Some(t.to_string()),
            _ => break,
        }
    }
    Ok(out)
}

// ---- file operations --------------------------------------------------------

async fn op_file_copy(ctx: &ExecCtx<'_>, auth: &mut GoogleAuth, i: usize) -> NodeResult<Vec<Item>> {
    let file_obj = ctx.param("fileId", i)?;
    let file_id = resolve_rlc_required(ctx, "fileId", i, "File", false)?;
    let options = ctx.param("options", i)?;
    let mut name = ctx.param_str("name", i, "")?;
    if name.is_empty() {
        let cached = file_obj.get("cachedResultName").and_then(Value::as_str).unwrap_or("").to_string();
        let original = if !cached.is_empty() {
            cached
        } else {
            let meta = drive_request(ctx, auth, "GET", &format!("/drive/v3/files/{file_id}"), None, &Map::from_iter([("fields".to_string(), json!("name")), ("supportsAllDrives".to_string(), json!(true))])).await?;
            meta.get("name").and_then(Value::as_str).unwrap_or("").to_string()
        };
        name = if original.is_empty() { String::new() } else { format!("Copy of {original}") };
    }
    let copy_requires_writer_permission = options.get("copyRequiresWriterPermission").map(truthy).unwrap_or(false);
    let qs = default_file_folder_qs();
    let mut parents = Vec::new();
    let same_folder = ctx.param_bool("sameFolder", i, true)?;
    if !same_folder {
        let drive_id = resolve_rlc(ctx, "driveId", i, RLC_DRIVE_DEFAULT, "Drive", true)?;
        let folder_id = resolve_rlc(ctx, "folderId", i, RLC_FOLDER_DEFAULT, "Folder", true)?;
        parents.push(set_parent_folder(&folder_id, &drive_id));
    }
    let mut body = Map::new();
    body.insert("copyRequiresWriterPermission".into(), json!(copy_requires_writer_permission));
    body.insert("parents".into(), json!(parents));
    if !name.is_empty() {
        body.insert("name".into(), json!(name));
    }
    if let Some(d) = options.get("description").and_then(Value::as_str).filter(|s| !s.is_empty()) {
        body.insert("description".into(), json!(d));
    }
    let resp = drive_request(ctx, auth, "POST", &format!("/drive/v3/files/{file_id}/copy"), Some(Value::Object(body)), &qs).await?;
    Ok(vec![Item::from_value(resp)])
}

async fn op_file_create_from_text(ctx: &ExecCtx<'_>, auth: &mut GoogleAuth, i: usize) -> NodeResult<Vec<Item>> {
    let name = { let n = ctx.param_str("name", i, "")?; if n.is_empty() { "Untitled".to_string() } else { n } };
    let options = ctx.param("options", i)?;
    let convert = options.get("convertToGoogleDocument").map(truthy).unwrap_or(false);
    let mime_type = if convert { DOCUMENT_MIME } else { "text/plain" };
    let drive_id = resolve_rlc(ctx, "driveId", i, RLC_DRIVE_DEFAULT, "Drive", true)?;
    let folder_id = resolve_rlc(ctx, "folderId", i, RLC_FOLDER_DEFAULT, "Folder", true)?;
    let mut metadata = Map::new();
    metadata.insert("name".into(), json!(name));
    metadata.insert("parents".into(), json!([set_parent_folder(&folder_id, &drive_id)]));
    metadata.insert("mimeType".into(), json!(mime_type));
    let mut body_parameters = metadata.clone();
    set_file_properties(&mut body_parameters, &options);
    let mut qs = default_file_folder_qs();
    set_update_common_params(&mut qs, &options);

    let response_id;
    if convert {
        let document = drive_request(ctx, auth, "POST", "/drive/v3/files", Some(Value::Object(body_parameters)), &qs).await?;
        let doc_id = document.get("id").and_then(Value::as_str).unwrap_or("").to_string();
        let content = ctx.param_str("content", i, "")?;
        let body = json!({"requests": [{"insertText": {"text": content, "endOfSegmentLocation": {"segmentId": ""}}}]});
        let docs_base = if auth.base.contains("googleapis.com") { "https://docs.googleapis.com".to_string() } else { auth.base.clone() };
        let url = format!("{docs_base}/v1/documents/{doc_id}:batchUpdate");
        let update_resp = google_auth::api_request(ctx, auth, "POST", &url, Some(body), &[], drive_error).await?;
        response_id = update_resp.get("documentId").cloned().unwrap_or(json!(doc_id));
    } else {
        let content = ctx.param_str("content", i, "")?.into_bytes();
        let metadata_json = serde_json::to_vec(&Value::Object(metadata)).unwrap_or_default();
        let (boundary, multipart_body) = multipart_related(&[("metadata", "application/json", metadata_json), ("data", mime_type, content)]);
        let upload_qs = Map::from_iter([("uploadType".to_string(), json!("multipart")), ("supportsAllDrives".to_string(), json!(true))]);
        let bytes = drive_request_raw(ctx, auth, "POST", "/upload/drive/v3/files", Some((multipart_body, format!("multipart/related; boundary={boundary}"))), &upload_qs).await?;
        let upload_data: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        let upload_id = upload_data.get("id").and_then(Value::as_str).unwrap_or("").to_string();
        qs.insert("addParents".into(), json!(set_parent_folder(&folder_id, &drive_id)));
        body_parameters.remove("parents");
        let response_data = drive_request(ctx, auth, "PATCH", &format!("/drive/v3/files/{upload_id}"), Some(Value::Object(body_parameters)), &qs).await?;
        response_id = response_data.get("id").cloned().unwrap_or(json!(upload_id));
    }
    Ok(vec![Item::from_value(json!({"id": response_id}))])
}

async fn op_file_delete(ctx: &ExecCtx<'_>, auth: &mut GoogleAuth, i: usize) -> NodeResult<Vec<Item>> {
    let file_id = resolve_rlc_required(ctx, "fileId", i, "File", false)?;
    let delete_permanently = ctx.param_bool("options.deletePermanently", i, false)?;
    let qs = Map::from_iter([("supportsAllDrives".to_string(), json!(true))]);
    if delete_permanently {
        drive_request(ctx, auth, "DELETE", &format!("/drive/v3/files/{file_id}"), None, &qs).await?;
    } else {
        drive_request(ctx, auth, "PATCH", &format!("/drive/v3/files/{file_id}"), Some(json!({"trashed": true})), &qs).await?;
    }
    Ok(vec![Item::from_value(json!({"id": file_id, "success": true}))])
}

async fn op_file_download(ctx: &ExecCtx<'_>, auth: &mut GoogleAuth, item: &Item, i: usize) -> NodeResult<Vec<Item>> {
    let file_id = resolve_rlc_required(ctx, "fileId", i, "File", false)?;
    let options = ctx.param("options", i)?;
    let meta_qs = Map::from_iter([("fields".to_string(), json!("mimeType,name")), ("supportsTeamDrives".to_string(), json!(true)), ("supportsAllDrives".to_string(), json!(true))]);
    let file = drive_request(ctx, auth, "GET", &format!("/drive/v3/files/{file_id}"), None, &meta_qs).await?;
    let file_mime = file.get("mimeType").and_then(Value::as_str).unwrap_or("").to_string();
    let file_name_meta = file.get("name").and_then(Value::as_str).unwrap_or("").to_string();

    let bytes = if file_mime.contains("vnd.google-apps") {
        let key = "options.googleFileConversion.conversion";
        let parts: Vec<&str> = file_mime.split('.').collect();
        let kind = parts.get(2).copied().unwrap_or("");
        let (param_name, default_mime) = match kind {
            "document" => ("docsToFormat", "application/vnd.openxmlformats-officedocument.wordprocessingml.document"),
            "presentation" => ("slidesToFormat", "application/vnd.openxmlformats-officedocument.presentationml.presentation"),
            "spreadsheet" => ("sheetsToFormat", "application/x-vnd.oasis.opendocument.spreadsheet"),
            _ => ("drawingsToFormat", "image/jpeg"),
        };
        let mime = ctx.param_str(&format!("{key}.{param_name}"), i, default_mime)?;
        let qs = Map::from_iter([("mimeType".to_string(), json!(mime)), ("supportsAllDrives".to_string(), json!(true))]);
        drive_request_raw(ctx, auth, "GET", &format!("/drive/v3/files/{file_id}/export"), None, &qs).await?
    } else {
        let qs = Map::from_iter([("alt".to_string(), json!("media")), ("supportsAllDrives".to_string(), json!(true))]);
        drive_request_raw(ctx, auth, "GET", &format!("/drive/v3/files/{file_id}"), None, &qs).await?
    };

    let mime_type = if file_mime.is_empty() { "application/octet-stream".to_string() } else { file_mime };
    let file_name = options.get("fileName").and_then(Value::as_str).filter(|s| !s.is_empty()).map(String::from).unwrap_or(file_name_meta);
    let prop = options.get("binaryPropertyName").and_then(Value::as_str).filter(|s| !s.is_empty()).unwrap_or("data").to_string();

    let mut new_item = item.clone();
    new_item.binary.get_or_insert_with(Map::new).insert(prop, binary_entry(&bytes, &file_name, &mime_type));
    Ok(vec![new_item])
}

async fn op_file_move(ctx: &ExecCtx<'_>, auth: &mut GoogleAuth, i: usize) -> NodeResult<Vec<Item>> {
    let file_id = resolve_rlc_required(ctx, "fileId", i, "File", false)?;
    let drive_id = resolve_rlc(ctx, "driveId", i, RLC_DRIVE_DEFAULT, "Drive", true)?;
    let folder_id = resolve_rlc(ctx, "folderId", i, RLC_FOLDER_DEFAULT, "Folder", true)?;
    let qs = default_file_folder_qs();
    let mut get_qs = qs.clone();
    get_qs.insert("fields".into(), json!("parents"));
    let current = drive_request(ctx, auth, "GET", &format!("/drive/v3/files/{file_id}"), None, &get_qs).await?;
    let parents: Vec<String> = current.get("parents").and_then(Value::as_array).into_iter().flatten().filter_map(|v| v.as_str().map(String::from)).collect();
    let mut update_qs = qs;
    update_qs.insert("addParents".into(), json!(set_parent_folder(&folder_id, &drive_id)));
    update_qs.insert("removeParents".into(), json!(parents.join(",")));
    let resp = drive_request(ctx, auth, "PATCH", &format!("/drive/v3/files/{file_id}"), None, &update_qs).await?;
    Ok(vec![Item::from_value(resp)])
}

async fn op_share(ctx: &ExecCtx<'_>, auth: &mut GoogleAuth, id: &str, i: usize) -> NodeResult<Vec<Item>> {
    let permissions = ctx.param("permissionsUi", i)?;
    let share_options = ctx.param("options", i)?;
    let mut body = Map::new();
    if let Some(vals) = permissions.pointer("/permissionsValues").and_then(Value::as_object) {
        for (k, v) in vals {
            if !value_to_string(v).is_empty() || matches!(v, Value::Bool(_)) {
                body.insert(k.clone(), v.clone());
            }
        }
    }
    let mut qs = Map::from_iter([("supportsAllDrives".to_string(), json!(true))]);
    if let Some(obj) = share_options.as_object() {
        for (k, v) in obj {
            qs.insert(k.clone(), v.clone());
        }
    }
    let resp = drive_request(ctx, auth, "POST", &format!("/drive/v3/files/{id}/permissions"), Some(Value::Object(body)), &qs).await?;
    Ok(vec![Item::from_value(resp)])
}

async fn op_file_update(ctx: &ExecCtx<'_>, auth: &mut GoogleAuth, item: &Item, i: usize) -> NodeResult<Vec<Item>> {
    let file_id = resolve_rlc_required(ctx, "fileId", i, "File", false)?;
    let change_content = ctx.param_bool("changeFileContent", i, false)?;
    let mut mime_type: Option<String> = None;
    if change_content {
        let field = ctx.param_str("inputDataFieldName", i, "data")?;
        let (bytes, meta) = binary_bytes(item, &field, i)?;
        let mime = meta.get("mimeType").and_then(Value::as_str).unwrap_or("application/octet-stream").to_string();
        mime_type = Some(mime.clone());
        let qs = Map::from_iter([("uploadType".to_string(), json!("media")), ("supportsAllDrives".to_string(), json!(true))]);
        drive_request_raw(ctx, auth, "PATCH", &format!("/upload/drive/v3/files/{file_id}"), Some((bytes, mime)), &qs).await?;
    }
    let options = ctx.param("options", i)?;
    let mut qs = Map::new();
    qs.insert("supportsAllDrives".into(), json!(true));
    set_update_common_params(&mut qs, &options);
    if let Some(fields) = options.get("fields") {
        qs.insert("fields".into(), json!(prepare_query_string(&string_list(fields))));
    }
    if options.get("trashed").map(truthy).unwrap_or(false) {
        qs.insert("trashed".into(), json!(true));
    }
    let mut body = Map::new();
    set_file_properties(&mut body, &options);
    let new_name = ctx.param_str("newUpdatedFileName", i, "")?;
    if !new_name.is_empty() {
        body.insert("name".into(), json!(new_name));
    }
    if let Some(m) = &mime_type {
        body.insert("mimeType".into(), json!(m));
    }
    let resp = drive_request(ctx, auth, "PATCH", &format!("/drive/v3/files/{file_id}"), Some(Value::Object(body)), &qs).await?;
    Ok(vec![Item::from_value(resp)])
}

async fn op_file_upload(ctx: &ExecCtx<'_>, auth: &mut GoogleAuth, item: &Item, i: usize) -> NodeResult<Vec<Item>> {
    let field = ctx.param_str("inputDataFieldName", i, "data")?;
    let (bytes, meta) = binary_bytes(item, &field, i)?;
    let original_filename = meta.get("fileName").and_then(Value::as_str).unwrap_or("").to_string();
    let mime_type = meta.get("mimeType").and_then(Value::as_str).unwrap_or("application/octet-stream").to_string();
    let name = { let n = ctx.param_str("name", i, "")?; if n.is_empty() { original_filename.clone() } else { n } };
    let drive_id = resolve_rlc(ctx, "driveId", i, RLC_DRIVE_DEFAULT, "Drive", true)?;
    let folder_id = resolve_rlc(ctx, "folderId", i, RLC_FOLDER_DEFAULT, "Folder", true)?;

    let metadata = json!({"name": name, "parents": [set_parent_folder(&folder_id, &drive_id)]});
    let metadata_json = serde_json::to_vec(&metadata).unwrap_or_default();
    let (boundary, multipart_body) = multipart_related(&[("metadata", "application/json", metadata_json), ("data", &mime_type, bytes)]);
    let upload_qs = Map::from_iter([("uploadType".to_string(), json!("multipart")), ("supportsAllDrives".to_string(), json!(true))]);
    let resp_bytes = drive_request_raw(ctx, auth, "POST", "/upload/drive/v3/files", Some((multipart_body, format!("multipart/related; boundary={boundary}"))), &upload_qs).await?;
    let upload_data: Value = serde_json::from_slice(&resp_bytes).unwrap_or(Value::Null);
    let upload_id = upload_data.get("id").and_then(Value::as_str).unwrap_or("").to_string();

    let options = ctx.param("options", i)?;
    let mut qs = default_file_folder_qs();
    qs.insert("addParents".into(), json!(set_parent_folder(&folder_id, &drive_id)));
    set_update_common_params(&mut qs, &options);
    let simplify = options.get("simplifyOutput").map(truthy).unwrap_or(true);
    if !simplify {
        qs.insert("fields".into(), json!("*"));
    }
    let mut body = Map::new();
    body.insert("mimeType".into(), json!(mime_type));
    body.insert("name".into(), json!(name));
    body.insert("originalFilename".into(), json!(original_filename));
    set_file_properties(&mut body, &options);
    let resp = drive_request(ctx, auth, "PATCH", &format!("/drive/v3/files/{upload_id}"), Some(Value::Object(body)), &qs).await?;
    Ok(vec![Item::from_value(resp)])
}

// ---- fileFolder operations --------------------------------------------------

async fn op_file_folder_search(ctx: &ExecCtx<'_>, auth: &mut GoogleAuth, i: usize) -> NodeResult<Vec<Item>> {
    let search_method = ctx.param_str("searchMethod", i, "name")?;
    let options = ctx.param("options", i)?;
    let query_string = ctx.param_str("queryString", i, "")?;
    let mut query: Vec<String> = Vec::new();
    if search_method == "name" {
        query.push(format!("name contains '{query_string}'"));
    } else {
        query.push(query_string);
    }
    let filter = ctx.param("filter", i)?;
    let mut drive_id = String::new();
    let mut folder_id = String::new();
    let mut returned_types: Vec<String> = Vec::new();
    if filter.as_object().map(|o| !o.is_empty()).unwrap_or(false) {
        if let Some(fid) = filter.get("folderId") {
            let mode = locator_mode(fid);
            let value = locator_str(fid).unwrap_or_default();
            folder_id = if mode == "url" && !value.is_empty() { extract_url_id(&value, "Folder", i, true)? } else { value };
        }
        if !folder_id.is_empty() && folder_id != RLC_FOLDER_DEFAULT {
            query.push(format!("'{folder_id}' in parents"));
        }
        if let Some(did) = filter.get("driveId") {
            let mode = locator_mode(did);
            let value = locator_str(did).unwrap_or_default();
            drive_id = if mode == "url" && !value.is_empty() { extract_url_id(&value, "Drive", i, true)? } else { value };
        }
        let what = filter.get("whatToSearch").and_then(Value::as_str).unwrap_or("all");
        if what == "folders" {
            query.push(format!("mimeType = '{FOLDER_MIME}'"));
        } else {
            if what == "files" {
                query.push(format!("mimeType != '{FOLDER_MIME}'"));
            }
            let file_types = string_list(filter.get("fileTypes").unwrap_or(&Value::Null));
            if !file_types.is_empty() && !file_types.iter().any(|t| t == "*") {
                for ft in &file_types {
                    returned_types.push(format!("mimeType = '{ft}'"));
                }
            }
        }
        if !filter.get("includeTrashed").map(truthy).unwrap_or(false) {
            query.push("trashed = false".to_string());
        }
    }
    if !returned_types.is_empty() {
        query.push(format!("({})", returned_types.join(" or ")));
    }
    let fields = string_list(options.get("fields").unwrap_or(&Value::Null));
    let query_fields = prepare_query_string(&fields);
    let mut qs = Map::new();
    qs.insert("fields".into(), json!(format!("nextPageToken, files({query_fields})")));
    qs.insert("q".into(), json!(query.into_iter().filter(|q| !q.is_empty()).collect::<Vec<_>>().join(" and ")));
    qs.insert("includeItemsFromAllDrives".into(), json!(true));
    qs.insert("supportsAllDrives".into(), json!(true));
    qs.insert("spaces".into(), json!("appDataFolder, drive"));
    qs.insert("corpora".into(), json!("allDrives"));
    update_drive_scopes(&mut qs, &drive_id);
    if drive_id.is_empty() && folder_id == RLC_FOLDER_DEFAULT {
        qs.insert("corpora".into(), json!("user"));
        qs.insert("spaces".into(), json!("drive"));
        qs.insert("includeItemsFromAllDrives".into(), json!(false));
        qs.insert("supportsAllDrives".into(), json!(false));
    }

    let return_all = ctx.param_bool("returnAll", i, false)?;
    let results: Vec<Value> = if return_all {
        request_all_items(ctx, auth, "files", "/drive/v3/files", &qs).await?
    } else {
        let limit = ctx.param_f64("limit", i, 50.0)? as i64;
        qs.insert("pageSize".into(), json!(limit));
        let resp = drive_request(ctx, auth, "GET", "/drive/v3/files", None, &qs).await?;
        resp.get("files").and_then(Value::as_array).cloned().unwrap_or_default()
    };
    Ok(results.into_iter().map(Item::from_value).collect())
}

// ---- folder operations -------------------------------------------------------

async fn op_folder_create(ctx: &ExecCtx<'_>, auth: &mut GoogleAuth, i: usize) -> NodeResult<Vec<Item>> {
    let name = { let n = ctx.param_str("name", i, "")?; if n.is_empty() { "Untitled".to_string() } else { n } };
    let drive_id = resolve_rlc(ctx, "driveId", i, RLC_DRIVE_DEFAULT, "Drive", true)?;
    let folder_id = resolve_rlc(ctx, "folderId", i, RLC_FOLDER_DEFAULT, "Folder", true)?;
    let mut body = Map::new();
    body.insert("name".into(), json!(name));
    body.insert("mimeType".into(), json!(FOLDER_MIME));
    body.insert("parents".into(), json!([set_parent_folder(&folder_id, &drive_id)]));
    let options = ctx.param("options", i)?;
    if let Some(c) = options.get("folderColorRgb").and_then(Value::as_str).filter(|s| !s.is_empty()) {
        body.insert("folderColorRgb".into(), json!(c));
    }
    let simplify = options.get("simplifyOutput").map(truthy).unwrap_or(true);
    let mut qs = default_file_folder_qs();
    if !simplify {
        qs.insert("fields".into(), json!("*"));
    }
    let resp = drive_request(ctx, auth, "POST", "/drive/v3/files", Some(Value::Object(body)), &qs).await?;
    Ok(vec![Item::from_value(resp)])
}

async fn op_folder_delete(ctx: &ExecCtx<'_>, auth: &mut GoogleAuth, i: usize) -> NodeResult<Vec<Item>> {
    let folder_id = resolve_rlc_required(ctx, "folderNoRootId", i, "Folder", true)?;
    let delete_permanently = ctx.param_bool("options.deletePermanently", i, false)?;
    let qs = Map::from_iter([("supportsAllDrives".to_string(), json!(true))]);
    if delete_permanently {
        drive_request(ctx, auth, "DELETE", &format!("/drive/v3/files/{folder_id}"), None, &qs).await?;
    } else {
        drive_request(ctx, auth, "PATCH", &format!("/drive/v3/files/{folder_id}"), Some(json!({"trashed": true})), &qs).await?;
    }
    Ok(vec![Item::from_value(json!({"fileId": folder_id, "success": true}))])
}

// ---- drive (shared drive) operations ----------------------------------------

async fn op_drive_create(ctx: &ExecCtx<'_>, auth: &mut GoogleAuth, i: usize) -> NodeResult<Vec<Item>> {
    let name = ctx.param_str("name", i, "")?;
    let options = ctx.param("options", i)?;
    let mut body = Map::new();
    body.insert("name".into(), json!(name));
    if let Some(obj) = options.as_object() {
        for (k, v) in obj {
            body.insert(k.clone(), v.clone());
        }
    }
    let qs = Map::from_iter([("requestId".to_string(), json!(uuid::Uuid::new_v4().to_string()))]);
    let resp = drive_request(ctx, auth, "POST", "/drive/v3/drives", Some(Value::Object(body)), &qs).await?;
    Ok(vec![Item::from_value(resp)])
}

async fn op_drive_delete(ctx: &ExecCtx<'_>, auth: &mut GoogleAuth, i: usize) -> NodeResult<Vec<Item>> {
    let drive_id = resolve_rlc_required(ctx, "driveId", i, "Drive", true)?;
    drive_request(ctx, auth, "DELETE", &format!("/drive/v3/drives/{drive_id}"), None, &Map::new()).await?;
    Ok(vec![Item::from_value(json!({"success": true}))])
}

async fn op_drive_get(ctx: &ExecCtx<'_>, auth: &mut GoogleAuth, i: usize) -> NodeResult<Vec<Item>> {
    let drive_id = resolve_rlc_required(ctx, "driveId", i, "Drive", true)?;
    let options = ctx.param("options", i)?;
    let mut qs = Map::new();
    if let Some(obj) = options.as_object() {
        for (k, v) in obj {
            qs.insert(k.clone(), v.clone());
        }
    }
    let resp = drive_request(ctx, auth, "GET", &format!("/drive/v3/drives/{drive_id}"), None, &qs).await?;
    Ok(vec![Item::from_value(resp)])
}

async fn op_drive_list(ctx: &ExecCtx<'_>, auth: &mut GoogleAuth, i: usize) -> NodeResult<Vec<Item>> {
    let options = ctx.param("options", i)?;
    let mut qs = Map::new();
    if let Some(obj) = options.as_object() {
        for (k, v) in obj {
            qs.insert(k.clone(), v.clone());
        }
    }
    let return_all = ctx.param_bool("returnAll", i, false)?;
    let results: Vec<Value> = if return_all {
        request_all_items(ctx, auth, "drives", "/drive/v3/drives", &qs).await?
    } else {
        let limit = ctx.param_f64("limit", i, 100.0)? as i64;
        qs.insert("pageSize".into(), json!(limit));
        let resp = drive_request(ctx, auth, "GET", "/drive/v3/drives", None, &qs).await?;
        resp.get("drives").and_then(Value::as_array).cloned().unwrap_or_default()
    };
    Ok(results.into_iter().map(Item::from_value).collect())
}

async fn op_drive_update(ctx: &ExecCtx<'_>, auth: &mut GoogleAuth, i: usize) -> NodeResult<Vec<Item>> {
    let drive_id = resolve_rlc_required(ctx, "driveId", i, "Drive", true)?;
    let options = ctx.param("options", i)?;
    let mut body = Map::new();
    if let Some(obj) = options.as_object() {
        for (k, v) in obj {
            body.insert(k.clone(), v.clone());
        }
    }
    let resp = drive_request(ctx, auth, "PATCH", &format!("/drive/v3/drives/{drive_id}"), Some(Value::Object(body)), &Map::new()).await?;
    Ok(vec![Item::from_value(resp)])
}

// ---- top-level dispatch -------------------------------------------------------

async fn run_one(ctx: &ExecCtx<'_>, auth: &mut GoogleAuth, resource: &str, operation: &str, item: &Item, i: usize) -> NodeResult<Vec<Item>> {
    match (resource, operation) {
        ("file", "copy") => op_file_copy(ctx, auth, i).await,
        ("file", "createFromText") => op_file_create_from_text(ctx, auth, i).await,
        ("file", "deleteFile") => op_file_delete(ctx, auth, i).await,
        ("file", "download") => op_file_download(ctx, auth, item, i).await,
        ("file", "move") => op_file_move(ctx, auth, i).await,
        ("file", "share") => {
            let file_id = resolve_rlc_required(ctx, "fileId", i, "File", false)?;
            op_share(ctx, auth, &file_id, i).await
        }
        ("file", "update") => op_file_update(ctx, auth, item, i).await,
        ("file", "upload") => op_file_upload(ctx, auth, item, i).await,
        ("fileFolder", "search") => op_file_folder_search(ctx, auth, i).await,
        ("folder", "create") => op_folder_create(ctx, auth, i).await,
        ("folder", "deleteFolder") => op_folder_delete(ctx, auth, i).await,
        ("folder", "share") => {
            let folder_id = resolve_rlc_required(ctx, "folderNoRootId", i, "Folder", true)?;
            op_share(ctx, auth, &folder_id, i).await
        }
        ("drive", "create") => op_drive_create(ctx, auth, i).await,
        ("drive", "deleteDrive") => op_drive_delete(ctx, auth, i).await,
        ("drive", "get") => op_drive_get(ctx, auth, i).await,
        ("drive", "list") => op_drive_list(ctx, auth, i).await,
        ("drive", "update") => op_drive_update(ctx, auth, i).await,
        ("file" | "fileFolder" | "folder" | "drive", other) => Err(unsupported(resource, other).at(i)),
        (other, _) => Err(NodeError::new(format!("Google Drive resource \"{other}\" is not supported natively yet")).at(i)),
    }
}

#[async_trait::async_trait]
impl NodeType for GoogleDrive {
    fn type_name(&self) -> &'static str {
        "n8n-nodes-base.googleDrive"
    }

    async fn execute(&self, ctx: &mut ExecCtx<'_>) -> NodeResult<NodeOutput> {
        let items = ctx.input().to_vec();
        let resource = ctx.param_str("resource", 0, "file")?;
        let operation = ctx.param_str("operation", 0, "upload")?;
        let mut auth = resolve_auth(ctx).await?;
        let mut out: Vec<Item> = Vec::new();
        for (i, item) in items.iter().enumerate() {
            match run_one(ctx, &mut auth, &resource, &operation, item, i).await {
                Ok(produced) => out.extend(produced.into_iter().map(|it| it.paired(i))),
                Err(e) if ctx.continue_on_fail() => ctx.push_error_item(&e, i),
                Err(e) => return Err(e),
            }
        }
        Ok(vec![out])
    }
}
