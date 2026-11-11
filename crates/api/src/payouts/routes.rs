use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use uuid::Uuid;

use crate::auth::extractor::{ApiJson, AuthUser};
use crate::error::ApiError;
use crate::payouts::dto::{
    parse_status, AdminPayoutListEnvelope, AdminPayoutResponse, CancelRequest, ListQuery,
    PayRequest, PayoutEnvelope, PayoutListEnvelope, PayoutResponse, RequestPayout,
};
use crate::payouts::service;
use crate::ratelimit::{enforce_user, policies};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/v1/me/payouts", get(mine).post(request))
        .route("/v1/me/payouts/:id/cancel", post(cancel_own))
        .route("/v1/admin/payouts", get(queue))
        .route("/v1/admin/payouts/:id/pay", post(pay))
        .route("/v1/admin/payouts/:id/cancel", post(cancel))
}

fn id(raw: &str) -> Result<Uuid, ApiError> {
    Uuid::parse_str(raw).map_err(|_| ApiError::Http {
        status: StatusCode::NOT_FOUND,
        code: "not_found",
        message: "Payout not found.".into(),
    })
}

async fn mine(
    State(state): State<AppState>,
    user: AuthUser,
    Query(query): Query<ListQuery>,
) -> Result<Json<PayoutListEnvelope>, ApiError> {
    enforce_user(&state, &policies::EARNINGS_READ, user.id).await?;
    let payouts = service::list_own(&state, &user, query.limit).await?;
    Ok(Json(PayoutListEnvelope {
        payouts: payouts.iter().map(PayoutResponse::from).collect(),
    }))
}

/// Asks for money to be paid out: `201`. The amount leaves the balance at once.
async fn request(
    State(state): State<AppState>,
    user: AuthUser,
    ApiJson(body): ApiJson<RequestPayout>,
) -> Result<(StatusCode, Json<PayoutEnvelope>), ApiError> {
    enforce_user(&state, &policies::PAYOUT_ACTION, user.id).await?;
    let payout = service::request(&state, &user, body.amount_paise).await?;
    Ok((
        StatusCode::CREATED,
        Json(PayoutEnvelope {
            payout: PayoutResponse::from(&payout),
        }),
    ))
}

async fn cancel_own(
    State(state): State<AppState>,
    user: AuthUser,
    Path(payout): Path<String>,
    ApiJson(_body): ApiJson<CancelRequest>,
) -> Result<Json<PayoutEnvelope>, ApiError> {
    enforce_user(&state, &policies::PAYOUT_ACTION, user.id).await?;
    let payout = service::cancel_own(&state, &user, id(&payout)?).await?;
    Ok(Json(PayoutEnvelope {
        payout: PayoutResponse::from(&payout),
    }))
}

async fn queue(
    State(state): State<AppState>,
    admin: AuthUser,
    Query(query): Query<ListQuery>,
) -> Result<Json<AdminPayoutListEnvelope>, ApiError> {
    enforce_user(&state, &policies::ADMIN_ACTION, admin.id).await?;
    let status = query.status.as_deref().map(parse_status).transpose()?;
    let rows = service::list_all(&state, &admin, status, query.limit).await?;
    Ok(Json(AdminPayoutListEnvelope {
        payouts: rows.iter().map(AdminPayoutResponse::from).collect(),
    }))
}

async fn pay(
    State(state): State<AppState>,
    admin: AuthUser,
    Path(payout): Path<String>,
    ApiJson(body): ApiJson<PayRequest>,
) -> Result<Json<PayoutEnvelope>, ApiError> {
    enforce_user(&state, &policies::ADMIN_ACTION, admin.id).await?;
    let payment = body.validate()?;
    let payout = service::pay(&state, &admin, id(&payout)?, payment).await?;
    Ok(Json(PayoutEnvelope {
        payout: PayoutResponse::from(&payout),
    }))
}

async fn cancel(
    State(state): State<AppState>,
    admin: AuthUser,
    Path(payout): Path<String>,
    ApiJson(body): ApiJson<CancelRequest>,
) -> Result<Json<PayoutEnvelope>, ApiError> {
    enforce_user(&state, &policies::ADMIN_ACTION, admin.id).await?;
    let reason = body.reason(true)?.unwrap_or_default();
    let payout = service::cancel(&state, &admin, id(&payout)?, reason).await?;
    Ok(Json(PayoutEnvelope {
        payout: PayoutResponse::from(&payout),
    }))
}
