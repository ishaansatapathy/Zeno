use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use unicode_normalization::UnicodeNormalization;
use uuid::Uuid;
use zeno_core::model::CourseStatus;

use crate::courses::repo::CourseRow;
use crate::error::{ApiError, FieldError};

const MAX_TITLE: usize = 120;
const MAX_DESCRIPTION: usize = 5_000;
const MAX_SHORT_DESCRIPTION: usize = 300;
const MAX_LANGUAGE: usize = 35;
pub const MAX_PRICE_PAISE: i64 = 100_000_000;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateCourseRequest {
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub short_description: String,
    #[serde(default)]
    pub price_paise: i64,
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default)]
    pub level: Option<String>,
}

#[derive(Debug)]
pub struct NewCourse {
    pub title: String,
    pub description: String,
    pub short_description: String,
    pub price_paise: i64,
    pub language: Option<String>,
    pub level: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateCourseRequest {
    pub version: Option<i32>,
    pub title: Option<String>,
    pub description: Option<String>,
    pub short_description: Option<String>,
    pub price_paise: Option<i64>,
    pub language: Option<Option<String>>,
    pub level: Option<String>,
}

#[derive(Debug, Default)]
pub struct CourseChanges {
    pub title: Option<String>,
    pub description: Option<String>,
    pub short_description: Option<String>,
    pub price_paise: Option<i64>,
    pub language: Option<Option<String>>,
    pub level: Option<String>,
}

pub(crate) fn normalise(raw: &str) -> String {
    raw.trim().nfc().collect()
}

pub(crate) fn text(
    errors: &mut Vec<FieldError>,
    field: &'static str,
    raw: String,
    min: usize,
    max: usize,
    allow_layout: bool,
) -> String {
    let value = normalise(&raw);
    let len = value.chars().count();
    if len < min || len > max {
        errors.push(FieldError::new(
            field,
            if min == 0 {
                format!("Must be at most {max} characters.")
            } else {
                format!("Must be between {min} and {max} characters.")
            },
        ));
    } else if value
        .chars()
        .any(|ch| ch.is_control() && !(allow_layout && matches!(ch, '\n' | '\t')))
    {
        errors.push(FieldError::new(field, "Contains invalid characters."));
    }
    value
}

fn price(errors: &mut Vec<FieldError>, value: i64) {
    if value != 0 && !(100..=MAX_PRICE_PAISE).contains(&value) {
        errors.push(FieldError::new(
            "price_paise",
            "Use 0 for free, or an amount from Rs 1 to Rs 10,00,000.",
        ));
    }
}

fn language(errors: &mut Vec<FieldError>, raw: String) -> String {
    let value = normalise(&raw).to_ascii_lowercase();
    let valid = (2..=MAX_LANGUAGE).contains(&value.len())
        && value
            .split('-')
            .all(|part| !part.is_empty() && part.chars().all(|ch| ch.is_ascii_alphanumeric()));
    if !valid {
        errors.push(FieldError::new(
            "language",
            "Use a language tag such as en, hi, or en-in.",
        ));
    }
    value
}

fn level(errors: &mut Vec<FieldError>, raw: String) -> String {
    let value = normalise(&raw).to_ascii_lowercase();
    if !matches!(
        value.as_str(),
        "all_levels" | "beginner" | "intermediate" | "advanced"
    ) {
        errors.push(FieldError::new(
            "level",
            "Choose all_levels, beginner, intermediate, or advanced.",
        ));
    }
    value
}

impl CreateCourseRequest {
    pub fn validate(self) -> Result<NewCourse, ApiError> {
        let mut errors = Vec::new();
        let title = text(&mut errors, "title", self.title, 3, MAX_TITLE, false);
        let description = text(
            &mut errors,
            "description",
            self.description,
            0,
            MAX_DESCRIPTION,
            true,
        );
        let short_description = text(
            &mut errors,
            "short_description",
            self.short_description,
            0,
            MAX_SHORT_DESCRIPTION,
            false,
        );
        price(&mut errors, self.price_paise);
        let language = self.language.map(|value| language(&mut errors, value));
        let level = level(
            &mut errors,
            self.level.unwrap_or_else(|| "all_levels".into()),
        );
        if !errors.is_empty() {
            return Err(ApiError::validation(errors));
        }
        Ok(NewCourse {
            title,
            description,
            short_description,
            price_paise: self.price_paise,
            language,
            level,
        })
    }
}

impl UpdateCourseRequest {
    pub fn validate(self) -> Result<(Option<i32>, CourseChanges), ApiError> {
        let mut errors = Vec::new();
        if self.version.is_some_and(|version| version <= 0) {
            errors.push(FieldError::new("version", "Must be a positive integer."));
        }
        let title = self
            .title
            .map(|value| text(&mut errors, "title", value, 3, MAX_TITLE, false));
        let description = self
            .description
            .map(|value| text(&mut errors, "description", value, 0, MAX_DESCRIPTION, true));
        let short_description = self.short_description.map(|value| {
            text(
                &mut errors,
                "short_description",
                value,
                0,
                MAX_SHORT_DESCRIPTION,
                false,
            )
        });
        if let Some(value) = self.price_paise {
            price(&mut errors, value);
        }
        let language_value = self
            .language
            .map(|value| value.map(|raw| language(&mut errors, raw)));
        let level_value = self.level.map(|value| level(&mut errors, value));
        let changes = CourseChanges {
            title,
            description,
            short_description,
            price_paise: self.price_paise,
            language: language_value,
            level: level_value,
        };
        if changes.title.is_none()
            && changes.description.is_none()
            && changes.short_description.is_none()
            && changes.price_paise.is_none()
            && changes.language.is_none()
            && changes.level.is_none()
        {
            errors.push(FieldError::new(
                "body",
                "Provide at least one field to update.",
            ));
        }
        if !errors.is_empty() {
            return Err(ApiError::validation(errors));
        }
        Ok((self.version, changes))
    }
}

pub fn slug_base(title: &str) -> String {
    let mut out = String::new();
    let mut separator = false;
    for ch in title.chars() {
        if ch.is_ascii_alphanumeric() {
            if separator && !out.is_empty() {
                out.push('-');
            }
            out.push(ch.to_ascii_lowercase());
            separator = false;
        } else {
            separator = true;
        }
    }
    out.truncate(out.len().min(120));
    out.trim_matches('-').to_owned()
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VersionRequest {
    pub version: i32,
}

#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct ListCoursesQuery {
    pub cursor: Option<String>,
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Copy)]
pub struct CourseCursor {
    pub created_at: DateTime<Utc>,
    pub id: Uuid,
}

