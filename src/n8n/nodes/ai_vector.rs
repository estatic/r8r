//! Retrieval building blocks (spec §6.8, plan 2.7): the embeddings,
//! document loader and text splitter sub-nodes, the Simple Vector Store
//! (`vectorStoreInMemory`), the Postgres PGVector Store
//! (`vectorStorePGVector`) and the Qdrant Vector Store (`vectorStoreQdrant`),
//! as n8n 2.35.7 runs them. Embedding requests,
//! document shapes, scores and ranking follow LangChain exactly
//! (`OpenAIEmbeddings`, `MemoryVectorStore`, `ml-distance`'s cosine,
//! `@langchain/community` 1.1.27's `PGVectorStore`, `@langchain/qdrant`
//! 1.0.1's `QdrantVectorStore`).

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
        Box::new(VectorStorePgVector),
        Box::new(VectorStoreQdrant),
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

/// `populateVectorStore` for one batch, in each store's order of calls:
/// PGVector (`PGVectorStore.fromDocuments`) ensures its table before
/// embedding, Qdrant (`addVectors`) its collection after; the in-memory
/// store just embeds.
async fn populate(ctx: &ExecCtx<'_>, embeddings: &Embeddings<'_>, docs: &[Document], item: usize) -> NodeResult<()> {
    let texts: Vec<String> = docs.iter().map(|d| d.page_content.clone()).collect();
    if is_pgvector(ctx.node) {
        let store = PgVector::open(ctx, ctx.node, item).await?;
        let result = match embeddings.embed_documents(ctx, &texts).await {
            Ok(vectors) => store.add(docs, &vectors).await,
            Err(e) => Err(e),
        };
        store.pool.close().await;
        return result;
    }
    if ctx.node.node_type == QDRANT {
        let store = Qdrant::open(ctx, ctx.node, item).await?;
        let vectors = embeddings.embed_documents(ctx, &texts).await?;
        if vectors.is_empty() {
            return Ok(());
        }
        store.ensure_collection(ctx, embeddings).await?;
        return store.add(ctx, docs, vectors).await;
    }
    let vectors = embeddings.embed_documents(ctx, &texts).await?;
    memory_add(&memory_key(ctx, item)?, docs, vectors, ctx.param_bool("clearStore", item, false)?);
    Ok(())
}

/// `handleLoadOperation`: the `topK` documents most similar to the prompt.
async fn load(ctx: &ExecCtx<'_>) -> NodeResult<NodeOutput> {
    let embeddings = load_embeddings(ctx, &ctx.node.name, 0).await?;
    let mut out = Vec::new();
    for i in 0..ctx.input().len() {
        let prompt = ctx.param_str("prompt", i, "")?;
        let k = ctx.param_f64("topK", i, 4.0)? as usize;
        let with_metadata = ctx.param_bool("includeDocumentMetadata", i, true)?;
        let filter = metadata_filter(ctx, ctx.node, i)?;
        if filter.is_some() && ctx.node.node_type == format!("{LC}vectorStoreInMemory") {
            return Err(NodeError::new("Metadata filters on the Simple Vector Store are not supported natively yet"));
        }
        let store = Store::open(ctx, ctx.node, &embeddings, i).await?;
        let result = match embeddings.embed_query(ctx, &prompt).await {
            Ok(query) => store.search(ctx, &embeddings, &query, k, filter.as_ref()).await,
            Err(e) => Err(e),
        };
        store.close().await;
        for hit in result? {
            let mut document = Map::from_iter([("pageContent".to_string(), json!(hit.doc.page_content))]);
            if with_metadata {
                document.insert("metadata".into(), Value::Object(hit.doc.metadata));
            }
            out.push(Item::new(Map::from_iter([("document".to_string(), Value::Object(document)), ("score".to_string(), json!(hit.score))])).paired(i));
        }
    }
    Ok(vec![out])
}

/// A vector store as n8n's `getVectorStoreClient` hands it out.
enum Store {
    Memory(String),
    Pg(PgVector),
    Qdrant(Qdrant),
}

