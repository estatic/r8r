//! Native node implementations (spec §6.6), keyed by n8n type name.

mod ai;
mod ai_chains;
pub mod text_split;
mod aws_sigv4;
mod airtable;
mod code;
mod compression;
mod conditions;
mod core;
mod data_table;
mod discord;
mod email;
pub mod email_imap;
mod files;
mod ftp;
mod gmail;
mod google_auth;
mod googledrive;
mod googlesheets;
mod github;
mod html;
mod http;
mod jwt;
pub mod kafka;
mod merge;
mod mime;
mod mongodb;
pub mod mqtt;
mod mssql;
mod mysql;
mod notion;
mod openai;
mod postgres;
mod python;
pub mod rabbitmq;
mod redis;
mod routing;
mod server_nodes;
mod set;
mod slack;
mod sql_common;
mod ssh;
mod ssh_common;
mod telegram;
mod transform;
mod utility;

pub use conditions::evaluate_conditions;
pub use http::{check_ssrf, is_internal};

use super::node::NodeType;
use serde_json::{Map, Value};
use std::collections::HashMap;

pub struct Registry {
    types: HashMap<&'static str, Box<dyn NodeType>>,
}

impl Registry {
    pub fn get(&self, type_name: &str) -> Option<&dyn NodeType> {
        self.types.get(type_name).map(|b| b.as_ref())
    }

    pub fn type_names(&self) -> Vec<&'static str> {
        let mut v: Vec<_> = self.types.keys().copied().collect();
        v.sort_unstable();
        v
    }

    fn add(&mut self, node: Box<dyn NodeType>) {
        self.types.insert(node.type_name(), node);
    }
}

impl Default for Registry {
    fn default() -> Self {
        let mut r = Registry { types: HashMap::new() };
        for n in core::all() {
            r.add(n);
        }
        r.add(Box::new(set::Set));
        r.add(Box::new(routing::If));
        r.add(Box::new(routing::Filter));
        r.add(Box::new(routing::Switch));
        r.add(Box::new(merge::Merge));
        r.add(Box::new(code::Code));
        r.add(Box::new(http::HttpRequest));
        for n in jwt::all() {
            r.add(n);
        }
        r.add(Box::new(compression::Compression));
        r.add(Box::new(redis::Redis));
        r.add(Box::new(rabbitmq::RabbitMq));
        r.add(Box::new(mqtt::Mqtt));
        r.add(Box::new(kafka::Kafka));
        for n in transform::all() {
            r.add(n);
        }
        for n in utility::all() {
            r.add(n);
        }
        for n in html::all() {
            r.add(n);
        }
        for n in ai::all() {
            r.add(n);
        }
        for n in ai_chains::all() {
            r.add(n);
        }
        for n in postgres::all() {
            r.add(n);
        }
        for n in mysql::all() {
            r.add(n);
        }
        for n in mongodb::all() {
            r.add(n);
        }
        for n in mssql::all() {
            r.add(n);
        }
        for n in server_nodes::all() {
            r.add(n);
        }
        r.add(Box::new(slack::Slack));
        r.add(Box::new(googlesheets::GoogleSheets));
        r.add(Box::new(gmail::Gmail));
        r.add(Box::new(googledrive::GoogleDrive));
        r.add(Box::new(github::Github));
        r.add(Box::new(telegram::Telegram));
        r.add(Box::new(notion::Notion));
        r.add(Box::new(airtable::Airtable));
        r.add(Box::new(discord::Discord));
        r.add(Box::new(data_table::DataTable));
        r.add(Box::new(openai::OpenAi));
        for n in files::all() {
            r.add(n);
        }
        for n in email::all() {
            r.add(n);
        }
        for n in ssh::all() {
            r.add(n);
        }
        for n in ftp::all() {
            r.add(n);
        }
        // Each native node n8n marks `usableAsTool` also exists as an AI
        // tool sub-node, `<type>Tool` (e.g. `httpRequestTool`).
        for &(name, base) in tool_variant_names() {
            if r.types.contains_key(base) {
                r.add(Box::new(ai::NodeAsTool { name }));
            }
        }
        r
    }
}

