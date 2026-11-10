use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{get, patch};
use axum::{Json, Router};
use uuid::Uuid;

use crate::auth::extractor::{ApiJson, AuthUser};
use crate::coupons::dto::{
    CouponEnvelope, CouponListEnvelope, CouponResponse, CreateCouponRequest, ListQuery,
    UpdateCouponRequest,
};
use crate::coupons::service;
use crate::error::ApiError;
use crate::ratelimit::{enforce_user, policies};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/v1/coupons", get(list).post(create))
        .route("/v1/coupons/:id", patch(update))
}

async fn create(
    State(state): State<AppState>,
    actor: AuthUser,
    ApiJson(body): ApiJson<CreateCouponRequest>,
) -> Result<(StatusCode, Json<CouponEnvelope>), ApiError> {
    enforce_user(&state, &policies::COUPON_WRITE, actor.id).await?;
    let coupon = service::create(&state, &actor, body.validate()?).await?;
    Ok((
        StatusCode::CREATED,
        Json(CouponEnvelope {
            coupon: CouponResponse::from(&coupon),
        }),
    ))
}

async fn list(
    State(state): State<AppState>,
    actor: AuthUser,
    Query(query): Query<ListQuery>,
) -> Result<Json<CouponListEnvelope>, ApiError> {
    let coupons = service::list(&state, &actor, query.limit).await?;
    Ok(Json(CouponListEnvelope {
        coupons: coupons.iter().map(CouponResponse::from).collect(),
    }))
}

async fn update(
    State(state): State<AppState>,
    actor: AuthUser,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<UpdateCouponRequest>,
) -> Result<Json<CouponEnvelope>, ApiError> {
    enforce_user(&state, &policies::COUPON_WRITE, actor.id).await?;
    let id = Uuid::parse_str(&id).map_err(|_| ApiError::Http {
        status: StatusCode::NOT_FOUND,
        code: "not_found",
        message: "Coupon not found.".into(),
    })?;
    let coupon = service::update(&state, &actor, id, body.validate()?).await?;
    Ok(Json(CouponEnvelope {
        coupon: CouponResponse::from(&coupon),
    }))
}
