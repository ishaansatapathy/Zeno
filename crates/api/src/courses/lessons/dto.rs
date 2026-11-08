use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use unicode_normalization::UnicodeNormalization;
use uuid::Uuid;
use zeno_core::model::VideoStatus;

use crate::courses::lessons::repo::LessonRow;
use crate::error::{ApiError, FieldError};

const MAX_TITLE: usize = 120;
const MAX_DESCRIPTION: usize = 5_000;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateLessonRequest {
    pub title: String,
    #[serde(default)]
    pub description: String,
    pub video_id: Uuid,
    #[serde(default)]
    pub is_free_preview: bool,
    pub position: Option<i32>,
}

#[derive(Debug)]
pub struct NewLesson {
    pub title: String,
    pub description: String,
    pub video_id: Uuid,
    pub is_free_preview: bool,
    pub position: Option<i32>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateLessonRequest {
    pub title: Option<String>,
    pub description: Option<String>,
    pub video_id: Option<Uuid>,
    pub is_free_preview: Option<bool>,
}

#[derive(Debug, Default)]
pub struct LessonChanges {
    pub title: Option<String>,
    pub description: Option<String>,
    pub video_id: Option<Uuid>,
    pub is_free_preview: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReorderLessonsRequest {
    pub lesson_ids: Vec<Uuid>,
}

fn text(
    errors: &mut Vec<FieldError>,
    field: &'static str,
    raw: String,
    min: usize,
    max: usize,
    allow_layout: bool,
) -> String {
    let value: String = raw.trim().nfc().collect();
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

impl CreateLessonRequest {
    pub fn validate(self) -> Result<NewLesson, ApiError> {
        let mut errors = Vec::new();
        let title = text(&mut errors, "title", self.title, 1, MAX_TITLE, false);
        let description = text(
            &mut errors,
            "description",
            self.description,
            0,
            MAX_DESCRIPTION,
            true,
        );
        if self.position.is_some_and(|position| position < 1) {
            errors.push(FieldError::new("position", "Must be at least 1."));
        }
        if !errors.is_empty() {
            return Err(ApiError::validation(errors));
        }
        Ok(NewLesson {
            title,
            description,
            video_id: self.video_id,
            is_free_preview: self.is_free_preview,
            position: self.position,
        })
    }
}

impl UpdateLessonRequest {
    pub fn validate(self) -> Result<LessonChanges, ApiError> {
        let mut errors = Vec::new();
        let changes = LessonChanges {
            title: self
                .title
                .map(|value| text(&mut errors, "title", value, 1, MAX_TITLE, false)),
            description: self
                .description
                .map(|value| text(&mut errors, "description", value, 0, MAX_DESCRIPTION, true)),
            video_id: self.video_id,
            is_free_preview: self.is_free_preview,
        };
        if changes.title.is_none()
            && changes.description.is_none()
            && changes.video_id.is_none()
            && changes.is_free_preview.is_none()
        {
            errors.push(FieldError::new(
                "body",
                "Provide at least one field to update.",
            ));
        }
        if !errors.is_empty() {
            return Err(ApiError::validation(errors));
        }
        Ok(changes)
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LessonVideoStatus {
    Uploading,
    Processing,
    Ready,
    Failed,
    Unavailable,
}

impl LessonVideoStatus {
    fn from_lesson(lesson: &LessonRow) -> Self {
        if lesson.video_deleted_at.is_some() {
            return Self::Unavailable;
        }
        match lesson.video_status {
            VideoStatus::Uploading => Self::Uploading,
            VideoStatus::Processing => Self::Processing,
            VideoStatus::Ready => Self::Ready,
            VideoStatus::Failed => Self::Failed,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct LessonResponse {
    pub id: Uuid,
    pub course_id: Uuid,
    pub title: String,
    pub description: String,
    pub position: i32,
    pub video_id: Uuid,
    pub is_free_preview: bool,
    pub video_status: LessonVideoStatus,
    pub duration_seconds: Option<i32>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl From<&LessonRow> for LessonResponse {
    fn from(lesson: &LessonRow) -> Self {
        Self {
            id: lesson.id,
            course_id: lesson.course_id,
            title: lesson.title.clone(),
            description: lesson.description.clone(),
            position: lesson.position,
            video_id: lesson.video_id,
            is_free_preview: lesson.is_free_preview,
            video_status: LessonVideoStatus::from_lesson(lesson),
            duration_seconds: lesson.duration_seconds,
            created_at: lesson.created_at,
            updated_at: lesson.updated_at,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct LessonEnvelope {
    pub lesson: LessonResponse,
    pub course_version: i32,
}

#[derive(Debug, Serialize)]
pub struct LessonListEnvelope {
    pub lessons: Vec<LessonResponse>,
    pub total_duration_seconds: i64,
    pub course_version: i32,
}

#[derive(Debug, Serialize)]
pub struct LessonOrderEnvelope {
    pub lessons: Vec<LessonResponse>,
    pub course_version: i32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validation_normalises_text_and_rejects_empty_updates() {
        let lesson = CreateLessonRequest {
            title: "  Intro  ".into(),
            description: "  Hello  ".into(),
            video_id: Uuid::new_v4(),
            is_free_preview: true,
            position: Some(1),
        }
        .validate()
        .unwrap();
        assert_eq!(lesson.title, "Intro");
        assert_eq!(lesson.description, "Hello");

        assert!(UpdateLessonRequest {
            title: None,
            description: None,
            video_id: None,
            is_free_preview: None,
        }
        .validate()
        .is_err());
    }
}
