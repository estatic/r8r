use crate::domain::Item;
use crate::node::{Node, NodeError, NodeExecutionContext, NodeOutput};
use async_trait::async_trait;
use serde_json::Value;

pub struct MergeNode;

#[async_trait]
impl Node for MergeNode {
    fn type_name(&self) -> &'static str {
        "core.merge"
    }
    fn display_name(&self) -> &'static str {
        "Merge"
    }
    fn description(&self) -> &'static str {
        "Joins the items of its two inputs: one after the other, matched by fields or position, or one input's."
    }
    fn category(&self) -> crate::node::NodeCategory {
        crate::node::NodeCategory::FlowControl
    }
    fn icon(&self) -> &'static str {
        "🔗"
    }
    fn input_count(&self) -> usize {
        2
    }

    async fn execute(&self, ctx: &NodeExecutionContext) -> Result<NodeOutput, NodeError> {
        let p = &ctx.parameters;
        let mode = p.get("mode").and_then(Value::as_str).unwrap_or("append");
        // Input 1 and input 2 (an unconnected or untaken input is empty).
        let input = |i: usize| ctx.input_groups.get(i).cloned().unwrap_or_default();
        let (one, two) = if ctx.input_groups.is_empty() { (ctx.input_items.clone(), Vec::new()) } else { (input(0), input(1)) };
        let clash = Clash::from(p);
        let items = match mode {
            "append" | "waitForAll" => one.into_iter().chain(two).collect(),
            "chooseBranch" => match p.get("output_input").and_then(Value::as_str).unwrap_or("input1") {
                "input1" => one,
                "input2" => two,
                other => return Err(fail(format!("unknown input \"{other}\" (expected \"input1\" or \"input2\")"))),
            },
            "combine" => match p.get("combine_by").and_then(Value::as_str).unwrap_or("matchingFields") {
                "matchingFields" => by_fields(p, &one, &two, clash)?,
                "position" => {
                    let unpaired = p.get("include_unpaired").and_then(Value::as_bool).unwrap_or(false);
                    let n = if unpaired { one.len().max(two.len()) } else { one.len().min(two.len()) };
                    (0..n).map(|i| join(one.get(i), two.get(i), clash)).collect()
                }
                "allCombinations" => {
                    if one.len().saturating_mul(two.len()) > MAX_ITEMS {
                        return Err(fail(format!("all combinations of {} and {} items would be more than {MAX_ITEMS} items", one.len(), two.len())));
                    }
                    one.iter().flat_map(|a| two.iter().map(move |b| join(Some(a), Some(b), clash))).collect()
                }
                other => return Err(fail(format!("unknown combine_by \"{other}\" (expected matchingFields, position or allCombinations)"))),
            },
            "mergeByKey" => merge_by_key(p, &ctx.input_items)?,
            other => return Err(fail(format!("unknown mode \"{other}\" (expected append, combine or chooseBranch)"))),
        };
        Ok(vec![items])
    }
}

/// Most items a combine may produce, so one step can't exhaust memory.
const MAX_ITEMS: usize = 1_000_000;

fn fail(msg: impl std::fmt::Display) -> NodeError {
    NodeError::ExecutionFailed(format!("core.merge: {msg}"))
}

/// Which input's value wins when both items have a field.
#[derive(Clone, Copy)]
enum Clash {
    PreferInput1,
    PreferInput2,
}

impl Clash {
    fn from(p: &Value) -> Self {
        match p.get("clash").and_then(Value::as_str) {
            Some("preferInput1") => Clash::PreferInput1,
            _ => Clash::PreferInput2,
        }
    }
}

/// The fields of both items in one (either may be missing).
fn join(a: Option<&Item>, b: Option<&Item>, clash: Clash) -> Item {
    let fields = |i: Option<&Item>| i.and_then(|i| i.json.as_object().cloned()).unwrap_or_default();
    let (mut base, top) = match clash {
        Clash::PreferInput2 => (fields(a), fields(b)),
        Clash::PreferInput1 => (fields(b), fields(a)),
    };
    base.extend(top);
    Item { json: Value::Object(base), binary: serde_json::json!({}) }
}

