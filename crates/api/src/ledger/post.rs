//! Posting to the book. Every function here runs inside the caller's transaction and is
//! idempotent: the second time an event is posted it finds the journal and does nothing.

use sqlx::PgConnection;
use uuid::Uuid;

use crate::ledger::fee::Fee;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Account {
    Creator(Uuid),
    Customers,
    GatewayFees,
    Payouts,
    Adjustments,
}

impl Account {
    pub fn name(self) -> String {
        match self {
            Self::Creator(id) => format!("creator:{id}"),
            Self::Customers => "customers".to_owned(),
            Self::GatewayFees => "gateway_fees".to_owned(),
            Self::Payouts => "payouts".to_owned(),
            Self::Adjustments => "adjustments".to_owned(),
        }
    }

    fn creator(self) -> Option<Uuid> {
        match self {
            Self::Creator(id) => Some(id),
            _ => None,
        }
    }
}

/// Starts a journal, or `None` if this event already has one.
async fn open_journal(
    conn: &mut PgConnection,
    kind: &str,
    source_id: Uuid,
    order_id: Option<Uuid>,
    memo: Option<&str>,
) -> Result<Option<Uuid>, sqlx::Error> {
    sqlx::query_scalar(
        "insert into ledger_journals (kind, source_id, order_id, memo) values ($1, $2, $3, $4) \
         on conflict (kind, source_id) do nothing returning id",
    )
    .bind(kind)
    .bind(source_id)
    .bind(order_id)
    .bind(memo)
    .fetch_optional(conn)
    .await
}

async fn entry(
    conn: &mut PgConnection,
    journal: Uuid,
    account: Account,
    amount_paise: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "insert into ledger_entries (journal_id, account, creator_id, amount_paise) \
         values ($1, $2, $3, $4)",
    )
    .bind(journal)
    .bind(account.name())
    .bind(account.creator())
    .bind(amount_paise)
    .execute(conn)
    .await?;
    Ok(())
}

/// One movement of money between two accounts, posted as a journal of its own.
struct Movement<'a> {
    kind: &'a str,
    source_id: Uuid,
    order_id: Option<Uuid>,
    memo: Option<&'a str>,
    from: Account,
    to: Account,
    /// Positive.
    amount_paise: i64,
}

/// Posts the movement. `false` if the event had already been posted.
async fn transfer(conn: &mut PgConnection, movement: &Movement<'_>) -> Result<bool, sqlx::Error> {
    debug_assert!(movement.amount_paise > 0);
    let Some(journal) = open_journal(
        conn,
        movement.kind,
        movement.source_id,
        movement.order_id,
        movement.memo,
    )
    .await?
    else {
        return Ok(false);
    };
    entry(conn, journal, movement.from, -movement.amount_paise).await?;
    entry(conn, journal, movement.to, movement.amount_paise).await?;
    Ok(true)
}

/// What a confirmed payment is worth to the creator it was bought from.
#[derive(Debug, Clone, Copy)]
pub struct Sale {
    pub payment_id: Uuid,
    pub order_id: Uuid,
    pub creator_id: Uuid,
    pub gross_paise: i64,
    pub fee: Fee,
}

/// Records a sale: the creator is credited the full amount the customer paid, and debited the
/// provider's fee. Keyed by the payment, so posting it again changes nothing.
pub async fn sale(conn: &mut PgConnection, sale: &Sale) -> Result<bool, sqlx::Error> {
    let posted = transfer(
        conn,
        &Movement {
            kind: "sale",
            source_id: sale.payment_id,
            order_id: Some(sale.order_id),
            memo: None,
            from: Account::Customers,
            to: Account::Creator(sale.creator_id),
            amount_paise: sale.gross_paise,
        },
    )
    .await?;
    if sale.fee.paise > 0 {
        transfer(
            conn,
            &Movement {
                kind: "gateway_fee",
                source_id: sale.payment_id,
                order_id: Some(sale.order_id),
                memo: None,
                from: Account::Creator(sale.creator_id),
                to: Account::GatewayFees,
                amount_paise: sale.fee.paise,
            },
        )
        .await?;
    }
    Ok(posted)
}

