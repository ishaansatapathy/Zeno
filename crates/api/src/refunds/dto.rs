use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use unicode_normalization::UnicodeNormalization;
use uuid::Uuid;
use zeno_core::model::RefundStatus;

use crate::error::{ApiError, FieldError};
use crate::refunds::repo::{RefundRow, StaffRefundRow};

const MAX_NOTE_CHARS: usize = 300;

#[derive(Debug, Serialize)]
pub struct RefundResponse {
    pub id: Uuid,
    pub order_id: Uuid,
    pub status: RefundStatus,
    pub amount_paise: i64,
    /// Who set it going: `student`, `creator`, `admin`, or `provider` (made in the provider's
    /// dashboard).
    pub origin: String,
    pub reason: Option<String>,
    /// Why it was declined, or why the provider could not make it.
    pub decision_note: Option<String>,
    pub requested_at: DateTime<Utc>,
    pub decided_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
}

impl From<&RefundRow> for RefundResponse {
    fn from(row: &RefundRow) -> Self {
        Self {
            id: row.id,
            order_id: row.order_id,
            status: row.status,
            amount_paise: row.amount_paise,
            origin: row.origin.clone(),
            reason: row.reason.clone(),
            decision_note: row.decision_note.clone(),
            requested_at: row.requested_at,
            decided_at: row.decided_at,
            completed_at: row.completed_at,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct RefundEnvelope {
    pub refund: RefundResponse,
}

/// A refund as the course's creator sees it: with whose order it is.
#[derive(Debug, Serialize)]
pub struct StaffRefundResponse {
    #[serde(flatten)]
    pub refund: RefundResponse,
    pub buyer_id: Uuid,
    pub buyer_name: String,
}

impl From<&StaffRefundRow> for StaffRefundResponse {
    fn from(row: &StaffRefundRow) -> Self {
        Self {
            refund: RefundResponse::from(&row.refund),
            buyer_id: row.buyer_id,
            buyer_name: row.buyer_name.clone(),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct RefundListEnvelope {
    pub refunds: Vec<StaffRefundResponse>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct ReasonRequest {
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct ListQuery {
    pub status: Option<String>,
    pub limit: Option<u32>,
}

impl ReasonRequest {
    /// A note of 1 to 300 characters, or nothing. `required` turns nothing into an error.
    pub fn note(self, required: bool) -> Result<Option<String>, ApiError> {
        let invalid =
            |message: &str| ApiError::validation(vec![FieldError::new("reason", message)]);
        match self.reason {
            None if required => Err(invalid("Say why.")),
            None => Ok(None),
            Some(raw) => {
                let text: String = raw.trim().nfc().collect();
                let chars = text.chars().count();
                if chars == 0 {
                    return if required {
                        Err(invalid("Say why."))
                    } else {
                        Ok(None)
                    };
                }
                if chars > MAX_NOTE_CHARS || text.chars().any(char::is_control) {
                    return Err(invalid(
                        "Use up to 300 characters, without control characters.",
                    ));
                }
                Ok(Some(text))
            }
        }
    }
}

pub fn parse_status(raw: &str) -> Result<RefundStatus, ApiError> {
    match raw {
        "requested" => Ok(RefundStatus::Requested),
        "declined" => Ok(RefundStatus::Declined),
        "processing" => Ok(RefundStatus::Processing),
        "succeeded" => Ok(RefundStatus::Succeeded),
        "failed" => Ok(RefundStatus::Failed),
        _ => Err(ApiError::validation(vec![FieldError::new(
            "status",
            "Use requested, declined, processing, succeeded or failed.",
        )])),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(reason: Option<&str>, required: bool) -> Result<Option<String>, ApiError> {
        ReasonRequest {
            reason: reason.map(str::to_owned),
        }
        .note(required)
    }

    #[test]
    fn notes_are_trimmed_and_bounded() {
        assert_eq!(note(None, false).unwrap(), None);
        assert_eq!(note(Some("   "), false).unwrap(), None);
        assert_eq!(
            note(Some("  too slow  "), true).unwrap().as_deref(),
            Some("too slow")
        );
        assert!(note(None, true).is_err());
        assert!(note(Some(" "), true).is_err());
        assert!(note(Some(&"x".repeat(300)), true).is_ok());
        assert!(note(Some(&"x".repeat(301)), true).is_err());
        assert!(note(Some("line\nbreak"), true).is_err());
    }

    #[test]
    fn statuses_parse_exactly() {
        assert_eq!(
            parse_status("processing").unwrap(),
            RefundStatus::Processing
        );
        for bad in ["", "Processing", "done", "processing "] {
            assert!(parse_status(bad).is_err(), "{bad:?}");
        }
    }
}
