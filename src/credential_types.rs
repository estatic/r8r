#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum FieldType {
    Text,
    Password,
}

#[derive(Debug, Clone, Copy, serde::Serialize)]
pub struct CredentialField {
    pub name: &'static str,
    pub label: &'static str,
    pub field_type: FieldType,
    pub required: bool,
}

#[derive(Debug, Clone, Copy, serde::Serialize)]
pub struct CredentialTypeSchema {
    pub credential_type: &'static str,
    pub display_name: &'static str,
    /// True for the three generic, auth-shape-driven types
    /// (`bearerToken`/`apiKeyHeader`/`basicAuth`) offered to any node
    /// that declares no fixed `credential_types` (see
    /// `core.httpRequest`); false for the three node-specific types.
    /// This is how the frontend's "+ New credential" dropdown decides
    /// which entries to offer an unrestricted node, without hardcoding
    /// type-name lists client-side.
    pub generic: bool,
    pub fields: &'static [CredentialField],
}

pub fn known_credential_types() -> &'static [CredentialTypeSchema] {
    &[
        CredentialTypeSchema {
            credential_type: "telegramApi",
            display_name: "Telegram Bot",
            generic: false,
            fields: &[CredentialField { name: "bot_token", label: "Bot Token", field_type: FieldType::Password, required: true }],
        },
        CredentialTypeSchema {
            credential_type: "anthropicApi",
            display_name: "Anthropic API",
            generic: false,
            fields: &[CredentialField { name: "api_key", label: "API Key", field_type: FieldType::Password, required: true }],
        },
        CredentialTypeSchema {
            credential_type: "openaiApi",
            display_name: "OpenAI API",
            generic: false,
            fields: &[
                CredentialField { name: "api_key", label: "API Key", field_type: FieldType::Password, required: true },
                CredentialField { name: "base_url", label: "Base URL (optional)", field_type: FieldType::Text, required: false },
            ],
        },
        CredentialTypeSchema {
            credential_type: "bearerToken",
            display_name: "Bearer Token",
            generic: true,
            fields: &[CredentialField { name: "token", label: "Token", field_type: FieldType::Password, required: true }],
        },
        CredentialTypeSchema {
            credential_type: "apiKeyHeader",
            display_name: "API Key (Header)",
            generic: true,
            fields: &[
                CredentialField { name: "header_name", label: "Header Name", field_type: FieldType::Text, required: true },
                CredentialField { name: "value", label: "Value", field_type: FieldType::Password, required: true },
            ],
        },
        CredentialTypeSchema {
            credential_type: "basicAuth",
            display_name: "Basic Auth",
            generic: true,
            fields: &[
                CredentialField { name: "username", label: "Username", field_type: FieldType::Text, required: true },
                CredentialField { name: "password", label: "Password (optional)", field_type: FieldType::Password, required: false },
            ],
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn find(credential_type: &str) -> &'static CredentialTypeSchema {
        known_credential_types()
            .iter()
            .find(|s| s.credential_type == credential_type)
            .unwrap_or_else(|| panic!("no schema registered for {credential_type}"))
    }

    #[test]
    fn telegram_api_requires_bot_token() {
        let schema = find("telegramApi");
        assert!(!schema.generic);
        assert_eq!(schema.fields.len(), 1);
        assert_eq!(schema.fields[0].name, "bot_token");
        assert_eq!(schema.fields[0].field_type, FieldType::Password);
        assert!(schema.fields[0].required);
    }

    #[test]
    fn anthropic_api_requires_api_key() {
        let schema = find("anthropicApi");
        assert!(!schema.generic);
        assert_eq!(schema.fields.len(), 1);
        assert_eq!(schema.fields[0].name, "api_key");
        assert_eq!(schema.fields[0].field_type, FieldType::Password);
        assert!(schema.fields[0].required);
    }

    #[test]
    fn openai_api_requires_api_key_and_has_optional_base_url() {
        let schema = find("openaiApi");
        assert!(!schema.generic);
        assert_eq!(schema.fields.len(), 2);
        assert_eq!(schema.fields[0].name, "api_key");
        assert!(schema.fields[0].required);
        assert_eq!(schema.fields[1].name, "base_url");
        assert_eq!(schema.fields[1].field_type, FieldType::Text);
        assert!(!schema.fields[1].required);
    }

    #[test]
    fn bearer_token_is_generic_and_requires_token() {
        let schema = find("bearerToken");
        assert!(schema.generic);
        assert_eq!(schema.fields.len(), 1);
        assert_eq!(schema.fields[0].name, "token");
        assert!(schema.fields[0].required);
    }

    #[test]
    fn api_key_header_is_generic_and_requires_header_name_and_value() {
        let schema = find("apiKeyHeader");
        assert!(schema.generic);
        assert_eq!(schema.fields.len(), 2);
        assert_eq!(schema.fields[0].name, "header_name");
        assert_eq!(schema.fields[0].field_type, FieldType::Text);
        assert!(schema.fields[0].required);
        assert_eq!(schema.fields[1].name, "value");
        assert_eq!(schema.fields[1].field_type, FieldType::Password);
        assert!(schema.fields[1].required);
    }

    #[test]
    fn basic_auth_is_generic_requires_username_password_optional() {
        let schema = find("basicAuth");
        assert!(schema.generic);
        assert_eq!(schema.fields.len(), 2);
        assert_eq!(schema.fields[0].name, "username");
        assert!(schema.fields[0].required);
        assert_eq!(schema.fields[1].name, "password");
        assert_eq!(schema.fields[1].field_type, FieldType::Password);
        assert!(!schema.fields[1].required);
    }
}
