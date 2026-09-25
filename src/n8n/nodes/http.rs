//! HTTP Request v4 and the outbound SSRF guard (spec §5.3).

use crate::n8n::config::Config;
use crate::n8n::node::{ExecCtx, NodeError, NodeResult, NodeType};
use crate::n8n::types::{Item, NodeOutput};
use serde_json::{json, Map, Value};
use std::net::IpAddr;

pub struct HttpRequest;

fn is_internal(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_loopback() || v4.is_private() || v4.is_link_local() || v4.is_unspecified() || v4.is_broadcast() || v4.octets()[0] == 0
                || (v4.octets()[0] == 100 && (v4.octets()[1] & 0xc0) == 64) // 100.64/10 carrier-grade NAT
        }
        IpAddr::V6(v6) => {
            v6.is_loopback() || v6.is_unspecified() || (v6.segments()[0] & 0xfe00) == 0xfc00 || (v6.segments()[0] & 0xffc0) == 0xfe80
                || v6.to_ipv4_mapped().is_some_and(|v4| is_internal(IpAddr::V4(v4)))
        }
    }
}

/// Refuses URLs whose host is, or resolves to, a loopback, private,
/// link-local or otherwise internal address, unless the host is listed in
/// `R8R_SSRF_ALLOWED_HOSTS`.
pub async fn check_ssrf(url: &reqwest::Url, config: &Config) -> Result<(), String> {
    let Some(host) = url.host_str() else { return Err(format!("Request to {url} blocked: the URL has no host")) };
    let bare = host.trim_start_matches('[').trim_end_matches(']');
    if config.ssrf_allowed_hosts.iter().any(|h| h.eq_ignore_ascii_case(bare)) {
        return Ok(());
    }
    let addrs: Vec<IpAddr> = match bare.parse::<IpAddr>() {
        Ok(ip) => vec![ip],
        Err(_) => {
            let port = url.port_or_known_default().unwrap_or(80);
            tokio::net::lookup_host((bare, port)).await.map(|it| it.map(|a| a.ip()).collect()).unwrap_or_default()
        }
    };
    if let Some(ip) = addrs.iter().find(|ip| is_internal(**ip)) {
        return Err(format!(
            "Request to {bare} blocked: it resolves to the internal address {ip}. Add the host to R8R_SSRF_ALLOWED_HOSTS to allow it."
        ));
    }
    Ok(())
}

fn status_message(code: u16) -> String {
    match code {
        400 => "Bad request - please check your parameters".into(),
        401 => "Authorization failed - please check your credentials".into(),
        403 => "Forbidden - perhaps check your credentials?".into(),
        404 => "The resource you are requesting could not be found".into(),
        405 => "Method not allowed - please check you are using the right HTTP method".into(),
        429 => "The service is receiving too many requests from you".into(),
        500 => "The service was not able to process your request".into(),
        502 => "Bad gateway - the service failed to handle your request".into(),
        503 => "Service unavailable - try again later or consider setting this node to retry automatically (in the node settings)".into(),
        504 => "Gateway timed out - perhaps try again later?".into(),
        _ => format!("Request failed with status code {code}"),
    }
}

fn pairs_to_object(v: &Value) -> Map<String, Value> {
    let mut out = Map::new();
    for p in v.as_array().into_iter().flatten() {
        if let Some(name) = p["name"].as_str() {
            out.insert(name.to_string(), p.get("value").cloned().unwrap_or(Value::Null));
        }
    }
    out
}

fn value_to_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

fn json_param(v: Value, what: &str) -> NodeResult<Value> {
    match v {
        Value::String(s) if s.trim().is_empty() => Ok(json!({})),
        Value::String(s) => serde_json::from_str(&s).map_err(|e| NodeError::new(format!("{what} must be valid JSON")).describe(e.to_string())),
        other => Ok(other),
    }
}

struct Response {
    status: u16,
    reason: String,
    headers: Map<String, Value>,
    body: Value,
}

struct Auth {
    kind: String,
    cred_id: String,
    data: Value,
}

impl HttpRequest {
    async fn auth(&self, ctx: &ExecCtx<'_>) -> NodeResult<Option<Auth>> {
        let method = ctx.raw_param("authentication").and_then(Value::as_str).unwrap_or("none");
        let kind = match method {
            "genericCredentialType" => ctx.raw_param("genericAuthType").and_then(Value::as_str).unwrap_or("").to_string(),
            "predefinedCredentialType" => ctx.raw_param("nodeCredentialType").and_then(Value::as_str).unwrap_or("").to_string(),
            _ => return Ok(None),
        };
        let (cred_id, data) = ctx.credentials(&kind).await?;
        Ok(Some(Auth { kind, cred_id, data }))
    }

