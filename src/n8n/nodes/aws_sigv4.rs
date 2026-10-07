//! AWS Signature Version 4 for JSON APIs (Bedrock runtime): signs `host`,
//! `content-type`, `x-amz-date` and, with temporary credentials,
//! `x-amz-security-token`, over the SHA-256 of the body.

use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};

pub struct AwsCredentials<'a> {
    pub access_key_id: &'a str,
    pub secret_access_key: &'a str,
    pub session_token: Option<&'a str>,
}

fn hmac(key: &[u8], data: &str) -> Vec<u8> {
    let mut mac = Hmac::<Sha256>::new_from_slice(key).expect("HMAC takes any key length");
    mac.update(data.as_bytes());
    mac.finalize().into_bytes().to_vec()
}

/// Encodes an already percent-encoded path again, segment by segment, as
/// SigV4 asks of every service but S3.
fn canonical_uri(path: &str) -> String {
    path.split('/').map(|seg| seg.replace('%', "%25")).collect::<Vec<_>>().join("/")
}

/// The headers to add to the request (`x-amz-date`, `authorization`, and
/// `x-amz-security-token` when set). `path` is the request path as sent
/// (percent-encoded), `host` the `Host` header value (with a non-default
/// port), `amz_date` like `20150830T123600Z`.
#[allow(clippy::too_many_arguments)]
pub fn sign(creds: &AwsCredentials<'_>, region: &str, service: &str, method: &str, host: &str, path: &str, query: &str, content_type: Option<&str>, body: &[u8], amz_date: &str) -> Vec<(&'static str, String)> {
    let date = &amz_date[..8];
    let mut headers: Vec<(&str, String)> = Vec::new();
    if let Some(ct) = content_type {
        headers.push(("content-type", ct.to_string()));
    }
    headers.push(("host", host.to_string()));
    headers.push(("x-amz-date", amz_date.to_string()));
    if let Some(token) = creds.session_token {
        headers.push(("x-amz-security-token", token.to_string()));
    }
    let canonical_headers: String = headers.iter().map(|(k, v)| format!("{k}:{}\n", v.trim())).collect();
    let signed_headers = headers.iter().map(|(k, _)| *k).collect::<Vec<_>>().join(";");
    let payload_hash = hex::encode(Sha256::digest(body));
    let canonical_request = format!("{method}\n{}\n{query}\n{canonical_headers}\n{signed_headers}\n{payload_hash}", canonical_uri(path));
    let scope = format!("{date}/{region}/{service}/aws4_request");
    let string_to_sign = format!("AWS4-HMAC-SHA256\n{amz_date}\n{scope}\n{}", hex::encode(Sha256::digest(canonical_request.as_bytes())));
    let k_date = hmac(format!("AWS4{}", creds.secret_access_key).as_bytes(), date);
    let k_region = hmac(&k_date, region);
    let k_service = hmac(&k_region, service);
    let k_signing = hmac(&k_service, "aws4_request");
    let signature = hex::encode(hmac(&k_signing, &string_to_sign));
    let mut out = vec![
        ("x-amz-date", amz_date.to_string()),
        ("authorization", format!("AWS4-HMAC-SHA256 Credential={}/{scope}, SignedHeaders={signed_headers}, Signature={signature}", creds.access_key_id)),
    ];
    if let Some(token) = creds.session_token {
        out.push(("x-amz-security-token", token.to_string()));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// AWS's SigV4 test suite, `get-vanilla`.
    #[test]
    fn signs_the_get_vanilla_example() {
        let creds = AwsCredentials { access_key_id: "AKIDEXAMPLE", secret_access_key: "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY", session_token: None };
        let headers = sign(&creds, "us-east-1", "service", "GET", "example.amazonaws.com", "/", "", None, b"", "20150830T123600Z");
        let auth = &headers.iter().find(|(k, _)| *k == "authorization").unwrap().1;
        assert_eq!(
            auth,
            "AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20150830/us-east-1/service/aws4_request, SignedHeaders=host;x-amz-date, Signature=5fa00fa31553b73ebf1942676e86291e8372ff2a2260956d9b8aae1d763fbf31"
        );
    }

    #[test]
    fn encoded_path_segments_are_encoded_twice() {
        assert_eq!(canonical_uri("/model/anthropic.claude-v2%3A1/converse"), "/model/anthropic.claude-v2%253A1/converse");
    }
}
