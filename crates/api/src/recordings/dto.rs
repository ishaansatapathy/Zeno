use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;
use zeno_core::model::VideoStatus;

use crate::courses::dto::text;
use crate::error::{ApiError, FieldError};
use crate::recordings::repo::RecordingRow;

pub const MAX_TITLE: usize = 120;
pub const DEFAULT_TITLE: &str = "Untitled recording";
/// What produced a recording. Fixed for its life.
pub const SOURCES: &[&str] = &["upload", "extension", "studio", "stream", "meeting"];
/// Who may watch. Nothing but the owner can until share links exist; this only records intent.
pub const VISIBILITIES: &[&str] = &["private", "link", "public"];
/// Where a recording is in its life; what `status_of` can answer.
pub const STATUSES: &[&str] = &["capturing", "uploading", "processing", "ready", "failed"];

#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateRecordingRequest {
    /// A client-chosen id (a random UUID). Sending the same one again returns the recording the
    /// first attempt made, so a retry after a dropped connection never makes a duplicate.
    pub id: Option<Uuid>,
    /// Defaults to "Untitled recording".
    #[schema(min_length = 1, max_length = 120)]
    pub title: Option<String>,
    /// What produces the recording. Fixed for its life.
    pub source: String,
    /// An upload of the caller's own to attach straight away.
    pub video_id: Option<Uuid>,
}

#[derive(Debug)]
pub struct NewRecording {
    pub id: Uuid,
    pub title: String,
    pub source: &'static str,
    pub video_id: Option<Uuid>,
}

fn choose(
    errors: &mut Vec<FieldError>,
    field: &'static str,
    raw: &str,
    allowed: &'static [&'static str],
) -> Option<&'static str> {
    let value = raw.trim().to_ascii_lowercase();
    let found = allowed.iter().copied().find(|option| *option == value);
    if found.is_none() {
        errors.push(FieldError::new(
            field,
            format!("Choose one of: {}.", allowed.join(", ")),
        ));
    }
    found
}

