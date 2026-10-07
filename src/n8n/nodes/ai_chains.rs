//! AI chain root nodes that turn text into structured answers with the
//! connected chat model (spec §6.8): Sentiment Analysis, and the shared
//! LangChain structured-output machinery they use. Prompts, format
//! instructions and parsing follow n8n 2.35.7 / LangChain exactly, checked
//! against captured n8n requests (`10-ai/*.feature`).

use super::ai::load_model;
use crate::n8n::node::{ExecCtx, NodeError, NodeResult, NodeType};
use crate::n8n::types::{Item, NodeOutput};
use crate::n8n::workflow::Node;
use serde_json::{json, Map, Value};

pub fn all() -> Vec<Box<dyn NodeType>> {
    vec![Box::new(SentimentAnalysis)]
}

// ---- structured output (LangChain StructuredOutputParser) ---------------------

/// `StructuredOutputParser.getFormatInstructions()`: the doubled braces are
/// literal (they reach the model as written).
fn format_instructions(schema: &Value) -> String {
    let mut s = schema.clone();
    if let Some(o) = s.as_object_mut() {
        o.insert("$schema".into(), json!("http://json-schema.org/draft-07/schema#"));
    }
    format!(
        "You must format your output as a JSON value that adheres to a given \"JSON Schema\" instance.\n\n\"JSON Schema\" is a declarative language that allows you to annotate and validate JSON documents.\n\nFor example, the example \"JSON Schema\" instance {{{{\"properties\": {{{{\"foo\": {{{{\"description\": \"a list of test words\", \"type\": \"array\", \"items\": {{{{\"type\": \"string\"}}}}}}}}}}}}, \"required\": [\"foo\"]}}}}}}}}\nwould match an object with one required property, \"foo\". The \"type\" property specifies \"foo\" must be an \"array\", and the \"description\" property semantically describes it as \"a list of test words\". The items within \"foo\" must be strings.\nThus, the object {{{{\"foo\": [\"bar\", \"baz\"]}}}} is a well-formatted instance of this example \"JSON Schema\". The object {{{{\"properties\": {{{{\"foo\": [\"bar\", \"baz\"]}}}}}}}} is not well-formatted.\n\nYour output will be parsed and type-checked according to the provided schema instance, so make sure all fields in your output match the schema exactly and there are no trailing commas!\n\nHere is the JSON Schema instance your output must adhere to. Include the enclosing markdown codeblock:\n```json\n{}\n```\n",
        serde_json::to_string(&s).unwrap()
    )
}

