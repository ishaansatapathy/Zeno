use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use uuid::Uuid;

use crate::auth::extractor::{ApiJson, AuthUser};
use crate::error::ApiError;
use crate::ratelimit::{enforce_user, policies};
use crate::refunds::dto::{
    parse_status, ListQuery, ReasonRequest, RefundEnvelope, RefundListEnvelope, RefundResponse,
    StaffRefundResponse,
};
use crate::refunds::service;
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/v1/orders/:id/request-refund", post(request))
        .route("/v1/orders/:id/refund", post(approve))
        .route("/v1/orders/:id/refund/decline", post(decline))
        .route("/v1/courses/:id/refunds", get(list))
}

fn id(raw: &str, subject: &'static str) -> Result<Uuid, ApiError> {
    Uuid::parse_str(raw).map_err(|_| ApiError::Http {
        status: StatusCode::NOT_FOUND,
        code: "not_found",
        message: format!("{subject} not found."),
    })
}

/// A student asks for their money back. `201` for a new request, `200` if they had already asked.
async fn request(
    State(state): State<AppState>,
    user: AuthUser,
    Path(order): Path<String>,
    ApiJson(body): ApiJson<ReasonRequest>,
) -> Result<(StatusCode, Json<RefundEnvelope>), ApiError> {
    enforce_user(&state, &policies::REFUND_REQUEST, user.id).await?;
    let reason = body.note(false)?;
    let (refund, created) = service::request(&state, &user, id(&order, "Order")?, reason).await?;
    let status = if created {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };
    Ok((
        status,
        Json(RefundEnvelope {
            refund: RefundResponse::from(&refund),
        }),
    ))
}

/// A creator or admin refunds the order (approving a student's request if there is one). `202`:
/// the refund is with the provider and is final only when the provider confirms it.
async fn approve(
    State(state): State<AppState>,
    actor: AuthUser,
    Path(order): Path<String>,
    ApiJson(body): ApiJson<ReasonRequest>,
) -> Result<(StatusCode, Json<RefundEnvelope>), ApiError> {
    enforce_user(&state, &policies::REFUND_ACTION, actor.id).await?;
    let reason = body.note(false)?;
    let refund = service::approve(&state, &actor, id(&order, "Order")?, reason).await?;
    Ok((
        StatusCode::ACCEPTED,
        Json(RefundEnvelope {
            refund: RefundResponse::from(&refund),
        }),
    ))
}

async fn decline(
    State(state): State<AppState>,
    actor: AuthUser,
    Path(order): Path<String>,
    ApiJson(body): ApiJson<ReasonRequest>,
) -> Result<Json<RefundEnvelope>, ApiError> {
    enforce_user(&state, &policies::REFUND_ACTION, actor.id).await?;
    let note = body.note(true)?.unwrap_or_default();
    let refund = service::decline(&state, &actor, id(&order, "Order")?, note).await?;
    Ok(Json(RefundEnvelope {
        refund: RefundResponse::from(&refund),
    }))
}

async fn list(
    State(state): State<AppState>,
    actor: AuthUser,
    Path(course): Path<String>,
    Query(query): Query<ListQuery>,
) -> Result<Json<RefundListEnvelope>, ApiError> {
    enforce_user(&state, &policies::REFUND_ACTION, actor.id).await?;
    let status = query.status.as_deref().map(parse_status).transpose()?;
    let rows =
        service::list_for_course(&state, &actor, id(&course, "Course")?, status, query.limit)
            .await?;
    Ok(Json(RefundListEnvelope {
        refunds: rows.iter().map(StaffRefundResponse::from).collect(),
    }))
}
