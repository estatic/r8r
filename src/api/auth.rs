use crate::domain::{User, UserRole};
use crate::state::AppState;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;
use std::sync::OnceLock;
use uuid::Uuid;

/// A fixed, precomputed Argon2 hash of a placeholder password, used to equalize
/// the cost of the "user not found" and "wrong password" branches of `login`.
///
/// Without this, `login` would return 401 near-instantly when the email isn't
/// registered (no hash to verify against) but only after a slow Argon2 verify
/// (~50-200ms) when it is registered but the password is wrong. That timing
/// difference is a user-enumeration side channel: an attacker can tell which
/// emails have accounts purely from response latency, even though both
/// branches return an identical 401 body. Running a real Argon2 verify against
/// this dummy hash on the "not found" path keeps both branches doing the same
/// amount of work.
static DUMMY_PASSWORD_HASH: OnceLock<String> = OnceLock::new();

fn dummy_hash() -> &'static str {
    DUMMY_PASSWORD_HASH.get_or_init(|| {
        crate::auth::hash_password("dummy-password-for-timing-equalization")
            .expect("dummy hash generation must not fail")
    })
}

#[derive(Deserialize)]
pub struct Credentials {
    pub email: String,
    pub password: String,
}

pub async fn register(
    State(state): State<AppState>,
    Json(payload): Json<Credentials>,
) -> impl IntoResponse {
    let password_hash = match crate::auth::hash_password(&payload.password) {
        Ok(h) => h,
        Err(e) => {
            tracing::error!(error = %e, "failed to hash password during registration");
            return (StatusCode::INTERNAL_SERVER_ERROR, "hash failed").into_response();
        }
    };
    let user = User {
        id: Uuid::new_v4(),
        email: payload.email,
        password_hash,
        role: UserRole::Owner,
        created_at: chrono::Utc::now(),
    };
    if let Err(e) = state.storage.create_user(&user).await {
        // Most commonly a unique-email constraint violation (user already
        // exists), which is an expected, user-caused outcome rather than a
        // server bug -- but still worth a warning so a genuine storage error
        // masquerading as "already exists" is visible in the logs.
        tracing::warn!(error = %e, "failed to create user during registration");
        return (StatusCode::CONFLICT, "user already exists").into_response();
    }
    let token = crate::auth::issue_token(user.id, &state.jwt_secret).unwrap();
    (StatusCode::CREATED, Json(serde_json::json!({"token": token}))).into_response()
}

pub async fn login(
    State(state): State<AppState>,
    Json(payload): Json<Credentials>,
) -> impl IntoResponse {
    match state.storage.get_user_by_email(&payload.email).await {
        Ok(Some(user)) => {
            match crate::auth::verify_password(&payload.password, &user.password_hash) {
                Ok(true) => {
                    let token = crate::auth::issue_token(user.id, &state.jwt_secret).unwrap();
                    Json(serde_json::json!({"token": token})).into_response()
                }
                _ => StatusCode::UNAUTHORIZED.into_response(),
            }
        }
        _ => {
            // No such user: still run a real Argon2 verify (against a fixed dummy
            // hash) so this branch costs the same as the "wrong password" branch
            // above, and the response timing doesn't leak whether the email is
            // registered. The result is always discarded.
            let _ = crate::auth::verify_password(&payload.password, dummy_hash());
            StatusCode::UNAUTHORIZED.into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dummy_hash_is_valid_and_verifiable_without_panicking() {
        let hash = dummy_hash();
        // Must be a real, parseable Argon2 hash, not a placeholder string.
        assert!(hash.starts_with("$argon2"));
        // Verifying an arbitrary candidate password against it must not panic
        // and must go through the same Argon2 verify code path used for a real
        // user's password_hash.
        let result = crate::auth::verify_password("some-random-guess", hash);
        assert!(result.is_ok());
        assert!(!result.unwrap());
    }

    #[test]
    fn dummy_hash_is_stable_across_calls() {
        // OnceLock must hand back the same hash every call, not recompute it
        // (recomputing would still equalize timing but defeats the point of
        // caching it).
        assert_eq!(dummy_hash(), dummy_hash());
    }
}
