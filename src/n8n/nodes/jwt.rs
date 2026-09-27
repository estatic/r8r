//! JWT (spec §6.6): sign, decode and verify JSON Web Tokens against the
//! `jwtAuth` credential (a passphrase secret for HS*, a PEM key pair for
//! RS*/ES*/PS*).
//!
//! Faithful to n8n's `Jwt.node.js`: notably, the "Payload Claims" builder
//! passes its fields (`audience`, `expiresIn`, `issuer`, `jwtid`,
//! `notBefore`, `subject`) straight into the JWT payload *by those literal
//! names* -- it does not translate them into the registered claim
//! abbreviations (`aud`, `exp`, `iss`, `jti`, `nbf`, `sub`). A caller who
//! wants a real `exp`/`nbf`/etc. must use "Use JSON to Build Payload" and
//! write the registered claim name directly in the JSON.

use crate::n8n::node::{ExecCtx, NodeError, NodeResult, NodeType};
use crate::n8n::types::{Item, NodeOutput};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation};
use serde_json::{json, Value};
use std::str::FromStr;

pub fn all() -> Vec<Box<dyn NodeType>> {
    vec![Box::new(Jwt)]
}

struct Jwt;

/// n8n's `formatPemBlock`: a key pasted without `-----BEGIN ... -----`
/// framing is wrapped into a proper PEM block (64-char lines); a key that
/// already has that framing passes through unchanged (just trimmed).
fn format_pem_block(key: &str, public: bool) -> String {
    let trimmed = key.trim();
    if trimmed.contains("-----BEGIN") {
        return trimmed.to_string();
    }
    let label = if public { "PUBLIC KEY" } else { "PRIVATE KEY" };
    let body: String = trimmed.chars().filter(|c| !c.is_whitespace()).collect();
    let mut lines = vec![format!("-----BEGIN {label}-----")];
    for chunk in body.as_bytes().chunks(64) {
        lines.push(String::from_utf8_lossy(chunk).into_owned());
    }
    lines.push(format!("-----END {label}-----"));
    lines.join("\n")
}

fn parse_algorithm(name: &str) -> NodeResult<Algorithm> {
    Algorithm::from_str(name).map_err(|_| NodeError::new(format!("The algorithm \"{name}\" is not supported")))
}

/// Whether `alg` needs an HMAC passphrase, an RSA PEM key or an EC PEM key.
enum KeyFamily {
    Hmac,
    Rsa,
    Ec,
}

fn key_family(alg: Algorithm) -> KeyFamily {
    match alg {
        Algorithm::HS256 | Algorithm::HS384 | Algorithm::HS512 => KeyFamily::Hmac,
        Algorithm::ES256 | Algorithm::ES384 => KeyFamily::Ec,
        _ => KeyFamily::Rsa,
    }
}

fn encoding_key(alg: Algorithm, key_type: &str, secret: &str, private_key: &str) -> NodeResult<EncodingKey> {
    match key_family(alg) {
        KeyFamily::Hmac => {
            if key_type == "pemKey" {
                return Err(NodeError::new(format!("The {alg:?} algorithm needs a passphrase secret, not a PEM key")));
            }
            Ok(EncodingKey::from_secret(secret.as_bytes()))
        }
        family => {
            if key_type != "pemKey" {
                return Err(NodeError::new(format!("The {alg:?} algorithm needs a PEM private key")));
            }
            let pem = format_pem_block(private_key, false);
            let result = match family {
                KeyFamily::Ec => EncodingKey::from_ec_pem(pem.as_bytes()),
                _ => EncodingKey::from_rsa_pem(pem.as_bytes()),
            };
            result.map_err(|e| NodeError::new(format!("The private key could not be parsed: {e}")))
        }
    }
}

fn decoding_key(alg: Algorithm, key_type: &str, secret: &str, public_key: &str) -> NodeResult<DecodingKey> {
    match key_family(alg) {
        KeyFamily::Hmac => {
            if key_type == "pemKey" {
                return Err(NodeError::new(format!("The {alg:?} algorithm needs a passphrase secret, not a PEM key")));
            }
            Ok(DecodingKey::from_secret(secret.as_bytes()))
        }
        family => {
            if key_type != "pemKey" {
                return Err(NodeError::new(format!("The {alg:?} algorithm needs a PEM public key")));
            }
            let pem = format_pem_block(public_key, true);
            let result = match family {
                KeyFamily::Ec => DecodingKey::from_ec_pem(pem.as_bytes()),
                _ => DecodingKey::from_rsa_pem(pem.as_bytes()),
            };
            result.map_err(|e| NodeError::new(format!("The public key could not be parsed: {e}")))
        }
    }
}

/// Base64url-decodes one dot-separated JWT segment as JSON, without any
/// signature check -- used by `decode`, which (per n8n / the JWT spec)
/// never verifies.
fn decode_segment(segment: &str) -> NodeResult<Value> {
    let bytes = URL_SAFE_NO_PAD.decode(segment).map_err(|_| NodeError::new("The JWT token could not be decoded"))?;
    serde_json::from_slice(&bytes).map_err(|_| NodeError::new("The JWT token could not be decoded"))
}

fn split_token(token: &str) -> NodeResult<(&str, &str, &str)> {
    let parts: Vec<&str> = token.split('.').collect();
    match parts.as_slice() {
        [h, p, s] => Ok((h, p, s)),
        _ => Err(NodeError::new("The JWT token could not be decoded").describe("A JWT has three dot-separated parts: header, payload and signature")),
    }
}

