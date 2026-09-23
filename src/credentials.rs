use crate::domain::Workflow;
use crate::storage::Storage;
use std::collections::HashMap;
use uuid::Uuid;
use crate::credential_types::{known_credential_types, CredentialTypeSchema, FieldType};

pub async fn resolve_credentials_for_workflow(
    storage: &dyn Storage,
    workflow: &Workflow,
) -> anyhow::Result<HashMap<Uuid, serde_json::Value>> {
    let mut ids = std::collections::HashSet::new();
    for node in &workflow.nodes {
        if let Some(id_str) = node.parameters.get("auth").and_then(|a| a.get("credential_id")).and_then(|v| v.as_str()) {
            let id = Uuid::parse_str(id_str)
                .map_err(|e| anyhow::anyhow!("node {} has an invalid credential_id: {e}", node.id))?;
            ids.insert(id);
        }
    }

    let mut resolved = HashMap::new();
    for id in ids {
        let credential = storage
            .get_credential(id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("referenced credential {id} does not exist"))?;
        resolved.insert(id, credential.data);
    }
    Ok(resolved)
}

/// The field schema for a credential type, if it is one of the known types.
pub fn schema_for(credential_type: &str) -> Option<&'static CredentialTypeSchema> {
    known_credential_types().iter().find(|s| s.credential_type == credential_type)
}

/// `(id, name)` of every workflow with a node whose
/// `parameters.auth.credential_id` is `id` -- the field
/// `resolve_credentials_for_workflow` reads.
pub fn workflows_using_credential(workflows: &[crate::domain::Workflow], id: Uuid) -> Vec<(Uuid, String)> {
    let id = id.to_string();
    workflows
        .iter()
        .filter(|wf| {
            wf.nodes.iter().any(|n| {
                n.parameters.get("auth").and_then(|a| a.get("credential_id")).and_then(|v| v.as_str()) == Some(id.as_str())
            })
        })
        .map(|wf| (wf.id, wf.name.clone()))
        .collect()
}

/// Applies a PATCH body to stored credential data. Typed (schema known):
/// per schema field, a non-blank patch value replaces the stored one;
/// `""`/absent keeps it; `null` removes an optional field; other keys are
/// ignored. Untyped: replace.
pub fn merge_credential_data(
    schema: Option<&CredentialTypeSchema>,
    stored: &serde_json::Value,
    patch: &serde_json::Value,
) -> serde_json::Value {
    let Some(schema) = schema else {
        return patch.clone();
    };
    let mut merged = stored.as_object().cloned().unwrap_or_default();
    for field in schema.fields {
        match patch.get(field.name) {
            None => {}
            // Explicit null clears an optional field (the form sends it for
            // a visible value the user emptied); a required one is kept.
            Some(serde_json::Value::Null) => {
                if !field.required {
                    merged.remove(field.name);
                }
            }
            Some(serde_json::Value::String(s)) if s.is_empty() => {}
            Some(v) => {
                merged.insert(field.name.to_string(), v.clone());
            }
        }
    }
    serde_json::Value::Object(merged)
}

