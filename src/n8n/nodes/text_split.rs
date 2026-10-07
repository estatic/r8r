//! LangChain's text splitters (`@langchain/textsplitters` 1.0.1, as n8n
//! 2.35.7 uses them): `CharacterTextSplitter` and
//! `RecursiveCharacterTextSplitter`, with `createDocuments`' line-number
//! metadata. Work is done on UTF-16 code units, so lengths, chunk
//! boundaries and indices match JavaScript's string semantics exactly;
//! `testdata/text_split_fixtures.json` holds outputs of the real package.

use serde_json::{json, Map, Value};

pub enum Kind {
    /// One separator; it is dropped unless `keep_separator`.
    Character { separator: String },
    /// Tries separators in order, recursing into pieces still too long;
    /// keeps the separator at the start of the following piece.
    Recursive { separators: Vec<String> },
}

pub struct Splitter {
    pub chunk_size: usize,
    pub chunk_overlap: usize,
    pub keep_separator: bool,
    pub kind: Kind,
}

/// A chunk with its metadata (`loc.lines.{from,to}` added).
pub struct Document {
    pub page_content: String,
    pub metadata: Map<String, Value>,
}

type U16s = Vec<u16>;

fn u16s(s: &str) -> U16s {
    s.encode_utf16().collect()
}

fn find(hay: &[u16], needle: &[u16], from: usize) -> Option<usize> {
    if needle.is_empty() {
        return Some(from.min(hay.len()));
    }
    (from..hay.len().saturating_sub(needle.len() - 1)).find(|&i| hay[i..].starts_with(needle))
}

/// JS `String.prototype.trim` on code units (whitespace and line terminators).
fn trim(s: &[u16]) -> &[u16] {
    let ws = |c: &u16| char::from_u32(*c as u32).is_some_and(|c| c.is_whitespace()) || *c == 0xFEFF;
    let start = s.iter().position(|c| !ws(c)).unwrap_or(s.len());
    let end = s.iter().rposition(|c| !ws(c)).map(|i| i + 1).unwrap_or(start);
    &s[start..end.max(start)]
}

impl Splitter {
    pub fn character(chunk_size: usize, chunk_overlap: usize, separator: &str) -> Result<Self, String> {
        Self::new(chunk_size, chunk_overlap, false, Kind::Character { separator: separator.to_string() })
    }

    /// LangChain's defaults: separators `["\n\n", "\n", " ", ""]`, separator kept.
    pub fn recursive(chunk_size: usize, chunk_overlap: usize) -> Result<Self, String> {
        let separators = ["\n\n", "\n", " ", ""].iter().map(|s| s.to_string()).collect();
        Self::new(chunk_size, chunk_overlap, true, Kind::Recursive { separators })
    }

    pub fn new(chunk_size: usize, chunk_overlap: usize, keep_separator: bool, kind: Kind) -> Result<Self, String> {
        if chunk_overlap >= chunk_size {
            return Err("Cannot have chunkOverlap >= chunkSize".into());
        }
        Ok(Self { chunk_size, chunk_overlap, keep_separator, kind })
    }

    pub fn split_text(&self, text: &str) -> Vec<String> {
        let text = u16s(text);
        let chunks = match &self.kind {
            Kind::Character { separator } => {
                let sep = u16s(separator);
                let splits = self.split_on_separator(&text, &sep);
                self.merge_splits(&splits, if self.keep_separator { &[] } else { &sep })
            }
            Kind::Recursive { separators } => self.split_recursive(&text, &separators.iter().map(|s| u16s(s)).collect::<Vec<_>>()),
        };
        chunks.iter().map(|c| String::from_utf16_lossy(c)).collect()
    }

    fn split_on_separator(&self, text: &[u16], sep: &[u16]) -> Vec<U16s> {
        let mut splits: Vec<U16s> = Vec::new();
        if sep.is_empty() {
            splits = text.iter().map(|c| vec![*c]).collect();
        } else if self.keep_separator {
            // `split(/(?=sep)/)`: cut before every occurrence after the start.
            let mut start = 0;
            let mut i = 1;
            while let Some(at) = find(text, sep, i) {
                splits.push(text[start..at].to_vec());
                start = at;
                i = at + 1;
            }
            splits.push(text[start..].to_vec());
        } else {
            let mut start = 0;
            while let Some(at) = find(text, sep, start) {
                splits.push(text[start..at].to_vec());
                start = at + sep.len();
            }
            splits.push(text[start..].to_vec());
        }
        splits.into_iter().filter(|s| !s.is_empty()).collect()
    }

    fn join(docs: &[U16s], sep: &[u16]) -> Option<U16s> {
        let mut joined = Vec::new();
        for (i, d) in docs.iter().enumerate() {
            if i > 0 {
                joined.extend_from_slice(sep);
            }
            joined.extend_from_slice(d);
        }
        let t = trim(&joined).to_vec();
        (!t.is_empty()).then_some(t)
    }

