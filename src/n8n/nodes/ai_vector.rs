//! Retrieval building blocks (spec §6.8, plan 2.7): the embeddings,
//! document loader and text splitter sub-nodes, and the Simple Vector Store
//! (`vectorStoreInMemory`), as n8n 2.35.7 runs them. Embedding requests,
//! document shapes, cosine scores and ranking follow LangChain exactly
//! (`OpenAIEmbeddings`, `MemoryVectorStore`, `ml-distance`'s cosine).

use super::ai::record;
use super::doc_loader::n8n_json_loader;
use super::text_split::{Document, Kind, Splitter};
use crate::n8n::node::{ExecCtx, NodeError, NodeResult, NodeType};
use crate::n8n::types::{now_ms, Item, NodeOutput};
use crate::n8n::workflow::Node;
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

const LC: &str = "@n8n/n8n-nodes-langchain.";

pub fn all() -> Vec<Box<dyn NodeType>> {
    vec![
        Box::new(VectorStoreInMemory),
        Box::new(super::ai::SubNode("@n8n/n8n-nodes-langchain.embeddingsOpenAi")),
        Box::new(super::ai::SubNode("@n8n/n8n-nodes-langchain.documentDefaultDataLoader")),
        Box::new(super::ai::SubNode("@n8n/n8n-nodes-langchain.textSplitterRecursiveCharacterTextSplitter")),
        Box::new(super::ai::SubNode("@n8n/n8n-nodes-langchain.textSplitterCharacterTextSplitter")),
        Box::new(super::ai::SubNode("@n8n/n8n-nodes-langchain.retrieverVectorStore")),
    ]
}

fn sub_node<'a>(ctx: &'a ExecCtx<'_>, parent: &str, kind: &str) -> Option<&'a Node> {
    ctx.workflow.sub_nodes(parent, kind).into_iter().filter_map(|n| ctx.workflow.node(&n)).find(|n| !n.disabled)
}

// ---- embeddings ----------------------------------------------------------------

/// The `ai_embedding` sub-node: LangChain's `OpenAIEmbeddings`.
pub(super) struct Embeddings<'a> {
    node: &'a Node,
    url: String,
    api_key: String,
    model: String,
    dimensions: Option<Value>,
    batch_size: usize,
    strip_new_lines: bool,
    encoding_format: Option<String>,
}

pub(super) async fn load_embeddings<'a>(ctx: &'a ExecCtx<'_>, parent: &str, item: usize) -> NodeResult<Embeddings<'a>> {
    let node = sub_node(ctx, parent, "ai_embedding").ok_or_else(|| NodeError::new("An Embeddings sub-node must be connected and enabled"))?;
    if node.node_type != format!("{LC}embeddingsOpenAi") {
        return Err(NodeError::new(format!("The embeddings \"{}\" ({}) are not supported natively yet", node.name, node.node_type)));
    }
    let p = ctx.resolve_value(&node.parameters, item)?;
    let (_, cred) = ctx.credentials_for(node, "openAiApi").await?;
    let o = &p["options"];
    let url = o["baseURL"].as_str().filter(|s| !s.is_empty()).or(cred["url"].as_str().filter(|s| !s.is_empty())).unwrap_or("https://api.openai.com/v1").trim_end_matches('/').to_string();
    let default_model = if node.type_version >= 1.2 { "text-embedding-3-small" } else { "text-embedding-ada-002" };
    let model = match &p["model"] {
        Value::Object(m) => m.get("value").and_then(Value::as_str).unwrap_or(default_model).to_string(),
        Value::String(s) if !s.is_empty() => s.clone(),
        _ => default_model.to_string(),
    };
    Ok(Embeddings {
        node,
        url,
        api_key: cred["apiKey"].as_str().unwrap_or("").to_string(),
        model,
        dimensions: o.get("dimensions").cloned().filter(|d| d.as_f64().is_some_and(|n| n > 0.0)),
        batch_size: o["batchSize"].as_u64().unwrap_or(512).max(1) as usize,
        strip_new_lines: o["stripNewLines"].as_bool().unwrap_or(true),
        encoding_format: o["encodingFormat"].as_str().map(String::from),
    })
}

