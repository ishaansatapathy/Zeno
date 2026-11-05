use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use axum_extra::extract::CookieJar;

use crate::auth::audit;
use crate::auth::cookies::{self, ACCESS_COOKIE, REFRESH_COOKIE};
use crate::auth::dto::{
    ActivityItem, ActivityResponse, EmailRequest, LoginRequest, MessageResponse, RegisterRequest,
    ResetPasswordRequest, TokenRequest, UserEnvelope, UserResponse,
};
use crate::auth::extractor::{ApiJson, AuthUser, Client};
use crate::auth::limits::{self, HOUR};
use crate::auth::service::{self, Authenticated, RefreshFailure};
use crate::auth::validation;
use crate::error::ApiError;
use crate::net::limit_bucket;
use crate::state::AppState;

const REGISTER_MESSAGE: &str =
    "If this email can be used, we've sent a link to verify it. Check your inbox.";
const RECOVERY_MESSAGE: &str = "If an account exists for that email, we've sent instructions.";
const VERIFICATION_MESSAGE: &str = "If that account still needs verifying, we've sent a new link.";

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/register", post(register))
        .route("/verify-email", post(verify_email))
        .route("/resend-verification", post(resend_verification))
        .route("/login", post(login))
        .route("/refresh", post(refresh))
        .route("/logout", post(logout))
        .route("/forgot-password", post(forgot_password))
        .route("/reset-password", post(reset_password))
        .route("/me", get(me))
        .route("/activity", get(activity))
}

fn session_response(
    state: &AppState,
    jar: CookieJar,
    auth: &Authenticated,
) -> (CookieJar, Json<UserEnvelope>) {
    let jar = cookies::set_session(jar, auth, state.config.cookie_secure);
    (
        jar,
        Json(UserEnvelope {
            user: UserResponse::from(&auth.user),
        }),
    )
}

fn accepted(message: &'static str) -> (StatusCode, Json<MessageResponse>) {
    (StatusCode::ACCEPTED, Json(MessageResponse { message }))
}

async fn register(
    State(state): State<AppState>,
    Client(ctx): Client,
    ApiJson(body): ApiJson<RegisterRequest>,
) -> Result<(StatusCode, Json<MessageResponse>), ApiError> {
    limits::enforce(
        &*state.counters,
        &format!("register:ip:{}", limit_bucket(ctx.ip)),
        10,
        HOUR,
    )
    .await?;
    let input = body.validate()?;
    service::register(&state, input, &ctx).await?;
    Ok(accepted(REGISTER_MESSAGE))
}

async fn verify_email(
    State(state): State<AppState>,
    Client(ctx): Client,
    jar: CookieJar,
    ApiJson(body): ApiJson<TokenRequest>,
) -> Result<(CookieJar, Json<UserEnvelope>), ApiError> {
    let auth = service::verify_email(&state, body.token.expose(), &ctx).await?;
    Ok(session_response(&state, jar, &auth))
}

async fn resend_verification(
    State(state): State<AppState>,
    Client(ctx): Client,
    ApiJson(body): ApiJson<EmailRequest>,
) -> Result<(StatusCode, Json<MessageResponse>), ApiError> {
    let email = validation::email(&body.email).map_err(|e| ApiError::validation(vec![e]))?;
    limits::enforce(
        &*state.counters,
        &format!("mail:ip:{}", limit_bucket(ctx.ip)),
        10,
        HOUR,
    )
    .await?;
    // Past the per-address cap we stay silent instead of erroring, so the cap itself
    // reveals nothing about the account and can't be used to spam someone's inbox.
    let within_cap = limits::hit(&*state.counters, &format!("verify:email:{email}"), 3, HOUR)
        .await
        .is_none();
    if within_cap {
        service::defer(&state, "resend verification", move |state| async move {
            service::resend_verification(&state, &email).await
        });
    }
    Ok(accepted(VERIFICATION_MESSAGE))
}

async fn login(
    State(state): State<AppState>,
    Client(ctx): Client,
    jar: CookieJar,
    ApiJson(body): ApiJson<LoginRequest>,
) -> Result<(CookieJar, Json<UserEnvelope>), ApiError> {
    let auth = service::login(&state, &body.email, body.password.into_inner(), &ctx).await?;
    Ok(session_response(&state, jar, &auth))
}

