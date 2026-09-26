//! Credentials: creating them (API or CLI import), n8n-encrypted fixtures,
//! and checking exports.

use super::{docstring, table};
use crate::support::json::{assert_matches, parse_strict, Mode};
use crate::support::n8n_crypto;
use crate::support::process::ENCRYPTION_KEY;
use crate::world::{Auth, R8rWorld};
use cucumber::gherkin::Step;
use cucumber::{given, then};
use serde_json::{json, Value};

/// With a server running: `POST /api/v1/credentials`. Headless: written to a
/// file and loaded with `r8r import:credentials --input=<file>` (n8n's CLI
/// encrypts plain-object `data` on import). Either way the id is remembered
/// as `%{CREDENTIAL_ID:<name>}`.
#[given(expr = "the credential {string} of type {string} with the data:")]
async fn credential(w: &mut R8rWorld, name: String, cred_type: String, step: &Step) {
    let data = parse_strict(&w.expand(docstring(step)), "credential data");
    if w.servers.contains_key("main") {
        let body = json!({"name": name, "type": cred_type, "data": data});
        let resp = super::api::call(w, Auth::ApiKey("owner".into()), "POST", "/api/v1/credentials", Some(body)).await;
        let id = serde_json::from_str::<Value>(&resp.body)
            .ok()
            .and_then(|j| j["id"].as_str().map(String::from))
            .unwrap_or_else(|| panic!("creating credential {name} failed:\n{}", resp.describe()));
        w.vars.insert(format!("CREDENTIAL_ID:{name}"), id);
        return;
    }
    let id = format!("bdd{}", &uuid::Uuid::new_v4().simple().to_string()[..13]);
    let file = w.dir.path().join(format!("credential-{id}.json"));
    let export = json!([{"id": id, "name": name, "type": cred_type, "data": data}]);
    std::fs::write(&file, serde_json::to_vec_pretty(&export).unwrap()).unwrap();
    let args = vec!["import:credentials".to_string(), format!("--input={}", file.display())];
    let out = w.cli(&args, super::cli::DEFAULT_TIMEOUT).await;
    assert_eq!(out.code, Some(0), "credential import failed:\n{}", out.describe());
    w.vars.insert(format!("CREDENTIAL_ID:{name}"), id);
}

/// Writes an `export:credentials`-style file whose `data` fields are
/// encrypted exactly as n8n does, with `key` (`default` = the harness's
/// `N8N_ENCRYPTION_KEY`). Table: id | name | type | data (JSON).
#[given(expr = "an n8n credentials export {string} encrypted with the key {string}:")]
async fn n8n_export(w: &mut R8rWorld, file: String, key: String, step: &Step) {
    let key = if key == "default" { ENCRYPTION_KEY.to_string() } else { key };
    let rows = table(step);
    let header = &rows[0];
    let col = |row: &Vec<String>, name: &str| row[header.iter().position(|h| h == name).expect(name)].trim().to_string();
    let mut out = Vec::new();
    for row in &rows[1..] {
        let data = col(row, "data");
        parse_strict(&data, "credential data");
        out.push(json!({
            "id": col(row, "id"),
            "name": col(row, "name"),
            "type": col(row, "type"),
            "data": n8n_crypto::encrypt(&key, &data),
            "isManaged": false,
        }));
    }
    std::fs::write(w.dir.path().join(file), serde_json::to_vec_pretty(&out).unwrap()).unwrap();
}

fn find_credential(w: &R8rWorld, file: &str, name: &str) -> Value {
    let (_, json) = super::cli::read_json_file(w, file);
    let list = match &json {
        Value::Array(a) => a.clone(),
        other => vec![other.clone()],
    };
    list.into_iter().find(|c| c["name"] == name).unwrap_or_else(|| panic!("no credential {name:?} in {file}: {json}"))
}

#[then(expr = "the file {string} has the credential {string} with the decrypted data:")]
async fn decrypted_export(w: &mut R8rWorld, file: String, name: String, step: &Step) {
    let cred = find_credential(w, &file, &name);
    let expected = parse_strict(docstring(step), "expected data");
    assert_matches(&expected, &cred["data"], Mode::Exact).unwrap_or_else(|e| panic!("{e}\ncredential: {cred}"));
}

/// Checks the export is n8n-readable: CryptoJS AES with the given key.
#[then(expr = "the file {string} has the credential {string} whose data n8n can decrypt with the key {string} to:")]
async fn n8n_decryptable(w: &mut R8rWorld, file: String, name: String, key: String, step: &Step) {
    let key = if key == "default" { ENCRYPTION_KEY.to_string() } else { key };
    let cred = find_credential(w, &file, &name);
    let blob = cred["data"].as_str().unwrap_or_else(|| panic!("data is not an encrypted string: {cred}"));
    let plain = n8n_crypto::decrypt(&key, blob).unwrap_or_else(|e| panic!("n8n could not decrypt {blob:?}: {e}"));
    let actual = parse_strict(&plain, "decrypted data");
    let expected = parse_strict(docstring(step), "expected data");
    assert_matches(&expected, &actual, Mode::Exact).unwrap_or_else(|e| panic!("{e}"));
}