/// An embedding from the API: a float array, or (what the OpenAI SDK asks
/// for by default) base64 little-endian float32.
fn decode_embedding(v: &Value) -> NodeResult<Vec<f64>> {
    match v {
        Value::Array(a) => Ok(a.iter().map(|x| x.as_f64().unwrap_or(0.0)).collect()),
        Value::String(s) => {
            use base64::Engine as _;
            let bytes = base64::engine::general_purpose::STANDARD.decode(s).map_err(|e| NodeError::new(format!("Invalid embedding data: {e}")))?;
            Ok(bytes.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]) as f64).collect())
        }
        _ => Err(NodeError::new("The embeddings API returned no embedding")),
    }
}

impl Embeddings<'_> {
    async fn request(&self, ctx: &ExecCtx<'_>, input: Value) -> NodeResult<Vec<Vec<f64>>> {
        // The OpenAI SDK asks for base64 unless the user picked a format.
        let mut body = json!({"model": self.model, "input": input, "encoding_format": self.encoding_format.clone().unwrap_or_else(|| "base64".into())});
        if let Some(d) = &self.dimensions {
            body["dimensions"] = d.clone();
        }
        let url = reqwest::Url::parse(&format!("{}/embeddings", self.url)).map_err(|e| NodeError::new(format!("Invalid embeddings URL: {e}")))?;
        super::check_ssrf(&url, ctx.config()).await.map_err(NodeError::new)?;
        let resp = ctx.services.http.post(url).bearer_auth(&self.api_key).json(&body).send().await.map_err(|e| NodeError::new(format!("The embeddings provider could not be reached: {}", e.without_url())))?;
        let status = resp.status();
        let json: Value = resp.json().await.unwrap_or(Value::Null);
        if !status.is_success() {
            let message = json.pointer("/error/message").and_then(Value::as_str).map(String::from).unwrap_or_else(|| format!("The embeddings provider answered {status}"));
            return Err(NodeError::api(message.replace(&self.api_key, "***"), Some(status.as_u16()), None));
        }
        let mut data: Vec<&Value> = json["data"].as_array().into_iter().flatten().collect();
        data.sort_by_key(|d| d["index"].as_u64().unwrap_or(0));
        data.iter().map(|d| decode_embedding(&d["embedding"])).collect()
    }

    fn prepare(&self, text: &str) -> String {
        if self.strip_new_lines {
            text.replace('\n', " ")
        } else {
            text.to_string()
        }
    }

    /// `embedDocuments`: batches of `batchSize`, recorded on the sub-node.
    pub async fn embed_documents(&self, ctx: &ExecCtx<'_>, texts: &[String]) -> NodeResult<Vec<Vec<f64>>> {
        let started = now_ms();
        let prepared: Vec<String> = texts.iter().map(|t| self.prepare(t)).collect();
        let mut out = Vec::new();
        for batch in prepared.chunks(self.batch_size) {
            match self.request(ctx, json!(batch)).await {
                Ok(v) => out.extend(v),
                Err(e) => {
                    record(ctx, &self.node.name, "ai_embedding", json!({"documents": texts}), Err(&e), started);
                    return Err(e);
                }
            }
        }
        record(ctx, &self.node.name, "ai_embedding", json!({"documents": texts}), Ok(json!({"response": out})), started);
        Ok(out)
    }

    /// `embedQuery`: one string in, one vector out.
    pub async fn embed_query(&self, ctx: &ExecCtx<'_>, text: &str) -> NodeResult<Vec<f64>> {
        let started = now_ms();
        let result = self.request(ctx, json!(self.prepare(text))).await.and_then(|mut v| if v.is_empty() { Err(NodeError::new("The embeddings API returned no embedding")) } else { Ok(v.remove(0)) });
        match &result {
            Ok(v) => record(ctx, &self.node.name, "ai_embedding", json!({"query": text}), Ok(json!({"response": v})), started),
            Err(e) => record(ctx, &self.node.name, "ai_embedding", json!({"query": text}), Err(e), started),
        }
        result
    }
}