impl Store {
    /// PGVector ensures its tables, Qdrant its collection
    /// (`fromExistingCollection`).
    async fn open(ctx: &ExecCtx<'_>, node: &Node, embeddings: &Embeddings<'_>, item: usize) -> NodeResult<Store> {
        if is_pgvector(node) {
            Ok(Store::Pg(PgVector::open(ctx, node, item).await?))
        } else if node.node_type == QDRANT {
            let q = Qdrant::open(ctx, node, item).await?;
            q.ensure_collection(ctx, embeddings).await?;
            Ok(Store::Qdrant(q))
        } else if node.node_type == format!("{LC}vectorStoreInMemory") {
            Ok(Store::Memory(memory_key_of(ctx, node, item)?))
        } else {
            Err(NodeError::new(format!("The vector store \"{}\" ({}) is not supported natively yet", node.name, node.node_type)))
        }
    }

    /// `similaritySearchVectorWithScore`.
    async fn search(&self, ctx: &ExecCtx<'_>, embeddings: &Embeddings<'_>, query: &[f64], k: usize, filter: Option<&Map<String, Value>>) -> NodeResult<Vec<Hit>> {
        match self {
            Store::Memory(key) => Ok(memory_search(key, query, k).into_iter().map(|(doc, score)| Hit { doc, score, id: None }).collect()),
            Store::Pg(pg) => pg.search(query, k, filter).await,
            Store::Qdrant(q) => {
                q.ensure_collection(ctx, embeddings).await?;
                q.search(ctx, query, k, filter).await
            }
        }
    }

    /// `releaseVectorStoreClient`.
    async fn close(self) {
        if let Store::Pg(pg) = self {
            pg.pool.close().await;
        }
    }
}

/// A search result: LangChain's `[Document, score]`, with the document's
/// `id` (PGVector sets it to the row's ID; in-memory documents have none).
pub(super) struct Hit {
    pub doc: Document,
    pub score: f64,
    pub id: Option<Value>,
}

impl Hit {
    /// `JSON.stringify(document)`: `pageContent`, `metadata`, then `id` if set.
    pub fn to_json(&self) -> Value {
        let mut v = json!({"pageContent": self.doc.page_content, "metadata": self.doc.metadata});
        if let Some(id) = &self.id {
            v["id"] = id.clone();
        }
        v
    }
}

/// `getMetadataFiltersValues`: the "Metadata Filter" option as a
/// name → value object, else the `searchFilterJson` option.
fn metadata_filter(ctx: &ExecCtx<'_>, node: &Node, item: usize) -> NodeResult<Option<Map<String, Value>>> {
    let options = ctx.resolve_value(&node.parameters["options"], item)?;
    if let Some(values) = options.pointer("/metadata/metadataValues").and_then(Value::as_array) {
        if !values.is_empty() {
            return Ok(Some(values.iter().filter_map(|v| Some((v["name"].as_str()?.to_string(), v["value"].clone()))).collect()));
        }
    }
    match &options["searchFilterJson"] {
        Value::Object(m) => Ok(Some(m.clone())),
        Value::String(s) if !s.trim().is_empty() => match serde_json::from_str(s) {
            Ok(Value::Object(m)) => Ok(Some(m)),
            _ => Err(NodeError::new("Parameter 'options.searchFilterJson' could not be parsed as an object")),
        },
        _ => Ok(None),
    }
}

// ---- Postgres PGVector store ----------------------------------------------------------

struct VectorStorePgVector;

const PGVECTOR: &str = "@n8n/n8n-nodes-langchain.vectorStorePGVector";

fn is_pgvector(node: &Node) -> bool {
    node.node_type == PGVECTOR
}

#[async_trait::async_trait]
impl NodeType for VectorStorePgVector {
    fn type_name(&self) -> &'static str {
        PGVECTOR
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

/// A plain identifier, optionally schema-qualified (`qualified`). LangChain
/// puts table and column names into its SQL as is; r8r only takes names it
/// can't be injected through.
fn check_identifier(what: &str, name: &str, qualified: bool) -> NodeResult<()> {
    let re = if qualified { r"^[A-Za-z_][A-Za-z0-9_$]*(\.[A-Za-z_][A-Za-z0-9_$]*)?$" } else { r"^[A-Za-z_][A-Za-z0-9_$]*$" };
    if regex::Regex::new(re).unwrap().is_match(name) {
        Ok(())
    } else {
        Err(NodeError::new(format!("Invalid {what} \"{name}\"")).describe(if qualified { "Use letters, digits and underscores, optionally as schema.table" } else { "Use letters, digits and underscores" }))
    }
}

/// `normalizeVectorStoreError`: a database error as a `NodeApiError`.
fn pg_error(prefix: &str) -> impl Fn(sqlx::Error) -> NodeError + '_ {
    move |e| {
        let message = match &e {
            sqlx::Error::Database(d) => d.message().to_string(),
            other => other.to_string(),
        };
        let message = format!("{prefix}{message}");
        NodeError::api(message.clone(), None, Some(message))
    }
}

