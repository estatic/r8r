//! AES-256-GCM encryption for credential storage at rest.
//!
//! `encrypt`/`decrypt` operate on a caller-supplied 32-byte key: a fresh
//! random 12-byte nonce is generated per `encrypt` call, prepended to the
//! ciphertext, and the whole thing is base64-encoded. This module is
//! self-contained: it knows nothing about `domain`/`storage`.

use aes_gcm::aead::{Aead, KeyInit, OsRng};
use aes_gcm::{Aes256Gcm, AeadCore, Key, Nonce};
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;

/// Encrypts `plaintext` with `key` using AES-256-GCM and a fresh random
/// 96-bit nonce. Returns `base64(nonce || ciphertext)`.
pub fn encrypt(key: &[u8; 32], plaintext: &str) -> anyhow::Result<String> {
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key));
    let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
    let ciphertext = cipher
        .encrypt(&nonce, plaintext.as_bytes())
        .map_err(|_| anyhow::anyhow!("encryption failed"))?;
    let mut combined = nonce.to_vec();
    combined.extend_from_slice(&ciphertext);
    Ok(BASE64.encode(combined))
}

/// Reverses `encrypt`. Returns `Err` on malformed base64, a blob too short
/// to contain a nonce, or a failed AEAD tag check (wrong key or tampered
/// data) — never partial/garbage plaintext.
pub fn decrypt(key: &[u8; 32], blob: &str) -> anyhow::Result<String> {
    let combined = BASE64
        .decode(blob)
        .map_err(|_| anyhow::anyhow!("invalid base64"))?;
    if combined.len() < 12 {
        return Err(anyhow::anyhow!("ciphertext blob too short"));
    }
    let (nonce_bytes, ciphertext) = combined.split_at(12);
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key));
    let nonce = Nonce::from_slice(nonce_bytes);
    let plaintext = cipher
        .decrypt(nonce, ciphertext)
        .map_err(|_| anyhow::anyhow!("decryption failed"))?;
    String::from_utf8(plaintext).map_err(|_| anyhow::anyhow!("decrypted data is not valid UTF-8"))
}

/// Reads the named env var, base64-decodes it, and returns it as a 32-byte
/// key. `Err` if unset, not valid base64, or not exactly 32 bytes decoded.
pub fn load_key_from_env(var_name: &str) -> anyhow::Result<[u8; 32]> {
    let encoded = std::env::var(var_name)
        .map_err(|_| anyhow::anyhow!("{var_name} environment variable must be set"))?;
    let decoded = BASE64
        .decode(&encoded)
        .map_err(|_| anyhow::anyhow!("{var_name} is not valid base64"))?;
    decoded
        .try_into()
        .map_err(|v: Vec<u8>| anyhow::anyhow!("{var_name} must decode to exactly 32 bytes, got {}", v.len()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_key() -> [u8; 32] {
        [7u8; 32]
    }

    #[test]
    fn encrypt_then_decrypt_round_trips() {
        let key = test_key();
        let ciphertext = encrypt(&key, "super secret token").unwrap();
        let plaintext = decrypt(&key, &ciphertext).unwrap();
        assert_eq!(plaintext, "super secret token");
    }

    #[test]
    fn two_encryptions_of_the_same_plaintext_differ() {
        // Different random nonces each call -> different ciphertext blobs,
        // even for identical input. Proves the nonce isn't fixed/reused.
        let key = test_key();
        let a = encrypt(&key, "same input").unwrap();
        let b = encrypt(&key, "same input").unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn decrypt_with_wrong_key_fails() {
        let ciphertext = encrypt(&test_key(), "secret").unwrap();
        let wrong_key = [9u8; 32];
        assert!(decrypt(&wrong_key, &ciphertext).is_err());
    }

    #[test]
    fn decrypt_malformed_blob_returns_err_not_panic() {
        assert!(decrypt(&test_key(), "not-valid-base64!!!").is_err());
        assert!(decrypt(&test_key(), "").is_err());
    }

    #[test]
    fn load_key_from_env_reads_and_decodes_base64() {
        let key_bytes = [3u8; 32];
        let encoded = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, key_bytes);
        std::env::set_var("R8R_TEST_CREDENTIALS_KEY", &encoded);
        let loaded = load_key_from_env("R8R_TEST_CREDENTIALS_KEY").unwrap();
        assert_eq!(loaded, key_bytes);
        std::env::remove_var("R8R_TEST_CREDENTIALS_KEY");
    }

    #[test]
    fn load_key_from_env_errors_when_unset() {
        std::env::remove_var("R8R_TEST_CREDENTIALS_KEY_MISSING");
        assert!(load_key_from_env("R8R_TEST_CREDENTIALS_KEY_MISSING").is_err());
    }

    #[test]
    fn load_key_from_env_errors_on_wrong_length() {
        let short = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, [1u8; 16]);
        std::env::set_var("R8R_TEST_CREDENTIALS_KEY_SHORT", &short);
        assert!(load_key_from_env("R8R_TEST_CREDENTIALS_KEY_SHORT").is_err());
        std::env::remove_var("R8R_TEST_CREDENTIALS_KEY_SHORT");
    }
}