/// `StructuredOutputParser.parse`: the JSON inside a ```json fence (or the
/// whole text), newlines inside strings escaped and the rest dropped, then
/// checked against the schema (unknown keys dropped, as zod does).
fn parse_structured(text: &str, schema: &Value) -> Result<Value, String> {
    let trimmed = text.trim();
    let fenced = regex::Regex::new(r"^```(?:json)?\s*([\s\S]*?)```").unwrap().captures(trimmed).map(|c| c[1].to_string());
    let fenced = fenced.or_else(|| regex::Regex::new(r"```json\s*([\s\S]*?)```").unwrap().captures(trimmed).map(|c| c[1].to_string()));
    let body = fenced.unwrap_or_else(|| trimmed.to_string());
    let strings = regex::Regex::new(r#""([^"\\]*(\\.[^"\\]*)*)""#).unwrap();
    let escaped = strings.replace_all(&body, |c: &regex::Captures| format!("\"{}\"", c[1].replace('\n', "\\n"))).replace('\n', "");
    let value: Value = serde_json::from_str(&escaped).map_err(|e| format!("Failed to parse. Text: \"{text}\". Error: SyntaxError: {e}"))?;
    validate(&value, schema, "").map_err(|e| format!("Failed to parse. Text: \"{text}\". Error: {e}"))
}

/// The zod checks n8n's schemas use: types, enums, bounds, required keys.
fn validate(v: &Value, schema: &Value, path: &str) -> Result<Value, String> {
    let at = if path.is_empty() { "the value".to_string() } else { format!("\"{path}\"") };
    if let Some(options) = schema["enum"].as_array() {
        if !options.contains(v) {
            return Err(format!("Invalid enum value at {at}: expected one of {}", serde_json::to_string(options).unwrap()));
        }
    }
    match schema["type"].as_str() {
        Some("object") => {
            let o = v.as_object().ok_or_else(|| format!("Expected object at {at}"))?;
            let mut out = Map::new();
            let props = schema["properties"].as_object().cloned().unwrap_or_default();
            for key in schema["required"].as_array().into_iter().flatten().filter_map(Value::as_str) {
                if !o.contains_key(key) {
                    return Err(format!("Required at \"{}\"", if path.is_empty() { key.to_string() } else { format!("{path}.{key}") }));
                }
            }
            for (k, sub) in &props {
                if let Some(x) = o.get(k) {
                    out.insert(k.clone(), validate(x, sub, &if path.is_empty() { k.clone() } else { format!("{path}.{k}") })?);
                }
            }
            if schema.get("properties").is_none() {
                return Ok(v.clone());
            }
            Ok(Value::Object(out))
        }
        Some("array") => {
            let a = v.as_array().ok_or_else(|| format!("Expected array at {at}"))?;
            a.iter().enumerate().map(|(i, x)| validate(x, &schema["items"], &format!("{path}[{i}]"))).collect::<Result<Vec<_>, _>>().map(Value::Array)
        }
        Some("number") | Some("integer") => {
            let n = v.as_f64().ok_or_else(|| format!("Expected number at {at}"))?;
            if schema["minimum"].as_f64().is_some_and(|m| n < m) || schema["maximum"].as_f64().is_some_and(|m| n > m) {
                return Err(format!("Number out of range at {at}"));
            }
            Ok(v.clone())
        }
        Some("string") => v.as_str().map(|_| v.clone()).ok_or_else(|| format!("Expected string at {at}")),
        Some("boolean") => v.as_bool().map(|_| v.clone()).ok_or_else(|| format!("Expected boolean at {at}")),
        _ => Ok(v.clone()),
    }
}

/// LangChain's `NAIVE_FIX_TEMPLATE` (`OutputFixingParser`).
fn fix_prompt(instructions: &str, completion: &str, error: &str) -> String {
    format!("Instructions:\n--------------\n{instructions}\n--------------\nCompletion:\n--------------\n{completion}\n--------------\n\nAbove, the Completion did not satisfy the constraints given in the Instructions.\nError:\n--------------\n{error}\n--------------\n\nPlease try again. Please only respond with an answer that satisfies the constraints laid out in the Instructions:")
}

/// Runs `messages` through the model and parses the answer against
/// `schema`; with `auto_fix`, one more model call asks it to fix an answer
/// that doesn't parse (`OutputFixingParser`).
async fn structured_call(ctx: &ExecCtx<'_>, item: usize, messages: &[Value], schema: &Value, auto_fix: bool) -> NodeResult<Value> {
    let model = load_model(ctx, item).await?;
    let reply = model.chat(ctx, messages, &[]).await?;
    let text = reply["content"].as_str().unwrap_or("").to_string();
    match parse_structured(&text, schema) {
        Ok(v) => Ok(v),
        Err(e) if auto_fix => {
            let prompt = fix_prompt(&format_instructions(schema), &text, &format!("OutputParserException: {e}"));
            let fixed = model.chat(ctx, &[json!({"role": "user", "content": prompt})], &[]).await?;
            parse_structured(fixed["content"].as_str().unwrap_or(""), schema).map_err(NodeError::new)
        }
        Err(e) => Err(NodeError::new(e)),
    }
}

/// The comma-separated category list of a node's raw parameters (outputs
/// are known before any expression runs, as in n8n's `configuredOutputs`).
fn categories(raw: &str) -> Vec<String> {
    raw.split(',').map(|c| c.trim().to_string()).filter(|c| !c.is_empty()).collect()
}

// ---- Sentiment Analysis ----------------------------------------------------------

const SENTIMENT_SYSTEM: &str = "You are highly intelligent and accurate sentiment analyzer. Analyze the sentiment of the provided text. Categorize it into one of the following: {categories}. Use the provided formatting instructions. Only output the JSON.";
const SENTIMENT_CATEGORIES: &str = "Positive, Neutral, Negative";

struct SentimentAnalysis;

#[async_trait::async_trait]
impl NodeType for SentimentAnalysis {
    fn type_name(&self) -> &'static str {
        "@n8n/n8n-nodes-langchain.sentimentAnalysis"
    }

    fn outputs(&self, node: &Node) -> usize {
        categories(node.parameters.pointer("/options/categories").and_then(Value::as_str).unwrap_or(SENTIMENT_CATEGORIES)).len().max(1)
    }

    async fn execute(&self, ctx: &mut ExecCtx<'_>) -> NodeResult<NodeOutput> {
        let mut out: NodeOutput = vec![Vec::new(); self.outputs(ctx.node)];
        for i in 0..ctx.input().len() {
            match sentiment(ctx, i).await {
                Ok((index, item)) => {
                    if let Some(o) = out.get_mut(index) {
                        o.push(item);
                    }
                }
                Err(e) if ctx.continue_on_fail() => ctx.push_error_item(&e.at(i), i),
                Err(e) => return Err(e.at(i)),
            }
        }
        Ok(out)
    }
}