/// LangChain's `PGVectorStore` as n8n's `getVectorStoreClient` /
/// `populateVectorStore` configure it.
pub(super) struct PgVector {
    pool: sqlx::PgPool,
    table: String,
    /// (collection name, collection table) when "Use Collection" is on.
    collection: Option<(String, String)>,
    id_col: String,
    vector_col: String,
    content_col: String,
    metadata_col: String,
    operator: &'static str,
}

impl PgVector {
    /// Connects and runs `initialize`: the `vector` extension, the table
    /// and (with a collection) the collection table.
    async fn open(ctx: &ExecCtx<'_>, node: &Node, item: usize) -> NodeResult<PgVector> {
        let table = match ctx.resolve_value(&node.parameters["tableName"], item)? {
            Value::Null => "n8n_vectors".to_string(),
            Value::Object(o) => o.get("value").and_then(Value::as_str).unwrap_or("").to_string(),
            v => v.as_str().unwrap_or("").to_string(),
        };
        check_identifier("table name", &table, true)?;
        // n8n reads the options from the first item.
        let o = ctx.resolve_value(&node.parameters["options"], 0)?;
        let c = &o["collection"]["values"];
        let collection = if c["useCollection"].as_bool().unwrap_or(false) {
            let name = c["collectionName"].as_str().unwrap_or("n8n").to_string();
            let table = c["collectionTableName"].as_str().unwrap_or("n8n_vector_collections").to_string();
            check_identifier("collection table name", &table, true)?;
            Some((name, table))
        } else {
            None
        };
        let cols = &o["columnNames"]["values"];
        let col = |key: &str, default: &str| -> NodeResult<String> {
            let name = cols[key].as_str().unwrap_or(default).to_string();
            check_identifier("column name", &name, false)?;
            Ok(name)
        };
        let operator = match o["distanceStrategy"].as_str().unwrap_or("cosine") {
            "cosine" => "<=>",
            "innerProduct" => "<#>",
            "euclidean" => "<->",
            other => return Err(NodeError::new(format!("Unknown distance strategy: {other}"))),
        };
        let (_, cred) = ctx.credentials_for(node, "postgres").await?;
        let store = PgVector {
            pool: super::postgres::connect(&cred, 30).await?,
            table,
            collection,
            id_col: col("idColumnName", "id")?,
            vector_col: col("vectorColumnName", "embedding")?,
            content_col: col("contentColumnName", "text")?,
            metadata_col: col("metadataColumnName", "metadata")?,
            operator,
        };
        if let Err(e) = store.ensure_tables().await {
            store.pool.close().await;
            return Err(e);
        }
        Ok(store)
    }

    /// `ensureTableInDatabase` + `ensureCollectionTableInDatabase`.
    async fn ensure_tables(&self) -> NodeResult<()> {
        sqlx::query("CREATE EXTENSION IF NOT EXISTS vector;").execute(&self.pool).await.map_err(pg_error(""))?;
        let (t, id, content, metadata, vector) = (&self.table, &self.id_col, &self.content_col, &self.metadata_col, &self.vector_col);
        sqlx::query(&format!("CREATE TABLE IF NOT EXISTS {t} (\"{id}\" uuid NOT NULL DEFAULT gen_random_uuid() PRIMARY KEY, \"{content}\" text, \"{metadata}\" jsonb, \"{vector}\" vector);"))
            .execute(&self.pool)
            .await
            .map_err(pg_error(""))?;
        if let Some((_, ct)) = &self.collection {
            // One multi-statement query, as LangChain sends it: on a second
            // run the ALTER TABLE fails with "already exists", which is fine.
            let index = ct.replace('.', "_");
            let fkey = t.replace('.', "_");
            let sql = format!(
                "CREATE TABLE IF NOT EXISTS {ct} (uuid uuid NOT NULL DEFAULT gen_random_uuid() PRIMARY KEY, name character varying, cmetadata jsonb);\n\
                 CREATE INDEX IF NOT EXISTS idx_{index}_name ON {ct}(name);\n\
                 ALTER TABLE {t} ADD COLUMN collection_id uuid;\n\
                 ALTER TABLE {t} ADD CONSTRAINT {fkey}_collection_id_fkey FOREIGN KEY (collection_id) REFERENCES {ct}(uuid) ON DELETE CASCADE;"
            );
            if let Err(e) = sqlx::raw_sql(&sql).execute(&self.pool).await {
                let e = pg_error("Error adding column or creating index: ")(e);
                if !e.message.contains("already exists") {
                    return Err(e);
                }
            }
        }
        Ok(())
    }

