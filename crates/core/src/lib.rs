//! Shared types, config and error handling used by every Zeno service.

pub mod config {
    #[derive(Debug, Clone)]
    pub struct Config {
        pub database_url: String,
        pub redis_url: String,
        pub bind_addr: String,
    }

    impl Config {
        pub fn from_env() -> Self {
            Self {
                database_url: std::env::var("DATABASE_URL")
                    .unwrap_or_else(|_| "postgres://zeno:zeno@localhost:5432/zeno".into()),
                redis_url: std::env::var("REDIS_URL")
                    .unwrap_or_else(|_| "redis://localhost:6379".into()),
                bind_addr: std::env::var("BIND_ADDR").unwrap_or_else(|_| "0.0.0.0:8080".into()),
            }
        }
    }
}

pub mod error {
    use thiserror::Error;

    #[derive(Debug, Error)]
    pub enum AppError {
        #[error("not found")]
        NotFound,
        #[error("unauthorized")]
        Unauthorized,
        #[error("database error: {0}")]
        Database(#[from] sqlx::Error),
        #[error("internal error: {0}")]
        Internal(String),
    }
}

/// Domain types, mirroring the eight-table data model: User, Course, Lesson,
/// Video, LiveSession, Enrollment, Payment, PlaybackSession.
pub mod model {
    use chrono::{DateTime, Utc};
    use serde::{Deserialize, Serialize};
    use uuid::Uuid;

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum Role {
        Student,
        Creator,
        Admin,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct User {
        pub id: Uuid,
        pub name: String,
        pub phone: Option<String>,
        pub email: Option<String>,
        #[serde(skip_serializing)]
        pub password_hash: String,
        pub role: Role,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum CourseStatus {
        Draft,
        Published,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct Course {
        pub id: Uuid,
        pub creator_id: Uuid,
        pub title: String,
        pub description: String,
        pub price_paise: i64,
        pub status: CourseStatus,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct Lesson {
        pub id: Uuid,
        pub course_id: Uuid,
        pub position: i32,
        pub video_id: Option<Uuid>,
        pub live_session_id: Option<Uuid>,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum VideoStatus {
        Uploading,
        Processing,
        Ready,
        Failed,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct Video {
        pub id: Uuid,
        pub status: VideoStatus,
        pub duration_seconds: Option<i32>,
        pub renditions: Vec<String>,
        pub key_ref: Option<String>,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum LiveState {
        Scheduled,
        Live,
        Ended,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct LiveSession {
        pub id: Uuid,
        pub course_id: Uuid,
        #[serde(skip_serializing)]
        pub stream_key_hash: String,
        pub scheduled_at: DateTime<Utc>,
        pub state: LiveState,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct Enrollment {
        pub id: Uuid,
        pub user_id: Uuid,
        pub course_id: Uuid,
        pub payment_id: Uuid,
        pub expires_at: Option<DateTime<Utc>>,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum PaymentStatus {
        Pending,
        Confirmed,
        Failed,
        Refunded,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct Payment {
        pub id: Uuid,
        pub provider_ref: String,
        pub amount_paise: i64,
        pub status: PaymentStatus,
        pub raw_event: serde_json::Value,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct PlaybackSession {
        pub id: Uuid,
        pub user_id: Uuid,
        pub video_id: Uuid,
        pub device: String,
        pub ip: String,
        pub started_at: DateTime<Utc>,
        pub last_position_seconds: i32,
    }
}
