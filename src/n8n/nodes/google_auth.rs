//! Shared Google OAuth2 + service-account (RS256 JWT) authentication,
//! extracted from the Google Sheets node (spec §6.6) so the Gmail node can
//! reuse it. Handles:
//! - OAuth2: reads the already-obtained access token from the credential's
//!   `oauthTokenData`, and on a 401 refreshes it with the stored refresh
//!   token (persisting the new token data back to the credential store).
//! - Service Account: signs and exchanges an RS256 JWT for an access
//!   token (optionally impersonating `delegatedEmail` via the JWT `sub`
//!   claim), the way n8n's `getGoogleAccessToken` does.
//!
//! Each caller supplies its own OAuth2 credential type name, scopes,
//! default API base URL and default authentication method, since Sheets
//! and Gmail differ on all four; the credential's `url` field (present on
//! both `googleSheetsOAuth2Api`/`gmailOAuth2` and `googleApi`) overrides
//! the base URL so wiremock can stand in for the real Google host in
//! tests, mirroring Slack's/Notion's `url` field.

use super::check_ssrf;
use crate::n8n::node::{ExecCtx, NodeError, NodeResult};
use serde_json::{json, Value};

pub enum AuthKind {
    ServiceAccount,
    OAuth2 { cred_id: String, cred_data: Value },
}

pub struct GoogleAuth {
    pub bearer: String,
    pub base: String,
    pub kind: AuthKind,
}

/// n8n's `formatPemBlock`, plus the `\n`-literal unescape n8n's
/// `getGoogleAccessToken`/`GoogleApi.credentials` both apply first.
fn format_pem_block(key: &str) -> String {
    let unescaped = key.replace("\\n", "\n");
    let trimmed = unescaped.trim();
    if trimmed.contains("-----BEGIN") {
        return trimmed.to_string();
    }
    let body: String = trimmed.chars().filter(|c| !c.is_whitespace()).collect();
    let mut lines = vec!["-----BEGIN PRIVATE KEY-----".to_string()];
    for chunk in body.as_bytes().chunks(64) {
        lines.push(String::from_utf8_lossy(chunk).into_owned());
    }
    lines.push("-----END PRIVATE KEY-----".to_string());
    lines.join("\n")
}

/// Exchanges a signed RS256 JWT for an access token using the `googleApi`
/// (Google Service Account) credential, matching n8n's
/// `getGoogleAccessToken`.
pub async fn service_account_token(ctx: &ExecCtx<'_>, scopes: &str, default_base: &str) -> NodeResult<GoogleAuth> {
    let (_, cred) = ctx.credentials("googleApi").await?;
    let email = cred["email"].as_str().unwrap_or("").trim().to_string();
    let private_key_raw = cred["privateKey"].as_str().unwrap_or("").to_string();
    if email.is_empty() || private_key_raw.is_empty() {
        return Err(NodeError::new("Google Service Account credentials are not set")
            .describe("Add a Service Account Email and Private Key to the Google Service Account API credential."));
    }
    let delegated = cred["delegatedEmail"].as_str().filter(|s| !s.is_empty()).unwrap_or(&email).to_string();
    let token_url = cred["tokenUrl"].as_str().filter(|s| !s.is_empty()).unwrap_or("https://oauth2.googleapis.com/token").to_string();
    let base = cred["url"].as_str().filter(|s| !s.is_empty()).unwrap_or(default_base).trim_end_matches('/').to_string();

    let pem = format_pem_block(&private_key_raw);
    let key = jsonwebtoken::EncodingKey::from_rsa_pem(pem.as_bytes()).map_err(|e| NodeError::new(format!("The private key could not be parsed: {e}")))?;
    let now = chrono::Utc::now().timestamp();
    let claims = json!({"iss": email, "sub": delegated, "scope": scopes, "aud": token_url, "iat": now, "exp": now + 3600});
    let header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::RS256);
    let assertion = jsonwebtoken::encode(&header, &claims, &key).map_err(|e| NodeError::new(format!("Could not sign the service-account JWT: {e}")))?;

    let url = reqwest::Url::parse(&token_url).map_err(|_| NodeError::new(format!("Invalid token URL: {token_url}")))?;
    check_ssrf(&url, ctx.config()).await.map_err(NodeError::new)?;
    let form = [("grant_type", "urn:ietf:params:oauth:grant-type:jwt-bearer"), ("assertion", assertion.as_str())];
    let resp = ctx.services.http.post(url).form(&form).send().await.map_err(|e| NodeError::new(format!("Could not get a Google OAuth2 access token: {e}")))?;
    let status = resp.status().as_u16();
    let body: Value = resp.json().await.unwrap_or(Value::Null);
    let token = body["access_token"]
        .as_str()
        .filter(|_| status < 400)
        .ok_or_else(|| NodeError::api("Could not get a Google OAuth2 access token", Some(status), Some(body.to_string())))?
        .to_string();
    Ok(GoogleAuth { bearer: token, base, kind: AuthKind::ServiceAccount })
}

