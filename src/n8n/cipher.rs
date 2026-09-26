//! Credential encryption compatible with n8n (spec §6.5, goal G3).
//!
//! n8n stores credential data as CryptoJS `AES.encrypt(json, key)`: base64
//! of `"Salted__" || salt(8) || AES-256-CBC(PKCS7)`, with key and IV derived
//! from the passphrase by OpenSSL's `EVP_BytesToKey` (MD5, one round). r8r
//! reads and writes this format, so a database can move between r8r and n8n
//! in both directions with the same `N8N_ENCRYPTION_KEY`.

use aes::cipher::{block_padding::Pkcs7, BlockDecryptMut, BlockEncryptMut, KeyIvInit};
use base64::Engine as _;
use md5::{Digest, Md5};

type Enc = cbc::Encryptor<aes::Aes256>;
type Dec = cbc::Decryptor<aes::Aes256>;

fn evp_bytes_to_key(passphrase: &[u8], salt: &[u8]) -> ([u8; 32], [u8; 16]) {
    let mut derived = Vec::with_capacity(48);
    let mut previous: Vec<u8> = Vec::new();
    while derived.len() < 48 {
        let mut hasher = Md5::new();
        hasher.update(&previous);
        hasher.update(passphrase);
        hasher.update(salt);
        previous = hasher.finalize().to_vec();
        derived.extend_from_slice(&previous);
    }
    let mut key = [0u8; 32];
    let mut iv = [0u8; 16];
    key.copy_from_slice(&derived[..32]);
    iv.copy_from_slice(&derived[32..48]);
    (key, iv)
}

pub fn encrypt(passphrase: &str, plaintext: &str) -> String {
    let salt: [u8; 8] = rand::random();
    let (key, iv) = evp_bytes_to_key(passphrase.as_bytes(), &salt);
    let len = plaintext.len();
    let mut buf = plaintext.as_bytes().to_vec();
    buf.resize(len + 16, 0);
    let ciphertext = Enc::new(&key.into(), &iv.into())
        .encrypt_padded_mut::<Pkcs7>(&mut buf, len)
        .expect("buffer has room for one block of padding")
        .to_vec();
    let mut out = b"Salted__".to_vec();
    out.extend_from_slice(&salt);
    out.extend_from_slice(&ciphertext);
    base64::engine::general_purpose::STANDARD.encode(out)
}

pub fn decrypt(passphrase: &str, blob: &str) -> anyhow::Result<String> {
    let raw = base64::engine::general_purpose::STANDARD
        .decode(blob.trim())
        .map_err(|_| anyhow::anyhow!("credential data is not valid base64"))?;
    if raw.len() < 32 || &raw[..8] != b"Salted__" {
        anyhow::bail!("credential data is not in n8n's encrypted format");
    }
    let (key, iv) = evp_bytes_to_key(passphrase.as_bytes(), &raw[8..16]);
    let mut buf = raw[16..].to_vec();
    let plain = Dec::new(&key.into(), &iv.into())
        .decrypt_padded_mut::<Pkcs7>(&mut buf)
        .map_err(|_| anyhow::anyhow!("Credentials could not be decrypted. The likely reason is that a different \"encryptionKey\" was used to encrypt the data."))?;
    String::from_utf8(plain.to_vec()).map_err(|_| anyhow::anyhow!("Credentials could not be decrypted: not UTF-8"))
}

pub fn encrypt_json(passphrase: &str, data: &serde_json::Value) -> String {
    encrypt(passphrase, &data.to_string())
}

pub fn decrypt_json(passphrase: &str, blob: &str) -> anyhow::Result<serde_json::Value> {
    let text = decrypt(passphrase, blob)?;
    serde_json::from_str(&text).map_err(|_| anyhow::anyhow!("Credentials could not be decrypted: the data is not JSON"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_uses_the_openssl_salted_format() {
        let blob = encrypt("key", r#"{"a":1}"#);
        assert!(blob.starts_with("U2FsdGVkX1"), "base64 of Salted__: {blob}");
        assert_eq!(decrypt("key", &blob).unwrap(), r#"{"a":1}"#);
    }

    #[test]
    fn decrypts_a_blob_produced_by_openssl() {
        // "Salted__" + salt 0102030405060708 + the ciphertext of
        // printf '{"token":"abc"}' | openssl enc -aes-256-cbc -md md5 -base64 -A -k test-key -S 0102030405060708
        let blob = "U2FsdGVkX18BAgMEBQYHCCYQEpCz15vFSLRl36UoMLs=";
        assert_eq!(decrypt("test-key", blob).unwrap(), r#"{"token":"abc"}"#);
    }

    #[test]
    fn a_wrong_key_is_an_error() {
        let blob = encrypt("right", r#"{"a":1}"#);
        assert!(decrypt("wrong", &blob).is_err() || decrypt("wrong", &blob).unwrap() != r#"{"a":1}"#);
    }
}
