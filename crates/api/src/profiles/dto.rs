use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use url::Url;

use crate::courses::dto::{normalise, text};
use crate::error::{ApiError, FieldError};
use crate::profiles::repo::ProfileRow;
use crate::state::AppState;

const MAX_LINKS: usize = 5;
const MAX_LINK_URL: usize = 200;

/// Paths and names the platform keeps for itself, so a handle can never impersonate a page.
const RESERVED: &[&str] = &[
    "about",
    "account",
    "admin",
    "administrator",
    "api",
    "app",
    "assets",
    "auth",
    "billing",
    "blog",
    "careers",
    "catalog",
    "checkout",
    "contact",
    "courses",
    "creator",
    "creators",
    "dashboard",
    "docs",
    "help",
    "home",
    "keys",
    "legal",
    "live",
    "login",
    "logout",
    "me",
    "news",
    "play",
    "pricing",
    "privacy",
    "profile",
    "public",
    "refunds",
    "register",
    "root",
    "search",
    "security",
    "settings",
    "signin",
    "signup",
    "static",
    "status",
    "student",
    "students",
    "support",
    "system",
    "teach",
    "teacher",
    "terms",
    "webhooks",
    "www",
    "zeno",
];

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileRequest {
    pub handle: String,
    pub display_name: String,
    #[serde(default)]
    pub bio: String,
    #[serde(default)]
    pub links: Vec<LinkRequest>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LinkRequest {
    pub label: String,
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Link {
    pub label: String,
    pub url: String,
}

#[derive(Debug)]
pub struct ProfileInput {
    pub handle: String,
    pub display_name: String,
    pub bio: String,
    pub links: Vec<Link>,
}

/// Lowercase letters, digits and single hyphens; 3 to 30 characters; not a reserved word.
pub fn valid_handle(handle: &str) -> bool {
    let bytes = handle.as_bytes();
    (3..=30).contains(&bytes.len())
        && bytes
            .iter()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'-')
        && !handle.starts_with('-')
        && !handle.ends_with('-')
        && !handle.contains("--")
        && !RESERVED.contains(&handle)
}

/// Only plain `https` links: no `javascript:`, no credentials, no embedded whitespace.
fn link(errors: &mut Vec<FieldError>, raw: LinkRequest) -> Option<Link> {
    let label = text(errors, "links", raw.label, 1, 30, false);
    let candidate = raw.url.trim();
    let parsed = (candidate.chars().count() <= MAX_LINK_URL
        && !candidate
            .chars()
            .any(|ch| ch.is_whitespace() || ch.is_control()))
    .then(|| Url::parse(candidate).ok())
    .flatten()
    .filter(|url| {
        url.scheme() == "https"
            && url.host_str().is_some_and(|host| host.contains('.'))
            && url.username().is_empty()
            && url.password().is_none()
    });
    match parsed {
        Some(url) => Some(Link {
            label,
            url: url.to_string(),
        }),
        None => {
            errors.push(FieldError::new(
                "links",
                "Each link needs a label and a plain https:// address.",
            ));
            None
        }
    }
}

impl ProfileRequest {
    pub fn validate(self) -> Result<ProfileInput, ApiError> {
        let mut errors = Vec::new();
        let handle = normalise(&self.handle).to_ascii_lowercase();
        if !valid_handle(&handle) {
            errors.push(FieldError::new(
                "handle",
                "Use 3 to 30 lowercase letters, digits or single hyphens, not starting or ending \
                 with a hyphen, and not a reserved word.",
            ));
        }
        let display_name = text(&mut errors, "display_name", self.display_name, 2, 60, false);
        let bio = text(&mut errors, "bio", self.bio, 0, 500, true);
        if self.links.len() > MAX_LINKS {
            errors.push(FieldError::new(
                "links",
                format!("At most {MAX_LINKS} links."),
            ));
        }
        let links: Vec<Link> = self
            .links
            .into_iter()
            .take(MAX_LINKS)
            .filter_map(|raw| link(&mut errors, raw))
            .collect();
        if errors.is_empty() {
            Ok(ProfileInput {
                handle,
                display_name,
                bio,
                links,
            })
        } else {
            Err(ApiError::validation(errors))
        }
    }
}

#[derive(Debug, Serialize)]
pub struct ProfileResponse {
    pub handle: String,
    pub display_name: String,
    pub bio: String,
    pub avatar_url: Option<String>,
    pub links: Vec<Link>,
    pub updated_at: DateTime<Utc>,
}

impl ProfileResponse {
    pub fn build(state: &AppState, row: &ProfileRow) -> Self {
        Self {
            handle: row.handle.clone(),
            display_name: row.display_name.clone(),
            bio: row.bio.clone(),
            avatar_url: row
                .avatar_key
                .as_deref()
                .map(|key| crate::images::public_url(state, key)),
            links: serde_json::from_value(row.links.clone()).unwrap_or_default(),
            updated_at: row.updated_at,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct ProfileEnvelope {
    pub profile: Option<ProfileResponse>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(handle: &str) -> ProfileRequest {
        ProfileRequest {
            handle: handle.into(),
            display_name: "  Asha Rao ".into(),
            bio: "Physics teacher.\nIIT-JEE.".into(),
            links: vec![LinkRequest {
                label: "YouTube".into(),
                url: " https://youtube.com/@asha ".into(),
            }],
        }
    }

    #[test]
    fn a_normal_profile_is_cleaned_up() {
        let input = request(" Asha-Physics ").validate().unwrap();
        assert_eq!(input.handle, "asha-physics");
        assert_eq!(input.display_name, "Asha Rao");
        assert_eq!(input.links[0].url, "https://youtube.com/@asha");
    }

    #[test]
    fn handles_are_strict() {
        for good in ["abc", "asha-rao", "physics101", "a1b", &"x".repeat(30)] {
            assert!(valid_handle(good), "{good}");
        }
        for bad in [
            "",
            "ab",
            "-abc",
            "abc-",
            "a--b",
            "Asha",
            "asha_rao",
            "asha.rao",
            "asha rao",
            "आशा",
            "admin",
            "api",
            "login",
            "zeno",
            &"x".repeat(31),
        ] {
            assert!(!valid_handle(bad), "{bad}");
        }
    }

    #[test]
    fn only_plain_https_links_are_kept() {
        for bad in [
            "javascript:alert(1)",
            "http://example.com",
            "https://user:pass@example.com",
            "https://localhost",
            "//example.com",
            "https://exa mple.com",
            "data:text/html;base64,AAAA",
            "ftp://example.com",
            "",
        ] {
            let mut r = request("asha-rao");
            r.links[0].url = bad.into();
            assert!(r.validate().is_err(), "{bad:?} must be refused");
        }
        let mut r = request("asha-rao");
        r.links = (0..6)
            .map(|i| LinkRequest {
                label: format!("l{i}"),
                url: "https://example.com".into(),
            })
            .collect();
        assert!(r.validate().is_err(), "more than five links");
    }

    #[test]
    fn names_and_bios_reject_control_characters() {
        let mut r = request("asha-rao");
        r.display_name = "A\u{0}sha".into();
        assert!(r.validate().is_err());
        let mut r = request("asha-rao");
        r.bio = "x".repeat(501);
        assert!(r.validate().is_err());
        let mut r = request("asha-rao");
        r.display_name = "आशा राव 🎓".into();
        assert!(r.validate().is_ok());
    }
}