/// Reads the already-obtained OAuth2 access token from `cred_type`'s
/// `oauthTokenData.access_token`.
pub async fn oauth2_token(ctx: &ExecCtx<'_>, cred_type: &str, display_name: &str, default_base: &str) -> NodeResult<GoogleAuth> {
    let (cred_id, cred) = ctx.credentials(cred_type).await?;
    let token = cred
        .pointer("/oauthTokenData/access_token")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| NodeError::new(format!("The {display_name} OAuth2 credential is not connected")).describe("Complete the OAuth2 authorization for this credential before using it."))?
        .to_string();
    let base = cred["url"].as_str().filter(|s| !s.is_empty()).unwrap_or(default_base).trim_end_matches('/').to_string();
    Ok(GoogleAuth { bearer: token, base, kind: AuthKind::OAuth2 { cred_id, cred_data: cred } })
}

/// Picks service-account or OAuth2 based on the node's `authentication`
/// parameter.
pub async fn resolve_auth(ctx: &ExecCtx<'_>, cred_type: &str, display_name: &str, scopes: &str, default_base: &str, default_method: &str) -> NodeResult<GoogleAuth> {
    let method = ctx.param_str("authentication", 0, default_method)?;
    if method == "serviceAccount" {
        service_account_token(ctx, scopes, default_base).await
    } else {
        oauth2_token(ctx, cred_type, display_name, default_base).await
    }
}

/// Refreshes an OAuth2 `GoogleAuth`'s access token with its stored refresh
/// token, persisting the new token data back to the credential store (a
/// no-op for service-account auth).
pub async fn refresh_oauth2(ctx: &ExecCtx<'_>, auth: &mut GoogleAuth) -> NodeResult<()> {
    let AuthKind::OAuth2 { cred_id, cred_data } = &mut auth.kind else { return Ok(()) };
    let refresh_token = cred_data
        .pointer("/oauthTokenData/refresh_token")
        .and_then(Value::as_str)
        .map(String::from)
        .ok_or_else(|| NodeError::new("The Google OAuth2 credential has no refresh token"))?;
    let token_url = cred_data["accessTokenUrl"].as_str().filter(|s| !s.is_empty()).unwrap_or("https://oauth2.googleapis.com/token").to_string();
    let client_id = cred_data["clientId"].as_str().unwrap_or("").to_string();
    let client_secret = cred_data["clientSecret"].as_str().unwrap_or("").to_string();
    let url = reqwest::Url::parse(&token_url).map_err(|_| NodeError::new(format!("Invalid access token URL: {token_url}")))?;
    check_ssrf(&url, ctx.config()).await.map_err(NodeError::new)?;
    let form = [("grant_type", "refresh_token"), ("refresh_token", refresh_token.as_str()), ("client_id", client_id.as_str()), ("client_secret", client_secret.as_str())];
    let resp = ctx.services.http.post(url).form(&form).send().await.map_err(|e| NodeError::new(format!("Could not refresh the Google OAuth2 access token: {e}")))?;
    let status = resp.status().as_u16();
    let body: Value = resp.json().await.unwrap_or(Value::Null);
    let token = body["access_token"]
        .as_str()
        .filter(|_| status < 400)
        .ok_or_else(|| NodeError::api("Could not refresh the Google OAuth2 access token", Some(status), Some(body.to_string())))?
        .to_string();
    auth.bearer = token;
    // Merged, not replaced, like n8n: Google's refresh response carries no
    // `refresh_token`, and dropping it would break the next refresh.
    merge_token_data(cred_data, body);
    if let Some(store) = &ctx.services.store {
        let _ = store.update_credential_data(cred_id, cred_data).await;
    }
    Ok(())
}

