use crate::domain::Workflow;
use crate::storage::Storage;
use std::collections::HashMap;
use uuid::Uuid;

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
}