async fn refresh(State(state): State<AppState>, Client(ctx): Client, jar: CookieJar) -> Response {
    let Some(raw) = jar.get(REFRESH_COOKIE).map(|c| c.value().to_owned()) else {
        return ApiError::unauthorized("unauthenticated", "Sign in to continue.").into_response();
    };
    match service::refresh(&state, &raw, &ctx).await {
        Ok(auth) => (
            StatusCode::NO_CONTENT,
            cookies::set_session(jar, &auth, state.config.cookie_secure),
        )
            .into_response(),
        Err(RefreshFailure::Conflict) => ApiError::unauthorized(
            "refresh_conflict",
            "Another request is already refreshing this session. Retry shortly.",
        )
        .into_response(),
        Err(RefreshFailure::Dead) => (
            cookies::clear_session(jar, state.config.cookie_secure),
            ApiError::unauthorized("session_expired", "Your session has ended. Sign in again."),
        )
            .into_response(),
        Err(RefreshFailure::Other(err)) => err.into_response(),
    }
}

async fn logout(
    State(state): State<AppState>,
    Client(ctx): Client,
    jar: CookieJar,
) -> Result<(CookieJar, StatusCode), ApiError> {
    let raw = jar.get(REFRESH_COOKIE).map(|c| c.value().to_owned());
    let access = jar
        .get(ACCESS_COOKIE)
        .and_then(|c| state.tokens.verify_access(c.value()).ok());
    service::logout(&state, raw.as_deref(), access.as_ref(), &ctx).await?;
    Ok((
        cookies::clear_session(jar, state.config.cookie_secure),
        StatusCode::NO_CONTENT,
    ))
}

async fn forgot_password(
    State(state): State<AppState>,
    Client(ctx): Client,
    ApiJson(body): ApiJson<EmailRequest>,
) -> Result<(StatusCode, Json<MessageResponse>), ApiError> {
    let email = validation::email(&body.email).map_err(|e| ApiError::validation(vec![e]))?;
    limits::enforce(
        &*state.counters,
        &format!("mail:ip:{}", limit_bucket(ctx.ip)),
        10,
        HOUR,
    )
    .await?;
    let within_cap = limits::hit(&*state.counters, &format!("reset:email:{email}"), 3, HOUR)
        .await
        .is_none();
    if within_cap {
        service::defer(&state, "forgot password", move |state| async move {
            service::forgot_password(&state, &email, &ctx).await
        });
    }
    Ok(accepted(RECOVERY_MESSAGE))
}

async fn reset_password(
    State(state): State<AppState>,
    Client(ctx): Client,
    ApiJson(body): ApiJson<ResetPasswordRequest>,
) -> Result<Json<MessageResponse>, ApiError> {
    validation::password(body.new_password.expose(), "new_password")
        .map_err(|e| ApiError::validation(vec![e]))?;
    service::reset_password(
        &state,
        body.token.expose(),
        body.new_password.into_inner(),
        &ctx,
    )
    .await?;
    Ok(Json(MessageResponse {
        message: "Password updated. Sign in with your new password.",
    }))
}

async fn me(State(state): State<AppState>, user: AuthUser) -> Result<Json<UserEnvelope>, ApiError> {
    let row = service::me(&state, user.id).await?;
    Ok(Json(UserEnvelope {
        user: UserResponse::from(&row),
    }))
}

/// The signed-in user's own recent security activity, newest first.
async fn activity(
    State(state): State<AppState>,
    user: AuthUser,
) -> Result<Json<ActivityResponse>, ApiError> {
    let rows = audit::recent_for_user(&state.db, user.id, 20).await?;
    let events = rows
        .into_iter()
        .map(|row| ActivityItem {
            kind: row.kind,
            at: row.created_at,
            ip: row.ip.map(|ip| ip.ip().to_string()),
            user_agent: row.user_agent,
        })
        .collect();
    Ok(Json(ActivityResponse { events }))
}
