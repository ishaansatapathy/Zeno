use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::{ApiError, FieldError};
use crate::ledger::integrity::Problem;
use crate::ledger::repo::EntryRow;
use crate::payouts::dto::PayoutResponse;

/// A creator's position, in paise. The identity
/// `balance = sales - gateway_fees - refunds + adjustments - paid_out` always holds.
#[derive(Debug, Serialize)]
pub struct EarningsResponse {
    pub currency: &'static str,
    /// What Zeno owes the creator now (negative: a refund came after a payout; sales pay it off).
    pub balance_paise: i64,
    /// Part of the balance still inside a refund window, so not yet payable.
    pub pending_paise: i64,
    /// What a payout may be requested for: the balance less what is pending, never below zero.
    pub available_paise: i64,
    pub minimum_payout_paise: i64,
    pub lifetime: Lifetime,
    /// The payout waiting to be paid, if there is one.
    pub open_payout: Option<PayoutResponse>,
}

#[derive(Debug, Serialize)]
pub struct Lifetime {
    /// Everything customers paid for the creator's courses.
    pub sales_paise: i64,
    /// What the payment provider kept.
    pub gateway_fees_paise: i64,
    /// What was returned to customers.
    pub refunds_paise: i64,
    /// Corrections made by an admin (signed).
    pub adjustments_paise: i64,
    /// Paid out, or reserved for a payout awaiting payment.
    pub paid_out_paise: i64,
}

#[derive(Debug, Serialize)]
pub struct EarningsEnvelope {
    pub earnings: EarningsResponse,
}

#[derive(Debug, Serialize)]
pub struct EntryResponse {
    pub id: Uuid,
    pub kind: String,
    pub amount_paise: i64,
    pub order_id: Option<Uuid>,
    pub course_title: Option<String>,
    pub note: Option<String>,
    pub created_at: DateTime<Utc>,
}

impl From<&EntryRow> for EntryResponse {
    fn from(row: &EntryRow) -> Self {
        Self {
            id: row.id,
            kind: row.kind.clone(),
            amount_paise: row.amount_paise,
            order_id: row.order_id,
            course_title: row.course_title.clone(),
            note: row.memo.clone(),
            created_at: row.created_at,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct EntriesEnvelope {
    pub entries: Vec<EntryResponse>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct EntriesQuery {
    pub cursor: Option<String>,
    pub limit: Option<u32>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct ExportQuery {
    pub from: Option<String>,
    pub to: Option<String>,
}

/// Where a page of entries ended.
#[derive(Debug, Clone, Copy)]
pub struct EntryCursor {
    pub created_at: DateTime<Utc>,
    pub id: Uuid,
}

impl EntryCursor {
    pub fn encode(self) -> String {
        URL_SAFE_NO_PAD.encode(format!("{}|{}", self.created_at.to_rfc3339(), self.id))
    }

    pub fn decode(raw: &str) -> Result<Self, ApiError> {
        let bad = || ApiError::bad_request("invalid_cursor", "The pagination cursor is invalid.");
        let bytes = URL_SAFE_NO_PAD.decode(raw).map_err(|_| bad())?;
        let text = String::from_utf8(bytes).map_err(|_| bad())?;
        let (at, id) = text.split_once('|').ok_or_else(bad)?;
        Ok(Self {
            created_at: DateTime::parse_from_rfc3339(at)
                .map_err(|_| bad())?
                .with_timezone(&Utc),
            id: Uuid::parse_str(id).map_err(|_| bad())?,
        })
    }
}

// ---- admin ---------------------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdjustmentRequest {
    pub creator_id: Uuid,
    /// Signed: positive adds to what the creator is owed, negative takes away.
    pub amount_paise: i64,
    pub reason: String,
}

pub const MAX_ADJUSTMENT_PAISE: i64 = 100_000_000;

pub struct NewAdjustment {
    pub creator_id: Uuid,
    pub amount_paise: i64,
    pub reason: String,
}

impl AdjustmentRequest {
    pub fn validate(self) -> Result<NewAdjustment, ApiError> {
        let mut errors = Vec::new();
        if self.amount_paise == 0 || self.amount_paise.unsigned_abs() > MAX_ADJUSTMENT_PAISE as u64
        {
            errors.push(FieldError::new(
                "amount_paise",
                "A non-zero amount of at most Rs 10,00,000 either way.",
            ));
        }
        let reason: String = self.reason.trim().to_owned();
        let chars = reason.chars().count();
        if !(3..=300).contains(&chars) || reason.chars().any(char::is_control) {
            errors.push(FieldError::new(
                "reason",
                "Say why, in 3 to 300 characters.",
            ));
        }
        if !errors.is_empty() {
            return Err(ApiError::validation(errors));
        }
        Ok(NewAdjustment {
            creator_id: self.creator_id,
            amount_paise: self.amount_paise,
            reason,
        })
    }
}

#[derive(Debug, Serialize)]
pub struct AdjustmentEnvelope {
    pub adjustment_id: Uuid,
    pub creator_id: Uuid,
    pub amount_paise: i64,
}

#[derive(Debug, Serialize)]
pub struct IntegrityResponse {
    pub ok: bool,
    pub problems: Vec<Problem>,
}

#[derive(Debug, Serialize)]
pub struct BackfillResponse {
    pub sales_posted: u32,
    pub refunds_posted: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursors_round_trip_and_garbage_is_refused() {
        let cursor = EntryCursor {
            created_at: Utc::now(),
            id: Uuid::new_v4(),
        };
        let back = EntryCursor::decode(&cursor.encode()).unwrap();
        assert_eq!(back.id, cursor.id);
        assert_eq!(
            back.created_at.timestamp_micros(),
            cursor.created_at.timestamp_micros()
        );
        for bad in [
            "",
            "not base64!",
            &URL_SAFE_NO_PAD.encode("nope"),
            &URL_SAFE_NO_PAD.encode("x|y"),
        ] {
            assert!(EntryCursor::decode(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn adjustments_need_an_amount_and_a_reason() {
        let ok = |amount, reason: &str| {
            AdjustmentRequest {
                creator_id: Uuid::nil(),
                amount_paise: amount,
                reason: reason.into(),
            }
            .validate()
        };
        assert!(ok(-500, "Chargeback lost on order X").is_ok());
        assert!(ok(100_000_000, "Goodwill").is_ok());
        for (amount, reason) in [
            (0, "fine"),
            (100_000_001, "fine"),
            (-100_000_001, "fine"),
            (i64::MIN, "fine"),
            (500, "no"),
            (500, "  "),
            (500, "multi\nline"),
        ] {
            assert!(ok(amount, reason).is_err(), "{amount} {reason:?}");
        }
    }
}
