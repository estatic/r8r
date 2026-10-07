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
    vec![Box::new(SentimentAnalysis), Box::new(TextClassifier), Box::new(InformationExtractor)]
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
        Some("string") => {
            let s = v.as_str().ok_or_else(|| format!("Expected string at {at}"))?;
            // zod's `.date()`: an ISO calendar date.
            if schema["format"].as_str() == Some("date") && chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").is_err() {
                return Err(format!("Invalid date at {at}"));
            }
            Ok(v.clone())
        }
        Some("boolean") => v.as_bool().map(|_| v.clone()).ok_or_else(|| format!("Expected boolean at {at}")),
        _ => Ok(v.clone()),
    }
}

/// LangChain's `NAIVE_FIX_TEMPLATE` (`OutputFixingParser`).
fn fix_prompt(instructions: &str, completion: &str, error: &str) -> String {
    format!("Instructions:\n--------------\n{instructions}\n--------------\nCompletion:\n--------------\n{completion}\n--------------\n\nAbove, the Completion did not satisfy the constraints given in the Instructions.\nError:\n--------------\n{error}\n--------------\n\nPlease try again. Please only respond with an answer that satisfies the constraints laid out in the Instructions:")
}

/// Why a structured call failed: the model call itself, or an answer that
/// doesn't fit the schema (LangChain's `OutputParserException`).
enum StructuredError {
    Model(NodeError),
    Parse,
}

impl StructuredError {
    /// n8n's `wrapLangChainParserError`: parse failures get one generic
    /// message, other errors stay as they are.
    fn wrapped(self) -> NodeError {
        match self {
            StructuredError::Model(e) => e,
            StructuredError::Parse => NodeError::new("Model output doesn't fit required format").describe("To continue the execution when this happens, change the 'On Error' parameter in the root node's settings"),
        }
    }
}

/// Runs `messages` through the model and parses the answer against
/// `schema`; with `auto_fix`, one more model call asks it to fix an answer
/// that doesn't parse (`OutputFixingParser`).
async fn structured_call(ctx: &ExecCtx<'_>, item: usize, messages: &[Value], schema: &Value, auto_fix: bool) -> Result<Value, StructuredError> {
    let model = load_model(ctx, item).await.map_err(StructuredError::Model)?;
    let reply = model.chat(ctx, messages, &[]).await.map_err(StructuredError::Model)?;
    let text = reply["content"].as_str().unwrap_or("").to_string();
    match parse_structured(&text, schema) {
        Ok(v) => Ok(v),
        Err(e) if auto_fix => {
            let prompt = fix_prompt(&format_instructions(schema), &text, &format!("OutputParserException: {e}"));
            let fixed = model.chat(ctx, &[json!({"role": "user", "content": prompt})], &[]).await.map_err(StructuredError::Model)?;
            parse_structured(fixed["content"].as_str().unwrap_or(""), schema).map_err(|_| StructuredError::Parse)
        }
        Err(_) => Err(StructuredError::Parse),
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
        .map_err(|_: StructuredError| NodeError::new("Error during parsing of LLM output, please check your LLM model and configuration"))?;
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

// ---- Text Classifier -------------------------------------------------------------

const CLASSIFIER_SYSTEM: &str = "Please classify the text provided by the user into one of the following categories: {categories}, and use the provided formatting instructions below. Don't explain, and only output the json.";

struct TextClassifier;

/// `(category, description)` pairs from the node's raw parameters.
fn classifier_categories(p: &Value) -> Vec<(String, String)> {
    p.pointer("/categories/categories")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|c| (c["category"].as_str().unwrap_or("").to_string(), c["description"].as_str().unwrap_or("").to_string()))
        .collect()
}

#[async_trait::async_trait]
impl NodeType for TextClassifier {
    fn type_name(&self) -> &'static str {
        "@n8n/n8n-nodes-langchain.textClassifier"
    }

    fn outputs(&self, node: &Node) -> usize {
        let other = node.parameters.pointer("/options/fallback").and_then(Value::as_str) == Some("other");
        (classifier_categories(&node.parameters).len() + usize::from(other)).max(1)
    }

    async fn execute(&self, ctx: &mut ExecCtx<'_>) -> NodeResult<NodeOutput> {
        let cats = classifier_categories(&json!({"categories": ctx.param("categories", 0)?}));
        if cats.is_empty() {
            return Err(NodeError::new("At least one category must be defined"));
        }
        let multi = ctx.param_bool("options.multiClass", 0, false)?;
        let fallback = ctx.param_str("options.fallback", 0, "discard")?;
        let auto_fix = ctx.param_bool("options.enableAutoFixing", 0, false)?;
        let mut props = Map::new();
        for (cat, description) in &cats {
            props.insert(cat.clone(), json!({"type": "boolean", "description": format!("Should be true if the input has category \"{cat}\" (description: {description})")}));
        }
        if fallback == "other" {
            props.insert("fallback".into(), json!({"type": "boolean", "description": "Should be true if none of the other categories apply"}));
        }
        let required: Vec<&String> = props.keys().collect();
        let schema = json!({"type": "object", "properties": props, "required": required, "additionalProperties": false});
        let multi_prompt = if multi { "Categories are not mutually exclusive, and multiple can be true" } else { "Categories are mutually exclusive, and only one can be true" };
        let fallback_prompt = if fallback == "other" { "If no categories apply, select the \"fallback\" option." } else { "If there is not a very fitting category, select none of the categories." };
        let names = cats.iter().map(|(c, _)| c.as_str()).collect::<Vec<_>>().join(", ");
        let mut out: NodeOutput = vec![Vec::new(); cats.len() + usize::from(fallback == "other")];
        for i in 0..ctx.input().len() {
            let result = async {
                let input = ctx.param_str("inputText", i, "")?;
                if input.is_empty() {
                    return Err(NodeError::new(format!("Text to classify for item {i} is not defined")));
                }
                let template = ctx.param_str("options.systemPromptTemplate", i, CLASSIFIER_SYSTEM)?;
                let system = format!("{}\n\t{}\n\t{multi_prompt}\n\t{fallback_prompt}", template.replace("{categories}", &names), format_instructions(&schema));
                let messages = [json!({"role": "system", "content": system}), json!({"role": "user", "content": input})];
                structured_call(ctx, i, &messages, &schema, auto_fix).await.map_err(StructuredError::wrapped)
            }
            .await;
            match result {
                Ok(answer) => {
                    let item = ctx.input()[i].clone().paired(i);
                    for (idx, (cat, _)) in cats.iter().enumerate() {
                        if answer[cat.as_str()].as_bool() == Some(true) {
                            out[idx].push(item.clone());
                        }
                    }
                    if fallback == "other" && answer["fallback"].as_bool() == Some(true) {
                        out.last_mut().expect("has the fallback output").push(item);
                    }
                }
                Err(e) if ctx.continue_on_fail() => out[0].push(Item::new(Map::from_iter([("error".to_string(), json!(e.message))])).paired(i)),
                Err(e) => return Err(e.at(i)),
            }
        }
        Ok(out)
    }
}

