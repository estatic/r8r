use argon2::password_hash::{rand_core::OsRng, SaltString};
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub fn hash_password(password: &str) -> anyhow::Result<String> {
    let salt = SaltString::generate(&mut OsRng);
    let hash = Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map_err(|e| anyhow::anyhow!("hash failed: {e}"))?;
    Ok(hash.to_string())
}

pub fn verify_password(password: &str, hash: &str) -> anyhow::Result<bool> {
    let parsed = PasswordHash::new(hash).map_err(|e| anyhow::anyhow!("bad hash: {e}"))?;
    Ok(Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok())
}

#[derive(Debug, Serialize, Deserialize)]
struct Claims {
    sub: String,
    exp: usize,
}

pub fn issue_token(user_id: Uuid, secret: &str) -> anyhow::Result<String> {
    let exp = (chrono::Utc::now() + chrono::Duration::hours(24)).timestamp() as usize;
    let claims = Claims { sub: user_id.to_string(), exp };
    Ok(encode(&Header::default(), &claims, &EncodingKey::from_secret(secret.as_bytes()))?)
}

pub fn verify_token(token: &str, secret: &str) -> anyhow::Result<Uuid> {
    let data = decode::<Claims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &Validation::default(),
    )?;
    Ok(Uuid::parse_str(&data.claims.sub)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_then_verify_succeeds() {
        let hash = hash_password("correct-horse").unwrap();
        assert!(verify_password("correct-horse", &hash).unwrap());
    }

    #[test]
    fn verify_fails_for_wrong_password() {
        let hash = hash_password("correct-horse").unwrap();
        assert!(!verify_password("wrong", &hash).unwrap());
    }

    #[test]
    fn issue_then_verify_returns_same_user_id() {
        let user_id = uuid::Uuid::new_v4();
        let token = issue_token(user_id, "test-secret").unwrap();
        let verified = verify_token(&token, "test-secret").unwrap();
        assert_eq!(verified, user_id);
    }

    #[test]
    fn verify_fails_with_wrong_secret() {
        let user_id = uuid::Uuid::new_v4();
        let token = issue_token(user_id, "test-secret").unwrap();
        assert!(verify_token(&token, "other-secret").is_err());
    }
}
