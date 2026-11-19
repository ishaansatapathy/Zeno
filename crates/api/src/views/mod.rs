//! Counting views without writing on every one.
//!
//! Pressing play must not cost a database write: ten thousand people opening a link in a minute
//! would be ten thousand row updates on the same few rows. So a view is counted in Redis (see
//! [`zeno_core::views`]) and a flush moves the counts to Postgres once per interval, one row per
//! recording per day, however many views that was.
//!
//! * [`record`] is the hot path: best effort, off the request, never fails a playback.
//! * [`flush`] is the batch job; safe to run on every instance at once.
//! * [`routes`] is the owner's read side.

pub mod flush;
pub mod repo;
pub mod routes;
pub mod service;

use std::net::IpAddr;

use ring::hmac;
use uuid::Uuid;
use zeno_core::views::Recorded;

use crate::net::limit_bucket;
use crate::state::AppState;

/// Who is watching, as far as de-duplicating goes: a keyed hash of the network bucket and the
/// browser string. Not an identity (two people behind one office address with the same browser
/// are one viewer), and not reversible: the address itself is never stored, and without the
/// server's secret the hash cannot even be checked against a guess.
pub fn viewer_id(secret: &str, ip: Option<IpAddr>, user_agent: Option<&str>) -> String {
    let key = hmac::Key::new(hmac::HMAC_SHA256, secret.as_bytes());
    let message = format!(
        "zeno-viewer-v1|{}|{}",
        limit_bucket(ip),
        user_agent.unwrap_or("").trim()
    );
    let tag = hmac::sign(&key, message.as_bytes());
    // 16 bytes is plenty to tell viewers apart and halves what Redis holds.
    tag.as_ref()[..16]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Counts one view of `recording` by `viewer`, off the request path. A counter that is down
/// loses that view and logs it; the person pressing play is never told.
pub fn record(state: &AppState, recording: Uuid, viewer: String) {
    let views = state.views.clone();
    state.background.spawn(async move {
        match views.record(recording, &viewer).await {
            Ok(Recorded::Counted | Recorded::Repeat) => {}
            Err(error) => tracing::warn!(%error, %recording, "view not counted"),
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(raw: &str) -> Option<IpAddr> {
        Some(raw.parse().unwrap())
    }

    #[test]
    fn the_same_visitor_gets_the_same_id() {
        let a = viewer_id("secret", ip("203.0.113.9"), Some("Firefox"));
        assert_eq!(a, viewer_id("secret", ip("203.0.113.9"), Some("Firefox")));
        assert_eq!(a.len(), 32);
    }

    #[test]
    fn a_different_address_browser_or_secret_gives_a_different_id() {
        let base = viewer_id("secret", ip("203.0.113.9"), Some("Firefox"));
        assert_ne!(
            base,
            viewer_id("secret", ip("203.0.113.10"), Some("Firefox"))
        );
        assert_ne!(base, viewer_id("secret", ip("203.0.113.9"), Some("Chrome")));
        assert_ne!(base, viewer_id("other", ip("203.0.113.9"), Some("Firefox")));
    }

    #[test]
    fn an_ipv6_household_is_one_viewer_per_browser() {
        let a = viewer_id("s", ip("2001:db8:1:2:aaaa::1"), Some("x"));
        let b = viewer_id("s", ip("2001:db8:1:2:bbbb::9"), Some("x"));
        assert_eq!(a, b);
    }

    #[test]
    fn the_id_is_hex_and_never_the_address() {
        let id = viewer_id("secret", ip("203.0.113.9"), Some("Firefox"));
        assert!(id.chars().all(|c| c.is_ascii_hexdigit()));
        assert!(!id.contains("203.0.113.9"));
    }
}
