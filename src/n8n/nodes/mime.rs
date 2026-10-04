//! Hand-rolled raw-MIME parsing shared by nodes that receive whole email
//! messages rather than building them: the Gmail node's `get`/`getAll` with
//! `simple: false` (n8n's `parseRawEmail`, originally on top of
//! `mailparser`) and the Email Trigger (IMAP) node (n8n's `@n8n/imap` +
//! `mailparser`).
//!
//! No mail-parsing crate is in the dependency tree, so this is a small
//! hand-rolled parser: it handles multipart/mixed/alternative, base64 and
//! quoted-printable bodies, which covers what both IMAP servers and
//! Gmail's API produce, but -- unlike `mailparser` -- does not decode
//! non-UTF-8 charsets, RFC 2047 encoded-words in headers, or fully RFC
//! 5322-compliant address/comment syntax.

use base64::Engine;

/// A MIME body part after decoding: a leaf of the (possibly multipart) MIME
/// tree, with its `Content-Transfer-Encoding` already undone.
pub struct ParsedPart {
    pub content_type: String,
    pub filename: Option<String>,
    pub is_attachment: bool,
    pub body: Vec<u8>,
}

/// Splits a raw message (or MIME part) into its header block and body at
/// the first blank line.
pub fn split_headers_body(s: &str) -> (&str, &str) {
    if let Some(pos) = s.find("\r\n\r\n") {
        (&s[..pos], &s[pos + 4..])
    } else if let Some(pos) = s.find("\n\n") {
        (&s[..pos], &s[pos + 2..])
    } else {
        (s, "")
    }
}

/// Parses a header block into `(name, value)` pairs, unfolding continuation
/// lines (leading whitespace) onto the previous header.
pub fn parse_header_block(block: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for raw_line in block.split('\n') {
        let line = raw_line.trim_end_matches('\r');
        if line.is_empty() {
            continue;
        }
        if (line.starts_with(' ') || line.starts_with('\t')) && !out.is_empty() {
            let last = out.last_mut().unwrap();
            last.1.push(' ');
            last.1.push_str(line.trim());
        } else if let Some(idx) = line.find(':') {
            out.push((line[..idx].trim().to_string(), line[idx + 1..].trim().to_string()));
        }
    }
    out
}

pub fn header_value<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
}

/// Reads `param="value"` or `param=value` out of a `Content-Type`/
/// `Content-Disposition` header value.
pub fn header_param(value: &str, param: &str) -> Option<String> {
    for part in value.split(';').skip(1) {
        let part = part.trim();
        if let Some(rest) = part.strip_prefix(&format!("{param}=")) {
            return Some(rest.trim_matches('"').to_string());
        }
    }
    None
}

fn quoted_printable_decode(s: &str) -> Vec<u8> {
    let bytes = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'=' {
            if i + 2 < bytes.len() && bytes[i + 1] == b'\r' && bytes[i + 2] == b'\n' {
                i += 3;
                continue;
            }
            if i + 1 < bytes.len() && bytes[i + 1] == b'\n' {
                i += 2;
                continue;
            }
            if i + 2 < bytes.len() {
                if let Ok(h) = u8::from_str_radix(std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or(""), 16) {
                    out.push(h);
                    i += 3;
                    continue;
                }
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    out
}

pub fn content_transfer_decode(encoding: &str, body: &str) -> Vec<u8> {
    match encoding.to_ascii_lowercase().as_str() {
        "base64" => {
            let cleaned: String = body.chars().filter(|c| !c.is_whitespace()).collect();
            base64::engine::general_purpose::STANDARD.decode(cleaned.as_bytes()).unwrap_or_default()
        }
        "quoted-printable" => quoted_printable_decode(body),
        _ => body.as_bytes().to_vec(),
    }
}

fn split_multipart<'a>(body: &'a str, delim: &str) -> Vec<&'a str> {
    let mut segments = Vec::new();
    let mut rest = body;
    while let Some(pos) = rest.find(delim) {
        let after = &rest[pos + delim.len()..];
        if after.starts_with("--") {
            break;
        }
        let after = after.strip_prefix("\r\n").or_else(|| after.strip_prefix('\n')).unwrap_or(after);
        match after.find(delim) {
            Some(next_pos) => {
                segments.push(&after[..next_pos]);
                rest = &after[next_pos..];
            }
            None => {
                segments.push(after);
                break;
            }
        }
    }
    segments
}

/// Recursively walks a (possibly multipart) MIME tree, appending every leaf
/// part to `out`.
pub fn parse_parts(headers: &[(String, String)], body_str: &str, out: &mut Vec<ParsedPart>) {
    let ctype_header = header_value(headers, "Content-Type").unwrap_or("text/plain").to_string();
    let main_type = ctype_header.split(';').next().unwrap_or("text/plain").trim().to_ascii_lowercase();
    if let Some(stripped) = main_type.strip_prefix("multipart/") {
        let _ = stripped;
        if let Some(boundary) = header_param(&ctype_header, "boundary") {
            let delim = format!("--{boundary}");
            for segment in split_multipart(body_str, &delim) {
                let (seg_headers_block, seg_body) = split_headers_body(segment);
                let seg_headers = parse_header_block(seg_headers_block);
                parse_parts(&seg_headers, seg_body, out);
            }
        }
        return;
    }
    let cte = header_value(headers, "Content-Transfer-Encoding").unwrap_or("7bit").to_string();
    let decoded = content_transfer_decode(&cte, body_str);
    let disposition = header_value(headers, "Content-Disposition").unwrap_or("").to_string();
    let filename = header_param(&disposition, "filename").or_else(|| header_param(&ctype_header, "name"));
    let is_attachment = disposition.to_ascii_lowercase().starts_with("attachment") || (filename.is_some() && !main_type.starts_with("text/"));
    out.push(ParsedPart { content_type: main_type, filename, is_attachment, body: decoded });
}

/// Parses a whole raw message (headers + body) into its header pairs and
/// leaf MIME parts.
pub fn parse_message(raw: &str) -> (Vec<(String, String)>, Vec<ParsedPart>) {
    let (header_block, body) = split_headers_body(raw);
    let headers = parse_header_block(header_block);
    let mut parts = Vec::new();
    parse_parts(&headers, body, &mut parts);
    (headers, parts)
}

/// Splits parsed leaf parts into `(text/plain, text/html, everything else)`,
/// the way both the Gmail and Email Trigger (IMAP) nodes pick the "body" of
/// a message and treat the rest as attachments.
pub fn split_text_and_attachments(parts: Vec<ParsedPart>) -> (String, Option<String>, Vec<ParsedPart>) {
    let mut text_body = String::new();
    let mut html_body: Option<String> = None;
    let mut attachments = Vec::new();
    for p in parts {
        if !p.is_attachment && p.content_type == "text/plain" && text_body.is_empty() {
            text_body = String::from_utf8_lossy(&p.body).into_owned();
        } else if !p.is_attachment && p.content_type == "text/html" && html_body.is_none() {
            html_body = Some(String::from_utf8_lossy(&p.body).into_owned());
        } else {
            attachments.push(p);
        }
    }
    (text_body, html_body, attachments)
}
