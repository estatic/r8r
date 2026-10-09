//! Web search and page scraping through Firecrawl's API (v2):
//! `POST /v2/search` and `POST /v2/scrape`, with the key from a
//! `firecrawlApi` credential as a Bearer token.

use crate::domain::Item;
use crate::node::{Node, NodeError, NodeExecutionContext, NodeOutput};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::OnceLock;
use std::time::Duration;

pub struct FirecrawlNode;

const DEFAULT_API_BASE_URL: &str = "https://api.firecrawl.dev";
/// Firecrawl's own per-request limit is 60 s by default; this leaves it room.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(90);
const DEFAULT_LIMIT: u64 = 5;
/// Firecrawl's maximum results per source.
const MAX_LIMIT: u64 = 100;
const SOURCES: [&str; 3] = ["web", "news", "images"];
const SCRAPE_FORMATS: [&str; 5] = ["markdown", "summary", "html", "links", "images"];

fn http_client() -> Result<&'static reqwest::Client, NodeError> {
    static CLIENT: OnceLock<Result<reqwest::Client, String>> = OnceLock::new();
    CLIENT
        .get_or_init(|| {
            reqwest::Client::builder()
                .timeout(REQUEST_TIMEOUT)
                // The API key rides in a header; never follow it elsewhere.
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .map_err(|e| e.to_string())
        })
        .as_ref()
        .map_err(|_| NodeError::ExecutionFailed("firecrawl: failed to build HTTP client".into()))
}

#[async_trait]
impl Node for FirecrawlNode {
    fn type_name(&self) -> &'static str {
        "firecrawl.search"
    }
    fn runs_per_item(&self) -> bool {
        true
    }
    fn display_name(&self) -> &'static str {
        "Web Search (Firecrawl)"
    }
    fn description(&self) -> &'static str {
        "Searches the web, or reads a web page as markdown, with Firecrawl."
    }
    fn category(&self) -> crate::node::NodeCategory {
        crate::node::NodeCategory::Action
    }
    fn keeps_input_fields(&self) -> bool {
        true
    }
    fn icon(&self) -> &'static str {
        "🔎"
    }
    fn credential_types(&self) -> &'static [&'static str] {
        &["firecrawlApi"]
    }

    async fn execute(&self, ctx: &NodeExecutionContext) -> Result<NodeOutput, NodeError> {
        execute_with_client(http_client()?, ctx).await
    }
}

fn fail(msg: impl std::fmt::Display) -> NodeError {
    NodeError::ExecutionFailed(format!("firecrawl: {msg}"))
}

/// The credential's key and the address it may be sent to.
fn credential(ctx: &NodeExecutionContext) -> Result<(String, String), NodeError> {
    let id = ctx
        .parameters
        .get("auth")
        .and_then(|a| a.get("credential_id"))
        .and_then(Value::as_str)
        .ok_or_else(|| fail("choose a Firecrawl credential"))?;
    let id = uuid::Uuid::parse_str(id).map_err(|e| fail(format!("invalid credential_id: {e}")))?;
    let data = ctx.credentials.get(&id).ok_or_else(|| fail(format!("credential {id} was not resolved for this run")))?;
    let key = data
        .get("api_key")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| fail("the credential has no API key"))?;
    // The key goes only where its credential says (a self-hosted Firecrawl).
    if ctx.parameters.get("api_base_url").and_then(Value::as_str).is_some_and(|s| !s.is_empty()) {
        return Err(fail("api_base_url can't be set on the node; set the Base URL on the Firecrawl credential instead"));
    }
    let base = data
        .get("base_url")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .unwrap_or(DEFAULT_API_BASE_URL)
        .trim_end_matches('/')
        .to_string();
    Ok((key.to_string(), base))
}

fn text_param<'a>(p: &'a Value, key: &str) -> Option<&'a str> {
    p.get(key).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty())
}

/// A whole number from a number or numeric text (expressions give either).
fn number_param(p: &Value, key: &str) -> Result<Option<u64>, NodeError> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) if s.trim().is_empty() => Ok(None),
        Some(Value::Number(n)) => n.as_u64().map(Some).ok_or_else(|| fail(format!("\"{key}\" must be a whole number"))),
        Some(Value::String(s)) => s.trim().parse().map(Some).map_err(|_| fail(format!("\"{key}\" must be a whole number"))),
        Some(other) => Err(fail(format!("\"{key}\" must be a whole number, got {other}"))),
    }
}

