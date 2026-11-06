use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use zeno_core::model::VideoStatus;

use crate::error::{ApiError, FieldError};
use crate::videos::repo::VideoRow;

const MAX_FILENAME_CHARS: usize = 255;
const MAX_TITLE_CHARS: usize = 120;
const MAX_CONTENT_TYPE_CHARS: usize = 100;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateVideoRequest {
    pub filename: String,
    pub size_bytes: u64,
    pub content_type: String,
    #[serde(default)]
    pub title: Option<String>,
}

#[derive(Debug)]
pub struct NewVideo {
    pub filename: String,
    pub title: String,
    pub size_bytes: u64,
    pub content_type: String,
}

impl CreateVideoRequest {
    pub fn validate(self, max_bytes: u64) -> Result<NewVideo, ApiError> {
        let mut errors = Vec::new();
        let filename = sanitize_filename(&self.filename);
        if self.size_bytes == 0 {
            errors.push(FieldError::new("size_bytes", "The file is empty."));
        } else if self.size_bytes > max_bytes {
            errors.push(FieldError::new(
                "size_bytes",
                format!("Files can be at most {} MiB.", max_bytes / (1024 * 1024)),
            ));
        }
        let content_type = self.content_type.trim().to_ascii_lowercase();
        if !is_acceptable_content_type(&content_type) {
            errors.push(FieldError::new(
                "content_type",
                "Upload a video file (content type video/*).",
            ));
        }
        let title = match self.title.as_deref().map(str::trim) {
            Some("") | None => title_from(&filename),
            Some(title) if title.chars().count() > MAX_TITLE_CHARS => {
                errors.push(FieldError::new(
                    "title",
                    format!("Title can be at most {MAX_TITLE_CHARS} characters."),
                ));
                String::new()
            }
            Some(title) if title.chars().any(char::is_control) => {
                errors.push(FieldError::new(
                    "title",
                    "Title contains invalid characters.",
                ));
                String::new()
            }
            Some(title) => title.to_owned(),
        };
        if !errors.is_empty() {
            return Err(ApiError::validation(errors));
        }
        Ok(NewVideo {
            filename,
            title,
            size_bytes: self.size_bytes,
            content_type,
        })
    }
}

/// Browsers report an odd or empty type for some containers (`.mkv`), and the declared type
/// proves nothing anyway; what the file really is gets decided by probing it. This only
/// keeps obvious non-video uploads out.
fn is_acceptable_content_type(value: &str) -> bool {
    if value.len() > MAX_CONTENT_TYPE_CHARS
        || !value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "/.+-_".contains(c))
    {
        return false;
    }
    value == "application/octet-stream"
        || value.strip_prefix("video/").is_some_and(|s| !s.is_empty())
}

/// For display only; storage keys never contain it. Drops any path, control characters and
/// excess length.
pub fn sanitize_filename(raw: &str) -> String {
    let name = raw.rsplit(['/', '\\']).next().unwrap_or_default();
    let cleaned: String = name
        .chars()
        .filter(|c| !c.is_control())
        .take(MAX_FILENAME_CHARS)
        .collect();
    let cleaned = cleaned.trim();
    if cleaned.is_empty() || cleaned == "." || cleaned == ".." {
        "video".to_owned()
    } else {
        cleaned.to_owned()
    }
}

fn title_from(filename: &str) -> String {
    let stem = filename.rsplit_once('.').map_or(filename, |(stem, _)| stem);
    let stem = if stem.is_empty() { filename } else { stem };
    stem.chars().take(MAX_TITLE_CHARS).collect()
}

#[derive(Debug, Serialize)]
pub struct VideoResponse {
    pub id: Uuid,
    pub title: String,
    pub status: VideoStatus,
    pub original_filename: String,
    pub size_bytes: i64,
    pub error_code: Option<&'static str>,
    pub duration_seconds: Option<i32>,
    pub width: Option<i32>,
    pub height: Option<i32>,
    pub renditions: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub ready_at: Option<DateTime<Utc>>,
}

impl From<&VideoRow> for VideoResponse {
    fn from(video: &VideoRow) -> Self {
        Self {
            id: video.id,
            title: video.title.clone(),
            status: video.status,
            original_filename: video.original_filename.clone(),
            size_bytes: video.size_bytes,
            error_code: video.error_code.as_deref().map(public_error_code),
            duration_seconds: video.duration_seconds,
            width: video.width,
            height: video.height,
            renditions: video.renditions.clone(),
            created_at: video.created_at,
            ready_at: video.ready_at,
        }
    }
}