// ---- document loader + text splitters --------------------------------------------

/// A splitter sub-node, recording its `splitText` calls like n8n's logWrapper.
struct SplitterNode<'a> {
    node: &'a Node,
    splitter: Splitter,
}

fn load_splitter<'a>(ctx: &'a ExecCtx<'_>, parent: &str, item: usize) -> NodeResult<Option<SplitterNode<'a>>> {
    let Some(node) = sub_node(ctx, parent, "ai_textSplitter") else { return Ok(None) };
    let p = ctx.resolve_value(&node.parameters, item)?;
    let size = p["chunkSize"].as_f64().unwrap_or(1000.0) as usize;
    let overlap = p["chunkOverlap"].as_f64().unwrap_or(0.0) as usize;
    let splitter = if node.node_type == format!("{LC}textSplitterRecursiveCharacterTextSplitter") {
        if p.pointer("/options/splitCode").and_then(Value::as_str).is_some_and(|s| !s.is_empty()) {
            return Err(NodeError::new("Splitting code by language is not supported natively yet"));
        }
        let separators = ["\n\n", "\n", " ", ""].iter().map(|s| s.to_string()).collect();
        Splitter::new(size, overlap, false, Kind::Recursive { separators })
    } else if node.node_type == format!("{LC}textSplitterCharacterTextSplitter") {
        Splitter::new(size, overlap, false, Kind::Character { separator: p["separator"].as_str().unwrap_or("").to_string() })
    } else {
        return Err(NodeError::new(format!("The text splitter \"{}\" ({}) is not supported natively yet", node.name, node.node_type)));
    }
    .map_err(NodeError::new)?;
    Ok(Some(SplitterNode { node, splitter }))
}

/// The `ai_document` sub-node: n8n's Default Data Loader (JSON input).
pub(super) struct DocumentLoader<'a> {
    node: &'a Node,
    splitter: Option<SplitterNode<'a>>,
    /// The simple mode's built-in splitter (v1.1).
    builtin: Option<Splitter>,
}

pub(super) fn load_document_loader<'a>(ctx: &'a ExecCtx<'_>, parent: &str, item: usize) -> NodeResult<DocumentLoader<'a>> {
    let node = sub_node(ctx, parent, "ai_document").ok_or_else(|| NodeError::new("A Document sub-node must be connected and enabled"))?;
    if node.node_type != format!("{LC}documentDefaultDataLoader") {
        return Err(NodeError::new(format!("The document loader \"{}\" ({}) is not supported natively yet", node.name, node.node_type)));
    }
    let p = &node.parameters;
    if p["dataType"].as_str().unwrap_or("json") != "json" {
        return Err(NodeError::new("Loading binary data as documents is not supported natively yet"));
    }
    let simple = node.type_version >= 1.1 && p["textSplittingMode"].as_str().unwrap_or("simple") == "simple";
    let (splitter, builtin) = if simple { (None, Some(Splitter::recursive(1000, 200).map_err(NodeError::new)?)) } else { (load_splitter(ctx, &node.name, item)?, None) };
    Ok(DocumentLoader { node, splitter, builtin })
}