/// Stored values of the schema's text fields only -- never password
/// fields, and nothing for an untyped credential.
pub fn non_secret_fields(
    schema: Option<&CredentialTypeSchema>,
    stored: &serde_json::Value,
) -> serde_json::Map<String, serde_json::Value> {
    let mut out = serde_json::Map::new();
    if let Some(schema) = schema {
        for field in schema.fields.iter().filter(|f| matches!(f.field_type, FieldType::Text)) {
            if let Some(v) = stored.get(field.name) {
                out.insert(field.name.to_string(), v.clone());
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{Credential, NodeInstance, User, UserRole};
    use crate::storage::sqlite::SqliteStorage;
    use chrono::Utc;

    async fn test_storage() -> SqliteStorage {
        SqliteStorage::new("sqlite::memory:", [0u8; 32]).await.unwrap()
    }

    async fn create_test_user(storage: &SqliteStorage, id: Uuid) {
        let user = User {
            id,
            email: format!("{id}@example.com"),
            password_hash: "irrelevant-for-this-test".into(),
            role: UserRole::Owner,
            created_at: Utc::now(),
        };
        storage.create_user(&user).await.unwrap();
    }

    fn node_with_credential(id: Uuid) -> NodeInstance {
        NodeInstance {
            id: "http1".into(),
            node_type: "core.httpRequest".into(),
            position: (0.0, 0.0),
            parameters: serde_json::json!({"auth": {"type": "bearer", "credential_id": id.to_string()}}),
            disabled: false,
            settings: Default::default(),
        }
    }

    fn workflow_with_nodes(nodes: Vec<NodeInstance>) -> Workflow {
        let now = Utc::now();
        Workflow { id: Uuid::new_v4(), name: "wf".into(), active: false, nodes, connections: vec![], created_at: now, updated_at: now }
    }

    #[tokio::test]
    async fn resolves_a_referenced_credential() {
        let storage = test_storage().await;
        let owner_id = Uuid::new_v4();
        create_test_user(&storage, owner_id).await;

        let cred = Credential {
            id: Uuid::new_v4(),
            name: "c1".into(),
            credential_type: "bearer".into(),
            data: serde_json::json!({"token": "secret-value"}),
            owner_id,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        storage.create_credential(&cred).await.unwrap();

        let wf = workflow_with_nodes(vec![node_with_credential(cred.id)]);
        let resolved = resolve_credentials_for_workflow(&storage, &wf).await.unwrap();
        assert_eq!(resolved.get(&cred.id), Some(&serde_json::json!({"token": "secret-value"})));
    }

    #[tokio::test]
    async fn workflow_with_no_credential_references_resolves_to_empty_map() {
        let storage = test_storage().await;
        let wf = workflow_with_nodes(vec![NodeInstance {
            id: "set1".into(),
            node_type: "core.set".into(),
            position: (0.0, 0.0),
            parameters: serde_json::json!({}),
            disabled: false,
            settings: Default::default(),
        }]);
        let resolved = resolve_credentials_for_workflow(&storage, &wf).await.unwrap();
        assert!(resolved.is_empty());
    }

    #[tokio::test]
    async fn referencing_a_nonexistent_credential_returns_error() {
        let storage = test_storage().await;
        let wf = workflow_with_nodes(vec![node_with_credential(Uuid::new_v4())]);
        let result = resolve_credentials_for_workflow(&storage, &wf).await;
        assert!(result.is_err());
    }

    fn wf_with_auth(name: &str, credential_id: Option<&str>) -> crate::domain::Workflow {
        let params = match credential_id {
            Some(id) => serde_json::json!({"auth": {"type": "bearer", "credential_id": id}}),
            None => serde_json::json!({"url": "https://example.com"}),
        };
        crate::domain::Workflow {
            id: Uuid::new_v4(),
            name: name.into(),
            active: false,
            nodes: vec![crate::domain::NodeInstance {
                id: "n1".into(),
                node_type: "core.httpRequest".into(),
                position: (0.0, 0.0),
                parameters: params,
                disabled: false,
                settings: Default::default(),
            }],
            connections: vec![],
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn finds_workflows_that_reference_the_credential() {
        let id = Uuid::new_v4();
        let other = Uuid::new_v4();
        let using = wf_with_auth("uses it", Some(&id.to_string()));
        let workflows = vec![using.clone(), wf_with_auth("other cred", Some(&other.to_string())), wf_with_auth("no auth", None)];
        assert_eq!(workflows_using_credential(&workflows, id), vec![(using.id, "uses it".to_string())]);
    }

    #[test]
    fn typed_merge_keeps_blank_fields_and_ignores_unknown_keys() {
        let schema = schema_for("apiKeyHeader");
        let stored = serde_json::json!({"header_name": "X-Key", "value": "old-secret"});
        let patch = serde_json::json!({"header_name": "X-Api-Key", "value": "", "extra": "nope"});
        assert_eq!(
            merge_credential_data(schema, &stored, &patch),
            serde_json::json!({"header_name": "X-Api-Key", "value": "old-secret"})
        );
        let patch = serde_json::json!({"value": "new-secret"});
        assert_eq!(
            merge_credential_data(schema, &stored, &patch),
            serde_json::json!({"header_name": "X-Key", "value": "new-secret"})
        );
    }

    #[test]
    fn null_clears_an_optional_field_but_never_a_required_one() {
        let schema = schema_for("openaiApi");
        let stored = serde_json::json!({"api_key": "sk-1", "base_url": "http://local:8080"});
        let patch = serde_json::json!({"base_url": null});
        assert_eq!(merge_credential_data(schema, &stored, &patch), serde_json::json!({"api_key": "sk-1"}));
        let patch = serde_json::json!({"api_key": null});
        assert_eq!(merge_credential_data(schema, &stored, &patch), stored);
    }

    #[test]
    fn untyped_merge_replaces_everything() {
        let stored = serde_json::json!({"a": 1, "b": 2});
        let patch = serde_json::json!({"c": 3});
        assert_eq!(merge_credential_data(None, &stored, &patch), serde_json::json!({"c": 3}));
    }

    #[test]
    fn merge_into_non_object_starts_fresh() {
        let schema = schema_for("bearerToken");
        let stored = serde_json::json!("legacy string");
        let patch = serde_json::json!({"token": "t"});
        assert_eq!(merge_credential_data(schema, &stored, &patch), serde_json::json!({"token": "t"}));
    }

    #[test]
    fn non_secret_fields_returns_text_fields_only() {
        let schema = schema_for("apiKeyHeader");
        let stored = serde_json::json!({"header_name": "X-Key", "value": "secret"});
        let fields = non_secret_fields(schema, &stored);
        assert_eq!(fields.get("header_name"), Some(&serde_json::json!("X-Key")));
        assert!(!fields.contains_key("value"));
        assert!(non_secret_fields(None, &stored).is_empty());
    }
}