    fn merge_splits(&self, splits: &[U16s], sep: &[u16]) -> Vec<U16s> {
        let mut docs = Vec::new();
        let mut current: std::collections::VecDeque<U16s> = std::collections::VecDeque::new();
        let mut total = 0usize;
        for d in splits {
            let len = d.len();
            if total + len + current.len() * sep.len() > self.chunk_size && !current.is_empty() {
                if let Some(doc) = Self::join(current.make_contiguous(), sep) {
                    docs.push(doc);
                }
                while total > self.chunk_overlap || (total + len + current.len() * sep.len() > self.chunk_size && total > 0) {
                    let Some(first) = current.pop_front() else { break };
                    total -= first.len();
                }
            }
            current.push_back(d.clone());
            total += len;
        }
        if let Some(doc) = Self::join(current.make_contiguous(), sep) {
            docs.push(doc);
        }
        docs
    }

    fn split_recursive(&self, text: &[u16], separators: &[U16s]) -> Vec<U16s> {
        let mut final_chunks = Vec::new();
        let mut separator = separators.last().cloned().unwrap_or_default();
        let mut rest: Option<&[U16s]> = None;
        for (i, s) in separators.iter().enumerate() {
            if s.is_empty() {
                separator = s.clone();
                break;
            }
            if find(text, s, 0).is_some() {
                separator = s.clone();
                rest = Some(&separators[i + 1..]);
                break;
            }
        }
        let splits = self.split_on_separator(text, &separator);
        let merge_sep: &[u16] = if self.keep_separator { &[] } else { &separator };
        let mut good: Vec<U16s> = Vec::new();
        for s in splits {
            if s.len() < self.chunk_size {
                good.push(s);
            } else {
                if !good.is_empty() {
                    final_chunks.extend(self.merge_splits(&good, merge_sep));
                    good.clear();
                }
                match rest {
                    None => final_chunks.push(s),
                    Some(r) => final_chunks.extend(self.split_recursive(&s, r)),
                }
            }
        }
        if !good.is_empty() {
            final_chunks.extend(self.merge_splits(&good, merge_sep));
        }
        final_chunks
    }

    /// `createDocuments`: splits each text, adding `loc.lines` (1-based
    /// line numbers of the chunk in its text) to a copy of its metadata.
    pub fn create_documents(&self, texts: &[String], metadatas: &[Map<String, Value>]) -> Vec<Document> {
        let mut out = Vec::new();
        let newlines = |s: &[u16]| s.iter().filter(|c| **c == b'\n' as u16).count() as i64;
        for (i, text) in texts.iter().enumerate() {
            let full = u16s(text);
            let mut line = 1i64;
            let mut prev: Option<(usize, usize)> = None; // (index, length) of the previous chunk
            for chunk in self.split_text(text) {
                let c = u16s(&chunk);
                let from = prev.map(|(i, _)| i + 1).unwrap_or(0);
                // JS `indexOf` returns -1 when not found; slices then treat
                // it as "from the end", which never happens for real chunks.
                let index = find(&full, &c, from).unwrap_or(0);
                match prev {
                    None => line += newlines(&full[..index.min(full.len())]),
                    Some((pi, plen)) => {
                        let end_prev = pi + plen;
                        if end_prev < index {
                            line += newlines(&full[end_prev..index]);
                        } else if end_prev > index {
                            line -= newlines(&full[index..end_prev.min(full.len())]);
                        }
                    }
                }
                let count = newlines(&c);
                let mut metadata = metadatas.get(i).cloned().unwrap_or_default();
                let mut loc = metadata.get("loc").and_then(Value::as_object).cloned().unwrap_or_default();
                loc.insert("lines".into(), json!({"from": line, "to": line + count}));
                metadata.insert("loc".into(), Value::Object(loc));
                out.push(Document { page_content: chunk, metadata });
                line += count;
                prev = Some((index, c.len()));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Outputs of `@langchain/textsplitters` 1.0.1 for the same inputs.
    #[test]
    fn matches_langchain_outputs() {
        let cases: Value = serde_json::from_str(include_str!("testdata/text_split_fixtures.json")).unwrap();
        for case in cases.as_array().unwrap() {
            let size = case["opts"]["chunkSize"].as_u64().unwrap() as usize;
            let overlap = case["opts"]["chunkOverlap"].as_u64().unwrap() as usize;
            let splitter = match case["kind"].as_str().unwrap() {
                "recursive" => Splitter::recursive(size, overlap).unwrap(),
                _ => Splitter::character(size, overlap, case["opts"]["separator"].as_str().unwrap()).unwrap(),
            };
            let docs = splitter.create_documents(&[case["text"].as_str().unwrap().to_string()], &[]);
            let chunks: Vec<&str> = docs.iter().map(|d| d.page_content.as_str()).collect();
            let expected: Vec<&str> = case["chunks"].as_array().unwrap().iter().map(|c| c.as_str().unwrap()).collect();
            assert_eq!(chunks, expected, "case {}", case["opts"]);
            let lines: Vec<Value> = docs.iter().map(|d| d.metadata["loc"]["lines"].clone()).collect();
            assert_eq!(Value::Array(lines), case["lines"], "line numbers, case {}", case["opts"]);
        }
    }

    #[test]
    fn overlap_must_be_smaller_than_the_chunk() {
        assert!(Splitter::recursive(10, 10).is_err());
    }
}