fn string_list(p: &Value, key: &str) -> Vec<String> {
    match p.get(key) {
        Some(Value::Array(a)) => a.iter().filter_map(Value::as_str).map(str::to_string).collect(),
        Some(Value::String(s)) => s.split(',').map(str::trim).filter(|s| !s.is_empty()).map(str::to_string).collect(),
        _ => vec![],
    }
}

/// The `/v2/search` body for the node's parameters.
fn search_body(p: &Value) -> Result<Value, NodeError> {
    let query = text_param(p, "query").ok_or_else(|| fail("enter a search query"))?;
    if query.chars().count() > 500 {
        return Err(fail("the search query is longer than 500 characters"));
    }
    let limit = number_param(p, "limit")?.unwrap_or(DEFAULT_LIMIT);
    if !(1..=MAX_LIMIT).contains(&limit) {
        return Err(fail(format!("\"limit\" must be between 1 and {MAX_LIMIT}")));
    }
    let mut sources = string_list(p, "sources");
    if sources.is_empty() {
        sources.push("web".into());
    }
    if let Some(bad) = sources.iter().find(|s| !SOURCES.contains(&s.as_str())) {
        return Err(fail(format!("unknown source \"{bad}\" (expected web, news or images)")));
    }
    let mut body = json!({"query": query, "limit": limit, "sources": sources});
    for (param, field) in [("time_range", "tbs"), ("country", "country"), ("location", "location")] {
        if let Some(v) = text_param(p, param) {
            body[field] = json!(v);
        }
    }
    let include = string_list(p, "include_domains");
    let exclude = string_list(p, "exclude_domains");
    if !include.is_empty() && !exclude.is_empty() {
        return Err(fail("use either \"include_domains\" or \"exclude_domains\", not both"));
    }
    if !include.is_empty() {
        body["includeDomains"] = json!(include);
    }
    if !exclude.is_empty() {
        body["excludeDomains"] = json!(exclude);
    }
    if p.get("scrape_results").and_then(Value::as_bool) == Some(true) {
        body["scrapeOptions"] = json!({"formats": [{"type": "markdown"}], "onlyMainContent": true});
    }
    Ok(body)
}

/// The `/v2/scrape` body for the node's parameters.
fn scrape_body(p: &Value) -> Result<Value, NodeError> {
    let url = text_param(p, "url").ok_or_else(|| fail("enter the URL to read"))?;
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err(fail(format!("\"{url}\" is not an http(s) URL")));
    }
    let mut formats = string_list(p, "formats");
    if formats.is_empty() {
        formats.push("markdown".into());
    }
    if let Some(bad) = formats.iter().find(|f| !SCRAPE_FORMATS.contains(&f.as_str())) {
        return Err(fail(format!("unknown format \"{bad}\" (expected {})", SCRAPE_FORMATS.join(", "))));
    }
    let only_main = p.get("only_main_content").and_then(Value::as_bool).unwrap_or(true);
    Ok(json!({"url": url, "formats": formats, "onlyMainContent": only_main}))
}

/// The limiter's key for an API key at an address; a hash, so the key
/// itself isn't held in yet another place.
fn limiter_key(base: &str, api_key: &str) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (base, api_key).hash(&mut h);
    format!("firecrawl:{:x}", h.finish())
}

/// Firecrawl's reason for a failed call, without anything secret in it.
fn api_error(status: reqwest::StatusCode, body: &Value) -> NodeError {
    let reason = body.get("error").and_then(Value::as_str).unwrap_or("no reason given");
    let hint = match status.as_u16() {
        401 => " (check the API key on the credential)",
        402 => " (the Firecrawl account is out of credits)",
        429 => " (rate limited; try again later or add retries to the node)",
        _ => "",
    };
    fail(format!("Firecrawl returned HTTP {}: {reason}{hint}", status.as_u16()))
}