// ---- Information Extractor -------------------------------------------------------

const EXTRACTOR_SYSTEM: &str = "You are an expert extraction algorithm.\nOnly extract relevant information from the text.\nIf you do not know the value of an attribute asked to extract, you may omit the attribute's value.";

struct InformationExtractor;

/// The schema to extract: from attribute definitions (n8n's
/// `makeZodSchemaFromAttributes`; only attributes marked required are),
/// from a JSON example (all fields required from v1.2) or a JSON Schema.
fn extractor_schema(ctx: &ExecCtx<'_>) -> NodeResult<Value> {
    let schema_type = ctx.param_str("schemaType", 0, "fromAttributes")?;
    let mut schema = match schema_type.as_str() {
        "fromAttributes" => {
            let attrs = ctx.param("attributes.attributes", 0)?;
            let attrs = attrs.as_array().cloned().unwrap_or_default();
            if attrs.is_empty() {
                return Err(NodeError::new("At least one attribute must be specified"));
            }
            let mut props = Map::new();
            let mut required = Vec::new();
            for a in &attrs {
                let name = a["name"].as_str().unwrap_or("").to_string();
                let mut prop = match a["type"].as_str().unwrap_or("string") {
                    "number" => json!({"type": "number"}),
                    "boolean" => json!({"type": "boolean"}),
                    "date" => json!({"type": "string", "format": "date"}),
                    "string" => json!({"type": "string"}),
                    _ => json!({}),
                };
                prop["description"] = a["description"].clone();
                if a["required"].as_bool() == Some(true) {
                    required.push(name.clone());
                }
                props.insert(name, prop);
            }
            let mut s = json!({"type": "object", "properties": props});
            if !required.is_empty() {
                s["required"] = json!(required);
            }
            s
        }
        "fromJson" => {
            let example: Value = serde_json::from_str(&ctx.param_str("jsonSchemaExample", 0, "")?).map_err(|e| NodeError::new(format!("Invalid JSON example: {e}")))?;
            super::ai::schema_from_example(&example, ctx.node.type_version >= 1.2)
        }
        _ => {
            let mut s: Value = serde_json::from_str(&ctx.param_str("inputSchema", 0, "")?).map_err(|e| NodeError::new(format!("Invalid JSON Schema: {e}")))?;
            if let Some(o) = s.as_object_mut() {
                o.remove("$schema");
            }
            s
        }
    };
    super::ai::close_objects(&mut schema);
    Ok(schema)
}

#[async_trait::async_trait]
impl NodeType for InformationExtractor {
    fn type_name(&self) -> &'static str {
        "@n8n/n8n-nodes-langchain.informationExtractor"
    }

    async fn execute(&self, ctx: &mut ExecCtx<'_>) -> NodeResult<NodeOutput> {
        let schema = extractor_schema(ctx)?;
        let mut out = Vec::new();
        for i in 0..ctx.input().len() {
            let result = async {
                let input = ctx.param_str("text", i, "")?;
                if input.trim().is_empty() {
                    return Err(NodeError::new(format!("Text for item {i} is not defined")));
                }
                let template = ctx.param_str("options.systemPromptTemplate", i, EXTRACTOR_SYSTEM)?;
                let system = format!("{template}\n{}", format_instructions(&schema));
                let messages = [json!({"role": "system", "content": system}), json!({"role": "user", "content": input})];
                // n8n always wraps this parser in an OutputFixingParser.
                structured_call(ctx, i, &messages, &schema, true).await.map_err(StructuredError::wrapped)
            }
            .await;
            match result {
                Ok(output) => out.push(Item::new(Map::from_iter([("output".to_string(), output)])).paired(i)),
                Err(e) if ctx.continue_on_fail() => out.push(Item::new(Map::from_iter([("error".to_string(), json!(e.message))])).paired(i)),
                Err(e) => return Err(e.at(i)),
            }
        }
        Ok(vec![out])
    }
}
