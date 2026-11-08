use axum::body::Bytes;
use axum::extract::rejection::BytesRejection;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::routing::{get, put};
use axum::{Json, Router};

use crate::auth::extractor::{ApiJson, AuthUser};
use crate::error::ApiError;
use crate::images;
use crate::profiles::dto::{ProfileEnvelope, ProfileRequest, ProfileResponse};
use crate::profiles::service;
use crate::state::AppState;

/// Mounted under `/v1/me`.
pub fn me_router() -> Router<AppState> {
    Router::new().route("/profile", get(show).put(save)).route(
        "/profile/avatar",
        put(set_avatar)
            .delete(clear_avatar)
            .layer(images::upload_limit()),
    )
}

async fn show(
    State(state): State<AppState>,
    user: AuthUser,
) -> Result<Json<ProfileEnvelope>, ApiError> {
    let row = service::get(&state, &user).await?;
    Ok(Json(ProfileEnvelope {
        profile: row.as_ref().map(|row| ProfileResponse::build(&state, row)),
    }))
}

async fn save(
    State(state): State<AppState>,
    user: AuthUser,
    ApiJson(body): ApiJson<ProfileRequest>,
) -> Result<Json<ProfileEnvelope>, ApiError> {
    let row = service::put(&state, &user, body.validate()?).await?;
    Ok(Json(ProfileEnvelope {
        profile: Some(ProfileResponse::build(&state, &row)),
    }))
}

async fn set_avatar(
    State(state): State<AppState>,
    user: AuthUser,
    headers: HeaderMap,
    body: Result<Bytes, BytesRejection>,
) -> Result<Json<ProfileEnvelope>, ApiError> {
    let bytes = images::read_upload(&headers, body)?;
    let row = service::set_avatar(&state, &user, bytes).await?;
    Ok(Json(ProfileEnvelope {
        profile: Some(ProfileResponse::build(&state, &row)),
    }))
}

async fn clear_avatar(
    State(state): State<AppState>,
    user: AuthUser,
) -> Result<Json<ProfileEnvelope>, ApiError> {
    let row = service::clear_avatar(&state, &user).await?;
    Ok(Json(ProfileEnvelope {
        profile: Some(ProfileResponse::build(&state, &row)),
    }))
}
