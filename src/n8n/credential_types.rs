//! n8n credential types (spec §6.5): their fields drive validation, the
//! public API's JSON schema, the editor's `/types/credentials.json`, and
//! which values are secrets.

use serde_json::{json, Map, Value};

pub struct Field {
    pub name: &'static str,
    pub display: &'static str,
    pub secret: bool,
    pub required: bool,
    pub default: Value,
    /// JSON-schema / editor field type: "string", "number" or "boolean".
    pub kind: &'static str,
}

pub struct CredentialType {
    pub name: &'static str,
    pub display_name: &'static str,
    pub fields: Vec<Field>,
}

/// What n8n puts in place of a secret when it sends credential data to the
/// editor.
pub const BLANK: &str = "__n8n_BLANK_VALUE_e5362baf-c777-4d57-a609-6eaf1f9e87f6";

fn f(name: &'static str, display: &'static str, secret: bool, required: bool, default: Value) -> Field {
    Field { name, display, secret, required, default, kind: "string" }
}

fn f_typed(name: &'static str, display: &'static str, secret: bool, required: bool, default: Value, kind: &'static str) -> Field {
    Field { name, display, secret, required, default, kind }
}

pub fn all() -> Vec<CredentialType> {
    vec![
        CredentialType { name: "httpHeaderAuth", display_name: "Header Auth", fields: vec![f("name", "Name", false, true, json!("")), f("value", "Value", true, true, json!(""))] },
        CredentialType { name: "httpBasicAuth", display_name: "Basic Auth", fields: vec![f("user", "User", false, true, json!("")), f("password", "Password", true, true, json!(""))] },
        CredentialType { name: "httpQueryAuth", display_name: "Query Auth", fields: vec![f("name", "Name", false, true, json!("")), f("value", "Value", true, true, json!(""))] },
        CredentialType { name: "httpBearerAuth", display_name: "Bearer Auth", fields: vec![f("token", "Bearer Token", true, true, json!(""))] },
        CredentialType {
            name: "oAuth2Api",
            display_name: "OAuth2 API",
            fields: vec![
                f("grantType", "Grant Type", false, false, json!("authorizationCode")),
                f("authUrl", "Authorization URL", false, false, json!("")),
                f("accessTokenUrl", "Access Token URL", false, true, json!("")),
                f("clientId", "Client ID", false, true, json!("")),
                f("clientSecret", "Client Secret", true, true, json!("")),
                f("scope", "Scope", false, false, json!("")),
                f("authQueryParameters", "Auth URI Query Parameters", false, false, json!("")),
                f("authentication", "Authentication", false, false, json!("header")),
            ],
        },
        CredentialType {
            name: "jwtAuth",
            display_name: "JWT Auth",
            fields: vec![
                f("keyType", "Key Type", false, false, json!("passphrase")),
                f("secret", "Secret", true, false, json!("")),
                f("privateKey", "Private Key", true, false, json!("")),
                f("publicKey", "Public Key", false, false, json!("")),
                f("algorithm", "Algorithm", false, false, json!("HS256")),
            ],
        },
        CredentialType {
            name: "smtp",
            display_name: "SMTP",
            fields: vec![
                f("user", "User", false, false, json!("")),
                f("password", "Password", true, false, json!("")),
                f("host", "Host", false, false, json!("")),
                f("port", "Port", false, false, json!(465)),
                f("secure", "SSL/TLS", false, false, json!(true)),
                f("disableStartTls", "Disable STARTTLS", false, false, json!(false)),
                f("hostName", "Client Host Name", false, false, json!("")),
            ],
        },
        CredentialType {
            name: "redis",
            display_name: "Redis",
            fields: vec![
                f("password", "Password", true, false, json!("")),
                f("user", "User", false, false, json!("")),
                f("host", "Host", false, false, json!("localhost")),
                f("port", "Port", false, false, json!(6379)),
                f("database", "Database Number", false, false, json!(0)),
                f("ssl", "SSL", false, false, json!(false)),
                f("disableTlsVerification", "Disable TLS Verification (insecure)", false, false, json!(false)),
            ],
        },
        CredentialType {
            name: "openAiApi",
            display_name: "OpenAi",
            fields: vec![f("apiKey", "API Key", true, true, json!("")), f("organizationId", "Organization ID", false, false, json!("")), f("url", "Base URL", false, false, json!("https://api.openai.com/v1"))],
        },
        CredentialType {
            name: "anthropicApi",
            display_name: "Anthropic",
            fields: vec![f("apiKey", "API Key", true, true, json!("")), f("url", "Base URL", false, false, json!("https://api.anthropic.com"))],
        },
        CredentialType {
            name: "postgres",
            display_name: "Postgres",
            fields: vec![
                f("host", "Host", false, false, json!("localhost")),
                f("database", "Database", false, false, json!("postgres")),
                f("user", "User", false, false, json!("postgres")),
                f("password", "Password", true, false, json!("")),
                f_typed("maxConnections", "Maximum Number of Connections", false, false, json!(100), "number"),
                f_typed("allowUnauthorizedCerts", "Ignore SSL Issues (Insecure)", false, false, json!(false), "boolean"),
                f("ssl", "SSL", false, false, json!("disable")),
                f_typed("port", "Port", false, false, json!(5432), "number"),
            ],
        },
        CredentialType {
            name: "mySql",
            display_name: "MySQL",
            fields: vec![
                f("host", "Host", false, false, json!("localhost")),
                f("database", "Database", false, false, json!("mysql")),
                f("user", "User", false, false, json!("mysql")),
                f("password", "Password", true, false, json!("")),
                f_typed("port", "Port", false, false, json!(3306), "number"),
                f_typed("connectTimeout", "Connect Timeout", false, false, json!(10000), "number"),
                f_typed("ssl", "SSL", false, false, json!(false), "boolean"),
                f("caCertificate", "CA Certificate", true, false, json!("")),
                f("clientPrivateKey", "Client Private Key", true, false, json!("")),
                f("clientCertificate", "Client Certificate", true, false, json!("")),
            ],
        },
        CredentialType {
            name: "telegramApi",
            display_name: "Telegram API",
            fields: vec![f("accessToken", "Access Token", true, true, json!("")), f("baseUrl", "Base URL", false, false, json!("https://api.telegram.org"))],
        },
        CredentialType {
            name: "slackApi",
            display_name: "Slack API",
            fields: vec![
                f("accessToken", "Access Token", true, true, json!("")),
                f("signatureSecret", "Signature Secret", true, false, json!("")),
                f("url", "Base URL", false, false, json!("https://slack.com/api")),
            ],
        },
        CredentialType {
            name: "slackOAuth2Api",
            display_name: "Slack OAuth2 API",
            fields: vec![
                f("grantType", "Grant Type", false, false, json!("authorizationCode")),
                f("authUrl", "Authorization URL", false, false, json!("https://slack.com/oauth/v2/authorize")),
                f("accessTokenUrl", "Access Token URL", false, false, json!("https://slack.com/api/oauth.v2.access")),
                f("clientId", "Client ID", false, false, json!("")),
                f("clientSecret", "Client Secret", true, false, json!("")),
                f("scope", "Scope", false, false, json!("")),
                f("authentication", "Authentication", false, false, json!("body")),
                f("signatureSecret", "Signature Secret", true, false, json!("")),
                f("url", "Base URL", false, false, json!("https://slack.com/api")),
            ],
        },
        CredentialType {
            name: "googleSheetsOAuth2Api",
            display_name: "Google Sheets OAuth2 API",
            fields: vec![
                f("grantType", "Grant Type", false, false, json!("authorizationCode")),
                f("authUrl", "Authorization URL", false, false, json!("https://accounts.google.com/o/oauth2/v2/auth")),
                f("accessTokenUrl", "Access Token URL", false, false, json!("https://oauth2.googleapis.com/token")),
                f("clientId", "Client ID", false, false, json!("")),
                f("clientSecret", "Client Secret", true, false, json!("")),
                f("scope", "Scope", false, false, json!("https://www.googleapis.com/auth/drive.file https://www.googleapis.com/auth/spreadsheets https://www.googleapis.com/auth/drive.metadata")),
                f("authentication", "Authentication", false, false, json!("body")),
                // Sheets API base URL; overridden in tests so wiremock can stand in
                // for sheets.googleapis.com (mirrors slackOAuth2Api's `url` field).
                f("url", "Base URL", false, false, json!("https://sheets.googleapis.com")),
            ],
        },
        CredentialType {
            name: "googleApi",
            display_name: "Google Service Account API",
            fields: vec![
                f("email", "Service Account Email", false, true, json!("")),
                f("privateKey", "Private Key", true, true, json!("")),
                f("inpersonate", "Impersonate a User", false, false, json!(false)),
                f("delegatedEmail", "Email", false, false, json!("")),
                // Sheets API base URL and the OAuth2 token exchange URL; both
                // overridden in tests so wiremock can stand in for
                // sheets.googleapis.com and oauth2.googleapis.com.
                f("url", "Base URL", false, false, json!("https://sheets.googleapis.com")),
                f("tokenUrl", "Token URL", false, false, json!("https://oauth2.googleapis.com/token")),
            ],
        },
        CredentialType {
            name: "notionApi",
            display_name: "Notion API",
            fields: vec![
                f("apiKey", "Internal Integration Secret", true, true, json!("")),
                // Notion's base URL; overridden in tests so wiremock can
                // stand in for api.notion.com (mirrors slackApi's `url`
                // field).
                f("url", "Base URL", false, false, json!("https://api.notion.com")),
            ],
        },
        CredentialType {
            name: "airtableTokenApi",
            display_name: "Airtable Personal Access Token API",
            fields: vec![
                f("accessToken", "Access Token", true, true, json!("")),
                // Airtable's API base URL; overridden in tests so wiremock
                // can stand in for api.airtable.com (mirrors slackApi's
                // `url` field).
                f("url", "Base URL", false, false, json!("https://api.airtable.com/v0")),
            ],
        },
        CredentialType {
            name: "airtableOAuth2Api",
            display_name: "Airtable OAuth2 API",
            fields: vec![
                f("grantType", "Grant Type", false, false, json!("pkce")),
                f("authUrl", "Authorization URL", false, false, json!("https://airtable.com/oauth2/v1/authorize")),
                f("accessTokenUrl", "Access Token URL", false, false, json!("https://airtable.com/oauth2/v1/token")),
                f("clientId", "Client ID", false, false, json!("")),
                f("clientSecret", "Client Secret", true, false, json!("")),
                f("scope", "Scope", false, false, json!("schema.bases:read data.records:read data.records:write")),
                f("authentication", "Authentication", false, false, json!("header")),
                f("url", "Base URL", false, false, json!("https://api.airtable.com/v0")),
            ],
        },
        CredentialType {
            name: "githubApi",
            display_name: "GitHub API",
            fields: vec![
                f("server", "Github Server", false, false, json!("https://api.github.com")),
                f("user", "User", false, false, json!("")),
                f("accessToken", "Access Token", true, false, json!("")),
            ],
        },
        CredentialType {
            name: "githubOAuth2Api",
            display_name: "GitHub OAuth2 API",
            fields: vec![
                f("grantType", "Grant Type", false, false, json!("authorizationCode")),
                f("server", "Github Server", false, false, json!("https://api.github.com")),
                f("authUrl", "Authorization URL", false, true, json!("")),
                f("accessTokenUrl", "Access Token URL", false, true, json!("")),
                f("clientId", "Client ID", false, false, json!("")),
                f("clientSecret", "Client Secret", true, false, json!("")),
                f(
                    "scope",
                    "Scope",
                    false,
                    false,
                    json!("repo,admin:repo_hook,admin:org,admin:org_hook,gist,notifications,user,write:packages,read:packages,delete:packages,workflow"),
                ),
                f("authQueryParameters", "Auth URI Query Parameters", false, false, json!("")),
                f("authentication", "Authentication", false, false, json!("header")),
            ],
        },
    ]
}