    /// `getOrCreateCollection`: the collection's UUID, created if missing.
    async fn collection_id(&self) -> NodeResult<Option<String>> {
        let Some((name, ct)) = &self.collection else { return Ok(None) };
        let found: Option<String> = sqlx::query_scalar(&format!("SELECT uuid::text from {ct} WHERE name = $1;")).bind(name).fetch_optional(&self.pool).await.map_err(pg_error(""))?;
        if found.is_some() {
            return Ok(found);
        }
        let created: Option<String> = sqlx::query_scalar(&format!("INSERT INTO {ct}(uuid, name, cmetadata) VALUES (gen_random_uuid(), $1, $2::jsonb) RETURNING uuid::text;"))
            .bind(name)
            .bind(None::<String>)
            .fetch_optional(&self.pool)
            .await
            .map_err(pg_error(""))?;
        Ok(created)
    }

    /// `addVectors`: rows of (content, vector, metadata[, collection_id]),
    /// inserted 500 at a time.
    async fn add(&self, docs: &[Document], vectors: &[Vec<f64>]) -> NodeResult<()> {
        let collection = self.collection_id().await?;
        let mut columns = vec![self.content_col.as_str(), self.vector_col.as_str(), self.metadata_col.as_str()];
        let mut casts = vec!["", "::vector", "::jsonb"];
        if collection.is_some() {
            columns.push("collection_id");
            casts.push("::uuid");
        }
        let rows: Vec<Vec<String>> = docs
            .iter()
            .zip(vectors)
            .map(|(d, v)| {
                let mut row = vec![d.page_content.replace('\0', ""), vector_literal(v).replace('\0', ""), Value::Object(d.metadata.clone()).to_string()];
                row.extend(collection.clone());
                row
            })
            .collect();
        let quoted: Vec<String> = columns.iter().map(|c| format!("\"{c}\"")).collect();
        for chunk in rows.chunks(500) {
            let values: Vec<String> = (0..chunk.len())
                .map(|j| format!("({})", casts.iter().enumerate().map(|(i, cast)| format!("${}{cast}", j * columns.len() + i + 1)).collect::<Vec<_>>().join(", ")))
                .collect();
            let sql = format!("INSERT INTO {}({}) VALUES {}", self.table, quoted.join(", "), values.join(", "));
            let mut q = sqlx::query(&sql);
            for v in chunk.iter().flatten() {
                q = q.bind(v);
            }
            q.execute(&self.pool).await.map_err(pg_error("Error inserting: "))?;
        }
        Ok(())
    }

    /// `searchPostgres`: the `k` nearest rows by the distance operator,
    /// scored with the raw distance (LangChain's default "distance"
    /// normalization), nearest first.
    async fn search(&self, query: &[f64], k: usize, filter: Option<&Map<String, Value>>) -> NodeResult<Vec<Hit>> {
        let mut params: Vec<Option<String>> = Vec::new();
        let mut clauses = Vec::new();
        let mut n = 2;
        if let Some(id) = self.collection_id().await? {
            n = 3;
            params.push(Some(id));
            clauses.push("collection_id = $3::uuid".to_string());
        }
        if let Some(f) = filter {
            filter_clauses(&self.metadata_col, f, &mut n, &mut clauses, &mut params);
        }
        let where_clause = if clauses.is_empty() { String::new() } else { format!("WHERE {}", clauses.join(" AND ")) };
        let sql = format!(
            "SELECT \"{c}\"::text AS _content, \"{m}\"::text AS _metadata, \"{i}\"::text AS _id, (\"{v}\" {op} $1::vector)::float8 AS \"_distance\" FROM {t} {where_clause} ORDER BY \"_distance\" ASC LIMIT $2",
            c = self.content_col,
            m = self.metadata_col,
            i = self.id_col,
            v = self.vector_col,
            op = self.operator,
            t = self.table,
        );
        let mut q = sqlx::query(&sql).bind(vector_literal(query)).bind(k as i64);
        for p in &params {
            q = q.bind(p);
        }
        let rows = q.fetch_all(&self.pool).await.map_err(pg_error(""))?;
        use sqlx::Row as _;
        Ok(rows
            .iter()
            .filter_map(|r| {
                let content: Option<String> = r.try_get("_content").ok()?;
                let distance: Option<f64> = r.try_get("_distance").ok()?;
                let metadata: Option<String> = r.try_get("_metadata").ok()?;
                let metadata = match metadata.and_then(|m| serde_json::from_str::<Value>(&m).ok()) {
                    Some(Value::Object(m)) => m,
                    _ => Map::new(),
                };
                Some(Hit { doc: Document { page_content: content?, metadata }, score: distance?, id: r.try_get::<Option<String>, _>("_id").ok().flatten().map(Value::String) })
            })
            .collect())
    }
}

