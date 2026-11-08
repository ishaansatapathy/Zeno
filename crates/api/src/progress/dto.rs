use chrono::{DateTime, Utc};
use serde::Serialize;
use uuid::Uuid;

use crate::images;
use crate::progress::logic::percent;
use crate::progress::repo::{ContinueRow, CourseLessonProgress};
use crate::state::AppState;

#[derive(Debug, Serialize)]
pub struct ContinueLesson {
    pub id: Uuid,
    pub title: String,
    pub position: i32,
}

#[derive(Debug, Serialize)]
pub struct ContinueItem {
    pub course_slug: String,
    pub course_title: String,
    pub thumbnail_url: Option<String>,
    pub lesson: ContinueLesson,
    pub duration_seconds: Option<i32>,
    pub resume_from_seconds: i32,
    pub percent_watched: u8,
    pub last_watched_at: DateTime<Utc>,
}

impl ContinueItem {
    pub fn build(state: &AppState, row: &ContinueRow) -> Self {
        Self {
            course_slug: row.course_slug.clone(),
            course_title: row.course_title.clone(),
            thumbnail_url: row
                .thumbnail_key
                .as_deref()
                .map(|key| images::public_url(state, key)),
            lesson: ContinueLesson {
                id: row.lesson_id,
                title: row.lesson_title.clone(),
                position: row.lesson_position,
            },
            duration_seconds: row.duration_seconds,
            resume_from_seconds: row.last_position_seconds,
            percent_watched: percent(row.watched_seconds, row.duration_seconds),
            last_watched_at: row.last_watched_at,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct ContinueEnvelope {
    pub items: Vec<ContinueItem>,
}

#[derive(Debug, Serialize)]
pub struct LessonProgress {
    pub lesson_id: Uuid,
    pub position: i32,
    pub title: String,
    pub is_free_preview: bool,
    pub duration_seconds: Option<i32>,
    pub resume_from_seconds: i32,
    pub percent_watched: u8,
    pub completed: bool,
}

impl From<&CourseLessonProgress> for LessonProgress {
    fn from(row: &CourseLessonProgress) -> Self {
        Self {
            lesson_id: row.lesson_id,
            position: row.position,
            title: row.title.clone(),
            is_free_preview: row.is_free_preview,
            duration_seconds: row.duration_seconds,
            resume_from_seconds: row.last_position_seconds.unwrap_or(0),
            percent_watched: if row.completed {
                100
            } else {
                percent(row.watched_seconds.unwrap_or(0), row.duration_seconds)
            },
            completed: row.completed,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct CourseProgress {
    pub lessons_total: usize,
    pub lessons_completed: usize,
    pub percent_complete: u8,
    pub lessons: Vec<LessonProgress>,
}

impl CourseProgress {
    pub fn build(rows: &[CourseLessonProgress]) -> Self {
        let lessons: Vec<LessonProgress> = rows.iter().map(LessonProgress::from).collect();
        let completed = lessons.iter().filter(|lesson| lesson.completed).count();
        let percent_complete = if lessons.is_empty() {
            0
        } else {
            u8::try_from(completed * 100 / lessons.len()).unwrap_or(100)
        };
        Self {
            lessons_total: lessons.len(),
            lessons_completed: completed,
            percent_complete,
            lessons,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct CourseProgressEnvelope {
    pub progress: CourseProgress,
}
