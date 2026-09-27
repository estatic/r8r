//! Compression node (spec §6.6): zip and gzip compress/decompress of binary
//! properties.
//!
//! n8n's real node (v1/v1.1) also supports `tar` and `tar.gz`; those are out
//! of scope here (not asked for), so an `outputFormat` of `tar`/`targz`, or
//! decompressing an archive whose `fileExtension` is `tar`/`tgz`, fails with
//! a clear "not supported" error rather than being silently mishandled.

use super::field_list;
use crate::n8n::node::{ExecCtx, NodeError, NodeResult, NodeType};
use crate::n8n::types::{Item, NodeOutput};
use base64::Engine as _;
use serde_json::{json, Map, Value};
use std::io::{Cursor, Read, Write};

pub struct Compression;

fn mime_for_ext(ext: &str) -> &'static str {
    match ext.to_ascii_lowercase().as_str() {
        "txt" => "text/plain",
        "json" => "application/json",
        "csv" => "text/csv",
        "html" | "htm" => "text/html",
        "xml" => "application/xml",
        "css" => "text/css",
        "js" => "text/javascript",
        "pdf" => "application/pdf",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "zip" => "application/zip",
        "gz" | "gzip" => "application/gzip",
        _ => "application/octet-stream",
    }
}

fn ext_for_mime(mime: &str) -> &'static str {
    match mime {
        "text/plain" => "txt",
        "application/json" => "json",
        "text/csv" => "csv",
        "text/html" => "html",
        "application/xml" => "xml",
        "text/css" => "css",
        "text/javascript" => "js",
        "application/pdf" => "pdf",
        "image/png" => "png",
        "image/jpeg" => "jpg",
        "image/gif" => "gif",
        "application/zip" => "zip",
        "application/gzip" => "gz",
        _ => "bin",
    }
}

/// n8n's binary data shape: base64 `data`, `mimeType`, `fileExtension`,
/// `fileSize` (`"<n> B"`) and an optional `fileName`. `fileExtension` is
/// taken from `file_name` when it has one, else derived from `mime`.
fn binary_entry(data: &[u8], mime: &str, file_name: Option<&str>) -> Value {
    let ext = file_name
        .and_then(|f| f.rsplit_once('.'))
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_else(|| ext_for_mime(mime).to_string());
    let mut v = json!({
        "data": base64::engine::general_purpose::STANDARD.encode(data),
        "mimeType": mime,
        "fileExtension": ext,
        "fileSize": format!("{} B", data.len()),
    });
    if let Some(f) = file_name {
        v["fileName"] = json!(f);
    }
    v
}

/// Decodes the named binary property of `item`, alongside its metadata map,
/// or a clear error naming the missing property (matches n8n's
/// `assertBinaryData`).
fn binary_bytes<'a>(item: &'a Item, name: &str, i: usize) -> NodeResult<(Vec<u8>, &'a Map<String, Value>)> {
    let entry = item.binary.as_ref().and_then(|b| b.get(name)).ok_or_else(|| NodeError::new(format!("Item has no binary field '{name}'")).at(i))?;
    let meta = entry.as_object().ok_or_else(|| NodeError::new(format!("Item has no binary field '{name}'")).at(i))?;
    let data = meta.get("data").and_then(Value::as_str).ok_or_else(|| NodeError::new(format!("Item has no binary field '{name}'")).at(i))?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(data.trim())
        .map_err(|e| NodeError::new(format!("The binary field '{name}' does not contain valid base64 data: {e}")).at(i))?;
    Ok((bytes, meta))
}

fn zip_bytes(entries: &[(String, Vec<u8>)]) -> Result<Vec<u8>, String> {
    let mut buf = Vec::new();
    {
        let mut writer = zip::ZipWriter::new(Cursor::new(&mut buf));
        let options = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        for (name, data) in entries {
            writer.start_file(name, options).map_err(|e| e.to_string())?;
            writer.write_all(data).map_err(|e| e.to_string())?;
        }
        writer.finish().map_err(|e| e.to_string())?;
    }
    Ok(buf)
}

