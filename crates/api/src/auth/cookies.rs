use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use chrono::{DateTime, Utc};

use crate::auth::service::Authenticated;

pub const ACCESS_COOKIE: &str = "zeno_at";
pub const REFRESH_COOKIE: &str = "zeno_rt";
/// The refresh token is only ever sent to the auth endpoints, never to the rest of the API.
pub const REFRESH_COOKIE_PATH: &str = "/v1/auth";

fn seconds_until(moment: DateTime<Utc>) -> i64 {
    (moment - Utc::now()).num_seconds().max(0)
}

fn build(
    name: &'static str,
    value: String,
    path: &'static str,
    max_age_secs: i64,
    secure: bool,
) -> Cookie<'static> {
    Cookie::build((name, value))
        .http_only(true)
        .secure(secure)
        .same_site(SameSite::Lax)
        .path(path)
        .max_age(time::Duration::seconds(max_age_secs))
        .build()
}

pub fn set_session(jar: CookieJar, auth: &Authenticated, secure: bool) -> CookieJar {
    jar.add(build(
        ACCESS_COOKIE,
        auth.access_token.clone(),
        "/",
        seconds_until(auth.access_expires_at),
        secure,
    ))
    .add(build(
        REFRESH_COOKIE,
        auth.refresh_token.clone(),
        REFRESH_COOKIE_PATH,
        seconds_until(auth.refresh_expires_at),
        secure,
    ))
}

/// Attributes (notably the path) must match the ones used when setting, or the browser
/// keeps the original cookie.
pub fn clear_session(jar: CookieJar, secure: bool) -> CookieJar {
    jar.add(build(ACCESS_COOKIE, String::new(), "/", 0, secure))
        .add(build(
            REFRESH_COOKIE,
            String::new(),
            REFRESH_COOKIE_PATH,
            0,
            secure,
        ))
}