/// A field by name, `a.b` reaching into nested objects.
fn field<'a>(item: &'a Item, path: &str) -> Option<&'a Value> {
    item.json.get(path).or_else(|| path.split('.').try_fold(&item.json, |v, k| v.get(k)))
}

/// Combine by matching fields: `match_fields: [{field1, field2}]` (all must
/// match), with `join_mode` deciding what is kept.
fn by_fields(p: &Value, one: &[Item], two: &[Item], clash: Clash) -> Result<Vec<Item>, NodeError> {
    let pairs: Vec<(String, String)> = p
        .get("match_fields")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|m| Some((m.get("field1")?.as_str()?.trim().to_string(), m.get("field2")?.as_str()?.trim().to_string())))
        .filter(|(a, b)| !a.is_empty() && !b.is_empty())
        .collect();
    if pairs.is_empty() {
        return Err(fail("choose the fields to match (input 1 field and input 2 field)"));
    }
    // Missing fields never match; numbers and their text form do ("7" = 7).
    let key = |v: &Value| match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    let matches = |a: &Item, b: &Item| {
        pairs.iter().all(|(f1, f2)| matches!((field(a, f1), field(b, f2)), (Some(x), Some(y)) if !x.is_null() && key(x) == key(y)))
    };
    let mut out = Vec::new();
    let mut two_matched = vec![false; two.len()];
    let mode = p.get("join_mode").and_then(Value::as_str).unwrap_or("keepMatches");
    match mode {
        "keepMatches" | "keepNonMatches" | "keepEverything" | "enrichInput1" => {
            let mut unmatched_one = Vec::new();
            for a in one {
                let mut found = false;
                for (j, b) in two.iter().enumerate() {
                    if matches(a, b) {
                        found = true;
                        two_matched[j] = true;
                        if mode != "keepNonMatches" {
                            out.push(join(Some(a), Some(b), clash));
                        }
                    }
                }
                if !found {
                    unmatched_one.push(a.clone());
                }
            }
            match mode {
                "keepNonMatches" | "keepEverything" => {
                    out.extend(unmatched_one);
                    out.extend(two.iter().zip(&two_matched).filter(|(_, m)| !**m).map(|(b, _)| b.clone()));
                }
                "enrichInput1" => {
                    // Every input-1 item, in its order: matched ones were joined above.
                    out = one
                        .iter()
                        .flat_map(|a| {
                            let joined: Vec<Item> = two.iter().filter(|b| matches(a, b)).map(|b| join(Some(a), Some(b), clash)).collect();
                            if joined.is_empty() { vec![a.clone()] } else { joined }
                        })
                        .collect();
                }
                _ => {}
            }
        }
        "enrichInput2" => {
            for b in two {
                let joined: Vec<Item> = one.iter().filter(|a| matches(a, b)).map(|a| join(Some(a), Some(b), clash)).collect();
                if joined.is_empty() {
                    out.push(b.clone());
                } else {
                    out.extend(joined);
                }
            }
        }
        other => {
            return Err(fail(format!(
                "unknown join_mode \"{other}\" (expected keepMatches, keepNonMatches, keepEverything, enrichInput1 or enrichInput2)"
            )))
        }
    }
    // Many-to-many matches multiply like combinations do.
    if out.len() > MAX_ITEMS {
        return Err(fail(format!("matching made more than {MAX_ITEMS} items; match on fields that are more unique")));
    }
    Ok(out)
}