impl DocumentLoader<'_> {
    /// `N8nJsonLoader.processItem` for one input item, recorded on the
    /// loader (and the splitter sub-node) like n8n's logWrapper.
    pub fn process_item(&self, ctx: &ExecCtx<'_>, item: usize) -> NodeResult<Vec<Document>> {
        let started = now_ms();
        let input = &ctx.input()[item];
        let p = ctx.resolve_value(&self.node.parameters, item)?;
        let mode = p["jsonMode"].as_str().unwrap_or("allInputData");
        let pointers = p.pointer("/options/pointers").and_then(Value::as_str).unwrap_or("");
        let splitter = self.splitter.as_ref().map(|s| &s.splitter).or(self.builtin.as_ref());
        let mut docs = n8n_json_loader(&input.json, mode, &p["jsonData"], pointers, None).map_err(NodeError::new)?;
        if let Some(s) = splitter {
            let texts: Vec<String> = docs.iter().map(|d| d.page_content.clone()).collect();
            if let Some(sn) = &self.splitter {
                for t in &texts {
                    let chunks = sn.splitter.split_text(t);
                    record(ctx, &sn.node.name, "ai_textSplitter", json!({"textSplitter": t}), Ok(json!({"response": chunks})), started);
                }
            }
            let metas: Vec<Map<String, Value>> = docs.into_iter().map(|d| d.metadata).collect();
            docs = s.create_documents(&texts, &metas);
        }
        // `getMetadataFiltersValues`: extra metadata from the options.
        if let Some(values) = p.pointer("/options/metadata/metadataValues").and_then(Value::as_array) {
            for d in docs.iter_mut() {
                for v in values {
                    if let Some(name) = v["name"].as_str() {
                        d.metadata.insert(name.to_string(), v["value"].clone());
                    }
                }
            }
        }
        let serialized: Vec<Value> = docs.iter().map(|d| json!({"pageContent": d.page_content, "metadata": d.metadata})).collect();
        record(ctx, &self.node.name, "ai_document", Value::Object(input.json.clone()), Ok(json!({"response": serialized})), started);
        Ok(docs)
    }
}

// ---- in-memory vector store --------------------------------------------------------

struct MemoryVector {
    content: String,
    embedding: Vec<f64>,
    metadata: Map<String, Value>,
}

/// n8n's `MemoryVectorStoreManager`: stores by memory key, process-wide.
fn stores() -> &'static Mutex<HashMap<String, Vec<MemoryVector>>> {
    static S: OnceLock<Mutex<HashMap<String, Vec<MemoryVector>>>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(HashMap::new()))
}

/// `ml-distance`'s cosine, in the same order of operations as LangChain.
fn cosine(a: &[f64], b: &[f64]) -> f64 {
    let (mut p, mut p2, mut q2) = (0.0, 0.0, 0.0);
    for i in 0..a.len() {
        let bi = b.get(i).copied().unwrap_or(f64::NAN);
        p += a[i] * bi;
        p2 += a[i] * a[i];
        q2 += bi * bi;
    }
    p / (p2.sqrt() * q2.sqrt())
}

/// `MemoryVectorStore.similaritySearchVectorWithScore`: by similarity,
/// highest first, ties in insertion order, the first `k`.
pub(super) fn memory_search(key: &str, query: &[f64], k: usize) -> Vec<(Document, f64)> {
    let stores = stores().lock().unwrap();
    let mut scored: Vec<(Document, f64)> = stores
        .get(key)
        .into_iter()
        .flatten()
        .map(|v| (Document { page_content: v.content.clone(), metadata: v.metadata.clone() }, cosine(query, &v.embedding)))
        .collect();
    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    scored.truncate(k);
    scored
}

fn memory_add(key: &str, docs: &[Document], vectors: Vec<Vec<f64>>, clear: bool) {
    let mut stores = stores().lock().unwrap();
    if clear {
        stores.remove(key);
    }
    let store = stores.entry(key.to_string()).or_default();
    for (d, e) in docs.iter().zip(vectors) {
        store.push(MemoryVector { content: d.page_content.clone(), embedding: e, metadata: d.metadata.clone() });
    }
}

struct VectorStoreInMemory;

/// The store's key: from v1.2 a resource locator shared across workflows,
/// before that prefixed with the workflow ID.
pub(super) fn memory_key(ctx: &ExecCtx<'_>, item: usize) -> NodeResult<String> {
    memory_key_of(ctx, ctx.node, item)
}