fn gzip_bytes(data: &[u8]) -> Vec<u8> {
    let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    // Writing to an in-memory Vec cannot fail.
    enc.write_all(data).expect("gzip encode into a memory buffer");
    enc.finish().expect("gzip finish into a memory buffer")
}

fn gunzip_bytes(data: &[u8]) -> std::io::Result<Vec<u8>> {
    let mut dec = flate2::read::GzDecoder::new(data);
    let mut out = Vec::new();
    dec.read_to_end(&mut out)?;
    Ok(out)
}

/// n8n's `fileName.split('.')[0]`: only the text before the *first* dot
/// survives, even if the name has more than one.
fn first_segment(name: &str) -> &str {
    name.split('.').next().unwrap_or(name)
}

impl Compression {
    fn compress(&self, ctx: &ExecCtx<'_>, i: usize, item: &Item) -> NodeResult<Item> {
        let raw_names = ctx.param_str("binaryPropertyName", i, "data")?;
        let names = field_list(&Value::String(raw_names));
        let output_format = ctx.param_str("outputFormat", i, "zip")?;
        match output_format.as_str() {
            "zip" => {
                let mut entries = Vec::with_capacity(names.len());
                for name in &names {
                    let (bytes, meta) = binary_bytes(item, name, i)?;
                    let file_name = meta.get("fileName").and_then(Value::as_str).map(String::from).unwrap_or_else(|| name.clone());
                    entries.push((file_name, bytes));
                }
                let file_name = {
                    let f = ctx.param_str("fileName", i, "")?;
                    if f.is_empty() {
                        "data.zip".to_string()
                    } else {
                        f
                    }
                };
                let output_prop = ctx.param_str("binaryPropertyOutput", i, "data")?;
                let archive = zip_bytes(&entries).map_err(|e| NodeError::new(format!("Could not create the zip archive: {e}")).at(i))?;
                let mut binary = Map::new();
                binary.insert(output_prop, binary_entry(&archive, "application/zip", Some(&file_name)));
                Ok(Item { json: item.json.clone(), binary: Some(binary), paired_item: None }.paired(i))
            }
            "gzip" => {
                let mut binary = Map::new();
                for (idx, name) in names.iter().enumerate() {
                    let (bytes, meta) = binary_bytes(item, name, i)?;
                    let compressed = gzip_bytes(&bytes);
                    let output_prefix = ctx.param_str("binaryPropertyOutput", i, "data")?;
                    let prop = if idx == 0 { output_prefix.clone() } else { format!("{output_prefix}{idx}") };
                    let user_file_name = ctx.param_str("fileName", i, "")?;
                    let orig_name = meta.get("fileName").and_then(Value::as_str);
                    let base = if !user_file_name.is_empty() {
                        user_file_name.trim_end_matches(".gzip").trim_end_matches(".gz").to_string()
                    } else {
                        orig_name.map(first_segment).unwrap_or(name).to_string()
                    };
                    let orig_ext = meta.get("fileExtension").and_then(Value::as_str).map(|s| s.to_ascii_lowercase()).filter(|s| !s.is_empty());
                    let file_path = match &orig_ext {
                        Some(e) => format!("{base}.{e}.gz"),
                        None => format!("{base}.gz"),
                    };
                    binary.insert(prop, binary_entry(&compressed, "application/gzip", Some(&file_path)));
                }
                Ok(Item { json: item.json.clone(), binary: Some(binary), paired_item: None }.paired(i))
            }
            other => Err(NodeError::new(format!("The output format \"{other}\" is not supported; use \"zip\" or \"gzip\"")).at(i)),
        }
    }