/// Replaces a provisional fee with the provider's later reported fee. The original fee journal
/// stays untouched: one balanced correction per payment moves only the difference. The caller
/// holds the payment/order lock and records the reported fee in the same transaction.
pub async fn correct_fee(
    conn: &mut PgConnection,
    payment_id: Uuid,
    reported_paise: i64,
) -> Result<bool, sqlx::Error> {
    let sold: Option<(Option<Uuid>, Uuid)> = sqlx::query_as(
        "select j.order_id, e.creator_id from ledger_journals j \
         join ledger_entries e on e.journal_id = j.id and e.creator_id is not null \
         where j.kind = 'sale' and j.source_id = $1",
    )
    .bind(payment_id)
    .fetch_optional(&mut *conn)
    .await?;
    let Some((order_id, creator_id)) = sold else {
        return Ok(false);
    };
    let charged: i64 = sqlx::query_scalar(
        "select coalesce(-sum(e.amount_paise), 0)::bigint from ledger_journals j \
         join ledger_entries e on e.journal_id = j.id and e.creator_id = $2 \
         where j.source_id = $1 and j.kind in ('gateway_fee', 'gateway_fee_correction')",
    )
    .bind(payment_id)
    .bind(creator_id)
    .fetch_one(&mut *conn)
    .await?;
    let difference = reported_paise - charged;
    if difference == 0 {
        return Ok(false);
    }
    let (from, to, amount_paise) = if difference > 0 {
        (
            Account::Creator(creator_id),
            Account::GatewayFees,
            difference,
        )
    } else {
        (
            Account::GatewayFees,
            Account::Creator(creator_id),
            -difference,
        )
    };
    transfer(
        conn,
        &Movement {
            kind: "gateway_fee_correction",
            source_id: payment_id,
            order_id,
            memo: Some("Provider reported its final fee"),
            from,
            to,
            amount_paise,
        },
    )
    .await
}

/// Records money returned to a customer: the creator gives back what was returned. Only for
/// payments that were posted as sales (a refunded duplicate, or a payment refunded before it was
/// ever applied, never credited anyone). The provider keeps its fee, so the fee stays with the
/// creator. `false` if nothing was posted.
pub async fn refund(
    conn: &mut PgConnection,
    payment_id: Uuid,
    refund_id: Uuid,
    amount_paise: i64,
) -> Result<bool, sqlx::Error> {
    if amount_paise <= 0 {
        return Ok(false);
    }
    let sold: Option<(Option<Uuid>, Uuid)> = sqlx::query_as(
        "select j.order_id, e.creator_id from ledger_journals j \
         join ledger_entries e on e.journal_id = j.id and e.creator_id is not null \
         where j.kind = 'sale' and j.source_id = $1",
    )
    .bind(payment_id)
    .fetch_optional(&mut *conn)
    .await?;
    let Some((order_id, creator_id)) = sold else {
        return Ok(false);
    };
    transfer(
        conn,
        &Movement {
            kind: "refund",
            source_id: refund_id,
            order_id,
            memo: None,
            from: Account::Creator(creator_id),
            to: Account::Customers,
            amount_paise,
        },
    )
    .await
}

/// Reserves money for a payout: it leaves the creator's balance at once, so it cannot be asked
/// for twice.
pub async fn payout(
    conn: &mut PgConnection,
    payout_id: Uuid,
    creator_id: Uuid,
    amount_paise: i64,
) -> Result<bool, sqlx::Error> {
    transfer(
        conn,
        &Movement {
            kind: "payout",
            source_id: payout_id,
            order_id: None,
            memo: None,
            from: Account::Creator(creator_id),
            to: Account::Payouts,
            amount_paise,
        },
    )
    .await
}

/// Gives a cancelled payout's money back to the creator.
pub async fn payout_reversal(
    conn: &mut PgConnection,
    payout_id: Uuid,
    creator_id: Uuid,
    amount_paise: i64,
) -> Result<bool, sqlx::Error> {
    transfer(
        conn,
        &Movement {
            kind: "payout_reversal",
            source_id: payout_id,
            order_id: None,
            memo: None,
            from: Account::Payouts,
            to: Account::Creator(creator_id),
            amount_paise,
        },
    )
    .await
}

/// A correction by an admin: positive adds to what the creator is owed, negative takes away.
pub async fn adjustment(
    conn: &mut PgConnection,
    adjustment_id: Uuid,
    creator_id: Uuid,
    amount_paise: i64,
    reason: &str,
) -> Result<bool, sqlx::Error> {
    let (from, to) = if amount_paise > 0 {
        (Account::Adjustments, Account::Creator(creator_id))
    } else {
        (Account::Creator(creator_id), Account::Adjustments)
    };
    transfer(
        conn,
        &Movement {
            kind: "adjustment",
            source_id: adjustment_id,
            order_id: None,
            memo: Some(reason),
            from,
            to,
            amount_paise: amount_paise.abs(),
        },
    )
    .await
}
