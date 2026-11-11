use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use unicode_normalization::UnicodeNormalization;
use uuid::Uuid;
use zeno_core::model::PayoutStatus;

use crate::error::{ApiError, FieldError};
use crate::payouts::repo::{AdminPayoutRow, PayoutRow};

#[derive(Debug, Serialize)]
pub struct PayoutResponse {
    pub id: Uuid,
    pub amount_paise: i64,
    pub status: PayoutStatus,
    pub requested_at: DateTime<Utc>,
    pub method: Option<String>,
    /// The bank's reference for the transfer, once it has been made.
    pub reference: Option<String>,
    pub paid_at: Option<DateTime<Utc>>,
    pub cancelled_at: Option<DateTime<Utc>>,
    pub cancel_reason: Option<String>,
}

impl From<&PayoutRow> for PayoutResponse {
    fn from(row: &PayoutRow) -> Self {
        Self {
            id: row.id,
            amount_paise: row.amount_paise,
            status: row.status,
            requested_at: row.requested_at,
            method: row.method.clone(),
            reference: row.reference.clone(),
            paid_at: row.paid_at,
            cancelled_at: row.cancelled_at,
            cancel_reason: row.cancel_reason.clone(),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct PayoutEnvelope {
    pub payout: PayoutResponse,
}

#[derive(Debug, Serialize)]
pub struct PayoutListEnvelope {
    pub payouts: Vec<PayoutResponse>,
}

#[derive(Debug, Serialize)]
pub struct AdminPayoutResponse {
    #[serde(flatten)]
    pub payout: PayoutResponse,
    pub creator_id: Uuid,
    pub creator_name: String,
    pub creator_email: Option<String>,
}

impl From<&AdminPayoutRow> for AdminPayoutResponse {
    fn from(row: &AdminPayoutRow) -> Self {
        Self {
            payout: PayoutResponse::from(&row.payout),
            creator_id: row.payout.creator_id,
            creator_name: row.creator_name.clone(),
            creator_email: row.creator_email.clone(),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct AdminPayoutListEnvelope {
    pub payouts: Vec<AdminPayoutResponse>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequestPayout {
    pub amount_paise: i64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PayRequest {
    pub method: String,
    pub reference: String,
}

pub struct Payment {
    pub method: &'static str,
    pub reference: String,
}

impl PayRequest {
    pub fn validate(self) -> Result<Payment, ApiError> {
        let mut errors = Vec::new();
        let method = match self.method.as_str() {
            "bank_transfer" => "bank_transfer",
            "upi" => "upi",
            "other" => "other",
            _ => {
                errors.push(FieldError::new(
                    "method",
                    "Use bank_transfer, upi or other.",
                ));
                "other"
            }
        };
        let reference: String = self.reference.trim().nfc().collect();
        let chars = reference.chars().count();
        if !(3..=100).contains(&chars) || reference.chars().any(char::is_control) {
            errors.push(FieldError::new(
                "reference",
                "The bank's reference for the transfer, 3 to 100 characters.",
            ));
        }
        if !errors.is_empty() {
            return Err(ApiError::validation(errors));
        }
        Ok(Payment { method, reference })
    }
}

#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct CancelRequest {
    #[serde(default)]
    pub reason: Option<String>,
}

impl CancelRequest {
    /// A reason of 1 to 300 characters, or none. `required` turns none into an error.
    pub fn reason(self, required: bool) -> Result<Option<String>, ApiError> {
        let invalid =
            |message: &str| ApiError::validation(vec![FieldError::new("reason", message)]);
        let text: String = self.reason.unwrap_or_default().trim().nfc().collect();
        if text.is_empty() {
            return if required {
                Err(invalid("Say why."))
            } else {
                Ok(None)
            };
        }
        if text.chars().count() > 300 || text.chars().any(char::is_control) {
            return Err(invalid(
                "Use up to 300 characters, without control characters.",
            ));
        }
        Ok(Some(text))
    }
}

#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct ListQuery {
    pub status: Option<String>,
    pub limit: Option<u32>,
}

pub fn parse_status(raw: &str) -> Result<PayoutStatus, ApiError> {
    match raw {
        "requested" => Ok(PayoutStatus::Requested),
        "paid" => Ok(PayoutStatus::Paid),
        "cancelled" => Ok(PayoutStatus::Cancelled),
        _ => Err(ApiError::validation(vec![FieldError::new(
            "status",
            "Use requested, paid or cancelled.",
        )])),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_payment_needs_a_known_method_and_a_reference() {
        let ok = |method: &str, reference: &str| {
            PayRequest {
                method: method.into(),
                reference: reference.into(),
            }
            .validate()
        };
        assert!(ok("bank_transfer", "UTR 4021 9988 1").is_ok());
        assert!(ok("upi", "abc").is_ok());
        for (method, reference) in [
            ("cash", "UTR123"),
            ("upi", "ab"),
            ("upi", ""),
            ("upi", "line\nbreak"),
            ("upi", &"x".repeat(101)),
        ] {
            assert!(ok(method, reference).is_err(), "{method} {reference:?}");
        }
    }

    #[test]
    fn cancel_reasons_are_optional_for_creators_and_required_for_admins() {
        let reason = |text: Option<&str>, required| {
            CancelRequest {
                reason: text.map(str::to_owned),
            }
            .reason(required)
        };
        assert_eq!(reason(None, false).unwrap(), None);
        assert_eq!(
            reason(Some("  changed my mind "), false)
                .unwrap()
                .as_deref(),
            Some("changed my mind")
        );
        assert!(reason(None, true).is_err());
        assert!(reason(Some("  "), true).is_err());
        assert!(reason(Some(&"x".repeat(301)), false).is_err());
    }

    #[test]
    fn statuses_parse_exactly() {
        assert_eq!(parse_status("paid").unwrap(), PayoutStatus::Paid);
        assert!(parse_status("Paid").is_err());
        assert!(parse_status("").is_err());
    }
}
