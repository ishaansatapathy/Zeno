//! Request budgets: which routes are limited, by whom, how tightly, and what happens when the
//! counter store is down.
//!
//! Every limit is a named [`Policy`] so the numbers live in one place and show up in logs and
//! `429` responses. Two kinds of subject are used:
//!
//! * the **client address** for routes anyone can call (the public catalogue). Addresses are
//!   grouped (IPv6 by /64) and the budget is generous, because a college or a mobile carrier
//!   puts thousands of honest students behind one address;
//! * the **account** for signed-in actions, where the budget can be tight because it is per
//!   person.
//!
//! When the counter store is unreachable a policy either lets the request through (`Open`: the
//! catalogue should not go dark because Redis restarted) or refuses it with `503` (`Closed`:
//! for anything where an unlimited flood does real harm).

use std::net::{IpAddr, SocketAddr};

use axum::extract::{ConnectInfo, Request, State};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use uuid::Uuid;
use zeno_core::ratelimit::Rule;

use crate::error::ApiError;
use crate::net::{client_ip, limit_bucket};
use crate::state::AppState;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fail {
    /// Let the request through when the limiter cannot be consulted.
    Open,
    /// Refuse it (`503`).
    Closed,
}

#[derive(Debug, Clone, Copy)]
pub struct Policy {
    pub name: &'static str,
    pub rule: Rule,
    pub fail: Fail,
}

/// Every limit in the API.
pub mod policies {
    use super::{Fail, Policy};
    use zeno_core::ratelimit::Rule;

    /// Browsing and searching, per client address.
    pub const PUBLIC_CATALOG: Policy = Policy {
        name: "public_catalog",
        rule: Rule::per_minute(120),
        fail: Fail::Open,
    };
    /// Starting playback, per account. A person has two screens and presses play a few times.
    pub const PLAY_START: Policy = Policy {
        name: "play_start",
        rule: Rule::per_minute(20),
        fail: Fail::Closed,
    };
    /// Playback heartbeats, per account: two screens at one per 30 s is four a minute.
    pub const HEARTBEAT: Policy = Policy {
        name: "heartbeat",
        rule: Rule::per_minute(30),
        fail: Fail::Open,
    };
    /// Claiming a free course or any enrollment write, per account.
    pub const ENROLL: Policy = Policy {
        name: "enroll",
        rule: Rule::per_minute(20),
        fail: Fail::Closed,
    };
    /// Starting checkout, per account. Each call may cost a provider API call, and a person starts
    /// a handful at most.
    pub const CHECKOUT: Policy = Policy {
        name: "checkout",
        rule: Rule::per_minute(10),
        fail: Fail::Closed,
    };
    /// Webhook calls whose signature did not check out, per client address. Only failures are
    /// counted, so a flood of forgeries is turned away while the provider's real traffic is not
    /// limited at all.
    pub const WEBHOOK_REJECTED: Policy = Policy {
        name: "webhook_rejected",
        rule: Rule::per_minute(30),
        fail: Fail::Open,
    };
    /// Reading one's own orders (a page polls while a payment settles), per account.
    pub const ORDER_READ: Policy = Policy {
        name: "order_read",
        rule: Rule::per_minute(120),
        fail: Fail::Open,
    };
    /// Reading one's own progress, per account.
    pub const PROGRESS_READ: Policy = Policy {
        name: "progress_read",
        rule: Rule::per_minute(120),
        fail: Fail::Open,
    };
}

fn refuse(retry_after_secs: u64) -> ApiError {
    ApiError::TooManyRequests { retry_after_secs }
}

/// Counts one request against `subject` under `policy`.
pub async fn enforce(state: &AppState, policy: &Policy, subject: &str) -> Result<(), ApiError> {
    let key = format!("{}:{subject}", policy.name);
    match state.limiter.check(&key, policy.rule).await {
        Ok(decision) if decision.allowed => Ok(()),
        Ok(decision) => {
            tracing::info!(policy = policy.name, "rate limit reached");
            Err(refuse(
                decision.retry_after.as_secs_f64().ceil().max(1.0) as u64
            ))
        }
        Err(err) => match policy.fail {
            Fail::Open => {
                tracing::warn!(policy = policy.name, %err, "rate limiter down; letting the request through");
                Ok(())
            }
            Fail::Closed => {
                tracing::error!(policy = policy.name, %err, "rate limiter down; refusing the request");
                Err(ApiError::Unavailable("rate limiting is unavailable"))
            }
        },
    }
}

/// Counts one request against the caller's address (as `limit_by_address` does, for a single
/// handler that wants to count only some of its requests).
pub async fn enforce_address(
    state: &AppState,
    policy: &Policy,
    headers: &axum::http::HeaderMap,
    peer: Option<IpAddr>,
) -> Result<(), ApiError> {
    let ip = client_ip(headers, peer, state.config.trusted_proxy_hops);
    enforce(state, policy, &limit_bucket(ip)).await
}

pub async fn enforce_user(state: &AppState, policy: &Policy, user: Uuid) -> Result<(), ApiError> {
    enforce(state, policy, &user.to_string()).await
}

/// Middleware: `from_fn_with_state((state, policy), limit_by_address)` limits every route it
/// wraps by client address.
pub async fn limit_by_address(
    State((state, policy)): State<(AppState, Policy)>,
    request: Request,
    next: Next,
) -> Response {
    let peer: Option<IpAddr> = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ConnectInfo(addr)| addr.ip());
    let ip = client_ip(request.headers(), peer, state.config.trusted_proxy_hops);
    match enforce(&state, &policy, &limit_bucket(ip)).await {
        Ok(()) => next.run(request).await,
        Err(err) => err.into_response(),
    }
}