async fn execute_with_client(client: &reqwest::Client, ctx: &NodeExecutionContext) -> Result<NodeOutput, NodeError> {
    let p = &ctx.parameters;
    let operation = text_param(p, "operation").unwrap_or("search");
    let (path, body) = match operation {
        "search" => ("/v2/search", search_body(p)?),
        "scrape" => ("/v2/scrape", scrape_body(p)?),
        other => return Err(fail(format!("unknown operation \"{other}\" (expected \"search\" or \"scrape\")"))),
    };
    let (key, base) = credential(ctx)?;
    // Firecrawl limits requests per API key: with a limit set, every request
    // with this key (any run, any agent tool call) waits for its turn.
    if let Some(per_minute) = number_param(p, "max_requests_per_minute")? {
        let interval = super::rate_limit::interval_for(per_minute).ok_or_else(|| fail("\"max_requests_per_minute\" must be between 1 and 6000"))?;
        super::rate_limit::wait_turn(&limiter_key(&base, &key), interval).await;
    }
    // Errors never include the reqwest error or the request: the key is in it.
    let response = client
        .post(format!("{base}{path}"))
        .bearer_auth(&key)
        .json(&body)
        .send()
        .await
        .map_err(|e| fail(if e.is_timeout() { "the request to Firecrawl timed out" } else { "the request to Firecrawl failed" }))?;
    let status = response.status();
    let json: Value = response
        .json()
        .await
        .map_err(|_| fail(format!("Firecrawl returned HTTP {} with a body that isn't JSON", status.as_u16())))?;
    if !status.is_success() || json.get("success").and_then(Value::as_bool) == Some(false) {
        return Err(api_error(status, &json));
    }
    let data = json.get("data").cloned().unwrap_or(Value::Null);
    let items = if operation == "search" { search_items(&data) } else { vec![scrape_item(data)] };
    Ok(vec![items.into_iter().map(|json| Item { json, binary: json!({}) }).collect()])
}

/// One item per result, web first, each saying which source it came from.
fn search_items(data: &Value) -> Vec<Value> {
    let mut items = Vec::new();
    for source in SOURCES {
        for result in data.get(source).and_then(Value::as_array).into_iter().flatten() {
            let mut item = json!({"source": source});
            if let Some(obj) = result.as_object() {
                for (k, v) in obj {
                    // Leave out the empty scrape fields Firecrawl sends as null.
                    if !v.is_null() {
                        item[k] = v.clone();
                    }
                }
            }
            items.push(item);
        }
    }
    items
}

