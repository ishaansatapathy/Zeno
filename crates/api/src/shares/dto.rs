use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;
use zeno_core::model::VideoStatus;

use crate::error::{ApiError, FieldError};
use crate::recordings::dto::status_of;
use crate::shares::repo::{LinkRow, Target};

/// A password is a speed bump for casual sharing, not an account: short enough to read out over
/// a call, long enough not to be guessed in the few tries the limiter allows.
pub const MIN_PASSWORD: usize = 4;
pub const MAX_PASSWORD: usize = 128;
/// Links expire within ten years at most, and not in the next minute.
pub const MIN_LIFETIME: Duration = Duration::minutes(1);
pub const MAX_LIFETIME: Duration = Duration::days(3650);
/// How many links one recording may have live at once.
pub const MAX_LIVE_LINKS: i64 = 10;
/// Where a link stands; what `link_state` can answer.
pub const LINK_STATES: &[&str] = &["active", "expired", "revoked"];

#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateLinkRequest {
    /// A speed bump for casual sharing, not an account. Checked before a viewing starts.
    #[schema(value_type = Option<String>, format = Password, min_length = 4, max_length = 128)]
    pub password: Option<String>,
    /// From a minute to ten years from now. Without one the link never expires.
    pub expires_at: Option<DateTime<Utc>>,
}

#[derive(Debug)]
pub struct NewLink {
    pub password: Option<String>,
    pub expires_at: Option<DateTime<Utc>>,
}

fn password_ok(errors: &mut Vec<FieldError>, field: &'static str, raw: &str) {
    let len = raw.chars().count();
    if !(MIN_PASSWORD..=MAX_PASSWORD).contains(&len) {
        errors.push(FieldError::new(
            field,
            format!("Must be between {MIN_PASSWORD} and {MAX_PASSWORD} characters."),
        ));
    } else if raw.chars().any(char::is_control) {
        errors.push(FieldError::new(field, "Contains invalid characters."));
    }
}