pub fn get(name: &str) -> Option<CredentialType> {
    all().into_iter().find(|t| t.name == name)
}

/// Fields the system adds to stored data (never user input).
const SYSTEM_FIELDS: &[&str] = &["oauthTokenData"];

impl CredentialType {
    /// JSON schema as `GET /api/v1/credentials/schema/:type` serves it.
    pub fn json_schema(&self) -> Value {
        let props: Map<String, Value> = self.fields.iter().map(|f| (f.name.to_string(), json!({"type": f.kind}))).collect();
        let required: Vec<&str> = self.fields.iter().filter(|f| f.required).map(|f| f.name).collect();
        json!({"type": "object", "properties": props, "required": required, "additionalProperties": false})
    }

    /// Checks data from an API client.
    pub fn validate(&self, data: &Value) -> Result<(), String> {
        let obj = data.as_object().ok_or("request.body.data must be an object")?;
        for key in obj.keys() {
            if !self.fields.iter().any(|f| f.name == key) && !SYSTEM_FIELDS.contains(&key.as_str()) {
                return Err(format!("request.body.data is not allowed to have the additional property \"{key}\""));
            }
        }
        for f in self.fields.iter().filter(|f| f.required) {
            if !obj.contains_key(f.name) {
                return Err(format!("request.body.data requires property \"{}\"", f.name));
            }
        }
        Ok(())
    }

    /// Credential data with secrets blanked, as the editor receives it.
    pub fn redact(&self, data: &Value) -> Value {
        let mut out = Map::new();
        for (k, v) in data.as_object().into_iter().flatten() {
            let secret = SYSTEM_FIELDS.contains(&k.as_str()) || self.fields.iter().any(|f| f.name == k && f.secret);
            if secret {
                if !SYSTEM_FIELDS.contains(&k.as_str()) {
                    out.insert(k.clone(), json!(BLANK));
                }
            } else {
                out.insert(k.clone(), v.clone());
            }
        }
        Value::Object(out)
    }

    /// Description for the editor (`/types/credentials.json`).
    pub fn description(&self) -> Value {
        let properties: Vec<Value> = self
            .fields
            .iter()
            .map(|f| {
                let mut p = json!({"displayName": f.display, "name": f.name, "type": f.kind, "default": f.default});
                if f.secret {
                    p["typeOptions"] = json!({"password": true});
                }
                if f.required {
                    p["required"] = json!(true);
                }
                p
            })
            .collect();
        json!({"name": self.name, "displayName": self.display_name, "properties": properties})
    }
}
