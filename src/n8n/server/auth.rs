//! Users and authentication (spec §6.10, §6.11): the owner account,
//! invitations, `n8n-auth` session cookies and public API keys with n8n's
//! scopes.

use super::{data, ApiError, ApiResult, N8n};
use crate::n8n::store_ext::{ApiKey, User};
use axum::extract::{FromRequestParts, Path, State};
use axum::http::request::Parts;
use axum::http::{header, HeaderMap, HeaderValue};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub const COOKIE: &str = "n8n-auth";
const SESSION_SECS: i64 = 7 * 24 * 3600;

/// Every public API scope, as an owner's key may hold them.
pub const ALL_SCOPES: &[&str] = &[
    "workflow:create", "workflow:read", "workflow:update", "workflow:delete", "workflow:list",
    "workflow:activate", "workflow:deactivate", "workflow:move", "workflowTags:update", "workflowTags:list",
    "execution:read", "execution:list", "execution:delete", "execution:retry",
    "credential:create", "credential:delete", "credential:move", "credential:list",
    "tag:create", "tag:read", "tag:update", "tag:delete", "tag:list",
    "variable:create", "variable:delete", "variable:list", "variable:update",
    "user:read", "user:list", "user:create", "user:delete", "user:changeRole",
    "project:create", "project:list", "project:update", "project:delete",
    "sourceControl:pull", "securityAudit:generate",
];

/// Scopes a global member may put on an API key.
pub const MEMBER_SCOPES: &[&str] = &[
    "workflow:create", "workflow:read", "workflow:update", "workflow:delete", "workflow:list",
    "workflow:activate", "workflow:deactivate", "workflow:move", "workflowTags:update", "workflowTags:list",
    "execution:read", "execution:list", "execution:delete", "execution:retry",
    "credential:create", "credential:delete", "credential:move", "credential:list",
    "tag:read", "tag:list", "variable:list",
];

fn allowed_scopes(user: &User) -> &'static [&'static str] {
    if user.is_admin() {
        ALL_SCOPES
    } else {
        MEMBER_SCOPES
    }
}

pub fn hash_password(password: &str) -> anyhow::Result<String> {
    Ok(bcrypt::hash(password, 10)?)
}

/// n8n's rules: 8–64 characters with a number and an uppercase letter.
pub fn validate_password(password: &str) -> Result<(), ApiError> {
    let len = password.chars().count();
    if !(8..=64).contains(&len) {
        return Err(ApiError::bad_request("Password must be 8 to 64 characters long."));
    }
    if !password.chars().any(|c| c.is_ascii_digit()) {
        return Err(ApiError::bad_request("Password must contain at least 1 number."));
    }
    if !password.chars().any(|c| c.is_uppercase()) {
        return Err(ApiError::bad_request("Password must contain at least 1 uppercase letter."));
    }
    Ok(())
}