    fn decompress(&self, ctx: &ExecCtx<'_>, i: usize, item: &Item) -> NodeResult<Item> {
        let raw_names = ctx.param_str("binaryPropertyName", i, "data")?;
        let names = field_list(&Value::String(raw_names));
        let output_prefix = ctx.param_str("outputPrefix", i, "file_")?;
        let mut binary = Map::new();
        let mut zip_index = 0usize;
        for (idx, name) in names.iter().enumerate() {
            let (bytes, meta) = binary_bytes(item, name, i)?;
            let ext = meta.get("fileExtension").and_then(Value::as_str).map(|s| s.to_ascii_lowercase()).filter(|s| !s.is_empty());
            let Some(ext) = ext else {
                return Err(NodeError::new(format!("File extension not found for binary data {name}")).at(i));
            };
            match ext.as_str() {
                "zip" => {
                    let mut archive = zip::ZipArchive::new(Cursor::new(&bytes))
                        .map_err(|e| NodeError::new(format!("The file in '{name}' is not a valid zip archive: {e}")).at(i))?;
                    for j in 0..archive.len() {
                        let mut file = archive.by_index(j).map_err(|e| NodeError::new(format!("Could not read the zip archive: {e}")).at(i))?;
                        if file.is_dir() || file.name().contains("__MACOSX") {
                            continue;
                        }
                        let entry_name = file.name().to_string();
                        let mut data = Vec::new();
                        file.read_to_end(&mut data).map_err(|e| NodeError::new(format!("Could not read '{entry_name}' from the zip archive: {e}")).at(i))?;
                        let entry_ext = entry_name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase());
                        let mime = entry_ext.as_deref().map(mime_for_ext).unwrap_or("application/octet-stream");
                        let prop = format!("{output_prefix}{zip_index}");
                        zip_index += 1;
                        binary.insert(prop, binary_entry(&data, mime, Some(&entry_name)));
                    }
                }
                "gz" | "gzip" => {
                    let decompressed = gunzip_bytes(&bytes).map_err(|e| NodeError::new(format!("The file in '{name}' is not a valid gzip archive: {e}")).at(i))?;
                    let orig_name = meta.get("fileName").and_then(Value::as_str);
                    let base = orig_name.map(first_segment).unwrap_or(name).to_string();
                    let mut extracted_ext: Option<String> = None;
                    if let Some(n) = orig_name {
                        if n.to_ascii_lowercase().ends_with(".gz") && n.len() >= 3 {
                            let stripped = &n[..n.len() - 3];
                            if let Some((_, e)) = stripped.rsplit_once('.') {
                                extracted_ext = Some(e.to_ascii_lowercase());
                            }
                        }
                    }
                    let mime = extracted_ext.as_deref().map(mime_for_ext).unwrap_or("application/octet-stream").to_string();
                    let final_ext = extracted_ext.unwrap_or_else(|| ext_for_mime(&mime).to_string());
                    let file_name = format!("{base}.{final_ext}");
                    let prop = format!("{output_prefix}{idx}");
                    binary.insert(prop, binary_entry(&decompressed, &mime, Some(&file_name)));
                }
                other => {
                    return Err(NodeError::new(format!("Unsupported archive format \".{other}\" for binary data {name}"))
                        .describe("The Decompress operation supports the following formats: zip and gzip")
                        .at(i));
                }
            }
        }
        Ok(Item { json: item.json.clone(), binary: Some(binary), paired_item: None }.paired(i))
    }
}

#[async_trait::async_trait]
impl NodeType for Compression {
    fn type_name(&self) -> &'static str {
        "n8n-nodes-base.compression"
    }

    async fn execute(&self, ctx: &mut ExecCtx<'_>) -> NodeResult<NodeOutput> {
        let input = if ctx.input().is_empty() { vec![Item::default()] } else { ctx.input().to_vec() };
        let mut out = Vec::new();
        for (i, item) in input.iter().enumerate() {
            let operation = ctx.param_str("operation", i, "decompress")?;
            let result = match operation.as_str() {
                "compress" => self.compress(ctx, i, item),
                "decompress" => self.decompress(ctx, i, item),
                other => Err(NodeError::new(format!("The operation \"{other}\" is not known")).at(i)),
            };
            match result {
                Ok(item) => out.push(item),
                Err(e) if ctx.continue_on_fail() => ctx.push_error_item(&e, i),
                Err(e) => return Err(e),
            }
        }
        Ok(vec![out])
    }
}