fn memory_key_of(ctx: &ExecCtx<'_>, node: &Node, item: usize) -> NodeResult<String> {
    let raw = ctx.resolve_value(&node.parameters["memoryKey"], item)?;
    let raw = if raw.is_null() { json!("vector_store_key") } else { raw };
    if node.type_version <= 1.1 {
        let key = raw.as_str().unwrap_or("vector_store_key");
        return Ok(format!("{}__{key}", ctx.workflow.id.clone().unwrap_or_default()));
    }
    Ok(match raw {
        Value::Object(o) => o.get("value").and_then(Value::as_str).unwrap_or("vector_store_key").to_string(),
        Value::String(s) => s,
        _ => "vector_store_key".to_string(),
    })
}

#[async_trait::async_trait]
impl NodeType for VectorStoreInMemory {
    fn type_name(&self) -> &'static str {
        "@n8n/n8n-nodes-langchain.vectorStoreInMemory"
    }

    async fn execute(&self, ctx: &mut ExecCtx<'_>) -> NodeResult<NodeOutput> {
        let mode = ctx.param_str("mode", 0, "retrieve")?;
        match mode.as_str() {
            "insert" => insert(ctx).await,
            "load" => load(ctx).await,
            "retrieve" | "retrieve-as-tool" => Err(NodeError::new(format!("\"{}\" in {mode} mode is a sub-node: connect it to a chain, agent or retriever", ctx.node.name))),
            other => Err(NodeError::new(format!("The vector store mode \"{other}\" is not supported natively yet"))),
        }
    }
}

/// `handleInsertOperation`: each item through the document loader, then
/// (from v1.1) embedded and stored in batches of `embeddingBatchSize`. As in
/// n8n, "Clear Store" applies to every batch.
async fn insert(ctx: &ExecCtx<'_>) -> NodeResult<NodeOutput> {
    let embeddings = load_embeddings(ctx, &ctx.node.name, 0).await?;
    let loader = load_document_loader(ctx, &ctx.node.name, 0)?;
    let mut out = Vec::new();
    let mut all_docs = Vec::new();
    for i in 0..ctx.input().len() {
        let docs = loader.process_item(ctx, i)?;
        for d in &docs {
            out.push(Item::new(Map::from_iter([("metadata".to_string(), Value::Object(d.metadata.clone())), ("pageContent".to_string(), json!(d.page_content))])).paired(i));
        }
        if ctx.node.type_version < 1.1 {
            populate(ctx, &embeddings, &docs, i).await?;
        } else {
            all_docs.extend(docs);
        }
    }
    if ctx.node.type_version >= 1.1 {
        let batch = (ctx.param_f64("embeddingBatchSize", 0, 200.0)? as usize).max(1);
        for chunk in all_docs.chunks(batch) {
            populate(ctx, &embeddings, chunk, 0).await?;
        }
    }
    Ok(vec![out])
}

async fn populate(ctx: &ExecCtx<'_>, embeddings: &Embeddings<'_>, docs: &[Document], item: usize) -> NodeResult<()> {
    let texts: Vec<String> = docs.iter().map(|d| d.page_content.clone()).collect();
    let vectors = embeddings.embed_documents(ctx, &texts).await?;
    memory_add(&memory_key(ctx, item)?, docs, vectors, ctx.param_bool("clearStore", item, false)?);
    Ok(())
}