impl CreateRecordingRequest {
    pub fn validate(self) -> Result<NewRecording, ApiError> {
        let mut errors = Vec::new();
        let source = choose(&mut errors, "source", &self.source, SOURCES);
        let title = match self.title {
            Some(raw) => text(&mut errors, "title", raw, 1, MAX_TITLE, false),
            None => DEFAULT_TITLE.to_owned(),
        };
        if self.id.is_some_and(|id| id.is_nil()) {
            errors.push(FieldError::new("id", "Must not be the nil UUID."));
        }
        match (errors.is_empty(), source) {
            (true, Some(source)) => Ok(NewRecording {
                id: self.id.unwrap_or_else(Uuid::new_v4),
                title,
                source,
                video_id: self.video_id,
            }),
            _ => Err(ApiError::validation(errors)),
        }
    }
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateRecordingRequest {
    /// The version you read. Required, here or in `If-Match`.
    #[schema(minimum = 1)]
    pub version: Option<i32>,
    #[schema(min_length = 1, max_length = 120)]
    pub title: Option<String>,
    /// Who may watch. Changing it to `private` ends every viewing under way through a link.
    pub visibility: Option<String>,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct RecordingChanges {
    pub title: Option<String>,
    pub visibility: Option<&'static str>,
}

impl UpdateRecordingRequest {
    pub fn validate(self) -> Result<(Option<i32>, RecordingChanges), ApiError> {
        let mut errors = Vec::new();
        if self.version.is_some_and(|version| version <= 0) {
            errors.push(FieldError::new("version", "Must be a positive integer."));
        }
        let title = self
            .title
            .map(|raw| text(&mut errors, "title", raw, 1, MAX_TITLE, false));
        let visibility = self
            .visibility
            .and_then(|raw| choose(&mut errors, "visibility", &raw, VISIBILITIES));
        if title.is_none() && visibility.is_none() && errors.is_empty() {
            errors.push(FieldError::new(
                "title",
                "Send a title or a visibility to change.",
            ));
        }
        if !errors.is_empty() {
            return Err(ApiError::validation(errors));
        }
        Ok((self.version, RecordingChanges { title, visibility }))
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AttachVideoRequest {
    pub video_id: Uuid,
}

#[derive(Debug, Deserialize, Default, IntoParams)]
#[serde(deny_unknown_fields)]
#[into_params(parameter_in = Query)]
pub struct ListRecordingsQuery {
    /// `next_cursor` from the previous page.
    pub cursor: Option<String>,
    /// 1 to 50; default 20.
    #[param(minimum = 1, maximum = 50)]
    pub limit: Option<u32>,
    /// Only recordings made this way.
    pub source: Option<String>,
}

impl ListRecordingsQuery {
    pub fn source(&self) -> Result<Option<&'static str>, ApiError> {
        let Some(raw) = self.source.as_deref() else {
            return Ok(None);
        };
        let mut errors = Vec::new();
        let source = choose(&mut errors, "source", raw, SOURCES);
        match source {
            Some(source) => Ok(Some(source)),
            None => Err(ApiError::validation(errors)),
        }
    }
}

/// Where a recording is in its life, from what the video underneath says.
pub fn status_of(video: Option<VideoStatus>) -> &'static str {
    match video {
        None => "capturing",
        Some(VideoStatus::Uploading) => "uploading",
        Some(VideoStatus::Processing) => "processing",
        Some(VideoStatus::Ready) => "ready",
        Some(VideoStatus::Failed) => "failed",
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct RecordingResponse {
    pub id: Uuid,
    pub owner_id: Uuid,
    pub title: String,
    pub source: String,
    pub visibility: String,
    /// Read from the video underneath; never stored.
    pub status: &'static str,
    pub video_id: Option<Uuid>,
    pub duration_seconds: Option<i32>,
    /// Changes on every edit; send it back to edit.
    pub version: i32,
    /// All-time views, as of the last flush (see `GET /v1/recordings/{id}/views`).
    pub view_count: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl From<RecordingRow> for RecordingResponse {
    fn from(row: RecordingRow) -> Self {
        Self {
            status: status_of(row.video_status),
            id: row.id,
            owner_id: row.owner_id,
            title: row.title,
            source: row.source,
            visibility: row.visibility,
            video_id: row.video_id,
            duration_seconds: row.duration_seconds,
            version: row.version,
            view_count: row.view_count,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct RecordingEnvelope {
    pub recording: RecordingResponse,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct RecordingListEnvelope {
    pub recordings: Vec<RecordingResponse>,
    /// Pass as `cursor` for the next page; absent on the last.
    pub next_cursor: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create(source: &str, title: Option<&str>) -> CreateRecordingRequest {
        CreateRecordingRequest {
            id: None,
            title: title.map(str::to_owned),
            source: source.to_owned(),
            video_id: None,
        }
    }

    #[test]
    fn a_new_recording_gets_a_default_title_and_an_id() {
        let made = create("Extension", None).validate().unwrap();
        assert_eq!(made.title, DEFAULT_TITLE);
        assert_eq!(made.source, "extension");
        assert!(!made.id.is_nil());
    }

    #[test]
    fn titles_are_trimmed_normalised_and_bounded() {
        let made = create("studio", Some("  Week 3: Cafe\u{0301}  "))
            .validate()
            .unwrap();
        assert_eq!(made.title, "Week 3: Caf\u{e9}");
        assert!(create("studio", Some("   ")).validate().is_err());
        assert!(create("studio", Some(&"x".repeat(MAX_TITLE + 1)))
            .validate()
            .is_err());
        assert!(create("studio", Some(&"x".repeat(MAX_TITLE)))
            .validate()
            .is_ok());
        assert!(create("studio", Some("tab\there")).validate().is_err());
    }

    #[test]
    fn only_known_sources_are_accepted_and_every_error_is_reported_at_once() {
        for source in SOURCES {
            assert!(create(source, None).validate().is_ok(), "{source}");
        }
        let err = create("telepathy", Some("")).validate().unwrap_err();
        let ApiError::Validation(errors) = err else {
            panic!("expected field errors");
        };
        let fields: Vec<_> = errors.iter().map(|e| e.field).collect();
        assert_eq!(fields, ["source", "title"]);
    }

    #[test]
    fn the_nil_id_is_refused() {
        let mut request = create("upload", None);
        request.id = Some(Uuid::nil());
        assert!(request.validate().is_err());
    }

    #[test]
    fn an_update_must_change_something_and_visibility_must_be_known() {
        let nothing = UpdateRecordingRequest {
            version: Some(1),
            title: None,
            visibility: None,
        };
        assert!(nothing.validate().is_err());
        let bad = UpdateRecordingRequest {
            version: Some(1),
            title: None,
            visibility: Some("everyone".into()),
        };
        assert!(bad.validate().is_err());
        let good = UpdateRecordingRequest {
            version: Some(2),
            title: Some(" New ".into()),
            visibility: Some("LINK".into()),
        };
        let (version, changes) = good.validate().unwrap();
        assert_eq!(version, Some(2));
        assert_eq!(changes.title.as_deref(), Some("New"));
        assert_eq!(changes.visibility, Some("link"));
        let stale = UpdateRecordingRequest {
            version: Some(0),
            title: Some("x".into()),
            visibility: None,
        };
        assert!(stale.validate().is_err());
    }

    #[test]
    fn every_status_is_listed() {
        for video in [
            None,
            Some(VideoStatus::Uploading),
            Some(VideoStatus::Processing),
            Some(VideoStatus::Ready),
            Some(VideoStatus::Failed),
        ] {
            assert!(STATUSES.contains(&status_of(video)));
        }
        assert_eq!(STATUSES.len(), 5);
    }

    #[test]
    fn status_follows_the_video() {
        assert_eq!(status_of(None), "capturing");
        assert_eq!(status_of(Some(VideoStatus::Uploading)), "uploading");
        assert_eq!(status_of(Some(VideoStatus::Processing)), "processing");
        assert_eq!(status_of(Some(VideoStatus::Ready)), "ready");
        assert_eq!(status_of(Some(VideoStatus::Failed)), "failed");
    }

    #[test]
    fn the_list_source_filter_is_validated() {
        let ok = ListRecordingsQuery {
            source: Some("Stream".into()),
            ..Default::default()
        };
        assert_eq!(ok.source().unwrap(), Some("stream"));
        let bad = ListRecordingsQuery {
            source: Some("x".into()),
            ..Default::default()
        };
        assert!(bad.source().is_err());
        assert_eq!(ListRecordingsQuery::default().source().unwrap(), None);
    }
}