/// Saved before the two-input modes: items of every input sharing `key`'s
/// value merged into one.
fn merge_by_key(p: &Value, items: &[Item]) -> Result<Vec<Item>, NodeError> {
    let key = p
        .get("key")
        .and_then(Value::as_str)
        .ok_or_else(|| NodeError::ExecutionFailed("mergeByKey mode requires a \"key\" parameter".into()))?;
    let mut order: Vec<String> = Vec::new();
    let mut grouped: std::collections::HashMap<String, serde_json::Map<String, Value>> = std::collections::HashMap::new();
    for item in items {
        let key_str = item.json.get(key).cloned().unwrap_or(Value::Null).to_string();
        let entry = grouped.entry(key_str.clone()).or_insert_with(|| {
            order.push(key_str.clone());
            serde_json::Map::new()
        });
        entry.extend(item.json.as_object().cloned().unwrap_or_default());
    }
    Ok(order
        .into_iter()
        .map(|k| Item { json: Value::Object(grouped.remove(&k).unwrap_or_default()), binary: serde_json::json!({}) })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn its(values: &[serde_json::Value]) -> Vec<Item> {
        values.iter().map(|v| Item { json: v.clone(), binary: serde_json::json!({}) }).collect()
    }

    async fn merge(params: serde_json::Value, one: &[serde_json::Value], two: &[serde_json::Value]) -> Result<Vec<serde_json::Value>, NodeError> {
        let ctx = NodeExecutionContext { parameters: params, input_groups: vec![its(one), its(two)], ..Default::default() };
        Ok(MergeNode.execute(&ctx).await?[0].iter().map(|i| i.json.clone()).collect())
    }

    fn by_id(join: &str) -> serde_json::Value {
        serde_json::json!({"mode": "combine", "combine_by": "matchingFields", "match_fields": [{"field1": "id", "field2": "user.id"}], "join_mode": join})
    }

    #[tokio::test]
    async fn combines_by_matching_fields_in_every_join_mode() {
        let one = [serde_json::json!({"id": 1, "name": "Ada"}), serde_json::json!({"id": 2, "name": "Bob"})];
        let two = [serde_json::json!({"user": {"id": "1"}, "city": "Oslo"}), serde_json::json!({"user": {"id": 3}, "city": "Rome"})];
        let ada_oslo = serde_json::json!({"id": 1, "name": "Ada", "user": {"id": "1"}, "city": "Oslo"});
        assert_eq!(merge(by_id("keepMatches"), &one, &two).await.unwrap(), vec![ada_oslo.clone()]);
        assert_eq!(merge(by_id("keepNonMatches"), &one, &two).await.unwrap(), vec![one[1].clone(), two[1].clone()]);
        assert_eq!(merge(by_id("keepEverything"), &one, &two).await.unwrap(), vec![ada_oslo.clone(), one[1].clone(), two[1].clone()]);
        assert_eq!(merge(by_id("enrichInput1"), &one, &two).await.unwrap(), vec![ada_oslo.clone(), one[1].clone()]);
        assert_eq!(merge(by_id("enrichInput2"), &one, &two).await.unwrap(), vec![ada_oslo, two[1].clone()]);
    }

    #[tokio::test]
    async fn input_2_wins_a_clash_unless_input_1_is_preferred() {
        let (one, two) = ([serde_json::json!({"k": 1, "v": "one"})], [serde_json::json!({"k": 1, "v": "two"})]);
        let mut p = serde_json::json!({"mode": "combine", "combine_by": "position"});
        assert_eq!(merge(p.clone(), &one, &two).await.unwrap(), vec![serde_json::json!({"k": 1, "v": "two"})]);
        p["clash"] = "preferInput1".into();
        assert_eq!(merge(p, &one, &two).await.unwrap(), vec![serde_json::json!({"k": 1, "v": "one"})]);
    }

    #[tokio::test]
    async fn combines_by_position_and_all_combinations() {
        let one = [serde_json::json!({"a": 1}), serde_json::json!({"a": 2})];
        let two = [serde_json::json!({"b": 1})];
        let pos = serde_json::json!({"mode": "combine", "combine_by": "position"});
        assert_eq!(merge(pos.clone(), &one, &two).await.unwrap(), vec![serde_json::json!({"a": 1, "b": 1})]);
        let mut unpaired = pos;
        unpaired["include_unpaired"] = true.into();
        assert_eq!(merge(unpaired, &one, &two).await.unwrap(), vec![serde_json::json!({"a": 1, "b": 1}), serde_json::json!({"a": 2})]);
        let all = serde_json::json!({"mode": "combine", "combine_by": "allCombinations"});
        assert_eq!(merge(all, &one, &[serde_json::json!({"b": 1}), serde_json::json!({"b": 2})]).await.unwrap().len(), 4);
    }

    #[tokio::test]
    async fn too_many_combinations_are_refused() {
        let many: Vec<serde_json::Value> = (0..1001).map(|i| serde_json::json!({"i": i})).collect();
        let err = merge(serde_json::json!({"mode": "combine", "combine_by": "allCombinations"}), &many, &many).await.unwrap_err();
        assert!(err.to_string().contains("more than 1000000 items"), "{err}");
    }

    #[tokio::test]
    async fn chooses_one_inputs_items() {
        let (one, two) = ([serde_json::json!({"a": 1})], [serde_json::json!({"b": 2})]);
        assert_eq!(merge(serde_json::json!({"mode": "chooseBranch", "output_input": "input2"}), &one, &two).await.unwrap(), vec![two[0].clone()]);
        assert_eq!(merge(serde_json::json!({"mode": "chooseBranch"}), &one, &two).await.unwrap(), vec![one[0].clone()]);
    }

    #[tokio::test]
    async fn matching_needs_fields_and_a_known_mode() {
        let err = merge(serde_json::json!({"mode": "combine"}), &[], &[]).await.unwrap_err();
        assert!(err.to_string().contains("choose the fields to match"), "{err}");
        assert!(merge(serde_json::json!({"mode": "zip"}), &[], &[]).await.is_err());
    }

    #[tokio::test]
    async fn append_mode_passes_items_through_unchanged() {
        let node = MergeNode;
        let items = vec![
            Item { json: serde_json::json!({"a": 1}), binary: serde_json::json!({}) },
            Item { json: serde_json::json!({"b": 2}), binary: serde_json::json!({}) },
        ];
        let ctx = NodeExecutionContext { parameters: serde_json::json!({"mode": "append"}), input_items: items.clone(), ..Default::default() };
        let result = node.execute(&ctx).await.unwrap();
        assert_eq!(result[0], items);
    }

    #[tokio::test]
    async fn default_mode_is_append() {
        let node = MergeNode;
        let items = vec![Item { json: serde_json::json!({"a": 1}), binary: serde_json::json!({}) }];
        let ctx = NodeExecutionContext { parameters: serde_json::json!({}), input_items: items.clone(), ..Default::default() };
        let result = node.execute(&ctx).await.unwrap();
        assert_eq!(result[0], items);
    }

    #[tokio::test]
    async fn wait_for_all_mode_behaves_like_append() {
        let node = MergeNode;
        let items = vec![Item { json: serde_json::json!({"a": 1}), binary: serde_json::json!({}) }];
        let ctx = NodeExecutionContext { parameters: serde_json::json!({"mode": "waitForAll"}), input_items: items.clone(), ..Default::default() };
        let result = node.execute(&ctx).await.unwrap();
        assert_eq!(result[0], items);
    }

    #[tokio::test]
    async fn merge_by_key_combines_items_sharing_a_key_value() {
        let node = MergeNode;
        let items = vec![
            Item { json: serde_json::json!({"id": 1, "name": "Ada"}), binary: serde_json::json!({}) },
            Item { json: serde_json::json!({"id": 1, "age": 30}), binary: serde_json::json!({}) },
            Item { json: serde_json::json!({"id": 2, "name": "Grace"}), binary: serde_json::json!({}) },
        ];
        let ctx = NodeExecutionContext {
            parameters: serde_json::json!({"mode": "mergeByKey", "key": "id"}),
            input_items: items,
            ..Default::default()
        };
        let result = node.execute(&ctx).await.unwrap();
        assert_eq!(result[0].len(), 2);
        assert_eq!(result[0][0].json, serde_json::json!({"id": 1, "name": "Ada", "age": 30}));
        assert_eq!(result[0][1].json, serde_json::json!({"id": 2, "name": "Grace"}));
    }

    #[tokio::test]
    async fn merge_by_key_without_key_parameter_returns_error() {
        let node = MergeNode;
        let ctx = NodeExecutionContext {
            parameters: serde_json::json!({"mode": "mergeByKey"}),
            input_items: vec![],
            ..Default::default()
        };
        let result = node.execute(&ctx).await;
        assert!(matches!(result, Err(NodeError::ExecutionFailed(_))));
    }
}
