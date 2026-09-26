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
    Field { name, display, secret, required, default }
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
            name: "telegramApi",
            display_name: "Telegram API",
            fields: vec![f("accessToken", "Access Token", true, true, json!("")), f("baseUrl", "Base URL", false, false, json!("https://api.telegram.org"))],
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
        let props: Map<String, Value> = self.fields.iter().map(|f| (f.name.to_string(), json!({"type": "string"}))).collect();
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
                let mut p = json!({"displayName": f.display, "name": f.name, "type": "string", "default": f.default});
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
