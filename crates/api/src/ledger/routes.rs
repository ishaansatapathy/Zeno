use axum::extract::{Query, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};

use crate::auth::extractor::{ApiJson, AuthUser};
use crate::error::ApiError;
use crate::ledger::dto::{
    AdjustmentEnvelope, AdjustmentRequest, BackfillResponse, EarningsEnvelope, EntriesEnvelope,
    EntriesQuery, EntryCursor, ExportQuery, IntegrityResponse,
};
use crate::ledger::service;
use crate::ratelimit::{enforce_user, policies};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/v1/me/earnings", get(earnings))
        .route("/v1/me/earnings/entries", get(entries))
        .route("/v1/me/earnings/export.csv", get(export))
        .route("/v1/admin/ledger/integrity", get(integrity))
        .route("/v1/admin/ledger/backfill", post(backfill))
        .route("/v1/admin/ledger/adjustments", post(adjust))
}

async fn earnings(
    State(state): State<AppState>,
    user: AuthUser,
) -> Result<Json<EarningsEnvelope>, ApiError> {
    enforce_user(&state, &policies::EARNINGS_READ, user.id).await?;
    Ok(Json(EarningsEnvelope {
        earnings: service::earnings(&state, &user).await?,
    }))
}

async fn entries(
    State(state): State<AppState>,
    user: AuthUser,
    Query(query): Query<EntriesQuery>,
) -> Result<Json<EntriesEnvelope>, ApiError> {
    enforce_user(&state, &policies::EARNINGS_READ, user.id).await?;
    let cursor = query
        .cursor
        .as_deref()
        .map(EntryCursor::decode)
        .transpose()?;
    let (entries, next_cursor) = service::entries(&state, &user, cursor, query.limit).await?;
    Ok(Json(EntriesEnvelope {
        entries,
        next_cursor,
    }))
}

/// The creator's entries as a spreadsheet. A download that only they get, and nobody caches.
async fn export(
    State(state): State<AppState>,
    user: AuthUser,
    Query(query): Query<ExportQuery>,
) -> Result<Response, ApiError> {
    enforce_user(&state, &policies::EARNINGS_EXPORT, user.id).await?;
    let (file_name, body) =
        service::export_csv(&state, &user, query.from.as_deref(), query.to.as_deref()).await?;
    Ok((
        [
            (header::CONTENT_TYPE, "text/csv; charset=utf-8".to_owned()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{file_name}\""),
            ),
            (header::CACHE_CONTROL, "private, no-store".to_owned()),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff".to_owned()),
        ],
        body,
    )
        .into_response())
}

async fn integrity(
    State(state): State<AppState>,
    admin: AuthUser,
) -> Result<Json<IntegrityResponse>, ApiError> {
    enforce_user(&state, &policies::ADMIN_ACTION, admin.id).await?;
    let problems = service::integrity(&state, &admin).await?;
    Ok(Json(IntegrityResponse {
        ok: problems.is_empty(),
        problems,
    }))
}

async fn backfill(
    State(state): State<AppState>,
    admin: AuthUser,
) -> Result<Json<BackfillResponse>, ApiError> {
    enforce_user(&state, &policies::ADMIN_ACTION, admin.id).await?;
    let (sales_posted, refunds_posted) = service::backfill(&state, &admin).await?;
    Ok(Json(BackfillResponse {
        sales_posted,
        refunds_posted,
    }))
}

async fn adjust(
    State(state): State<AppState>,
    admin: AuthUser,
    ApiJson(body): ApiJson<AdjustmentRequest>,
) -> Result<(StatusCode, Json<AdjustmentEnvelope>), ApiError> {
    enforce_user(&state, &policies::ADMIN_ACTION, admin.id).await?;
    let adjustment = service::adjust(&state, &admin, body.validate()?).await?;
    Ok((StatusCode::CREATED, Json(adjustment)))
}
