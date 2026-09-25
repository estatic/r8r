//! n8n's credential encryption, reimplemented test-side so scenarios can
//! build fixtures and check exports independently of r8r's own code.
//!
//! n8n stores `credentials_entity.data` as CryptoJS `AES.encrypt(json,
//! passphrase)`: base64 of `"Salted__" || salt(8) || AES-256-CBC(PKCS7)`,
//! with key and IV derived by OpenSSL's `EVP_BytesToKey` (MD5, 1 round).
//! Spec §6.5.

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
    let mut buf = plaintext.as_bytes().to_vec();
    let len = buf.len();
    buf.resize(len + 16, 0);
    let ciphertext = Enc::new(&key.into(), &iv.into())
        .encrypt_padded_mut::<Pkcs7>(&mut buf, len)
        .expect("buffer has room for padding")
        .to_vec();
    let mut out = b"Salted__".to_vec();
    out.extend_from_slice(&salt);
    out.extend_from_slice(&ciphertext);
    base64::engine::general_purpose::STANDARD.encode(out)
}

pub fn decrypt(passphrase: &str, blob: &str) -> Result<String, String> {
    let raw = base64::engine::general_purpose::STANDARD
        .decode(blob.trim())
        .map_err(|e| format!("not base64: {e}"))?;
    if raw.len() < 16 || &raw[..8] != b"Salted__" {
        return Err("missing CryptoJS/OpenSSL \"Salted__\" header".into());
    }
    let (key, iv) = evp_bytes_to_key(passphrase.as_bytes(), &raw[8..16]);
    let mut buf = raw[16..].to_vec();
    let plain = Dec::new(&key.into(), &iv.into())
        .decrypt_padded_mut::<Pkcs7>(&mut buf)
        .map_err(|_| "bad padding (wrong key?)".to_string())?;
    String::from_utf8(plain.to_vec()).map_err(|e| format!("not UTF-8: {e}"))
}
