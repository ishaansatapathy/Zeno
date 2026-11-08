use axum::async_trait;
use axum::extract::{FromRequestParts, Query};
use axum::http::request::Parts;
use chrono::{DateTime, Utc};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::catalog::repo::{CardRow, LessonRow};
use crate::courses::dto::{language, level, normalise, MAX_PRICE_PAISE};
use crate::error::{ApiError, FieldError};
use crate::images;
use crate::profiles::dto::ProfileResponse;
use crate::profiles::repo::ProfileRow;
use crate::state::AppState;

pub const DEFAULT_PAGE_SIZE: u32 = 12;
pub const MAX_PAGE_SIZE: u32 = 24;
pub const MAX_PAGE: u32 = 50;
const MAX_QUERY_CHARS: usize = 100;

/// `Query` that reports a bad query string in the API's own error format.
pub struct ApiQuery<T>(pub T);

#[async_trait]
impl<S, T> FromRequestParts<S> for ApiQuery<T>
where
    S: Send + Sync,
    T: DeserializeOwned,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        match Query::<T>::from_request_parts(parts, state).await {
            Ok(Query(value)) => Ok(ApiQuery(value)),
            Err(rejection) => Err(ApiError::bad_request(
                "invalid_query",
                rejection.body_text(),
            )),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Sort {
    Relevance,
    Newest,
    PriceAsc,
    PriceDesc,
}

#[derive(Debug, Default, Deserialize)]
pub struct CatalogQuery {
    pub q: Option<String>,
    pub language: Option<String>,
    pub level: Option<String>,
    pub free: Option<bool>,
    pub min_price: Option<i64>,
    pub max_price: Option<i64>,
    pub sort: Option<Sort>,
    pub page: Option<u32>,
    pub limit: Option<u32>,
}

/// A validated catalogue request.
#[derive(Debug, Default, Clone)]
pub struct Filter {
    pub text: Option<String>,
    pub language: Option<String>,
    pub level: Option<String>,
    pub free_only: bool,
    pub min_price: Option<i64>,
    pub max_price: Option<i64>,
    pub sort: Option<Sort>,
    pub creator_id: Option<Uuid>,
    pub limit: u32,
    pub offset: u32,
}

impl CatalogQuery {
    pub fn validate(self) -> Result<Filter, ApiError> {
        let mut errors = Vec::new();
        let text = self
            .q
            .map(|raw| normalise(&raw))
            .filter(|value| !value.is_empty());
        if let Some(text) = &text {
            if text.chars().count() > MAX_QUERY_CHARS || text.chars().any(char::is_control) {
                errors.push(FieldError::new(
                    "q",
                    format!("Search text can be at most {MAX_QUERY_CHARS} characters."),
                ));
            }
        }
        let language = self.language.map(|raw| language(&mut errors, raw));
        let level = self.level.map(|raw| level(&mut errors, raw));
        for (field, value) in [("min_price", self.min_price), ("max_price", self.max_price)] {
            if value.is_some_and(|value| !(0..=MAX_PRICE_PAISE).contains(&value)) {
                errors.push(FieldError::new(field, "Use an amount in paise from 0 up."));
            }
        }
        if let (Some(min), Some(max)) = (self.min_price, self.max_price) {
            if min > max {
                errors.push(FieldError::new(
                    "min_price",
                    "min_price cannot be above max_price.",
                ));
            }
        }
        let page = self.page.unwrap_or(1);
        if !(1..=MAX_PAGE).contains(&page) {
            errors.push(FieldError::new(
                "page",
                format!("Use a page from 1 to {MAX_PAGE}."),
            ));
        }
        let limit = self.limit.unwrap_or(DEFAULT_PAGE_SIZE);
        if !(1..=MAX_PAGE_SIZE).contains(&limit) {
            errors.push(FieldError::new(
                "limit",
                format!("Use a limit from 1 to {MAX_PAGE_SIZE}."),
            ));
        }
        if self.sort == Some(Sort::Relevance) && text.is_none() {
            errors.push(FieldError::new(
                "sort",
                "Relevance sorting needs a search text.",
            ));
        }
        if !errors.is_empty() {
            return Err(ApiError::validation(errors));
        }
        Ok(Filter {
            text,
            language,
            level,
            free_only: self.free.unwrap_or(false),
            min_price: self.min_price,
            max_price: self.max_price,
            sort: self.sort,
            creator_id: None,
            limit,
            offset: (page - 1) * limit,
        })
    }
}

#[derive(Debug, Serialize)]
pub struct CreatorBadge {
    pub handle: Option<String>,
    pub display_name: String,
    pub avatar_url: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct CourseCard {
    pub slug: String,
    pub title: String,
    pub short_description: String,
    pub price_paise: i64,
    pub currency: String,
    pub language: Option<String>,
    pub level: String,
    pub thumbnail_url: Option<String>,
    pub published_at: Option<DateTime<Utc>>,
    pub lesson_count: i64,
    pub total_duration_seconds: i64,
    pub creator: CreatorBadge,
}

impl CourseCard {
    pub fn build(state: &AppState, row: &CardRow) -> Self {
        Self {
            slug: row.slug.clone(),
            title: row.title.clone(),
            short_description: row.short_description.clone(),
            price_paise: row.price_paise,
            currency: row.currency.clone(),
            language: row.language.clone(),
            level: row.level.clone(),
            thumbnail_url: row
                .thumbnail_key
                .as_deref()
                .map(|key| images::public_url(state, key)),
            published_at: row.published_at,
            lesson_count: row.lesson_count,
            total_duration_seconds: row.total_seconds,
            creator: CreatorBadge {
                handle: row.creator_handle.clone(),
                display_name: row.creator_name.clone(),
                avatar_url: row
                    .creator_avatar_key
                    .as_deref()
                    .map(|key| images::public_url(state, key)),
            },
        }
    }
}

#[derive(Debug, Serialize)]
pub struct CatalogPage {
    pub courses: Vec<CourseCard>,
    pub page: u32,
    pub has_more: bool,
}

/// A lesson as a visitor sees it. The lesson id is given out only for free previews, because
/// that is the one case where a visitor can do something with it (start playback).
#[derive(Debug, Serialize)]
pub struct PublicLesson {
    pub position: i32,
    pub title: String,
    pub duration_seconds: Option<i32>,
    pub is_free_preview: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lesson_id: Option<Uuid>,
}

impl From<&LessonRow> for PublicLesson {
    fn from(row: &LessonRow) -> Self {
        Self {
            position: row.position,
            title: row.title.clone(),
            duration_seconds: row.duration_seconds,
            is_free_preview: row.is_free_preview,
            lesson_id: row.is_free_preview.then_some(row.id),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct CourseDetail {
    #[serde(flatten)]
    pub card: CourseCard,
    pub description: String,
    pub lessons: Vec<PublicLesson>,
}

#[derive(Debug, Serialize)]
pub struct CourseDetailEnvelope {
    pub course: CourseDetail,
}

#[derive(Debug, Serialize)]
pub struct CreatorPage {
    pub creator: ProfileResponse,
    pub courses: Vec<CourseCard>,
}

pub fn creator_page(state: &AppState, profile: &ProfileRow, cards: &[CardRow]) -> CreatorPage {
    CreatorPage {
        creator: ProfileResponse::build(state, profile),
        courses: cards
            .iter()
            .map(|row| CourseCard::build(state, row))
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(query: CatalogQuery) -> Filter {
        query.validate().unwrap()
    }

    #[test]
    fn defaults_are_a_first_page_of_twelve() {
        let filter = ok(CatalogQuery::default());
        assert_eq!((filter.limit, filter.offset), (12, 0));
        assert!(filter.text.is_none() && !filter.free_only);
    }

    #[test]
    fn paging_math_and_bounds() {
        let filter = ok(CatalogQuery {
            page: Some(3),
            limit: Some(24),
            ..Default::default()
        });
        assert_eq!(filter.offset, 48);
        for bad in [(0, 12), (51, 12), (1, 0), (1, 25)] {
            let query = CatalogQuery {
                page: Some(bad.0),
                limit: Some(bad.1),
                ..Default::default()
            };
            assert!(query.validate().is_err(), "{bad:?}");
        }
    }

    #[test]
    fn search_text_is_cleaned_and_bounded() {
        let filter = ok(CatalogQuery {
            q: Some("  Python  ".into()),
            ..Default::default()
        });
        assert_eq!(filter.text.as_deref(), Some("Python"));
        let blank = ok(CatalogQuery {
            q: Some("   ".into()),
            ..Default::default()
        });
        assert!(blank.text.is_none());
        for bad in ["x".repeat(101), "a\u{0}b".into(), "a\nb".into()] {
            let query = CatalogQuery {
                q: Some(bad),
                ..Default::default()
            };
            assert!(query.validate().is_err());
        }
    }

    #[test]
    fn price_filters_and_sorting_are_checked() {
        let both = CatalogQuery {
            min_price: Some(50_000),
            max_price: Some(10_000),
            ..Default::default()
        };
        assert!(both.validate().is_err());
        let negative = CatalogQuery {
            min_price: Some(-5),
            ..Default::default()
        };
        assert!(negative.validate().is_err());
        let free_to_cheap = CatalogQuery {
            min_price: Some(0),
            max_price: Some(49_900),
            ..Default::default()
        };
        assert!(free_to_cheap.validate().is_ok());
        let relevance_without_text = CatalogQuery {
            sort: Some(Sort::Relevance),
            ..Default::default()
        };
        assert!(relevance_without_text.validate().is_err());
    }

    #[test]
    fn language_and_level_are_normalised() {
        let filter = ok(CatalogQuery {
            language: Some("HI".into()),
            level: Some("Beginner".into()),
            ..Default::default()
        });
        assert_eq!(filter.language.as_deref(), Some("hi"));
        assert_eq!(filter.level.as_deref(), Some("beginner"));
        let bad = CatalogQuery {
            level: Some("expert".into()),
            ..Default::default()
        };
        assert!(bad.validate().is_err());
    }
}