/// `(variant, base)` full type names for `USABLE_AS_TOOL`, built once
/// (registry keys must be `'static`).
fn tool_variant_names() -> &'static [(&'static str, &'static str)] {
    static NAMES: std::sync::OnceLock<Vec<(&'static str, &'static str)>> = std::sync::OnceLock::new();
    NAMES.get_or_init(|| {
        USABLE_AS_TOOL
            .iter()
            .map(|b| {
                let base: &'static str = Box::leak(format!("n8n-nodes-base.{b}").into_boxed_str());
                (&*Box::leak(format!("{base}Tool").into_boxed_str()), base)
            })
            .collect()
    })
}

impl Registry {
    /// Whether `NODES_EXCLUDE` rules `type_name` out. A tool variant goes
    /// with its base node, as in n8n, which only builds tool variants for the
    /// node types it loaded: otherwise `executeCommandTool` would run an
    /// excluded Execute Command.
    pub fn is_excluded(&self, type_name: &str, excluded: &[String]) -> bool {
        let listed = |t: &str| excluded.iter().any(|e| e == t);
        listed(type_name) || self.tool_base(type_name).is_some_and(listed)
    }

    /// The node a `<type>Tool` variant runs, if `type_name` is one.
    pub fn tool_base(&self, type_name: &str) -> Option<&'static str> {
        let base = type_name.strip_suffix("Tool")?;
        let short = base.strip_prefix("n8n-nodes-base.")?;
        USABLE_AS_TOOL.contains(&short).then(|| self.types.get_key_value(base).map(|(k, _)| *k)).flatten()
    }
}

/// `n8n-nodes-base` node types with `usableAsTool` in n8n 2.35.7 (from its
/// `types/nodes.json`); those r8r runs natively get a tool variant.
pub const USABLE_AS_TOOL: &[&str] = &[
    "Brandfetch", "actionNetwork", "activeCampaign", "adalo", "affinity", "agileCrm", "airtable", "airtop",
    "amqp", "apiTemplateIo", "asana", "autopilot", "awsLambda", "awsS3", "awsSes", "awsSns", "awsTextract",
    "awsTranscribe", "bambooHr", "baserow", "beeminder", "bitly", "bitwarden", "bubble", "chargebee", "circleCi",
    "ciscoWebex", "clearbit", "clickUp", "clockify", "cloudflare", "cockpit", "coda", "coinGecko", "compression",
    "contentful", "convertKit", "copper", "crateDb", "crypto", "currents", "customerIo", "dataTable",
    "databricks", "dateTime", "deepL", "demio", "dhl", "discord", "discourse", "drift", "dropbox", "dropcontact",
    "e2eTest", "egoi", "elasticSecurity", "elasticsearch", "emailSend", "emelia", "erpNext", "executeCommand",
    "facebookGraphApi", "filemaker", "freshdesk", "freshservice", "freshworksCrm", "gSuiteAdmin", "getResponse",
    "ghost", "git", "github", "gitlab", "gmail", "goToWebinar", "gong", "googleAds", "googleAnalytics",
    "googleBigQuery", "googleBooks", "googleBusinessProfile", "googleCalendar", "googleChat",
    "googleCloudNaturalLanguage", "googleCloudStorage", "googleContacts", "googleDocs", "googleDrive",
    "googleFirebaseCloudFirestore", "googleFirebaseRealtimeDatabase", "googlePerspective", "googleSheets",
    "googleSlides", "googleTasks", "googleTranslate", "gotify", "grafana", "graphql", "grist", "hackerNews",
    "haloPSA", "harvest", "helpScout", "highLevel", "homeAssistant", "httpRequest", "hubspot", "humanticAi",
    "hunter", "intercom", "invoiceNinja", "iterable", "jenkins", "jinaAi", "jira", "jwt", "kafka", "keap",
    "koBoToolbox", "ldap", "lemlist", "line", "linear", "lingvaNex", "linkedIn", "loneScale", "magento2",
    "mailcheck", "mailchimp", "mailerLite", "mailgun", "mailjet", "mandrill", "marketstack", "matrix",
    "mattermost", "mautic", "medium", "messageAnAgent", "messageBird", "metabase", "microsoftDynamicsCrm",
    "microsoftEntra", "microsoftExcel", "microsoftExcelSharePoint", "microsoftGraphSecurity", "microsoftOneDrive",
    "microsoftOutlook", "microsoftSharePoint", "microsoftSql", "microsoftTeams", "microsoftToDo", "misp",
    "mistralAi", "mocean", "mondayCom", "mongoDb", "monicaCrm", "mqtt", "msg91", "mySql", "nasa", "netlify",
    "nextCloud", "nocoDb", "notion", "npm", "odoo", "okta", "oneSimpleApi", "onfleet", "openThesaurus",
    "openWeatherMap", "oracleDatabase", "oura", "paddle", "pagerDuty", "peekalink", "perplexity", "phantombuster",
    "philipsHue", "pipedrive", "plivo", "postBin", "postHog", "postgres", "profitWell", "pushbullet", "pushcut",
    "pushover", "questDb", "quickChart", "quickbase", "quickbooks", "rabbitmq", "raindrop", "reddit", "redis",
    "rocketchat", "rssFeedRead", "rundeck", "s3", "salesforce", "salesmate", "seaTable", "securityScorecard",
    "segment", "sendGrid", "sendInBlue", "sendy", "sentryIo", "serviceNow", "shopify", "signl4", "slack", "sms77",
    "snowflake", "splunk", "spotify", "stackby", "storyblok", "strapi", "strava", "stripe", "supabase",
    "syncroMsp", "taiga", "tapfiliate", "telegram", "theHive", "theHiveProject", "timescaleDb", "todoist", "totp",
    "travisCi", "trello", "twake", "twilio", "twist", "twitter", "unleashedSoftware", "uplead", "uproc",
    "uptimeRobot", "urlScanIo", "venafiTlsProtectCloud", "venafiTlsProtectDatacenter", "vero", "vonage",
    "webflow", "wekan", "whatsApp", "wooCommerce", "wordpress", "xero", "youTube", "yourls", "zammad", "zendesk",
    "zohoCrm", "zoom", "zulip"
];

