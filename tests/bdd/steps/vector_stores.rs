//! Set-up and read-back of external vector stores (`10-ai/vector_store_*`).

use crate::world::R8rWorld;
use cucumber::{given, then};

fn qdrant_url() -> String {
    std::env::var("R8R_BDD_QDRANT_URL").unwrap_or_else(|_| "http://127.0.0.1:6333".into())
}

#[given(expr = "the Qdrant collection {string} does not exist")]
async fn drop_qdrant_collection(_w: &mut R8rWorld, name: String) {
    let resp = reqwest::Client::new().delete(format!("{}/collections/{name}", qdrant_url())).send().await.expect("reach the bdd qdrant instance");
    assert!(resp.status().is_success(), "deleting collection {name}: {}", resp.status());
}

#[given(expr = "the Qdrant collection {string} exists with {int}-dimensional cosine vectors")]
async fn create_qdrant_collection(w: &mut R8rWorld, name: String, size: u64) {
    drop_qdrant_collection(w, name.clone()).await;
    let resp = reqwest::Client::new()
        .put(format!("{}/collections/{name}", qdrant_url()))
        .json(&serde_json::json!({"vectors": {"size": size, "distance": "Cosine"}}))
        .send()
        .await
        .expect("reach the bdd qdrant instance");
    assert!(resp.status().is_success(), "creating collection {name}: {}", resp.status());
}

#[then(expr = "the Qdrant collection {string} has {int} points of {int} dimensions compared by {string}")]
async fn qdrant_collection_info(_w: &mut R8rWorld, name: String, points: u64, size: u64, distance: String) {
    let info: serde_json::Value = reqwest::get(format!("{}/collections/{name}", qdrant_url())).await.expect("reach the bdd qdrant instance").json().await.unwrap();
    let r = &info["result"];
    assert_eq!(r["points_count"].as_u64(), Some(points), "{info}");
    assert_eq!(r.pointer("/config/params/vectors/size").and_then(|v| v.as_u64()), Some(size), "{info}");
    assert_eq!(r.pointer("/config/params/vectors/distance").and_then(|v| v.as_str()), Some(distance.as_str()), "{info}");
}