    async fn fetch_token(&self, ctx: &ExecCtx<'_>, auth: &mut Auth) -> NodeResult<String> {
        let d = &auth.data;
        let token_url = d["accessTokenUrl"].as_str().unwrap_or_default().to_string();
        let url = reqwest::Url::parse(&token_url).map_err(|_| NodeError::new(format!("Invalid access token URL: {token_url}")))?;
        check_ssrf(&url, ctx.config()).await.map_err(NodeError::new)?;
        let mut form: Vec<(String, String)> = Vec::new();
        let refresh = d.pointer("/oauthTokenData/refresh_token").and_then(Value::as_str).map(String::from);
        let grant = d["grantType"].as_str().unwrap_or("authorizationCode");
        if grant == "clientCredentials" || refresh.is_none() {
            form.push(("grant_type".into(), "client_credentials".into()));
            if let Some(scope) = d["scope"].as_str().filter(|s| !s.is_empty()) {
                form.push(("scope".into(), scope.into()));
            }
        } else {
            form.push(("grant_type".into(), "refresh_token".into()));
            form.push(("refresh_token".into(), refresh.unwrap()));
        }
        let client_id = d["clientId"].as_str().unwrap_or_default();
        let client_secret = d["clientSecret"].as_str().unwrap_or_default();
        let mut req = ctx.services.http.post(url);
        if d["authentication"].as_str() == Some("body") {
            form.push(("client_id".into(), client_id.into()));
            form.push(("client_secret".into(), client_secret.into()));
        } else {
            req = req.basic_auth(client_id, Some(client_secret));
        }
        let resp = req.form(&form).send().await.map_err(|e| NodeError::new(format!("Could not get an OAuth2 access token: {e}")))?;
        let status = resp.status().as_u16();
        let body: Value = resp.json().await.unwrap_or(Value::Null);
        let token = body["access_token"].as_str().filter(|_| status < 400).ok_or_else(|| {
            NodeError::api("Could not get an OAuth2 access token", Some(status), Some(body.to_string()))
        })?;
        let token = token.to_string();
        auth.data["oauthTokenData"] = body;
        if let Some(store) = &ctx.services.store {
            let _ = store.update_credential_data(&auth.cred_id, &auth.data).await;
        }
        Ok(token)
    }