fn valid_email(email: &str) -> bool {
    let mut parts = email.split('@');
    matches!((parts.next(), parts.next(), parts.next()), (Some(a), Some(b), None) if !a.is_empty() && b.contains('.'))
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Claims {
    id: String,
    hash: String,
    exp: i64,
}

fn user_hash(user: &User) -> String {
    use sha2::Digest;
    let digest = sha2::Sha256::digest(format!("{}:{}", user.email, user.password.as_deref().unwrap_or("")).as_bytes());
    hex::encode(digest)[..10].to_string()
}

fn secure_cookie() -> bool {
    !matches!(std::env::var("N8N_SECURE_COOKIE").ok().as_deref(), Some("false") | Some("0"))
}

pub fn session_cookie(n8n: &N8n, user: &User) -> String {
    let claims = Claims { id: user.id.clone(), hash: user_hash(user), exp: chrono::Utc::now().timestamp() + SESSION_SECS };
    let token = jsonwebtoken::encode(&jsonwebtoken::Header::default(), &claims, &jsonwebtoken::EncodingKey::from_secret(&n8n.jwt_secret)).expect("HS256 encodes");
    format!("{COOKIE}={token}; Max-Age={SESSION_SECS}; Path=/; HttpOnly; SameSite=Lax{}", if secure_cookie() { "; Secure" } else { "" })
}

fn with_cookie(mut resp: Response, cookie: &str) -> Response {
    if let Ok(v) = HeaderValue::from_str(cookie) {
        resp.headers_mut().append(header::SET_COOKIE, v);
    }
    resp
}

fn cookie_value(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .map(str::trim)
        .find_map(|c| c.strip_prefix(&format!("{name}=")).map(String::from))
}

async fn user_from_session(n8n: &N8n, headers: &HeaderMap) -> Option<User> {
    let token = cookie_value(headers, COOKIE)?;
    let claims = jsonwebtoken::decode::<Claims>(&token, &jsonwebtoken::DecodingKey::from_secret(&n8n.jwt_secret), &jsonwebtoken::Validation::default()).ok()?.claims;
    let user = n8n.store.get_user(&claims.id).await.ok()??;
    (user_hash(&user) == claims.hash && user.password.is_some()).then_some(user)
}

/// The logged-in editor user (`n8n-auth` cookie); 401 otherwise.
pub struct SessionUser(pub User);

#[axum::async_trait]
impl FromRequestParts<Arc<N8n>> for SessionUser {
    type Rejection = ApiError;
    async fn from_request_parts(parts: &mut Parts, n8n: &Arc<N8n>) -> Result<Self, ApiError> {
        user_from_session(n8n, &parts.headers).await.map(SessionUser).ok_or_else(ApiError::unauthorized)
    }
}

pub fn hash_api_key(key: &str) -> String {
    use sha2::Digest;
    hex::encode(sha2::Sha256::digest(key.as_bytes()))
}

/// The caller of the public API (`X-N8N-API-KEY`); 401 otherwise.
pub struct ApiUser {
    pub user: User,
    pub key: ApiKey,
}

impl ApiUser {
    pub fn require(&self, scope: &str) -> Result<(), ApiError> {
        if self.key.scopes.iter().any(|s| s == scope) && allowed_scopes(&self.user).contains(&scope) {
            Ok(())
        } else {
            Err(ApiError::forbidden())
        }
    }
}

#[axum::async_trait]
impl FromRequestParts<Arc<N8n>> for ApiUser {
    type Rejection = ApiError;
    async fn from_request_parts(parts: &mut Parts, n8n: &Arc<N8n>) -> Result<Self, ApiError> {
        let unauthorized = || ApiError::new(401, "'X-N8N-API-KEY' header required");
        let raw = parts.headers.get("x-n8n-api-key").and_then(|v| v.to_str().ok()).ok_or_else(unauthorized)?;
        let key = n8n.store.api_key_by_hash(&hash_api_key(raw)).await?.ok_or_else(ApiError::unauthorized)?;
        if key.expires_at.is_some_and(|e| e > 0 && e < chrono::Utc::now().timestamp()) {
            return Err(ApiError::unauthorized());
        }
        let user = n8n.store.get_user(&key.user_id).await?.ok_or_else(ApiError::unauthorized)?;
        Ok(ApiUser { user, key })
    }
}

// ---- handlers -------------------------------------------------------------

pub async fn settings(State(n8n): State<Arc<N8n>>) -> ApiResult {
    let has_owner = n8n.store.user_count().await? > 0;
    let c = &n8n.config;
    Ok(data(json!({
        "settingsMode": "public",
        "userManagement": {"showSetupOnFirstLoad": !has_owner, "authenticationMethod": "email", "quota": -1},
        "endpointWebhook": "webhook",
        "endpointWebhookTest": "webhook-test",
        "endpointWebhookWaiting": "webhook-waiting",
        "endpointForm": "form",
        "endpointFormTest": "form-test",
        "endpointFormWaiting": "form-waiting",
        "urlBaseWebhook": c.webhook_url,
        "urlBaseEditor": c.webhook_url,
        "executionMode": c.executions_mode,
        "timezone": c.timezone,
        "versionCli": env!("CARGO_PKG_VERSION"),
        "authCookie": {"secure": secure_cookie()},
        "publicApi": {"enabled": true, "latestVersion": 1, "path": "api"},
        "pushBackend": "websocket",
        "saveDataErrorExecution": std::env::var("EXECUTIONS_DATA_SAVE_ON_ERROR").unwrap_or_else(|_| "all".into()),
        "saveDataSuccessExecution": std::env::var("EXECUTIONS_DATA_SAVE_ON_SUCCESS").unwrap_or_else(|_| "all".into()),
        "saveManualExecutions": std::env::var("EXECUTIONS_DATA_SAVE_MANUAL_EXECUTIONS").map(|v| v != "false").unwrap_or(true),
        "defaultLocale": "en",
    })))
}

pub async fn owner_setup(State(n8n): State<Arc<N8n>>, Json(body): Json<Value>) -> ApiResult {
    if n8n.store.owner().await?.is_some_and(|o| o.password.is_some()) {
        return Err(ApiError::bad_request("Instance owner already setup"));
    }
    let email = body["email"].as_str().unwrap_or_default().trim().to_string();
    if !valid_email(&email) {
        return Err(ApiError::bad_request("Invalid email address"));
    }
    let password = body["password"].as_str().unwrap_or_default();
    validate_password(password)?;
    let hash = hash_password(password)?;
    let user = n8n.store.create_user(&email, body["firstName"].as_str(), body["lastName"].as_str(), Some(&hash), "global:owner").await?;
    tracing::info!(userId = %user.id, "owner account set up");
    Ok(with_cookie(data(user.to_json()), &session_cookie(&n8n, &user)))
}

fn too_many_failures(n8n: &N8n, email: &str) -> bool {
    let mut failures = n8n.login_failures.lock().unwrap();
    let list = failures.entry(email.to_string()).or_default();
    list.retain(|t| t.elapsed() < Duration::from_secs(60));
    list.len() >= 5
}

fn record_failure(n8n: &N8n, email: &str) {
    n8n.login_failures.lock().unwrap().entry(email.to_string()).or_default().push(Instant::now());
}

pub async fn login(State(n8n): State<Arc<N8n>>, Json(body): Json<Value>) -> ApiResult {
    let email = body["emailOrLdapLoginId"].as_str().or(body["email"].as_str()).unwrap_or_default().trim().to_ascii_lowercase();
    let password = body["password"].as_str().unwrap_or_default();
    if too_many_failures(&n8n, &email) {
        return Err(ApiError::new(429, "Too many requests"));
    }
    let user = n8n.store.get_user_by_email(&email).await?;
    let ok = match user.as_ref().and_then(|u| u.password.as_deref()) {
        Some(hash) => {
            let (password, hash) = (password.to_string(), hash.to_string());
            tokio::task::spawn_blocking(move || bcrypt::verify(password, &hash).unwrap_or(false)).await.unwrap_or(false)
        }
        None => false,
    };
    match (ok, user) {
        (true, Some(user)) => Ok(with_cookie(data(user.to_json()), &session_cookie(&n8n, &user))),
        _ => {
            record_failure(&n8n, &email);
            Err(ApiError::new(401, "Wrong username or password. Do you have caps lock on?"))
        }
    }
}

pub async fn current_user(SessionUser(user): SessionUser) -> ApiResult {
    Ok(data(user.to_json()))
}

pub async fn logout() -> ApiResult {
    let cookie = format!("{COOKIE}=; Path=/; Expires=Thu, 01 Jan 1970 00:00:00 GMT; HttpOnly; SameSite=Lax");
    Ok(with_cookie(data(json!({"loggedOut": true})), &cookie))
}

pub async fn create_api_key(State(n8n): State<Arc<N8n>>, SessionUser(user): SessionUser, Json(body): Json<Value>) -> ApiResult {
    let label = body["label"].as_str().unwrap_or("My API Key").to_string();
    let scopes: Vec<String> = match body["scopes"].as_array() {
        Some(a) => a.iter().filter_map(|s| s.as_str().map(String::from)).collect(),
        None => allowed_scopes(&user).iter().map(|s| s.to_string()).collect(),
    };
    let allowed = allowed_scopes(&user);
    if let Some(bad) = scopes.iter().find(|s| !allowed.contains(&s.as_str())) {
        return Err(ApiError::bad_request(format!("Invalid scope for this user: {bad}")));
    }
    let expires_at = body["expiresAt"].as_i64();
    use rand::Rng;
    let secret: String = rand::thread_rng().sample_iter(&rand::distributions::Alphanumeric).take(40).map(char::from).collect();
    let raw = format!("n8n_api_{secret}");
    let key = n8n.store.create_api_key(&user.id, &label, &hash_api_key(&raw), &scopes, expires_at).await?;
    Ok(data(json!({
        "id": key.id,
        "label": key.label,
        "rawApiKey": raw,
        "apiKey": format!("{}******{}", &raw[..8], &raw[raw.len() - 4..]),
        "scopes": key.scopes,
        "createdAt": key.created_at,
        "expiresAt": key.expires_at,
    })))
}

pub async fn list_api_keys(State(n8n): State<Arc<N8n>>, SessionUser(user): SessionUser) -> ApiResult {
    let keys = n8n.store.api_keys_of(&user.id).await?;
    Ok(data(Value::Array(
        keys.iter().map(|k| json!({"id": k.id, "label": k.label, "apiKey": "n8n_api_******", "scopes": k.scopes, "createdAt": k.created_at, "expiresAt": k.expires_at})).collect(),
    )))
}

pub async fn delete_api_key(State(n8n): State<Arc<N8n>>, SessionUser(user): SessionUser, Path(id): Path<String>) -> ApiResult {
    if n8n.store.delete_api_key(&user.id, &id).await? {
        Ok(data(json!({"success": true})))
    } else {
        Err(ApiError::not_found("API key not found"))
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
struct InviteClaims {
    #[serde(rename = "inviterId")]
    inviter_id: String,
    #[serde(rename = "inviteeId")]
    invitee_id: String,
    exp: i64,
}

/// Creates pending users and returns their invitation links.
pub async fn invite(n8n: &Arc<N8n>, inviter: &User, body: &Value) -> Result<Vec<Value>, ApiError> {
    if !inviter.is_admin() {
        return Err(ApiError::forbidden());
    }
    let list = body.as_array().ok_or_else(|| ApiError::bad_request("Expected an array of invitations"))?;
    let mut out = Vec::new();
    for inv in list {
        let email = inv["email"].as_str().unwrap_or_default().trim().to_ascii_lowercase();
        if !valid_email(&email) {
            return Err(ApiError::bad_request(format!("Invalid email address: {email}")));
        }
        let role = inv["role"].as_str().unwrap_or("global:member");
        if !["global:member", "global:admin"].contains(&role) {
            return Err(ApiError::bad_request(format!("Invalid role: {role}")));
        }
        let user = match n8n.store.get_user_by_email(&email).await? {
            Some(existing) if existing.password.is_some() => {
                out.push(json!({"user": {"id": existing.id, "email": email, "emailSent": false}, "error": "The user already exists"}));
                continue;
            }
            Some(existing) => existing,
            None => n8n.store.create_user(&email, None, None, None, role).await?,
        };
        let claims = InviteClaims { inviter_id: inviter.id.clone(), invitee_id: user.id.clone(), exp: chrono::Utc::now().timestamp() + 90 * 86400 };
        let token = jsonwebtoken::encode(&jsonwebtoken::Header::default(), &claims, &jsonwebtoken::EncodingKey::from_secret(&n8n.jwt_secret)).expect("HS256 encodes");
        let url = format!("{}signup?inviterId={}&inviteeId={}&token={token}", n8n.config.webhook_url, inviter.id, user.id);
        out.push(json!({"user": {"id": user.id, "email": email, "inviteAcceptUrl": url, "emailSent": false, "role": role}, "error": ""}));
    }
    Ok(out)
}

pub async fn create_invitations(State(n8n): State<Arc<N8n>>, SessionUser(user): SessionUser, Json(body): Json<Value>) -> ApiResult {
    Ok(data(Value::Array(invite(&n8n, &user, &body).await?)))
}

async fn accept(n8n: &Arc<N8n>, invitee_id: &str, body: &Value) -> ApiResult {
    let user = n8n.store.get_user(invitee_id).await?.ok_or_else(|| ApiError::bad_request("Invalid invitation"))?;
    if user.password.is_some() {
        return Err(ApiError::bad_request("This invite has been accepted already"));
    }
    let password = body["password"].as_str().unwrap_or_default();
    validate_password(password)?;
    let hash = hash_password(password)?;
    n8n.store
        .complete_user(&user.id, body["firstName"].as_str().unwrap_or(""), body["lastName"].as_str().unwrap_or(""), &hash)
        .await?;
    let user = n8n.store.get_user(&user.id).await?.expect("exists");
    Ok(with_cookie(data(user.to_json()), &session_cookie(n8n, &user)))
}

/// `POST /rest/invitations/accept` with the signed token from the link.
pub async fn accept_with_token(State(n8n): State<Arc<N8n>>, Json(body): Json<Value>) -> ApiResult {
    let token = body["token"].as_str().unwrap_or_default();
    let claims = jsonwebtoken::decode::<InviteClaims>(token, &jsonwebtoken::DecodingKey::from_secret(&n8n.jwt_secret), &jsonwebtoken::Validation::default())
        .map_err(|_| ApiError::bad_request("Invalid invite token"))?
        .claims;
    accept(&n8n, &claims.invitee_id, &body).await
}

/// `POST /rest/invitations/:id/accept` (older editors send `inviterId`).
pub async fn accept_by_id(State(n8n): State<Arc<N8n>>, Path(id): Path<String>, Json(body): Json<Value>) -> ApiResult {
    let inviter = body["inviterId"].as_str().unwrap_or_default();
    if n8n.store.get_user(inviter).await?.is_none_or(|u| !u.is_admin()) {
        return Err(ApiError::bad_request("Invalid invitation"));
    }
    accept(&n8n, &id, &body).await
}

pub async fn list_users_rest(State(n8n): State<Arc<N8n>>, SessionUser(_user): SessionUser) -> ApiResult {
    let users = n8n.store.list_users().await?;
    Ok(data(Value::Array(users.iter().map(User::to_json).collect())))
}

pub fn not_found_rest() -> Response {
    ApiError::not_found("Not Found").into_response()
}