/// Maps jsonwebtoken's verification errors onto n8n's user-facing wording.
fn verify_error(e: jsonwebtoken::errors::Error) -> NodeError {
    use jsonwebtoken::errors::ErrorKind;
    match e.kind() {
        ErrorKind::InvalidSignature => {
            NodeError::new("The JWT token can't be verified").describe("Be sure that the provided JWT token is correctly encoded and matches the selected credentials")
        }
        ErrorKind::ExpiredSignature => NodeError::new("The JWT token has expired"),
        ErrorKind::ImmatureSignature => NodeError::new("The JWT token is not active yet (its \"not before\" time is in the future)"),
        _ => NodeError::new(format!("The JWT token could not be verified: {e}")),
    }
}

fn get_token(ctx: &ExecCtx<'_>, i: usize) -> NodeResult<String> {
    let token = ctx.param_str("token", i, "")?;
    if token.is_empty() {
        return Err(NodeError::new("The JWT token was not provided").describe("Be sure to add a valid JWT token to the 'Token' parameter"));
    }
    Ok(token)
}

#[async_trait::async_trait]
impl NodeType for Jwt {
    fn type_name(&self) -> &'static str {
        "n8n-nodes-base.jwt"
    }

    async fn execute(&self, ctx: &mut ExecCtx<'_>) -> NodeResult<NodeOutput> {
        let mut out = Vec::new();
        let input = if ctx.input().is_empty() { vec![Item::default()] } else { ctx.input().to_vec() };

        // n8n resolves the credential once, up front, for every operation
        // (including decode, which never touches it).
        let (_, cred) = ctx.credentials("jwtAuth").await?;
        let cred_key_type = cred["keyType"].as_str().unwrap_or("passphrase").to_string();
        let cred_algorithm = cred["algorithm"].as_str().unwrap_or("HS256").to_string();
        let cred_secret = cred["secret"].as_str().unwrap_or("").to_string();
        let cred_private_key = cred["privateKey"].as_str().unwrap_or("").to_string();
        let cred_public_key = cred["publicKey"].as_str().unwrap_or("").to_string();

        for (i, _item) in input.iter().enumerate() {
            let operation = ctx.param_str("operation", i, "sign")?;
            match operation.as_str() {
                "sign" => {
                    let use_json = ctx.param_bool("useJson", i, false)?;
                    let payload = if use_json {
                        let raw = ctx.param_str("claimsJson", i, "{}")?;
                        serde_json::from_str::<Value>(&raw)
                            .map_err(|e| NodeError::new(format!("Payload Claims (JSON) is not valid JSON: {e}")).at(i))?
                    } else {
                        ctx.param("claims", i)?
                    };
                    if !payload.is_object() {
                        return Err(NodeError::new("Payload Claims (JSON) must be a JSON object").at(i));
                    }
                    let algorithm_name = ctx.param_str("options.algorithm", i, &cred_algorithm)?;
                    let alg = parse_algorithm(&algorithm_name).map_err(|e| e.at(i))?;
                    let key = encoding_key(alg, &cred_key_type, &cred_secret, &cred_private_key).map_err(|e| e.at(i))?;
                    let mut header = Header::new(alg);
                    let kid = ctx.param_str("options.kid", i, "")?;
                    if !kid.is_empty() {
                        header.kid = Some(kid);
                    }
                    let token = jsonwebtoken::encode(&header, &payload, &key).map_err(|e| NodeError::new(format!("The JWT could not be signed: {e}")).at(i))?;
                    out.push(Item::from_value(json!({ "token": token })).paired(i));
                }
                "decode" => {
                    let token = get_token(ctx, i).map_err(|e| e.at(i))?;
                    let (header_seg, payload_seg, signature_seg) = split_token(&token).map_err(|e| e.at(i))?;
                    let complete = ctx.param_bool("options.complete", i, false)?;
                    let payload = decode_segment(payload_seg).map_err(|e| e.at(i))?;
                    let value = if complete {
                        let header = decode_segment(header_seg).map_err(|e| e.at(i))?;
                        json!({ "header": header, "payload": payload, "signature": signature_seg })
                    } else {
                        json!({ "payload": payload })
                    };
                    out.push(Item::from_value(value).paired(i));
                }
                "verify" => {
                    let token = get_token(ctx, i).map_err(|e| e.at(i))?;
                    let algorithm_name = ctx.param_str("options.algorithm", i, &cred_algorithm)?;
                    let alg = parse_algorithm(&algorithm_name).map_err(|e| e.at(i))?;
                    let key = decoding_key(alg, &cred_key_type, &cred_secret, &cred_public_key).map_err(|e| e.at(i))?;
                    let ignore_expiration = ctx.param_bool("options.ignoreExpiration", i, false)?;
                    let ignore_not_before = ctx.param_bool("options.ignoreNotBefore", i, false)?;
                    let clock_tolerance = ctx.param_f64("options.clockTolerance", i, 0.0)?.max(0.0) as u64;
                    let complete = ctx.param_bool("options.complete", i, false)?;

                    let mut validation = Validation::new(alg);
                    // n8n's verify has no "required claims" concept: exp/nbf/aud/iss
                    // are only checked when present, never required.
                    validation.required_spec_claims.clear();
                    validation.validate_exp = !ignore_expiration;
                    validation.validate_nbf = !ignore_not_before;
                    validation.validate_aud = false;
                    validation.leeway = clock_tolerance;

                    let data = jsonwebtoken::decode::<Value>(&token, &key, &validation).map_err(|e| verify_error(e).at(i))?;
                    let value = if complete {
                        let (_, _, signature_seg) = split_token(&token).map_err(|e| e.at(i))?;
                        json!({ "header": data.header, "payload": data.claims, "signature": signature_seg })
                    } else {
                        json!({ "payload": data.claims })
                    };
                    out.push(Item::from_value(value).paired(i));
                }
                other => return Err(NodeError::new(format!("The JWT operation \"{other}\" is not supported")).at(i)),
            }
        }
        Ok(vec![out])
    }
}
