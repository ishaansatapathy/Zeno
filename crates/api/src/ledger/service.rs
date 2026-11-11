use axum::http::StatusCode;
use chrono::{DateTime, Duration, NaiveDate, TimeZone, Utc};
use uuid::Uuid;
use zeno_core::model::Role;

use crate::auth::extractor::AuthUser;
use crate::error::{ApiError, FieldError};
use crate::ledger::dto::{
    AdjustmentEnvelope, EarningsResponse, EntryCursor, EntryResponse, Lifetime, NewAdjustment,
};
use crate::ledger::fee;
use crate::ledger::integrity::{self, Problem};
use crate::ledger::post::{self, Sale};
use crate::ledger::{csv, repo};
use crate::payouts::dto::PayoutResponse;
use crate::payouts::repo as payouts;
use crate::state::AppState;

const EARNERS: &[Role] = &[Role::Creator, Role::Admin];
/// Indian time, the clock invoices and financial years are counted by.
const IST_MINUTES: i64 = 330;
const MAX_EXPORT_DAYS: i64 = 366;
const MAX_EXPORT_ROWS: i64 = 50_000;
const DEFAULT_EXPORT_DAYS: i64 = 90;
const BACKFILL_BATCH: i64 = 200;

// ---- what a creator sees -------------------------------------------------------------------------

pub async fn earnings(state: &AppState, user: &AuthUser) -> Result<EarningsResponse, ApiError> {
    user.require_role(EARNERS)?;
    let position = repo::position(&state.db, user.id).await?;
    let by_kind = repo::totals_by_kind(&state.db, user.id).await?;
    let of = |kind: &str| by_kind.get(kind).copied().unwrap_or(0);
    let open_payout = payouts::open_for_creator(&state.db, user.id).await?;
    Ok(EarningsResponse {
        currency: "INR",
        balance_paise: position.total_paise,
        pending_paise: position.pending_paise,
        available_paise: position.available_paise(),
        minimum_payout_paise: state.config.ledger.payout_minimum_paise,
        lifetime: Lifetime {
            sales_paise: of("sale"),
            gateway_fees_paise: -(of("gateway_fee") + of("gateway_fee_correction")),
            refunds_paise: -of("refund"),
            adjustments_paise: of("adjustment"),
            paid_out_paise: -(of("payout") + of("payout_reversal")),
        },
        open_payout: open_payout.as_ref().map(PayoutResponse::from),
    })
}

pub async fn entries(
    state: &AppState,
    user: &AuthUser,
    cursor: Option<EntryCursor>,
    limit: Option<u32>,
) -> Result<(Vec<EntryResponse>, Option<String>), ApiError> {
    user.require_role(EARNERS)?;
    let limit = i64::from(limit.unwrap_or(25).clamp(1, 100));
    let mut rows = repo::entries(
        &state.db,
        user.id,
        cursor.map(|c| (c.created_at, c.id)),
        limit + 1,
    )
    .await?;
    let more = rows.len() as i64 > limit;
    if more {
        rows.pop();
    }
    let next = more.then(|| rows.last()).flatten().map(|last| {
        EntryCursor {
            created_at: last.created_at,
            id: last.id,
        }
        .encode()
    });
    Ok((rows.iter().map(EntryResponse::from).collect(), next))
}

fn parse_day(field: &'static str, raw: &str) -> Result<NaiveDate, ApiError> {
    NaiveDate::parse_from_str(raw, "%Y-%m-%d")
        .map_err(|_| ApiError::validation(vec![FieldError::new(field, "A date like 2026-11-30.")]))
}

/// The start of an Indian calendar day, as a moment in UTC.
fn ist_day_start(day: NaiveDate) -> DateTime<Utc> {
    Utc.from_utc_datetime(&day.and_hms_opt(0, 0, 0).unwrap_or_default())
        - Duration::minutes(IST_MINUTES)
}

/// The creator's entries over a range of Indian days (both ends included) as CSV. Defaults to the
/// last 90 days. The range and the row count are capped so an export cannot swallow the server.
pub async fn export_csv(
    state: &AppState,
    user: &AuthUser,
    from: Option<&str>,
    to: Option<&str>,
) -> Result<(String, String), ApiError> {
    user.require_role(EARNERS)?;
    let today = (Utc::now() + Duration::minutes(IST_MINUTES)).date_naive();
    let last = match to {
        Some(raw) => parse_day("to", raw)?,
        None => today,
    };
    let first = match from {
        Some(raw) => parse_day("from", raw)?,
        None => last - Duration::days(DEFAULT_EXPORT_DAYS - 1),
    };
    if first > last {
        return Err(ApiError::validation(vec![FieldError::new(
            "from",
            "Must not be after the end date.",
        )]));
    }
    if (last - first).num_days() + 1 > MAX_EXPORT_DAYS {
        return Err(ApiError::validation(vec![FieldError::new(
            "from",
            "At most 366 days at a time.",
        )]));
    }
    let rows = repo::entries_between(
        &state.db,
        user.id,
        ist_day_start(first),
        ist_day_start(last + Duration::days(1)),
        MAX_EXPORT_ROWS + 1,
    )
    .await?;
    if rows.len() as i64 > MAX_EXPORT_ROWS {
        return Err(ApiError::Http {
            status: StatusCode::UNPROCESSABLE_ENTITY,
            code: "export_too_large",
            message: "That range has too many entries. Choose a shorter one.".into(),
        });
    }
    let mut body = String::with_capacity(64 + rows.len() * 160);
    body.push_str(csv::HEADER);
    body.push('\n');
    for row in &rows {
        body.push_str(&csv::row(row));
    }
    Ok((format!("earnings-{first}-to-{last}.csv"), body))
}