impl CourseCursor {
    pub fn encode(self) -> String {
        URL_SAFE_NO_PAD.encode(format!("{}|{}", self.created_at.to_rfc3339(), self.id))
    }

    pub fn decode(raw: &str) -> Result<Self, ApiError> {
        let bytes = URL_SAFE_NO_PAD.decode(raw).map_err(|_| {
            ApiError::bad_request("invalid_cursor", "The pagination cursor is invalid.")
        })?;
        let text = String::from_utf8(bytes).map_err(|_| {
            ApiError::bad_request("invalid_cursor", "The pagination cursor is invalid.")
        })?;
        let (created_at, id) = text.split_once('|').ok_or_else(|| {
            ApiError::bad_request("invalid_cursor", "The pagination cursor is invalid.")
        })?;
        Ok(Self {
            created_at: DateTime::parse_from_rfc3339(created_at)
                .map_err(|_| {
                    ApiError::bad_request("invalid_cursor", "The pagination cursor is invalid.")
                })?
                .with_timezone(&Utc),
            id: Uuid::parse_str(id).map_err(|_| {
                ApiError::bad_request("invalid_cursor", "The pagination cursor is invalid.")
            })?,
        })
    }
}

#[derive(Debug, Serialize)]
pub struct CourseResponse {
    pub id: Uuid,
    pub creator_id: Uuid,
    pub slug: String,
    pub title: String,
    pub description: String,
    pub short_description: String,
    pub price_paise: i64,
    pub currency: String,
    pub status: CourseStatus,
    pub language: Option<String>,
    pub level: String,
    pub version: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub published_at: Option<DateTime<Utc>>,
}

impl From<&CourseRow> for CourseResponse {
    fn from(course: &CourseRow) -> Self {
        Self {
            id: course.id,
            creator_id: course.creator_id,
            slug: course.slug.clone(),
            title: course.title.clone(),
            description: course.description.clone(),
            short_description: course.short_description.clone(),
            price_paise: course.price_paise,
            currency: course.currency.clone(),
            status: course.status,
            language: course.language.clone(),
            level: course.level.clone(),
            version: course.version,
            created_at: course.created_at,
            updated_at: course.updated_at,
            published_at: course.published_at,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct CourseEnvelope {
    pub course: CourseResponse,
}

#[derive(Debug, Serialize)]
pub struct CourseListEnvelope {
    pub courses: Vec<CourseResponse>,
    pub next_cursor: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_validation_normalises_and_rejects_bad_combinations() {
        let course = CreateCourseRequest {
            title: "  Physics 101  ".into(),
            description: " Intro ".into(),
            short_description: String::new(),
            price_paise: 49_900,
            language: Some("HI-IN".into()),
            level: Some("BEGINNER".into()),
        }
        .validate()
        .unwrap();
        assert_eq!(course.title, "Physics 101");
        assert_eq!(course.language.as_deref(), Some("hi-in"));
        assert_eq!(course.level, "beginner");

        for price in [-1, 1, 99, MAX_PRICE_PAISE + 1] {
            let request = CreateCourseRequest {
                title: "Good course".into(),
                description: String::new(),
                short_description: String::new(),
                price_paise: price,
                language: None,
                level: None,
            };
            assert!(request.validate().is_err(), "{price}");
        }
    }

    #[test]
    fn slugs_are_safe_and_non_ascii_titles_use_a_fallback_later() {
        assert_eq!(slug_base("  Rust & Systems: 101! "), "rust-systems-101");
        assert_eq!(slug_base("हिंदी गणित"), "");
    }

    #[test]
    fn cursors_round_trip_and_garbage_is_refused() {
        let cursor = CourseCursor {
            created_at: Utc::now(),
            id: Uuid::new_v4(),
        };
        let decoded = CourseCursor::decode(&cursor.encode()).unwrap();
        assert_eq!(decoded.id, cursor.id);
        assert_eq!(decoded.created_at, cursor.created_at);
        assert!(CourseCursor::decode("not-a-cursor").is_err());
    }
}