impl CreateLinkRequest {
    pub fn validate(self, now: DateTime<Utc>) -> Result<NewLink, ApiError> {
        let mut errors = Vec::new();
        if let Some(password) = &self.password {
            password_ok(&mut errors, "password", password);
        }
        if let Some(at) = self.expires_at {
            if at < now + MIN_LIFETIME {
                errors.push(FieldError::new(
                    "expires_at",
                    "Must be at least a minute from now.",
                ));
            } else if at > now + MAX_LIFETIME {
                errors.push(FieldError::new(
                    "expires_at",
                    "Must be within ten years from now.",
                ));
            }
        }
        if !errors.is_empty() {
            return Err(ApiError::validation(errors));
        }
        Ok(NewLink {
            password: self.password,
            expires_at: self.expires_at,
        })
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlayRequest {
    pub password: Option<String>,
}

/// Where a link stands, for its owner.
pub fn link_state(link: &LinkRow, now: DateTime<Utc>) -> &'static str {
    if link.revoked_at.is_some() {
        "revoked"
    } else if link.expires_at.is_some_and(|at| at <= now) {
        "expired"
    } else {
        "active"
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct LinkResponse {
    pub id: Uuid,
    pub code: String,
    /// The address to hand out.
    pub url: String,
    pub has_password: bool,
    pub state: &'static str,
    pub expires_at: Option<DateTime<Utc>>,
    pub revoked_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

impl LinkResponse {
    pub fn build(client_origin: &str, link: LinkRow, now: DateTime<Utc>) -> Self {
        Self {
            id: link.id,
            url: format!("{client_origin}/s/{}", link.code),
            has_password: link.password_hash.is_some(),
            state: link_state(&link, now),
            code: link.code,
            expires_at: link.expires_at,
            revoked_at: link.revoked_at,
            created_at: link.created_at,
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct LinkEnvelope {
    pub link: LinkResponse,
    /// The recording's visibility after the call: making a link switches a private recording to
    /// `link`, so the client can show it.
    pub recording_visibility: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct LinkListEnvelope {
    pub links: Vec<LinkResponse>,
}

/// What a visitor's page needs before anyone presses play. A password-protected link reveals
/// nothing but that it needs one: not even the title.
#[derive(Debug, Serialize, PartialEq)]
pub struct ShareView {
    pub code: String,
    pub requires_password: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_seconds: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize)]
pub struct ShareEnvelope {
    pub share: ShareView,
}

impl ShareView {
    pub fn of(code: &str, target: &Target) -> Self {
        let protected = target.password_hash.is_some();
        Self {
            code: code.to_owned(),
            requires_password: protected,
            title: (!protected).then(|| target.title.clone()),
            status: (!protected).then(|| status_of(target.video_status)),
            duration_seconds: if protected {
                None
            } else {
                target.duration_seconds
            },
            expires_at: target.expires_at,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct SharedWatermark {
    pub text: String,
    pub session_code: String,
}

/// A viewing started through a link. No heartbeat and no resume point: there is no account to
/// remember either for.
#[derive(Debug, Serialize)]
pub struct SharedPlayback {
    pub session_id: Uuid,
    /// Relative to the API's origin; hand it to the HLS player as is.
    pub playlist_url: String,
    pub expires_at: DateTime<Utc>,
    pub watermark: SharedWatermark,
}

#[derive(Debug, Serialize)]
pub struct SharedPlaybackEnvelope {
    pub playback: SharedPlayback,
}

/// Whether a video the link shows can be played right now.
pub fn playable(status: Option<VideoStatus>) -> bool {
    status == Some(VideoStatus::Ready)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn link() -> LinkRow {
        LinkRow {
            id: Uuid::new_v4(),
            code: "abcdefghijklmnopqrstuv".into(),
            recording_id: Uuid::new_v4(),
            password_hash: None,
            expires_at: None,
            revoked_at: None,
            created_at: Utc::now(),
        }
    }

    #[test]
    fn every_link_state_is_listed() {
        let now = Utc::now();
        let mut revoked = link();
        revoked.revoked_at = Some(now);
        let mut expired = link();
        expired.expires_at = Some(now - Duration::hours(1));
        for row in [link(), revoked, expired] {
            assert!(LINK_STATES.contains(&link_state(&row, now)));
        }
        assert_eq!(LINK_STATES.len(), 3);
    }

    fn target(password: bool) -> Target {
        Target {
            link_id: Uuid::new_v4(),
            recording_id: Uuid::new_v4(),
            password_hash: password.then(|| "hash".to_owned()),
            expires_at: None,
            revoked_at: None,
            title: "Week 3".into(),
            visibility: "link".into(),
            recording_deleted: false,
            video_id: Some(Uuid::new_v4()),
            video_status: Some(VideoStatus::Ready),
            duration_seconds: Some(125),
        }
    }

    #[test]
    fn expiry_must_be_a_minute_to_ten_years_away() {
        let now = Utc::now();
        let with = |at| {
            CreateLinkRequest {
                password: None,
                expires_at: Some(at),
            }
            .validate(now)
        };
        assert!(with(now - Duration::hours(1)).is_err());
        assert!(with(now + Duration::seconds(30)).is_err());
        assert!(with(now + Duration::minutes(2)).is_ok());
        assert!(with(now + Duration::days(3650)).is_ok());
        assert!(with(now + Duration::days(3651)).is_err());
        assert!(CreateLinkRequest {
            password: None,
            expires_at: None
        }
        .validate(now)
        .is_ok());
    }

    #[test]
    fn passwords_are_bounded_and_printable() {
        let now = Utc::now();
        let with = |p: &str| {
            CreateLinkRequest {
                password: Some(p.into()),
                expires_at: None,
            }
            .validate(now)
        };
        assert!(with("abc").is_err());
        assert!(with("abcd").is_ok());
        assert!(with(&"x".repeat(128)).is_ok());
        assert!(with(&"x".repeat(129)).is_err());
        assert!(with("tab\tinside").is_err());
        assert!(with("pass word 123").is_ok(), "spaces are fine");
        // Counted in characters, not bytes.
        assert!(with("\u{e9}\u{e9}\u{e9}\u{e9}").is_ok());
    }

    #[test]
    fn a_link_is_revoked_then_expired_then_active() {
        let now = Utc::now();
        let mut l = link();
        assert_eq!(link_state(&l, now), "active");
        l.expires_at = Some(now - Duration::seconds(1));
        assert_eq!(link_state(&l, now), "expired");
        l.revoked_at = Some(now);
        assert_eq!(link_state(&l, now), "revoked");
    }

    #[test]
    fn the_response_builds_the_address_and_never_carries_the_password() {
        let mut l = link();
        l.password_hash = Some("$argon2id$secret".into());
        let built = LinkResponse::build("https://zeno.example", l, Utc::now());
        assert_eq!(built.url, "https://zeno.example/s/abcdefghijklmnopqrstuv");
        assert!(built.has_password);
        assert!(!serde_json::to_string(&built).unwrap().contains("argon2"));
    }

    #[test]
    fn a_protected_link_tells_a_visitor_nothing_but_that_it_needs_a_password() {
        let open = ShareView::of("c", &target(false));
        assert_eq!(open.title.as_deref(), Some("Week 3"));
        assert_eq!(open.status, Some("ready"));
        assert_eq!(open.duration_seconds, Some(125));
        assert!(!open.requires_password);

        let shut = ShareView::of("c", &target(true));
        assert!(shut.requires_password);
        let json = serde_json::to_value(&shut).unwrap();
        assert_eq!(
            json,
            serde_json::json!({"code": "c", "requires_password": true})
        );
    }
}
