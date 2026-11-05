use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use zeno_core::model::Role;

use crate::auth::repo::UserRow;
use crate::auth::service::RegisterInput;
use crate::auth::validation;
use crate::error::{ApiError, FieldError};
use crate::secret::Secret;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SignupRole {
    Student,
    Creator,
}

impl From<SignupRole> for Role {
    fn from(role: SignupRole) -> Self {
        match role {
            SignupRole::Student => Role::Student,
            SignupRole::Creator => Role::Creator,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegisterRequest {
    pub name: String,
    pub email: String,
    pub password: Secret,
    #[serde(default)]
    pub role: Option<SignupRole>,
}

impl RegisterRequest {
    pub fn validate(self) -> Result<RegisterInput, ApiError> {
        let mut errors: Vec<FieldError> = Vec::new();
        let name = validation::name(&self.name)
            .map_err(|e| errors.push(e))
            .ok();
        let email = validation::email(&self.email)
            .map_err(|e| errors.push(e))
            .ok();
        if let Err(e) = validation::password(self.password.expose(), "password") {
            errors.push(e);
        } else if let Some(email) = &email {
            if let Err(e) =
                validation::password_not_from_email(self.password.expose(), email, "password")
            {
                errors.push(e);
            }
        }
        match (name, email, errors.is_empty()) {
            (Some(name), Some(email), true) => Ok(RegisterInput {
                name,
                email,
                password: self.password,
                // Admins are never self-service; the default is the least privileged role.
                role: self.role.map_or(Role::Student, Role::from),
            }),
            _ => Err(ApiError::validation(errors)),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LoginRequest {
    pub email: String,
    pub password: Secret,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EmailRequest {
    pub email: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TokenRequest {
    pub token: Secret,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResetPasswordRequest {
    pub token: Secret,
    pub new_password: Secret,
}

#[derive(Debug, Serialize)]
pub struct MessageResponse {
    pub message: &'static str,
}

#[derive(Debug, Serialize)]
pub struct UserResponse {
    pub id: Uuid,
    pub name: String,
    pub email: Option<String>,
    pub role: Role,
    pub email_verified: bool,
}

impl From<&UserRow> for UserResponse {
    fn from(user: &UserRow) -> Self {
        Self {
            id: user.id,
            name: user.name.clone(),
            email: user.email.clone(),
            role: user.role,
            email_verified: user.email_verified_at.is_some(),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct ActivityItem {
    pub kind: String,
    pub at: DateTime<Utc>,
    pub ip: Option<String>,
    pub user_agent: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ActivityResponse {
    pub events: Vec<ActivityItem>,
}

#[derive(Debug, Serialize)]
pub struct UserEnvelope {
    pub user: UserResponse,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(role: Option<SignupRole>) -> RegisterRequest {
        RegisterRequest {
            name: "Asha Rao".into(),
            email: "Asha@Example.com".into(),
            password: Secret::new("a-long-enough-password"),
            role,
        }
    }

    #[test]
    fn valid_registration_is_normalised_and_defaults_to_student() {
        let input = request(None).validate().unwrap();
        assert_eq!(input.email, "asha@example.com");
        assert_eq!(input.role, Role::Student);
        assert_eq!(
            request(Some(SignupRole::Creator)).validate().unwrap().role,
            Role::Creator
        );
    }

    #[test]
    fn every_invalid_field_is_reported_at_once() {
        let bad = RegisterRequest {
            name: "x".into(),
            email: "nope".into(),
            password: Secret::new("short"),
            role: None,
        };
        match bad.validate() {
            Err(ApiError::Validation(fields)) => {
                let names: Vec<_> = fields.iter().map(|f| f.field).collect();
                assert_eq!(names, ["name", "email", "password"]);
            }
            other => panic!("expected validation error, got {other:?}"),
        }
    }

    #[test]
    fn admin_role_cannot_be_requested() {
        let parsed: Result<RegisterRequest, _> = serde_json::from_str(
            r#"{"name":"Eve","email":"eve@x.com","password":"a-long-enough-password","role":"admin"}"#,
        );
        assert!(parsed.is_err());
    }

    #[test]
    fn unknown_fields_are_rejected() {
        let parsed: Result<RegisterRequest, _> = serde_json::from_str(
            r#"{"name":"Eve","email":"eve@x.com","password":"a-long-enough-password","is_admin":true}"#,
        );
        assert!(parsed.is_err());
    }
}
