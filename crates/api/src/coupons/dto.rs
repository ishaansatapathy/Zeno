use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use zeno_core::model::CouponKind;

use crate::coupons::repo::CouponRow;
use crate::error::{ApiError, FieldError};

const MIN_CODE: usize = 3;
const MAX_CODE: usize = 32;
const MAX_REDEMPTIONS: i32 = 1_000_000;
const MAX_PER_USER: i32 = 100;
pub const MAX_COUPONS_PER_CREATOR: i64 = 500;

/// The one form a code is stored and compared in: surrounding space dropped, ASCII upper-cased.
/// So "launch50", " Launch50 " and "LAUNCH50" are the same code, and a code with anything else in
/// it (inner spaces, symbols, non-ASCII look-alikes) is not a code at all.
pub fn normalize_code(raw: &str) -> Result<String, FieldError> {
    let code = raw.trim().to_ascii_uppercase();
    let allowed = |c: char| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_' || c == '-';
    let starts_well = code
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_uppercase() || c.is_ascii_digit());
    if (MIN_CODE..=MAX_CODE).contains(&code.len()) && starts_well && code.chars().all(allowed) {
        Ok(code)
    } else {
        Err(FieldError::new(
            "code",
            "Use 3 to 32 letters, digits, - or _, starting with a letter or digit.",
        ))
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateCouponRequest {
    pub code: String,
    pub kind: String,
    pub value: i64,
    #[serde(default)]
    pub course_id: Option<Uuid>,
    #[serde(default)]
    pub valid_from: Option<DateTime<Utc>>,
    #[serde(default)]
    pub valid_until: Option<DateTime<Utc>>,
    #[serde(default)]
    pub max_redemptions: Option<i32>,
    #[serde(default)]
    pub per_user_limit: Option<i32>,
}

#[derive(Debug)]
pub struct NewCoupon {
    pub code: String,
    pub kind: CouponKind,
    pub value: i64,
    pub course_id: Option<Uuid>,
    pub valid_from: Option<DateTime<Utc>>,
    pub valid_until: Option<DateTime<Utc>>,
    pub max_redemptions: Option<i32>,
    pub per_user_limit: i32,
}

impl CreateCouponRequest {
    pub fn validate(self) -> Result<NewCoupon, ApiError> {
        let mut errors = Vec::new();
        let code = match normalize_code(&self.code) {
            Ok(code) => code,
            Err(error) => {
                errors.push(error);
                String::new()
            }
        };
        let kind = match self.kind.as_str() {
            "percent" => CouponKind::Percent,
            "flat" => CouponKind::Flat,
            _ => {
                errors.push(FieldError::new("kind", "Use percent or flat."));
                CouponKind::Percent
            }
        };
        let value_ok = match kind {
            CouponKind::Percent => (1..=100).contains(&self.value),
            CouponKind::Flat => (100..=100_000_000).contains(&self.value),
        };
        if !value_ok && errors.iter().all(|e| e.field != "kind") {
            errors.push(FieldError::new(
                "value",
                match kind {
                    CouponKind::Percent => "A percent from 1 to 100.",
                    CouponKind::Flat => "An amount in paise from 100 (Rs 1) to 10,00,00,000.",
                },
            ));
        }
        if let (Some(from), Some(until)) = (self.valid_from, self.valid_until) {
            if until <= from {
                errors.push(FieldError::new("valid_until", "Must be after valid_from."));
            }
        }
        if let Some(until) = self.valid_until {
            if until <= Utc::now() {
                errors.push(FieldError::new("valid_until", "Must be in the future."));
            }
        }
        if self
            .max_redemptions
            .is_some_and(|max| !(1..=MAX_REDEMPTIONS).contains(&max))
        {
            errors.push(FieldError::new("max_redemptions", "From 1 to 1,000,000."));
        }
        let per_user_limit = self.per_user_limit.unwrap_or(1);
        if !(1..=MAX_PER_USER).contains(&per_user_limit) {
            errors.push(FieldError::new("per_user_limit", "From 1 to 100."));
        }
        if !errors.is_empty() {
            return Err(ApiError::validation(errors));
        }
        Ok(NewCoupon {
            code,
            kind,
            value: self.value,
            course_id: self.course_id,
            valid_from: self.valid_from,
            valid_until: self.valid_until,
            max_redemptions: self.max_redemptions,
            per_user_limit,
        })
    }
}

/// What a creator may change about a coupon that exists. The discount itself is fixed: a buyer
/// who was shown 50% must not find it has become 5%. Make a new coupon instead.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateCouponRequest {
    #[serde(default)]
    pub active: Option<bool>,
    #[serde(default)]
    pub valid_until: Option<DateTime<Utc>>,
    #[serde(default)]
    pub max_redemptions: Option<i32>,
}