async fn sentiment(ctx: &ExecCtx<'_>, i: usize) -> NodeResult<(usize, Item)> {
    let raw_categories = ctx.param_str("options.categories", i, SENTIMENT_CATEGORIES)?;
    let cats = categories(&raw_categories);
    if cats.is_empty() {
        return Err(NodeError::new("No sentiment categories provided"));
    }
    let schema = json!({
        "type": "object",
        "properties": {
            "sentiment": {"type": "string", "enum": cats},
            "strength": {"type": "number", "minimum": 0, "maximum": 1, "description": "Strength score for sentiment in relation to the category"},
            "confidence": {"type": "number", "minimum": 0, "maximum": 1},
        },
        "required": ["sentiment", "strength", "confidence"],
        "additionalProperties": false,
    });
    // n8n escapes the template's braces, then re-opens `{categories}`; the
    // format instructions follow after an indent that depends on the
    // node version's code path (batched from v1.1).
    let template = ctx.param_str("options.systemPromptTemplate", i, SENTIMENT_SYSTEM)?;
    let indent = if ctx.node.type_version >= 1.1 && ctx.param_f64("options.batching.batchSize", 0, 5.0)? > 1.0 { "\t\t\t\t" } else { "\t\t\t" };
    let system = format!("{}\n{indent}{}", template.replace("{categories}", &raw_categories), format_instructions(&schema));
    let input = ctx.param_str("inputText", i, "")?;
    let messages = [json!({"role": "system", "content": system}), json!({"role": "user", "content": input})];
    let auto_fix = ctx.param_bool("options.enableAutoFixing", i, false)?;
    let output = structured_call(ctx, i, &messages, &schema, auto_fix)
        .await
        .map_err(|_| NodeError::new("Error during parsing of LLM output, please check your LLM model and configuration"))?;
    let found = output["sentiment"].as_str().unwrap_or("");
    let index = cats.iter().position(|c| c.eq_ignore_ascii_case(found)).ok_or_else(|| NodeError::new("Error during parsing of LLM output, please check your LLM model and configuration"))?;
    let mut analysis = Map::from_iter([("category".to_string(), json!(found))]);
    if ctx.param_bool("options.includeDetailedResults", i, false)? {
        analysis.insert("strength".into(), output["strength"].clone());
        analysis.insert("confidence".into(), output["confidence"].clone());
    }
    let mut item = ctx.input()[i].clone();
    item.json.insert("sentimentAnalysis".into(), Value::Object(analysis));
    Ok((index, item.paired(i)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn structured_output_is_parsed_like_langchain() {
        let schema = json!({"type": "object", "properties": {"a": {"type": "string", "enum": ["x", "y"]}, "n": {"type": "number", "minimum": 0, "maximum": 1}}, "required": ["a"]});
        assert_eq!(parse_structured("```json\n{\"a\": \"x\", \"n\": 0.5, \"extra\": 1}\n```", &schema).unwrap(), json!({"a": "x", "n": 0.5}));
        assert_eq!(parse_structured("Sure: ```json\n{\"a\": \"y\"}\n``` done", &schema).unwrap(), json!({"a": "y"}));
        assert_eq!(parse_structured("{\"a\": \"line\nbreak\"}", &json!({"type": "object", "properties": {"a": {"type": "string"}}})).unwrap(), json!({"a": "line\nbreak"}));
        assert!(parse_structured("{\"a\": \"z\"}", &schema).is_err());
        assert!(parse_structured("{\"n\": 0.5}", &schema).is_err());
        assert!(parse_structured("{\"a\": \"x\", \"n\": 2}", &schema).is_err());
        assert!(parse_structured("not json", &schema).is_err());
    }
}
