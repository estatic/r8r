use crate::domain::{User, UserRole};
use crate::state::AppState;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;
use uuid::Uuid;

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
        Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, "hash failed").into_response(),
    };
    let user = User {
        id: Uuid::new_v4(),
        email: payload.email,
        password_hash,
        role: UserRole::Owner,
        created_at: chrono::Utc::now(),
    };
    if state.storage.create_user(&user).await.is_err() {
        return (StatusCode::CONFLICT, "user already exists").into_response();
    }
    let token = crate::auth::issue_token(user.id, &state.jwt_secret).unwrap();
    (StatusCode::CREATED, Json(serde_json::json!({"token": token}))).into_response()
}

pub async fn login(
    State(state): State<AppState>,
    Json(payload): Json<Credentials>,
) -> impl IntoResponse {
    let user = match state.storage.get_user_by_email(&payload.email).await {
        Ok(Some(u)) => u,
        _ => return StatusCode::UNAUTHORIZED.into_response(),
    };
    match crate::auth::verify_password(&payload.password, &user.password_hash) {
        Ok(true) => {
            let token = crate::auth::issue_token(user.id, &state.jwt_secret).unwrap();
            Json(serde_json::json!({"token": token})).into_response()
        }
        _ => StatusCode::UNAUTHORIZED.into_response(),
    }
}