    async fn send(&self, ctx: &ExecCtx<'_>, item: usize, auth: &mut Option<Auth>, extra_query: &Map<String, Value>) -> NodeResult<Response> {
        let method = ctx.param_str("method", item, "GET")?.to_uppercase();
        let url_text = ctx.param_str("url", item, "")?;
        let mut url = reqwest::Url::parse(url_text.trim()).map_err(|_| NodeError::new(format!("Invalid URL: {url_text}")).at(item))?;
        check_ssrf(&url, ctx.config()).await.map_err(|m| NodeError::new(m).at(item))?;

        let mut query: Map<String, Value> = Map::new();
        if ctx.param_bool("sendQuery", item, false)? {
            if ctx.param_str("specifyQuery", item, "keypair")? == "json" {
                query = json_param(ctx.param("jsonQuery", item)?, "Query parameters")?.as_object().cloned().unwrap_or_default();
            } else {
                query = pairs_to_object(&ctx.param("queryParameters.parameters", item)?);
            }
        }
        for (k, v) in extra_query {
            query.insert(k.clone(), v.clone());
        }
        let mut headers: Map<String, Value> = Map::new();
        if ctx.param_bool("sendHeaders", item, false)? {
            if ctx.param_str("specifyHeaders", item, "keypair")? == "json" {
                headers = json_param(ctx.param("jsonHeaders", item)?, "Headers")?.as_object().cloned().unwrap_or_default();
            } else {
                headers = pairs_to_object(&ctx.param("headerParameters.parameters", item)?);
            }
        }
        if let Some(a) = auth.as_ref() {
            match a.kind.as_str() {
                "httpQueryAuth" => {
                    query.insert(a.data["name"].as_str().unwrap_or_default().to_string(), a.data["value"].clone());
                }
                _ => {}
            }
        }
        {
            let mut pairs = url.query_pairs_mut();
            for (k, v) in &query {
                pairs.append_pair(k, &value_to_string(v));
            }
        }
        if url.query() == Some("") {
            url.set_query(None);
        }

        let body: Option<(String, Vec<u8>)> = if ctx.param_bool("sendBody", item, false)? {
            let content_type = ctx.param_str("contentType", item, "json")?;
            match content_type.as_str() {
                "form-urlencoded" => {
                    let fields = pairs_to_object(&ctx.param("bodyParameters.parameters", item)?);
                    let encoded = fields.iter().map(|(k, v)| format!("{}={}", urlencode(k), urlencode(&value_to_string(v)))).collect::<Vec<_>>().join("&");
                    Some(("application/x-www-form-urlencoded".into(), encoded.into_bytes()))
                }
                "raw" => Some((ctx.param_str("rawContentType", item, "text/plain")?, ctx.param_str("body", item, "")?.into_bytes())),
                _ => {
                    let value = if ctx.param_str("specifyBody", item, "keypair")? == "json" {
                        json_param(ctx.param("jsonBody", item)?, "JSON body")?
                    } else {
                        Value::Object(pairs_to_object(&ctx.param("bodyParameters.parameters", item)?))
                    };
                    Some(("application/json".into(), serde_json::to_vec(&value).unwrap()))
                }
            }
        } else {
            None
        };

        let timeout_ms = ctx.param_f64("options.timeout", item, 300_000.0)? as u64;
        let mut refreshed = false;
        loop {
            let mut req = ctx
                .services
                .http
                .request(reqwest::Method::from_bytes(method.as_bytes()).map_err(|_| NodeError::new(format!("Invalid method {method}")))?, url.clone())
                .timeout(std::time::Duration::from_millis(timeout_ms.max(1)));
            for (k, v) in &headers {
                req = req.header(k.as_str(), value_to_string(v));
            }
            if let Some((ct, bytes)) = &body {
                req = req.header("content-type", ct.as_str()).body(bytes.clone());
            }
            if let Some(a) = auth.as_mut() {
                match a.kind.as_str() {
                    "httpHeaderAuth" => req = req.header(a.data["name"].as_str().unwrap_or_default(), value_to_string(&a.data["value"])),
                    "httpBasicAuth" => req = req.basic_auth(a.data["user"].as_str().unwrap_or_default(), a.data["password"].as_str()),
                    "httpBearerAuth" => req = req.bearer_auth(value_to_string(&a.data["token"])),
                    "oAuth2Api" => {
                        let token = match a.data.pointer("/oauthTokenData/access_token").and_then(Value::as_str) {
                            Some(t) => t.to_string(),
                            None => self.fetch_token(ctx, a).await.map_err(|e| e.at(item))?,
                        };
                        req = req.bearer_auth(token);
                    }
                    _ => {}
                }
            }
            let resp = req.send().await.map_err(|e| {
                let msg = if e.is_timeout() { format!("The request timed out after {timeout_ms} ms") } else { format!("The request failed: {e}") };
                NodeError::api(msg, None, None).at(item)
            })?;
            let status = resp.status().as_u16();
            if status == 401 && !refreshed && auth.as_ref().is_some_and(|a| a.kind == "oAuth2Api") {
                refreshed = true;
                if let Some(a) = auth.as_mut() {
                    self.fetch_token(ctx, a).await.map_err(|e| e.at(item))?;
                }
                continue;
            }
            let reason = resp.status().canonical_reason().unwrap_or("").to_string();
            let mut header_map = Map::new();
            for (k, v) in resp.headers() {
                header_map.insert(k.as_str().to_string(), json!(v.to_str().unwrap_or_default()));
            }
            let is_json = header_map.get("content-type").and_then(Value::as_str).is_some_and(|c| c.contains("json"));
            let bytes = resp.bytes().await.map_err(|e| NodeError::api(format!("Reading the response failed: {e}"), Some(status), None).at(item))?;
            let text = String::from_utf8_lossy(&bytes).to_string();
            let format = ctx.param_str("options.response.response.responseFormat", item, "autodetect")?;
            let body_value = match format.as_str() {
                "text" => Value::String(text),
                "json" => serde_json::from_str(&text).map_err(|_| NodeError::new("Response body is not valid JSON. Change \"Response Format\" to \"Text\"").at(item))?,
                _ if is_json || (text.trim_start().starts_with(['{', '[']) && serde_json::from_str::<Value>(&text).is_ok()) => {
                    serde_json::from_str(&text).unwrap_or(Value::String(text))
                }
                _ => Value::String(text),
            };
            return Ok(Response { status, reason, headers: header_map, body: body_value });
        }
    }