// ---- helpers shared by nodes ----------------------------------------------

/// Reads a dotted path (`a.b.0.c`, `a.b[0].c`) from a JSON value.
pub fn get_path<'v>(value: &'v Value, path: &str) -> Option<&'v Value> {
    let mut v = value;
    for part in split_path(path) {
        v = match v {
            Value::Object(o) => o.get(&part)?,
            Value::Array(a) => a.get(part.parse::<usize>().ok()?)?,
            _ => return None,
        };
    }
    Some(v)
}

/// Writes a dotted path, creating objects on the way.
pub fn set_path(target: &mut Map<String, Value>, path: &str, value: Value) {
    let parts = split_path(path);
    let Some((last, init)) = parts.split_last() else { return };
    let mut cur = target;
    for part in init {
        let entry = cur.entry(part.clone()).or_insert_with(|| Value::Object(Map::new()));
        if !entry.is_object() {
            *entry = Value::Object(Map::new());
        }
        cur = entry.as_object_mut().unwrap();
    }
    cur.insert(last.clone(), value);
}

fn split_path(path: &str) -> Vec<String> {
    path.replace('[', ".").replace(']', "").split('.').filter(|p| !p.is_empty()).map(String::from).collect()
}

/// Flattens a JSON object into `(dotted.path, leaf value)` pairs: nested
/// objects are recursed into and joined with `.`; arrays and scalars are
/// kept as single leaf values. Used by spreadsheet-writing nodes to turn
/// item json into flat columns, the way n8n's `flattenObject` does.
pub fn flatten_json(json: &Map<String, Value>) -> Vec<(String, Value)> {
    fn walk(prefix: &str, value: &Value, out: &mut Vec<(String, Value)>) {
        match value {
            Value::Object(map) if !map.is_empty() => {
                for (k, v) in map {
                    let key = if prefix.is_empty() { k.clone() } else { format!("{prefix}.{k}") };
                    walk(&key, v, out);
                }
            }
            other => out.push((prefix.to_string(), other.clone())),
        }
    }
    let mut out = Vec::new();
    for (k, v) in json {
        walk(k, v, &mut out);
    }
    out
}

/// A comma-separated field list parameter (`"a, b"`), or an array.
pub fn field_list(value: &Value) -> Vec<String> {
    match value {
        Value::String(s) => s.split(',').map(|f| f.trim().to_string()).filter(|f| !f.is_empty()).collect(),
        Value::Array(a) => a.iter().filter_map(|v| v.as_str().map(String::from)).collect(),
        _ => vec![],
    }
}
