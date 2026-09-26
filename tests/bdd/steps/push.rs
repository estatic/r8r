//! Editor push channel (`/rest/push`, spec §6.9): WebSocket messages
//! `{ "type": ..., "data": ... }` as n8n sends them.

use crate::world::R8rWorld;
use cucumber::{given, then};
use futures_util::StreamExt;
use serde_json::Value;
use std::time::Duration;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

#[given(expr = "I am connected to the push channel")]
async fn connect(w: &mut R8rWorld) {
    let push_ref = format!("bdd-{}", uuid::Uuid::new_v4().simple());
    w.vars.insert("PUSH_REF".into(), push_ref.clone());
    let url = format!("ws://127.0.0.1:{}/rest/push?pushRef={push_ref}", w.server().port);
    let mut request = url.clone().into_client_request().unwrap();
    if let Some(cookie) = w.sessions.get("owner") {
        request.headers_mut().insert("cookie", cookie.parse().unwrap());
    }
    request.headers_mut().insert("origin", format!("http://127.0.0.1:{}", w.server().port).parse().unwrap());
    let (stream, _) = tokio_tungstenite::connect_async(request)
        .await
        .unwrap_or_else(|e| panic!("cannot open push WebSocket {url}: {e}"));
    let messages = w.push_messages.clone();
    w.push_task = Some(tokio::spawn(async move {
        let (_, mut read) = stream.split();
        while let Some(Ok(msg)) = read.next().await {
            if let Ok(text) = msg.into_text() {
                // n8n may batch several messages in one frame as an array.
                match serde_json::from_str::<Value>(&text) {
                    Ok(Value::Array(many)) => messages.lock().unwrap().extend(many),
                    Ok(one) => messages.lock().unwrap().push(one),
                    Err(_) => {}
                }
            }
        }
    }));
}

fn types(w: &R8rWorld) -> Vec<String> {
    w.push_messages.lock().unwrap().iter().filter_map(|m| m["type"].as_str().map(String::from)).collect()
}

fn is_subsequence(needle: &[String], hay: &[String]) -> bool {
    let mut it = hay.iter();
    needle.iter().all(|n| it.any(|h| h == n))
}

#[then(expr = "I receive the push messages in order:")]
async fn in_order(w: &mut R8rWorld, step: &cucumber::gherkin::Step) {
    let expected = super::list(step, None);
    let ok = super::eventually(Duration::from_secs(15), || {
        let t = types(w);
        let e = expected.clone();
        async move { is_subsequence(&e, &t).then_some(()) }
    })
    .await;
    assert!(ok.is_some(), "expected push messages {expected:?} in order; got {:?}", types(w));
}

#[then(expr = "a {string} push message names the node {string}")]
async fn names_node(w: &mut R8rWorld, ty: String, node: String) {
    let ok = super::eventually(Duration::from_secs(15), || {
        let msgs = w.push_messages.lock().unwrap().clone();
        let (ty, node) = (ty.clone(), node.clone());
        async move { msgs.iter().any(|m| m["type"] == ty.as_str() && m["data"]["nodeName"] == node.as_str()).then_some(()) }
    })
    .await;
    assert!(ok.is_some(), "no {ty} message for {node}; got {:?}", types(w));
}