    /// Turns a response into output items.
    fn items_from(&self, ctx: &ExecCtx<'_>, item: usize, resp: Response) -> NodeResult<Vec<Item>> {
        let never_error = ctx.param_bool("options.response.response.neverError", item, false)?;
        if resp.status >= 400 && !never_error {
            let description = match &resp.body {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            return Err(NodeError::api(status_message(resp.status), Some(resp.status), Some(format!("{} - {description}", resp.status))).at(item));
        }
        let full = ctx.param_bool("options.response.response.fullResponse", item, false)?;
        let out_field = ctx.param_str("options.response.response.outputPropertyName", item, "data")?;
        if full {
            let mut json = Map::new();
            json.insert("body".into(), resp.body);
            json.insert("headers".into(), Value::Object(resp.headers));
            json.insert("statusCode".into(), json!(resp.status));
            json.insert("statusMessage".into(), json!(resp.reason));
            return Ok(vec![Item::new(json).paired(item)]);
        }
        Ok(match resp.body {
            Value::Array(a) => a
                .into_iter()
                .map(|v| match v {
                    Value::Object(o) => Item::new(o).paired(item),
                    other => {
                        let mut m = Map::new();
                        m.insert(out_field.clone(), other);
                        Item::new(m).paired(item)
                    }
                })
                .collect(),
            Value::Object(o) => vec![Item::new(o).paired(item)],
            other => {
                let mut m = Map::new();
                m.insert(out_field, if other.is_null() { json!("") } else { other });
                vec![Item::new(m).paired(item)]
            }
        })
    }

    async fn run_item(&self, ctx: &ExecCtx<'_>, item: usize, auth: &mut Option<Auth>) -> NodeResult<Vec<Item>> {
        let pagination = ctx.raw_param("options.pagination.pagination").cloned();
        let Some(pagination) = pagination else {
            let resp = self.send(ctx, item, auth, &Map::new()).await?;
            return self.items_from(ctx, item, resp);
        };
        let max_requests = if pagination["limitPagesFetched"].as_bool().unwrap_or(false) { pagination["maxRequests"].as_u64().unwrap_or(100) } else { 1000 };
        let complete_when = pagination["paginationCompleteWhen"].as_str().unwrap_or("responseIsEmpty").to_string();
        let mut out = Vec::new();
        let mut page = 0u64;
        let mut previous: Value = Value::Null;
        loop {
            let ev = ctx.evaluator()?;
            ev.set_extra(&json!({"$response": previous, "$pageCount": page})).map_err(|e| NodeError::from(e).at(item))?;
            let mut extra_query = Map::new();
            for (p, param) in pagination.pointer("/parameters/parameters").and_then(Value::as_array).cloned().unwrap_or_default().iter().enumerate() {
                if param["type"].as_str().unwrap_or("qs") == "qs" {
                    let name = param["name"].as_str().unwrap_or_default().to_string();
                    let value = ctx.param(&format!("options.pagination.pagination.parameters.parameters.{p}.value"), item)?;
                    extra_query.insert(name, value);
                }
            }
            let resp = self.send(ctx, item, auth, &extra_query).await?;
            page += 1;
            let response_json = json!({"body": resp.body, "headers": resp.headers, "statusCode": resp.status});
            let empty = match &resp.body {
                Value::Array(a) => a.is_empty(),
                Value::Object(o) => o.is_empty(),
                Value::String(s) => s.is_empty(),
                Value::Null => true,
                _ => false,
            };
            let status = resp.status;
            out.extend(self.items_from(ctx, item, resp)?);
            previous = response_json;
            let done = match complete_when.as_str() {
                "responseIsEmpty" => empty,
                "receiveSpecificStatusCodes" => {
                    let codes = pagination["statusCodesWhenComplete"].as_str().unwrap_or_default();
                    codes.split(',').any(|c| c.trim().parse::<u16>().ok() == Some(status))
                }
                _ => {
                    let ev = ctx.evaluator()?;
                    ev.set_extra(&json!({"$response": previous, "$pageCount": page})).map_err(|e| NodeError::from(e).at(item))?;
                    ctx.param("options.pagination.pagination.completeExpression", item)?.as_bool().unwrap_or(true)
                }
            };
            if done || page >= max_requests {
                break;
            }
        }
        Ok(out)
    }
}

fn urlencode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

#[async_trait::async_trait]
impl NodeType for HttpRequest {
    fn type_name(&self) -> &'static str {
        "n8n-nodes-base.httpRequest"
    }

    async fn execute(&self, ctx: &mut ExecCtx<'_>) -> NodeResult<NodeOutput> {
        let mut auth = self.auth(ctx).await?;
        let count = ctx.input().len().max(1);
        let mut out = Vec::new();
        for i in 0..count {
            match self.run_item(ctx, i, &mut auth).await {
                Ok(items) => out.extend(items),
                Err(e) if ctx.continue_on_fail() => ctx.push_error_item(&e, i),
                Err(e) => return Err(e),
            }
        }
        Ok(vec![out])
    }
}
