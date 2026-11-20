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
use crate::error::{ApiError, ErrorBody};
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

/// Creates an account and emails a link to verify it.
///
/// The answer is the same whether or not the email was already registered, so this cannot be used
/// to find out who has an account.
#[utoipa::path(
    post,
    path = "/v1/auth/register",
    tag = "auth",
    request_body = RegisterRequest,
    responses(
        (status = 202, description = "Accepted. A link is on its way if the address can be used.", body = MessageResponse),
        (status = 422, description = "`validation_failed` (every invalid field at once, in `fields`) or `invalid_request` (the JSON has the wrong shape or an unknown field).", body = ErrorBody),
        (status = 429, description = "`too_many_requests`; wait `Retry-After` seconds.", body = ErrorBody, headers(("Retry-After" = u64, description = "Seconds until the budget is available again."))),
        (status = 503, description = "`unavailable`: the limiter is down, and this route fails closed.", body = ErrorBody),
    )
)]
pub(crate) async fn register(
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

/// Confirms an email address with the one-time token from the link, and signs the person in.
#[utoipa::path(
    post,
    path = "/v1/auth/verify-email",
    tag = "auth",
    request_body = TokenRequest,
    responses(
        (status = 200, description = "Verified and signed in.", body = UserEnvelope, headers(("Set-Cookie" = String, description = "`zeno_at` and `zeno_rt`; HttpOnly, SameSite=Lax."))),
        (status = 400, description = "`invalid_token` (the link is wrong, used or expired) or `invalid_request` (a malformed body).", body = ErrorBody),
    )
)]
pub(crate) async fn verify_email(
    State(state): State<AppState>,
    Client(ctx): Client,
    jar: CookieJar,
    ApiJson(body): ApiJson<TokenRequest>,
) -> Result<(CookieJar, Json<UserEnvelope>), ApiError> {
    let auth = service::verify_email(&state, body.token.expose(), &ctx).await?;
    Ok(session_response(&state, jar, &auth))
}

/// Sends a new verification link, if the account exists and still needs one.
#[utoipa::path(
    post,
    path = "/v1/auth/resend-verification",
    tag = "auth",
    request_body = EmailRequest,
    responses(
        (status = 202, description = "Accepted. Always the same answer.", body = MessageResponse),
        (status = 422, description = "`validation_failed` (see `fields`) or `invalid_request` (the JSON has the wrong shape or an unknown field).", body = ErrorBody),
        (status = 429, description = "`too_many_requests`", body = ErrorBody, headers(("Retry-After" = u64, description = "Seconds until the budget is available again."))),
        (status = 503, description = "`unavailable`: fails closed.", body = ErrorBody),
    )
)]
pub(crate) async fn resend_verification(
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
        .await?
        .is_none();
    if within_cap {
        service::defer(&state, "resend verification", move |state| async move {
            service::resend_verification(&state, &email).await
        });
    }
    Ok(accepted(VERIFICATION_MESSAGE))
}

/// Signs in with an email and password.
#[utoipa::path(
    post,
    path = "/v1/auth/login",
    tag = "auth",
    request_body = LoginRequest,
    responses(
        (status = 200, description = "Signed in.", body = UserEnvelope, headers(("Set-Cookie" = String, description = "`zeno_at` and `zeno_rt`; HttpOnly, SameSite=Lax."))),
        (status = 401, description = "`invalid_credentials`: the same answer for an unknown email and a wrong password.", body = ErrorBody),
        (status = 403, description = "`email_not_verified` (the password was right but the address is not confirmed) or `forbidden_origin`.", body = ErrorBody),
        (status = 429, description = "`too_many_requests`: too many failures for this email or address.", body = ErrorBody, headers(("Retry-After" = u64, description = "Seconds until the budget is available again."))),
        (status = 503, description = "`unavailable`: fails closed.", body = ErrorBody),
    )
)]
pub(crate) async fn login(
    State(state): State<AppState>,
    Client(ctx): Client,
    jar: CookieJar,
    ApiJson(body): ApiJson<LoginRequest>,
) -> Result<(CookieJar, Json<UserEnvelope>), ApiError> {
    let auth = service::login(&state, &body.email, body.password.into_inner(), &ctx).await?;
    Ok(session_response(&state, jar, &auth))
}