// ---- an admin ------------------------------------------------------------------------------------

/// Adds or removes money from what a creator is owed, with a reason on record. For the cases the
/// automatic postings cannot cover (a chargeback lost, a goodwill credit). The book itself is never
/// edited: this is a new journal.
pub async fn adjust(
    state: &AppState,
    admin: &AuthUser,
    new: NewAdjustment,
) -> Result<AdjustmentEnvelope, ApiError> {
    admin.require_role(&[Role::Admin])?;
    if !repo::is_creator(&state.db, new.creator_id).await? {
        return Err(ApiError::Http {
            status: StatusCode::NOT_FOUND,
            code: "not_found",
            message: "Creator not found.".into(),
        });
    }
    let mut tx = state.db.begin().await?;
    let id = repo::insert_adjustment(
        &mut tx,
        new.creator_id,
        new.amount_paise,
        &new.reason,
        admin.id,
    )
    .await?;
    post::adjustment(&mut tx, id, new.creator_id, new.amount_paise, &new.reason).await?;
    tx.commit().await?;
    Ok(AdjustmentEnvelope {
        adjustment_id: id,
        creator_id: new.creator_id,
        amount_paise: new.amount_paise,
    })
}

pub async fn integrity(state: &AppState, admin: &AuthUser) -> Result<Vec<Problem>, ApiError> {
    admin.require_role(&[Role::Admin])?;
    Ok(integrity::check(&state.db).await?)
}

/// Posts what the book is missing for sales and refunds that happened before it existed (or that
/// an earlier bug skipped). Safe to run again: everything posted is keyed by the event it records.
pub async fn backfill(state: &AppState, admin: &AuthUser) -> Result<(u32, u32), ApiError> {
    admin.require_role(&[Role::Admin])?;
    let mut sales = 0u32;
    loop {
        #[derive(sqlx::FromRow)]
        struct Missing {
            payment_id: Uuid,
            order_id: Uuid,
            creator_id: Option<String>,
            amount_paise: i64,
            fee_paise: Option<i64>,
        }
        let batch: Vec<Missing> = sqlx::query_as(
            "select p.id as payment_id, p.order_id, o.price_snapshot->>'creator_id' as creator_id, \
                    p.amount_paise, p.fee_paise \
             from payments p join orders o on o.id = p.order_id \
             where exists (select 1 from order_events oe where oe.payment_id = p.id and oe.kind = 'paid') \
               and not exists (select 1 from ledger_journals j where j.kind = 'sale' and j.source_id = p.id) \
             order by p.created_at, p.id limit $1",
        )
        .bind(BACKFILL_BATCH)
        .fetch_all(&state.db)
        .await?;
        if batch.is_empty() {
            break;
        }
        let mut tx = state.db.begin().await?;
        for missing in batch {
            let creator_id = missing
                .creator_id
                .as_deref()
                .and_then(|id| Uuid::parse_str(id).ok())
                .ok_or_else(|| {
                    anyhow::anyhow!("order {} has no creator in its snapshot", missing.order_id)
                })?;
            let posted = post::sale(
                &mut tx,
                &Sale {
                    payment_id: missing.payment_id,
                    order_id: missing.order_id,
                    creator_id,
                    gross_paise: missing.amount_paise,
                    fee: fee::resolve(
                        missing.amount_paise,
                        missing.fee_paise,
                        state.config.ledger.fallback_fee_bps,
                    ),
                },
            )
            .await?;
            sales += u32::from(posted);
        }
        tx.commit().await?;
    }

    let mut refunds = 0u32;
    loop {
        let batch: Vec<(Uuid, Uuid, i64)> = sqlx::query_as(
            "select r.id, r.payment_id, r.amount_paise from refunds r \
             join ledger_journals sj on sj.kind = 'sale' and sj.source_id = r.payment_id \
             where r.status = 'succeeded' \
               and not exists (select 1 from ledger_journals j where j.kind = 'refund' and j.source_id = r.id) \
             order by r.completed_at, r.id limit $1",
        )
        .bind(BACKFILL_BATCH)
        .fetch_all(&state.db)
        .await?;
        if batch.is_empty() {
            break;
        }
        let mut tx = state.db.begin().await?;
        for (refund_id, payment_id, amount) in batch {
            refunds += u32::from(post::refund(&mut tx, payment_id, refund_id, amount).await?);
        }
        tx.commit().await?;
    }
    Ok((sales, refunds))
}
