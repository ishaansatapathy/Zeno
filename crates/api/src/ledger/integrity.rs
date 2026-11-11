//! The book checked against everything else, every night.
//!
//! The ledger is written in the same transactions as the things it records, so these checks should
//! always come back empty. They exist for the day they do not (a bug, a hand-run SQL statement, a
//! restored backup, a refund the provider should never have made) so that it is found by a query
//! and not by an angry creator. Every problem is a
//! row naming what is wrong and which record to look at; nothing here changes anything.

use std::time::Duration;

use serde::Serialize;
use sqlx::PgPool;

use crate::state::AppState;

const LIMIT: i64 = 50;
const EVERY: Duration = Duration::from_secs(24 * 60 * 60);

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Problem {
    /// What kind of inconsistency this is.
    pub kind: &'static str,
    /// The record to look at (a journal, payment, order, refund, payout or adjustment id).
    pub subject: String,
    pub detail: String,
}

fn problems(kind: &'static str, rows: Vec<(String, String)>) -> impl Iterator<Item = Problem> {
    rows.into_iter().map(move |(subject, detail)| Problem {
        kind,
        subject,
        detail,
    })
}

/// Runs every check. Empty means the book agrees with the rest of the system.
pub async fn check(db: &PgPool) -> Result<Vec<Problem>, sqlx::Error> {
    let mut found = Vec::new();

    // 1. A journal that does not add up to zero, or has no entries at all.
    found.extend(problems(
        "unbalanced_journal",
        sqlx::query_as(
            "select j.id::text, \
                    case when count(e.id) = 0 then 'has no entries' \
                         else 'off by ' || sum(e.amount_paise) || ' paise' end \
             from ledger_journals j left join ledger_entries e on e.journal_id = j.id \
             group by j.id \
             having count(e.id) = 0 or sum(e.amount_paise) <> 0 limit $1",
        )
        .bind(LIMIT)
        .fetch_all(db)
        .await?,
    ));

    // 2. A payment that paid for an order but never reached the book.
    found.extend(problems(
        "missing_sale",
        sqlx::query_as(
            "select p.id::text, 'the order was marked paid by this payment but it has no sale journal' \
             from payments p \
             where exists (select 1 from order_events oe where oe.payment_id = p.id and oe.kind = 'paid') \
               and not exists (select 1 from ledger_journals j where j.kind = 'sale' and j.source_id = p.id) \
             limit $1",
        )
        .bind(LIMIT)
        .fetch_all(db)
        .await?,
    ));

    // 3. A sale posted for a different amount than the payment.
    found.extend(problems(
        "sale_amount_mismatch",
        sqlx::query_as(
            "select p.id::text, 'the payment is ' || p.amount_paise || ' paise, the sale posted ' || e.amount_paise \
             from ledger_journals j \
             join payments p on p.id = j.source_id \
             join ledger_entries e on e.journal_id = j.id and e.creator_id is not null \
             where j.kind = 'sale' and e.amount_paise <> p.amount_paise limit $1",
        )
        .bind(LIMIT)
        .fetch_all(db)
        .await?,
    ));

    // 4. A fee the provider reported that the book does not carry.
    found.extend(problems(
        "fee_mismatch",
        sqlx::query_as(
            "select p.id::text, 'the provider reported ' || p.fee_paise || ' paise, the book charged ' || coalesce(-fe.amount_paise, 0) \
             from payments p \
             join ledger_journals sj on sj.kind = 'sale' and sj.source_id = p.id \
             left join ( \
                 select j.source_id, sum(e.amount_paise) as amount_paise from ledger_journals j \
                 join ledger_entries e on e.journal_id = j.id and e.creator_id is not null \
                 where j.kind in ('gateway_fee', 'gateway_fee_correction') \
                 group by j.source_id) fe on fe.source_id = p.id \
             where p.fee_paise is not null and p.fee_paise <= p.amount_paise \
               and p.fee_paise <> coalesce(-fe.amount_paise, 0) limit $1",
        )
        .bind(LIMIT)
        .fetch_all(db)
        .await?,
    ));

    // 5. What the creator is owed for an order, against what the payment, its fee and its refunds
    //    say they should be owed. The expected side is read from the payments and refunds tables,
    //    not from the ledger, so this catches both kinds of drift.
    found.extend(problems(
        "order_net_mismatch",
        sqlx::query_as(
            "select p.order_id::text, \
                    'the ledger owes ' || coalesce(c.net, 0) || ' paise, expected ' \
                    || (p.amount_paise - coalesce(f.fee, 0) - coalesce(r.refunded, 0)) \
             from payments p \
             join ledger_journals sj on sj.kind = 'sale' and sj.source_id = p.id \
             left join lateral ( \
                 select sum(e.amount_paise) as net from ledger_entries e \
                 join ledger_journals j on j.id = e.journal_id \
                 where j.order_id = p.order_id and e.creator_id is not null) c on true \
             left join lateral ( \
                 select -sum(e.amount_paise) as fee from ledger_entries e \
                 join ledger_journals j on j.id = e.journal_id \
                 where j.kind in ('gateway_fee', 'gateway_fee_correction') \
                   and j.source_id = p.id and e.creator_id is not null) f on true \
             left join lateral ( \
                 select sum(r.amount_paise) as refunded from refunds r \
                 where r.payment_id = p.id and r.status = 'succeeded') r on true \
             where coalesce(c.net, 0) <> p.amount_paise - coalesce(f.fee, 0) - coalesce(r.refunded, 0) \
             limit $1",
        )
        .bind(LIMIT)
        .fetch_all(db)
        .await?,
    ));

    // 6. Refunds and their journals agree both ways.
    found.extend(problems(
        "refund_without_refund",
        sqlx::query_as(
            "select j.source_id::text, 'a refund journal with no succeeded refund behind it' \
             from ledger_journals j where j.kind = 'refund' \
               and not exists (select 1 from refunds r where r.id = j.source_id and r.status = 'succeeded') \
             limit $1",
        )
        .bind(LIMIT)
        .fetch_all(db)
        .await?,
    ));
    found.extend(problems(
        "refund_not_posted",
        sqlx::query_as(
            "select r.id::text, 'a succeeded refund of a sold payment has no refund journal' \
             from refunds r \
             join ledger_journals sj on sj.kind = 'sale' and sj.source_id = r.payment_id \
             where r.status = 'succeeded' \
               and not exists (select 1 from ledger_journals j where j.kind = 'refund' and j.source_id = r.id) \
             limit $1",
        )
        .bind(LIMIT)
        .fetch_all(db)
        .await?,
    ));

    // 6b. A payment returned for more than it was. The database refuses to record it, so this
    //     is the check for the day that rule was bypassed or has not been applied to old rows.
    found.extend(problems(
        "refund_exceeds_payment",
        sqlx::query_as(
            "select p.id::text, 'the payment is ' || p.amount_paise || ' paise, succeeded refunds total ' || r.returned \
             from payments p \
             join (select payment_id, sum(amount_paise) as returned from refunds \
                   where status = 'succeeded' group by payment_id) r on r.payment_id = p.id \
             where r.returned > p.amount_paise limit $1",
        )
        .bind(LIMIT)
        .fetch_all(db)
        .await?,
    ));

    // 7. Payouts and their journals agree.
    found.extend(problems(
        "payout_not_posted",
        sqlx::query_as(
            "select p.id::text, 'a ' || p.status::text || ' payout has no payout journal' \
             from payouts p where p.status in ('requested', 'paid') \
               and not exists (select 1 from ledger_journals j where j.kind = 'payout' and j.source_id = p.id) \
             limit $1",
        )
        .bind(LIMIT)
        .fetch_all(db)
        .await?,
    ));
    found.extend(problems(
        "payout_not_reversed",
        sqlx::query_as(
            "select p.id::text, 'a cancelled payout has not been given back' \
             from payouts p where p.status = 'cancelled' \
               and not exists (select 1 from ledger_journals j where j.kind = 'payout_reversal' and j.source_id = p.id) \
             limit $1",
        )
        .bind(LIMIT)
        .fetch_all(db)
        .await?,
    ));
    found.extend(problems(
        "payout_without_payout",
        sqlx::query_as(
            "select j.source_id::text, 'a ' || j.kind || ' journal with no matching payout' \
             from ledger_journals j \
             where (j.kind = 'payout' and not exists (select 1 from payouts p where p.id = j.source_id)) \
                or (j.kind = 'payout_reversal' and not exists \
                    (select 1 from payouts p where p.id = j.source_id and p.status = 'cancelled')) \
             limit $1",
        )
        .bind(LIMIT)
        .fetch_all(db)
        .await?,
    ));

    // 8. Adjustments and their journals agree both ways.
    found.extend(problems(
        "adjustment_mismatch",
        sqlx::query_as(
            "(select a.id::text, 'an adjustment with no journal' from ledger_adjustments a               where not exists (select 1 from ledger_journals j where j.kind = 'adjustment' and j.source_id = a.id)               limit $1)              union all              (select j.source_id::text, 'an adjustment journal with no adjustment' from ledger_journals j               where j.kind = 'adjustment'                 and not exists (select 1 from ledger_adjustments a where a.id = j.source_id)               limit $1)",
        )
        .bind(LIMIT)
        .fetch_all(db)
        .await?,
    ));

    Ok(found)
}

/// Runs the check once a day for as long as the process lives, shouting if it finds anything.
pub fn spawn_daily(state: AppState) {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(EVERY);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            match check(&state.db).await {
                Ok(found) if found.is_empty() => tracing::info!("ledger integrity check: clean"),
                Ok(found) => {
                    for problem in &found {
                        tracing::error!(
                            alert = "ledger_integrity",
                            kind = problem.kind,
                            subject = %problem.subject,
                            detail = %problem.detail,
                            "the ledger disagrees with the rest of the system"
                        );
                    }
                }
                Err(error) => tracing::error!(?error, "ledger integrity check could not run"),
            }
        }
    });
}