/// Rotates the session. Reads the refresh cookie; sends no body and returns none.
///
/// A refresh token is single-use. If two tabs refresh at once, one gets `401 refresh_conflict`
/// and should simply retry.
#[utoipa::path(
    post,
    path = "/v1/auth/refresh",
    tag = "auth",
    responses(
        (status = 204, description = "A new pair of cookies was set.", headers(("Set-Cookie" = String, description = "New `zeno_at` and `zeno_rt`."))),
        (status = 401, description = "`unauthenticated` (no cookie), `refresh_conflict` (retry shortly) or `session_expired` (the cookies are cleared; sign in again).", body = ErrorBody),
    )
)]
pub(crate) async fn refresh(
    State(state): State<AppState>,
    Client(ctx): Client,
    jar: CookieJar,
) -> Response {
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

/// Ends this session and clears the cookies. Safe to repeat.
#[utoipa::path(
    post,
    path = "/v1/auth/logout",
    tag = "auth",
    responses(
        (status = 204, description = "Signed out.", headers(("Set-Cookie" = String, description = "Both cookies, expired."))),
    )
)]
pub(crate) async fn logout(
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

/// Emails a password reset link, if the account exists.
#[utoipa::path(
    post,
    path = "/v1/auth/forgot-password",
    tag = "auth",
    request_body = EmailRequest,
    responses(
        (status = 202, description = "Accepted. Always the same answer.", body = MessageResponse),
        (status = 422, description = "`validation_failed` (see `fields`) or `invalid_request` (the JSON has the wrong shape or an unknown field).", body = ErrorBody),
        (status = 429, description = "`too_many_requests`", body = ErrorBody, headers(("Retry-After" = u64, description = "Seconds until the budget is available again."))),
        (status = 503, description = "`unavailable`: fails closed.", body = ErrorBody),
    )
)]
pub(crate) async fn forgot_password(
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
        .await?
        .is_none();
    if within_cap {
        service::defer(&state, "forgot password", move |state| async move {
            service::forgot_password(&state, &email, &ctx).await
        });
    }
    Ok(accepted(RECOVERY_MESSAGE))
}

/// Sets a new password with the one-time token from the reset link, and ends every other session.
#[utoipa::path(
    post,
    path = "/v1/auth/reset-password",
    tag = "auth",
    request_body = ResetPasswordRequest,
    responses(
        (status = 200, description = "Password updated. The person signs in again.", body = MessageResponse),
        (status = 400, description = "`invalid_token` (the link is wrong, used or expired) or `invalid_request` (a malformed body).", body = ErrorBody),
        (status = 422, description = "`validation_failed` (the new password is not allowed) or `invalid_request` (the JSON has the wrong shape).", body = ErrorBody),
    )
)]
pub(crate) async fn reset_password(
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

/// Who is signed in.
#[utoipa::path(
    get,
    path = "/v1/auth/me",
    tag = "auth",
    security(("session" = [])),
    responses(
        (status = 200, description = "The signed-in account.", body = UserEnvelope),
        (status = 401, description = "`unauthenticated`: no session, or it expired. Refresh and retry once.", body = ErrorBody),
    )
)]
pub(crate) async fn me(
    State(state): State<AppState>,
    user: AuthUser,
) -> Result<Json<UserEnvelope>, ApiError> {
    let row = service::me(&state, user.id).await?;
    Ok(Json(UserEnvelope {
        user: UserResponse::from(&row),
    }))
}

/// The signed-in user's own recent security activity, newest first.
#[utoipa::path(
    get,
    path = "/v1/auth/activity",
    tag = "auth",
    security(("session" = [])),
    responses(
        (status = 200, description = "Up to 20 events, newest first.", body = ActivityResponse),
        (status = 401, description = "`unauthenticated`", body = ErrorBody),
    )
)]
pub(crate) async fn activity(
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