/// The page, with its final address on top.
fn scrape_item(data: Value) -> Value {
    let mut item = json!({});
    if let Some(url) = data.pointer("/metadata/url").or_else(|| data.pointer("/metadata/sourceURL")) {
        item["url"] = url.clone();
    }
    if let Some(obj) = data.as_object() {
        for (k, v) in obj {
            if !v.is_null() {
                item[k] = v.clone();
            }
        }
    }
    item
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{body_json, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn ctx(server: &MockServer, mut params: Value) -> NodeExecutionContext {
        let id = uuid::Uuid::new_v4();
        params["auth"] = json!({"credential_id": id.to_string()});
        NodeExecutionContext {
            parameters: params,
            credentials: std::collections::HashMap::from([(id, json!({"api_key": "fc-secret", "base_url": server.uri()}))]),
            ..Default::default()
        }
    }

    async fn run(server: &MockServer, params: Value) -> Result<Vec<Value>, NodeError> {
        let client = reqwest::Client::new();
        let out = execute_with_client(&client, &ctx(server, params)).await?;
        Ok(out[0].iter().map(|i| i.json.clone()).collect())
    }

    #[tokio::test]
    async fn searches_and_returns_one_item_per_result() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v2/search"))
            .and(header("authorization", "Bearer fc-secret"))
            .and(body_json(json!({"query": "rust workflow engines", "limit": 2, "sources": ["web", "news"], "tbs": "qdr:w"})))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "success": true,
                "data": {
                    "web": [
                        {"title": "A", "url": "https://a.example", "description": "first", "markdown": null},
                        {"title": "B", "url": "https://b.example", "description": "second"}
                    ],
                    "news": [{"title": "N", "url": "https://n.example", "snippet": "news", "date": "2026-10-01", "position": 1}]
                },
                "creditsUsed": 2
            })))
            .mount(&server)
            .await;
        let items = run(&server, json!({"query": "rust workflow engines", "limit": "2", "sources": ["web", "news"], "time_range": "qdr:w"})).await.unwrap();
        assert_eq!(
            items,
            vec![
                json!({"source": "web", "title": "A", "url": "https://a.example", "description": "first"}),
                json!({"source": "web", "title": "B", "url": "https://b.example", "description": "second"}),
                json!({"source": "news", "title": "N", "url": "https://n.example", "snippet": "news", "date": "2026-10-01", "position": 1}),
            ]
        );
    }

    #[tokio::test]
    async fn search_can_scrape_each_result_as_markdown() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v2/search"))
            .and(body_json(json!({
                "query": "q", "limit": 5, "sources": ["web"],
                "scrapeOptions": {"formats": [{"type": "markdown"}], "onlyMainContent": true}
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"success": true, "data": {"web": [{"url": "https://a", "markdown": "# A"}]}})))
            .mount(&server)
            .await;
        let items = run(&server, json!({"query": "q", "scrape_results": true})).await.unwrap();
        assert_eq!(items[0]["markdown"], "# A");
    }

    #[tokio::test]
    async fn scrapes_a_page() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v2/scrape"))
            .and(body_json(json!({"url": "https://example.com", "formats": ["markdown", "links"], "onlyMainContent": true})))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "success": true,
                "data": {"markdown": "# Example", "html": null, "links": ["https://iana.org"], "metadata": {"title": "Example", "url": "https://example.com/", "statusCode": 200}}
            })))
            .mount(&server)
            .await;
        let items = run(&server, json!({"operation": "scrape", "url": "https://example.com", "formats": ["markdown", "links"]})).await.unwrap();
        assert_eq!(
            items,
            vec![json!({
                "url": "https://example.com/", "markdown": "# Example", "links": ["https://iana.org"],
                "metadata": {"title": "Example", "url": "https://example.com/", "statusCode": 200}
            })]
        );
    }

    #[tokio::test]
    async fn reports_firecrawls_reason_without_the_key() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(402).set_body_json(json!({"success": false, "error": "Insufficient credits"})))
            .mount(&server)
            .await;
        let err = run(&server, json!({"query": "q"})).await.unwrap_err().to_string();
        assert!(err.contains("HTTP 402: Insufficient credits") && err.contains("out of credits"), "{err}");
        assert!(!err.contains("fc-secret"));
    }

    #[tokio::test]
    async fn checks_its_parameters_before_calling_firecrawl() {
        let server = MockServer::start().await; // no mocks: any call would 404
        for (params, says) in [
            (json!({}), "enter a search query"),
            (json!({"query": "q", "limit": 0}), "between 1 and 100"),
            (json!({"query": "q", "sources": ["maps"]}), "unknown source \"maps\""),
            (json!({"query": "q", "include_domains": "a.com", "exclude_domains": "b.com"}), "not both"),
            (json!({"operation": "scrape", "url": "ftp://x"}), "not an http(s) URL"),
            (json!({"operation": "scrape", "url": "https://x", "formats": ["pdf"]}), "unknown format \"pdf\""),
            (json!({"operation": "crawl"}), "unknown operation"),
            (json!({"query": "q", "api_base_url": "https://evil.example"}), "can't be set on the node"),
        ] {
            let err = run(&server, params.clone()).await.unwrap_err().to_string();
            assert!(err.contains(says), "{params}: {err}");
        }
    }

    #[tokio::test]
    async fn a_rate_limit_spaces_requests_with_the_same_key() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v2/search"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"success": true, "data": {"web": []}})))
            .mount(&server)
            .await;
        // 600 a minute = one every 100 ms: three requests take at least 200 ms.
        let started = std::time::Instant::now();
        for _ in 0..3 {
            run(&server, json!({"query": "q", "max_requests_per_minute": 600})).await.unwrap();
        }
        assert!(started.elapsed() >= Duration::from_millis(200), "{:?}", started.elapsed());
        let err = run(&server, json!({"query": "q", "max_requests_per_minute": 0})).await.unwrap_err();
        assert!(err.to_string().contains("between 1 and 6000"), "{err}");
    }

    #[tokio::test]
    async fn needs_a_credential_with_a_key() {
        let ctx = NodeExecutionContext { parameters: json!({"query": "q"}), ..Default::default() };
        let err = execute_with_client(&reqwest::Client::new(), &ctx).await.unwrap_err().to_string();
        assert!(err.contains("choose a Firecrawl credential"), "{err}");
    }
}
