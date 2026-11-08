use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use zeno_core::model::{CourseStatus, EnrollmentSource};

use crate::auth::validation;
use crate::enrollments::repo::{EnrollmentRow, MyCourseRow, StudentRow};
use crate::error::{ApiError, FieldError};

#[derive(Debug)]
pub enum Contact {
    Email(String),
    Phone(String),
    UserId(Uuid),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GrantRequest {
    pub user: String,
    pub expires_at: Option<DateTime<Utc>>,
}

#[derive(Debug)]
pub struct GrantInput {
    pub contact: Contact,
    pub expires_at: Option<DateTime<Utc>>,
}

fn phone(raw: &str) -> Option<String> {
    let value = raw.trim();
    let digits = value.strip_prefix('+')?;
    ((8..=15).contains(&digits.len()) && digits.chars().all(|ch| ch.is_ascii_digit()))
        .then(|| value.to_owned())
}

impl GrantRequest {
    pub fn validate(self) -> Result<GrantInput, ApiError> {
        let raw = self.user.trim();
        let contact = if let Ok(id) = Uuid::parse_str(raw) {
            Contact::UserId(id)
        } else if raw.contains('@') {
            Contact::Email(
                validation::email(raw).map_err(|error| ApiError::validation(vec![error]))?,
            )
        } else if let Some(phone) = phone(raw) {
            Contact::Phone(phone)
        } else {
            return Err(ApiError::validation(vec![FieldError::new(
                "user",
                "Use a user ID, valid email, or E.164 phone number.",
            )]));
        };
        if self
            .expires_at
            .is_some_and(|expires_at| expires_at <= Utc::now())
        {
            return Err(ApiError::validation(vec![FieldError::new(
                "expires_at",
                "Expiration must be in the future.",
            )]));
        }
        Ok(GrantInput {
            contact,
            expires_at: self.expires_at,
        })
    }
}

#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct RevokeQuery {
    pub reason: Option<String>,
}

impl RevokeQuery {
    pub fn validate(self) -> Result<String, ApiError> {
        let reason = self
            .reason
            .unwrap_or_else(|| "Revoked by course owner.".into())
            .trim()
            .to_owned();
        if reason.is_empty() || reason.chars().count() > 300 || reason.chars().any(char::is_control)
        {
            return Err(ApiError::validation(vec![FieldError::new(
                "reason",
                "Reason must contain 1 to 300 printable characters.",
            )]));
        }
        Ok(reason)
    }
}

#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct ListQuery {
    pub cursor: Option<String>,
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Copy)]
pub struct EnrollmentCursor {
    pub created_at: DateTime<Utc>,
    pub id: Uuid,
}

impl EnrollmentCursor {
    pub fn encode(self) -> String {
        URL_SAFE_NO_PAD.encode(format!("{}|{}", self.created_at.to_rfc3339(), self.id))
    }

    pub fn decode(raw: &str) -> Result<Self, ApiError> {
        let invalid =
            || ApiError::bad_request("invalid_cursor", "The pagination cursor is invalid.");
        let bytes = URL_SAFE_NO_PAD.decode(raw).map_err(|_| invalid())?;
        let text = String::from_utf8(bytes).map_err(|_| invalid())?;
        let (created_at, id) = text.split_once('|').ok_or_else(invalid)?;
        Ok(Self {
            created_at: DateTime::parse_from_rfc3339(created_at)
                .map_err(|_| invalid())?
                .with_timezone(&Utc),
            id: Uuid::parse_str(id).map_err(|_| invalid())?,
        })
    }
}

#[derive(Debug, Serialize)]
pub struct EnrollmentResponse {
    pub id: Uuid,
    pub user_id: Uuid,
    pub course_id: Uuid,
    pub source: EnrollmentSource,
    pub granted_by: Option<Uuid>,
    pub active: bool,
    pub expires_at: Option<DateTime<Utc>>,
    pub revoked_at: Option<DateTime<Utc>>,
    pub revoked_by: Option<Uuid>,
    pub revocation_kind: Option<String>,
    pub revoke_reason: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl From<&EnrollmentRow> for EnrollmentResponse {
    fn from(row: &EnrollmentRow) -> Self {
        Self {
            id: row.id,
            user_id: row.user_id,
            course_id: row.course_id,
            source: row.source,
            granted_by: row.granted_by,
            active: row.revoked_at.is_none()
                && row
                    .expires_at
                    .is_none_or(|expires_at| expires_at > Utc::now()),
            expires_at: row.expires_at,
            revoked_at: row.revoked_at,
            revoked_by: row.revoked_by,
            revocation_kind: row.revocation_kind.clone(),
            revoke_reason: row.revoke_reason.clone(),
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct EnrollmentEnvelope {
    pub enrollment: EnrollmentResponse,
}

#[derive(Debug, Serialize)]
pub struct GrantResponse {
    pub status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enrollment: Option<EnrollmentResponse>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub invited_email: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct MyCourseResponse {
    pub enrollment: EnrollmentResponse,
    pub slug: String,
    pub title: String,
    pub status: CourseStatus,
    pub price_paise: i64,
}

impl From<&MyCourseRow> for MyCourseResponse {
    fn from(row: &MyCourseRow) -> Self {
        Self {
            enrollment: EnrollmentResponse::from(&row.enrollment),
            slug: row.slug.clone(),
            title: row.title.clone(),
            status: row.status,
            price_paise: row.price_paise,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct MyCoursesEnvelope {
    pub courses: Vec<MyCourseResponse>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct StudentResponse {
    pub enrollment: EnrollmentResponse,
    pub name: String,
    pub email: Option<String>,
    pub phone: Option<String>,
}

impl From<&StudentRow> for StudentResponse {
    fn from(row: &StudentRow) -> Self {
        Self {
            enrollment: EnrollmentResponse::from(&row.enrollment),
            name: row.name.clone(),
            email: row.email.clone(),
            phone: row.phone.clone(),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct StudentsEnvelope {
    pub students: Vec<StudentResponse>,
    pub next_cursor: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grants_accept_ids_emails_and_strict_e164_phones() {
        for user in [
            Uuid::new_v4().to_string(),
            "Student@X.IN".into(),
            "+919876543210".into(),
        ] {
            assert!(GrantRequest {
                user,
                expires_at: None
            }
            .validate()
            .is_ok());
        }
        for user in ["9876543210", "+12-34", "not a contact"] {
            assert!(GrantRequest {
                user: user.into(),
                expires_at: None
            }
            .validate()
            .is_err());
        }
    }
}