/// Only these codes ever reach clients; anything else collapses to a generic one so internal
/// detail cannot leak through a status response.
pub fn public_error_code(code: &str) -> &'static str {
    match code {
        "unsupported_media" => "unsupported_media",
        "no_video_stream" => "no_video_stream",
        "too_long" => "too_long",
        "resolution_too_high" => "resolution_too_high",
        "size_mismatch" => "size_mismatch",
        _ => "processing_failed",
    }
}

#[derive(Debug, Serialize)]
pub struct PartUrl {
    pub number: u16,
    pub size: u64,
    pub url: String,
}

#[derive(Debug, Serialize)]
pub struct UploadResponse {
    pub part_size: u64,
    pub total_parts: u16,
    /// Parts storage already holds in full, so a resuming client can skip them.
    pub uploaded: Vec<u16>,
    /// Parts still to upload, each with a ready-to-use URL.
    pub parts: Vec<PartUrl>,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct CreateVideoResponse {
    pub video: VideoResponse,
    pub upload: UploadResponse,
}

#[derive(Debug, Serialize)]
pub struct VideoEnvelope {
    pub video: VideoResponse,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(
        filename: &str,
        size: u64,
        content_type: &str,
        title: Option<&str>,
    ) -> CreateVideoRequest {
        CreateVideoRequest {
            filename: filename.into(),
            size_bytes: size,
            content_type: content_type.into(),
            title: title.map(str::to_owned),
        }
    }

    const LIMIT: u64 = 10 * 1024 * 1024 * 1024;

    #[test]
    fn a_normal_request_gets_a_title_from_the_filename() {
        let video = request("Lecture 1 - Limits.MP4", 5000, "Video/MP4", None)
            .validate(LIMIT)
            .unwrap();
        assert_eq!(video.title, "Lecture 1 - Limits");
        assert_eq!(video.content_type, "video/mp4");
        let titled = request("a.mp4", 1, "video/mp4", Some("  Chapter 3  "))
            .validate(LIMIT)
            .unwrap();
        assert_eq!(titled.title, "Chapter 3");
    }

    #[test]
    fn filenames_are_stripped_of_paths_and_control_characters() {
        assert_eq!(sanitize_filename("C:\\Users\\me\\clip.mp4"), "clip.mp4");
        assert_eq!(sanitize_filename("../../etc/passwd"), "passwd");
        assert_eq!(sanitize_filename("a\u{0}b\nc.mp4"), "abc.mp4");
        assert_eq!(sanitize_filename(".."), "video");
        assert_eq!(sanitize_filename("   "), "video");
        assert_eq!(sanitize_filename("dir/"), "video");
        assert_eq!(sanitize_filename(&"x".repeat(400)).chars().count(), 255);
    }

    #[test]
    fn sizes_must_be_positive_and_within_the_limit() {
        assert!(request("a.mp4", 0, "video/mp4", None)
            .validate(LIMIT)
            .is_err());
        assert!(request("a.mp4", LIMIT, "video/mp4", None)
            .validate(LIMIT)
            .is_ok());
        assert!(request("a.mp4", LIMIT + 1, "video/mp4", None)
            .validate(LIMIT)
            .is_err());
    }

    #[test]
    fn only_video_like_content_types_pass() {
        for ok in [
            "video/mp4",
            "video/x-matroska",
            "application/octet-stream",
            " VIDEO/WEBM ",
        ] {
            assert!(request("a", 1, ok, None).validate(LIMIT).is_ok(), "{ok}");
        }
        for bad in [
            "",
            "text/html",
            "image/png",
            "video/",
            "video/mp4; charset=x",
            "application/x-msdownload",
            "video/\u{0}",
        ] {
            assert!(
                request("a", 1, bad, None).validate(LIMIT).is_err(),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn every_problem_is_reported_together_and_unknown_fields_are_refused() {
        let err = request("a.mp4", 0, "text/html", Some(&"t".repeat(200)))
            .validate(LIMIT)
            .unwrap_err();
        match err {
            ApiError::Validation(fields) => {
                let names: Vec<_> = fields.iter().map(|f| f.field).collect();
                assert_eq!(names, ["size_bytes", "content_type", "title"]);
            }
            other => panic!("{other:?}"),
        }
        let parsed: Result<CreateVideoRequest, _> = serde_json::from_str(
            r#"{"filename":"a","size_bytes":1,"content_type":"video/mp4","owner_id":"x"}"#,
        );
        assert!(parsed.is_err());
    }

    #[test]
    fn internal_error_codes_never_reach_clients() {
        assert_eq!(public_error_code("unsupported_media"), "unsupported_media");
        assert_eq!(
            public_error_code("ffmpeg exited 1: /tmp/secret/path"),
            "processing_failed"
        );
    }
}