/// Issues a Google API request with bearer auth, retrying once after a 401
/// by refreshing the OAuth2 token (a no-op retry point for service-account
/// auth, which never gets refreshed). `map_err` turns a `>=400` response
/// into a `NodeError`, letting each caller keep its own error-message
/// conventions (e.g. Sheets' 403 permissions hint vs Gmail's 404/409
/// hints).
pub async fn api_request(
    ctx: &ExecCtx<'_>,
    auth: &mut GoogleAuth,
    method: &str,
    url_str: &str,
    body: Option<Value>,
    query: &[(String, String)],
    map_err: impl Fn(u16, &Value) -> NodeError,
) -> NodeResult<Value> {
    let mut refreshed = false;
    loop {
        let mut url = reqwest::Url::parse(url_str).map_err(|_| NodeError::new(format!("Invalid Google API URL: {url_str}")))?;
        check_ssrf(&url, ctx.config()).await.map_err(NodeError::new)?;
        if !query.is_empty() {
            let mut pairs = url.query_pairs_mut();
            for (k, v) in query {
                pairs.append_pair(k, v);
            }
        }
        let m = reqwest::Method::from_bytes(method.as_bytes()).map_err(|_| NodeError::new(format!("Invalid HTTP method \"{method}\"")))?;
        let mut req = ctx.services.http.request(m, url).bearer_auth(&auth.bearer);
        if let Some(b) = &body {
            req = req.json(b);
        }
        let resp = req.send().await.map_err(|e| NodeError::api(format!("The request to Google failed: {e}"), None, None))?;
        let status = resp.status().as_u16();
        if status == 401 && !refreshed && matches!(auth.kind, AuthKind::OAuth2 { .. }) {
            refreshed = true;
            refresh_oauth2(ctx, auth).await?;
            continue;
        }
        let text = resp.text().await.unwrap_or_default();
        let value: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
        if status >= 400 {
            return Err(map_err(status, &value));
        }
        return Ok(value);
    }
}

/// Like `api_request`, but for raw-byte bodies/responses (multipart file
/// uploads, binary file downloads), which don't fit `api_request`'s
/// JSON-only `req.json(b)` / JSON-parsed-response shape. Same bearer auth +
/// 401-refresh-retry behavior; `map_err` only fires on `>=400` (the
/// response body is parsed as JSON best-effort for the error payload, same
/// as `api_request`).
pub async fn api_request_raw(
    ctx: &ExecCtx<'_>,
    auth: &mut GoogleAuth,
    method: &str,
    url_str: &str,
    body: Option<(Vec<u8>, String)>,
    query: &[(String, String)],
    map_err: impl Fn(u16, &Value) -> NodeError,
) -> NodeResult<Vec<u8>> {
    let mut refreshed = false;
    loop {
        let mut url = reqwest::Url::parse(url_str).map_err(|_| NodeError::new(format!("Invalid Google API URL: {url_str}")))?;
        check_ssrf(&url, ctx.config()).await.map_err(NodeError::new)?;
        if !query.is_empty() {
            let mut pairs = url.query_pairs_mut();
            for (k, v) in query {
                pairs.append_pair(k, v);
            }
        }
        let m = reqwest::Method::from_bytes(method.as_bytes()).map_err(|_| NodeError::new(format!("Invalid HTTP method \"{method}\"")))?;
        let mut req = ctx.services.http.request(m, url).bearer_auth(&auth.bearer);
        if let Some((bytes, ctype)) = &body {
            req = req.header(reqwest::header::CONTENT_TYPE, ctype.as_str()).body(bytes.clone());
        }
        let resp = req.send().await.map_err(|e| NodeError::api(format!("The request to Google failed: {e}"), None, None))?;
        let status = resp.status().as_u16();
        if status == 401 && !refreshed && matches!(auth.kind, AuthKind::OAuth2 { .. }) {
            refreshed = true;
            refresh_oauth2(ctx, auth).await?;
            continue;
        }
        let bytes = resp.bytes().await.unwrap_or_default().to_vec();
        if status >= 400 {
            let value: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
            return Err(map_err(status, &value));
        }
        return Ok(bytes);
    }
}

/// Merges a token endpoint response into `oauthTokenData`, keeping fields the
/// response omits (notably `refresh_token`).
pub(crate) fn merge_token_data(cred_data: &mut Value, response: Value) {
    if !cred_data["oauthTokenData"].is_object() {
        cred_data["oauthTokenData"] = Value::Object(Default::default());
    }
    if let (Some(existing), Value::Object(fresh)) = (cred_data["oauthTokenData"].as_object_mut(), response) {
        existing.extend(fresh);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_refresh_response_without_refresh_token_keeps_the_stored_one() {
        let mut cred = json!({"oauthTokenData": {"access_token": "old", "refresh_token": "r-1", "scope": "s"}});
        merge_token_data(&mut cred, json!({"access_token": "new", "expires_in": 3599}));
        assert_eq!(cred["oauthTokenData"], json!({"access_token": "new", "refresh_token": "r-1", "scope": "s", "expires_in": 3599}));
    }

    #[test]
    fn a_rotated_refresh_token_replaces_the_stored_one() {
        let mut cred = json!({"oauthTokenData": {"access_token": "old", "refresh_token": "r-1"}});
        merge_token_data(&mut cred, json!({"access_token": "new", "refresh_token": "r-2"}));
        assert_eq!(cred["oauthTokenData"]["refresh_token"], "r-2");
    }

    #[test]
    fn token_data_is_created_when_missing() {
        let mut cred = json!({});
        merge_token_data(&mut cred, json!({"access_token": "a"}));
        assert_eq!(cred["oauthTokenData"], json!({"access_token": "a"}));
    }
}