/// `[x,y,...]`, the text form pgvector parses.
fn vector_literal(v: &[f64]) -> String {
    format!("[{}]", v.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(","))
}

/// A filter value as node-postgres sends it: strings as is, others as text.
fn param_text(v: &Value) -> Option<String> {
    match v {
        Value::Null => None,
        Value::String(s) => Some(s.clone()),
        other => Some(other.to_string()),
    }
}

/// `buildFilterClauses`: equality on `metadata->>'key'`, or the `in`,
/// `notIn`, `arrayContains`, `gt`/`gte`/`lt`/`lte` (numbers) and `neq`
/// operators. Keys are escaped as SQL string literals.
fn filter_clauses(column: &str, filter: &Map<String, Value>, n: &mut usize, clauses: &mut Vec<String>, params: &mut Vec<Option<String>>) {
    let mut next = |params: &mut Vec<Option<String>>, v: Option<String>, cast: &str| {
        *n += 1;
        params.push(v);
        format!("${}{cast}", *n)
    };
    for (key, value) in filter {
        let key = key.replace('\'', "''");
        let Value::Object(ops) = value else {
            let p = next(params, param_text(value), "::text");
            clauses.push(format!("{column}->>'{key}' = {p}"));
            continue;
        };
        for (op, sql) in [("in", "IN"), ("notIn", "NOT IN")] {
            if let Some(list) = ops.get(op).and_then(Value::as_array) {
                let ps: Vec<String> = list.iter().map(|v| next(params, param_text(v), "::text")).collect();
                clauses.push(format!("{column}->>'{key}' {sql} ({})", ps.join(",")));
            }
        }
        if let Some(list) = ops.get("arrayContains").and_then(Value::as_array) {
            let ps: Vec<String> = list.iter().map(|v| next(params, param_text(v), "::text")).collect();
            clauses.push(format!("{column}->'{key}' ?| array[{}]", ps.join(",")));
        }
        for (op, sql) in [("gt", ">"), ("gte", ">="), ("lt", "<"), ("lte", "<=")] {
            if let Some(v) = ops.get(op).filter(|v| v.is_number()) {
                let p = next(params, param_text(v), "::numeric");
                clauses.push(format!("({column}->>'{key}')::numeric {sql} {p}"));
            }
        }
        if let Some(v) = ops.get("neq") {
            let text = match v {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            let p = next(params, Some(text), "::text");
            clauses.push(format!("({column}->>'{key}' IS NULL OR ({column}->>'{key}')::text != {p})"));
        }
    }
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

// ---- Qdrant store ----------------------------------------------------------------------

struct VectorStoreQdrant;

const QDRANT: &str = "@n8n/n8n-nodes-langchain.vectorStoreQdrant";

#[async_trait::async_trait]
impl NodeType for VectorStoreQdrant {
    fn type_name(&self) -> &'static str {
        QDRANT
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

/// A failed Qdrant call, as the JS client's `ApiError` carries it.
struct QdrantError {
    status: u16,
    status_text: String,
    /// `data.status.error`, Qdrant's own explanation.
    error: Option<String>,
}

impl QdrantError {
    /// `normalizeVectorStoreError` on the client's `ApiError`.
    fn into_node_error(self) -> NodeError {
        NodeError::api(self.status_text.clone(), Some(self.status), self.error.or(Some(self.status_text)))
    }
}

/// LangChain's `QdrantVectorStore` on n8n's `createQdrantClient`.
pub(super) struct Qdrant {
    /// `{scheme}://{host}:{port}`: the client keeps no path from the URL.
    base: String,
    api_key: String,
    collection: String,
    content_key: String,
    metadata_key: String,
    collection_config: Option<Value>,
}

impl Qdrant {
    async fn open(ctx: &ExecCtx<'_>, node: &Node, item: usize) -> NodeResult<Qdrant> {
        let collection = match ctx.resolve_value(&node.parameters["qdrantCollection"], item)? {
            Value::Object(o) => o.get("value").and_then(Value::as_str).unwrap_or("").to_string(),
            v => v.as_str().unwrap_or("").to_string(),
        };
        let o = ctx.resolve_value(&node.parameters["options"], item)?;
        let key = |name: &str, default: &str| -> NodeResult<String> {
            match &o[name] {
                Value::Null => Ok(default.to_string()),
                Value::String(s) if s.is_empty() => Ok(default.to_string()),
                Value::String(s) => Ok(s.clone()),
                _ => Err(NodeError::new(format!("Parameter \"{name}\" is not string"))),
            }
        };
        let collection_config = match &o["collectionConfig"] {
            Value::String(s) if !s.trim().is_empty() => Some(serde_json::from_str(s).map_err(|_| NodeError::new("Parameter 'options.collectionConfig' could not be parsed as JSON"))?),
            Value::Object(m) => Some(Value::Object(m.clone())),
            _ => None,
        };
        let (_, cred) = ctx.credentials_for(node, "qdrantApi").await?;
        let raw = cred["qdrantUrl"].as_str().unwrap_or("");
        let invalid = || NodeError::new(format!("Invalid Qdrant URL: {raw}. Please provide a valid URL with protocol (http/https)"));
        let url = reqwest::Url::parse(raw).map_err(|_| invalid())?;
        let host = url.host_str().ok_or_else(invalid)?;
        let https = url.scheme() == "https";
        let port = url.port().unwrap_or(if https { 443 } else { 80 });
        let host = if host.contains(':') && !host.starts_with('[') { format!("[{host}]") } else { host.to_string() };
        Ok(Qdrant {
            base: format!("{}://{host}:{port}", if https { "https" } else { "http" }),
            api_key: cred["apiKey"].as_str().unwrap_or("").to_string(),
            collection,
            content_key: key("contentPayloadKey", "content")?,
            metadata_key: key("metadataPayloadKey", "metadata")?,
            collection_config,
        })
    }

    /// One REST call; `Ok` with the response's `result`.
    async fn call(&self, ctx: &ExecCtx<'_>, method: reqwest::Method, segments: &[&str], query: Option<&str>, body: Option<Value>) -> NodeResult<Result<Value, QdrantError>> {
        let mut url = reqwest::Url::parse(&self.base).map_err(|e| NodeError::new(format!("Invalid Qdrant URL: {e}")))?;
        url.path_segments_mut().map_err(|_| NodeError::new("Invalid Qdrant URL"))?.extend(segments);
        url.set_query(query);
        super::check_ssrf(&url, ctx.config()).await.map_err(NodeError::new)?;
        let mut req = ctx.services.http.request(method, url).header("api-key", &self.api_key);
        if let Some(b) = body {
            req = req.json(&b);
        }
        let resp = req.send().await.map_err(|e| NodeError::new(format!("The Qdrant server could not be reached: {}", e.without_url())))?;
        let status = resp.status();
        let json: Value = resp.json().await.unwrap_or(Value::Null);
        if status.is_success() {
            return Ok(Ok(json["result"].clone()));
        }
        Ok(Err(QdrantError {
            status: status.as_u16(),
            status_text: status.canonical_reason().unwrap_or("").to_string(),
            error: json.pointer("/status/error").and_then(Value::as_str).map(String::from),
        }))
    }

    /// `ensureCollection`: created when missing, from "Collection Config"
    /// or as `{size: <dimensions of embedQuery("test")>, distance: Cosine}`.
    async fn ensure_collection(&self, ctx: &ExecCtx<'_>, embeddings: &Embeddings<'_>) -> NodeResult<()> {
        let list = self.call(ctx, reqwest::Method::GET, &["collections"], None, None).await?.map_err(QdrantError::into_node_error)?;
        if list["collections"].as_array().into_iter().flatten().any(|c| c["name"].as_str() == Some(&self.collection)) {
            return Ok(());
        }
        let config = match &self.collection_config {
            Some(c) => c.clone(),
            None => json!({"vectors": {"size": embeddings.embed_query(ctx, "test").await?.len(), "distance": "Cosine"}}),
        };
        self.call(ctx, reqwest::Method::PUT, &["collections", &self.collection], None, Some(config)).await?.map_err(QdrantError::into_node_error)?;
        Ok(())
    }

    /// `addVectors`: one upsert of points with random UUIDs and
    /// `{content, metadata}` payloads.
    async fn add(&self, ctx: &ExecCtx<'_>, docs: &[Document], vectors: Vec<Vec<f64>>) -> NodeResult<()> {
        let points: Vec<Value> = docs
            .iter()
            .zip(vectors)
            .map(|(d, v)| {
                let mut payload = Map::new();
                payload.insert(self.content_key.clone(), json!(d.page_content));
                payload.insert(self.metadata_key.clone(), Value::Object(d.metadata.clone()));
                json!({"id": uuid::Uuid::new_v4().to_string(), "vector": v, "payload": payload})
            })
            .collect();
        match self.call(ctx, reqwest::Method::PUT, &["collections", &self.collection, "points"], Some("wait=true"), Some(json!({"points": points}))).await? {
            Ok(_) => Ok(()),
            Err(e) => {
                let message = format!("{} {}: {}", e.status, e.status_text, e.error.as_deref().unwrap_or("undefined"));
                Err(NodeError::api(message.clone(), None, Some(message)))
            }
        }
    }

    /// `similaritySearchVectorWithScore`: Qdrant's query API, scored by
    /// the collection's metric (cosine similarity by default).
    async fn search(&self, ctx: &ExecCtx<'_>, query: &[f64], k: usize, filter: Option<&Map<String, Value>>) -> NodeResult<Vec<Hit>> {
        let mut body = json!({"query": query, "limit": k, "with_payload": [self.metadata_key, self.content_key], "with_vector": false});
        if let Some(f) = filter {
            body["filter"] = Value::Object(f.clone());
        }
        let result = self.call(ctx, reqwest::Method::POST, &["collections", &self.collection, "points", "query"], None, Some(body)).await?.map_err(QdrantError::into_node_error)?;
        Ok(result["points"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|p| {
                let payload = &p["payload"];
                let page_content = match &payload[&self.content_key] {
                    Value::String(s) => s.clone(),
                    Value::Null => String::new(),
                    other => other.to_string(),
                };
                let metadata = payload[&self.metadata_key].as_object().cloned().unwrap_or_default();
                Hit { doc: Document { page_content, metadata }, score: p["score"].as_f64().unwrap_or(0.0), id: Some(p["id"].clone()) }
            })
            .collect())
    }
}

// ---- retrieval -----------------------------------------------------------------------

/// A similarity search on a vector store sub-node (retrieve modes): the
/// query embedded with the store's own embeddings sub-node, recorded on
/// the store like n8n's logWrapper.
pub(super) async fn store_search(ctx: &ExecCtx<'_>, store: &Node, query: &str, k: usize, item: usize) -> NodeResult<Vec<Hit>> {
    let started = now_ms();
    let embeddings = load_embeddings(ctx, &store.name, item).await?;
    let filter = metadata_filter(ctx, store, item)?;
    let result = match Store::open(ctx, store, &embeddings, item).await {
        Ok(s) => {
            let r = match embeddings.embed_query(ctx, query).await {
                Ok(vector) => s.search(ctx, &embeddings, &vector, k, filter.as_ref()).await,
                Err(e) => Err(e),
            };
            s.close().await;
            r
        }
        Err(e) => Err(e),
    };
    let hits = match result {
        Ok(hits) => hits,
        Err(e) => {
            record(ctx, &store.name, "ai_vectorStore", json!({"query": query}), Err(&e), started);
            return Err(e);
        }
    };
    let docs: Vec<Value> = hits.iter().map(Hit::to_json).collect();
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
    Ok(store_search(ctx, store, query, k, item).await?.into_iter().map(|h| h.doc).collect())
}