#[derive(Debug)]
pub struct CouponChanges {
    pub active: Option<bool>,
    pub valid_until: Option<DateTime<Utc>>,
    pub max_redemptions: Option<i32>,
}

impl UpdateCouponRequest {
    pub fn validate(self) -> Result<CouponChanges, ApiError> {
        let mut errors = Vec::new();
        if self
            .max_redemptions
            .is_some_and(|max| !(1..=MAX_REDEMPTIONS).contains(&max))
        {
            errors.push(FieldError::new("max_redemptions", "From 1 to 1,000,000."));
        }
        if self.active.is_none() && self.valid_until.is_none() && self.max_redemptions.is_none() {
            errors.push(FieldError::new(
                "body",
                "Provide active, valid_until or max_redemptions.",
            ));
        }
        if !errors.is_empty() {
            return Err(ApiError::validation(errors));
        }
        Ok(CouponChanges {
            active: self.active,
            valid_until: self.valid_until,
            max_redemptions: self.max_redemptions,
        })
    }
}

#[derive(Debug, Serialize)]
pub struct CouponResponse {
    pub id: Uuid,
    pub code: String,
    pub kind: CouponKind,
    pub value: i64,
    pub course_id: Option<Uuid>,
    pub valid_from: Option<DateTime<Utc>>,
    pub valid_until: Option<DateTime<Utc>>,
    pub max_redemptions: Option<i32>,
    pub per_user_limit: i32,
    /// Redemptions in use (a redemption on an order that expired is given back).
    pub redeemed_count: i32,
    pub active: bool,
    pub created_at: DateTime<Utc>,
}

impl From<&CouponRow> for CouponResponse {
    fn from(row: &CouponRow) -> Self {
        Self {
            id: row.id,
            code: row.code.clone(),
            kind: row.kind,
            value: row.value,
            course_id: row.course_id,
            valid_from: row.valid_from,
            valid_until: row.valid_until,
            max_redemptions: row.max_redemptions,
            per_user_limit: row.per_user_limit,
            redeemed_count: row.redeemed_count,
            active: row.active,
            created_at: row.created_at,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct CouponEnvelope {
    pub coupon: CouponResponse,
}

#[derive(Debug, Serialize)]
pub struct CouponListEnvelope {
    pub coupons: Vec<CouponResponse>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct ListQuery {
    pub limit: Option<u32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_matched_without_regard_to_case_or_surrounding_space() {
        for raw in ["launch50", "  Launch50 ", "LAUNCH50", "\tlaunch50\n"] {
            assert_eq!(normalize_code(raw).unwrap(), "LAUNCH50", "{raw:?}");
        }
        assert_eq!(normalize_code("new-year_2027").unwrap(), "NEW-YEAR_2027");
    }

    #[test]
    fn anything_else_is_not_a_code() {
        let long = "A".repeat(33);
        for raw in [
            "",
            "ab",
            "has space",
            "inner  space",
            "-leading",
            "_leading",
            "pâtisserie",
            "ＬＡＵＮＣＨ",
            "semi;colon",
            "slash/er",
            "emoji🙂",
            &long,
        ] {
            assert!(normalize_code(raw).is_err(), "{raw:?}");
        }
        assert!(normalize_code(&"A".repeat(32)).is_ok());
        assert!(normalize_code("abc").is_ok());
    }

    fn create(kind: &str, value: i64) -> CreateCouponRequest {
        CreateCouponRequest {
            code: "LAUNCH50".into(),
            kind: kind.into(),
            value,
            course_id: None,
            valid_from: None,
            valid_until: None,
            max_redemptions: None,
            per_user_limit: None,
        }
    }

    #[test]
    fn values_are_checked_against_the_kind() {
        assert!(create("percent", 50).validate().is_ok());
        assert!(create("percent", 100).validate().is_ok());
        assert!(create("flat", 100).validate().is_ok());
        for (kind, value) in [
            ("percent", 0),
            ("percent", 101),
            ("flat", 99),
            ("flat", 0),
            ("flat", 100_000_001),
            ("bogus", 10),
        ] {
            assert!(create(kind, value).validate().is_err(), "{kind} {value}");
        }
    }

    #[test]
    fn limits_and_windows_are_checked() {
        let mut request = create("percent", 10);
        request.max_redemptions = Some(0);
        assert!(request.validate().is_err());
        let mut request = create("percent", 10);
        request.per_user_limit = Some(101);
        assert!(request.validate().is_err());
        let mut request = create("percent", 10);
        request.valid_from = Some(Utc::now() + chrono::Duration::days(2));
        request.valid_until = Some(Utc::now() + chrono::Duration::days(1));
        assert!(request.validate().is_err());
        let mut request = create("percent", 10);
        request.valid_until = Some(Utc::now() - chrono::Duration::days(1));
        assert!(request.validate().is_err(), "an offer that already ended");
    }
}