/// `handleLoadOperation`: the `topK` documents most similar to the prompt.
async fn load(ctx: &ExecCtx<'_>) -> NodeResult<NodeOutput> {
    let embeddings = load_embeddings(ctx, &ctx.node.name, 0).await?;
    let mut out = Vec::new();
    for i in 0..ctx.input().len() {
        if ctx.raw_param("options").and_then(|o| o.get("metadata")).is_some() {
            return Err(NodeError::new("Metadata filters on the Simple Vector Store are not supported natively yet"));
        }
        let prompt = ctx.param_str("prompt", i, "")?;
        let k = ctx.param_f64("topK", i, 4.0)? as usize;
        let with_metadata = ctx.param_bool("includeDocumentMetadata", i, true)?;
        let query = embeddings.embed_query(ctx, &prompt).await?;
        for (doc, score) in memory_search(&memory_key(ctx, i)?, &query, k) {
            let mut document = Map::from_iter([("pageContent".to_string(), json!(doc.page_content))]);
            if with_metadata {
                document.insert("metadata".into(), Value::Object(doc.metadata));
            }
            out.push(Item::new(Map::from_iter([("document".to_string(), Value::Object(document)), ("score".to_string(), json!(score))])).paired(i));
        }
    }
    Ok(vec![out])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cosine_and_ranking_match_langchain() {
        assert!((cosine(&[1.0, 0.0], &[1.0, 0.0]) - 1.0).abs() < 1e-12);
        assert_eq!(cosine(&[1.0, 0.0], &[0.0, 1.0]), 0.0);
        let d = |t: &str| Document { page_content: t.into(), metadata: Map::new() };
        memory_add("t", &[d("a"), d("b"), d("c")], vec![vec![1.0, 0.0], vec![0.0, 1.0], vec![1.0, 0.0]], true);
        let hits: Vec<String> = memory_search("t", &[1.0, 0.1], 2).into_iter().map(|(d, _)| d.page_content).collect();
        assert_eq!(hits, ["a", "c"], "ties keep insertion order");
    }

    #[test]
    fn base64_float32_embeddings_decode() {
        use base64::Engine as _;
        let bytes: Vec<u8> = [0.5f32, -1.0, 0.1].iter().flat_map(|f| f.to_le_bytes()).collect();
        let v = decode_embedding(&json!(base64::engine::general_purpose::STANDARD.encode(bytes))).unwrap();
        assert_eq!(v, vec![0.5, -1.0, 0.1f32 as f64]);
    }
}

// ---- retrieval -----------------------------------------------------------------------

/// A similarity search on a vector store sub-node (retrieve modes): the
/// query embedded with the store's own embeddings sub-node, recorded on
/// the store like n8n's logWrapper.
pub(super) async fn store_search(ctx: &ExecCtx<'_>, store: &Node, query: &str, k: usize, item: usize) -> NodeResult<Vec<(Document, f64)>> {
    if store.node_type != format!("{LC}vectorStoreInMemory") {
        return Err(NodeError::new(format!("The vector store \"{}\" ({}) is not supported natively yet", store.name, store.node_type)));
    }
    let started = now_ms();
    let embeddings = load_embeddings(ctx, &store.name, item).await?;
    let vector = embeddings.embed_query(ctx, query).await?;
    let hits = memory_search(&memory_key_of(ctx, store, item)?, &vector, k);
    let docs: Vec<Value> = hits.iter().map(|(d, _)| json!({"pageContent": d.page_content, "metadata": d.metadata})).collect();
    record(ctx, &store.name, "ai_vectorStore", json!({"query": query}), Ok(json!({"response": docs})), started);
    Ok(hits)
}

/// The documents an `ai_retriever` sub-node (Vector Store Retriever)
/// finds for `query`.
pub(super) async fn retrieve(ctx: &ExecCtx<'_>, item: usize, query: &str) -> NodeResult<Vec<Document>> {
    let retriever = sub_node(ctx, &ctx.node.name, "ai_retriever").ok_or_else(|| NodeError::new("A Retriever sub-node must be connected and enabled"))?;
    if retriever.node_type != format!("{LC}retrieverVectorStore") {
        return Err(NodeError::new(format!("The retriever \"{}\" ({}) is not supported natively yet", retriever.name, retriever.node_type)));
    }
    let k = ctx.resolve_value(&retriever.parameters["topK"], item)?.as_f64().unwrap_or(4.0) as usize;
    let store = sub_node(ctx, &retriever.name, "ai_vectorStore").ok_or_else(|| NodeError::new("A Vector Store sub-node must be connected to the retriever"))?;
    Ok(store_search(ctx, store, query, k, item).await?.into_iter().map(|(d, _)| d).collect())
}
