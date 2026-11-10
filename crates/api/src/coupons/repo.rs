use chrono::{DateTime, Utc};
use sqlx::{PgConnection, PgExecutor};
use uuid::Uuid;
use zeno_core::model::CouponKind;

use crate::coupons::dto::{CouponChanges, NewCoupon};

const COLUMNS: &str = "id, creator_id, code, kind, value, course_id, valid_from, valid_until, \
    max_redemptions, per_user_limit, redeemed_count, active, created_at, updated_at";

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct CouponRow {
    pub id: Uuid,
    pub creator_id: Uuid,
    pub code: String,
    pub kind: CouponKind,
    pub value: i64,
    pub course_id: Option<Uuid>,
    pub valid_from: Option<DateTime<Utc>>,
    pub valid_until: Option<DateTime<Utc>>,
    pub max_redemptions: Option<i32>,
    pub per_user_limit: i32,
    pub redeemed_count: i32,
    pub active: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// `None` if this creator already has a coupon with that code.
pub async fn insert<'e>(
    db: impl PgExecutor<'e>,
    creator_id: Uuid,
    new: &NewCoupon,
) -> Result<Option<CouponRow>, sqlx::Error> {
    sqlx::query_as(&format!(
        "insert into coupons \
            (creator_id, code, kind, value, course_id, valid_from, valid_until, max_redemptions, \
             per_user_limit) \
         values ($1,$2,$3,$4,$5,$6,$7,$8,$9) \
         on conflict (creator_id, code) do nothing returning {COLUMNS}"
    ))
    .bind(creator_id)
    .bind(&new.code)
    .bind(new.kind)
    .bind(new.value)
    .bind(new.course_id)
    .bind(new.valid_from)
    .bind(new.valid_until)
    .bind(new.max_redemptions)
    .bind(new.per_user_limit)
    .fetch_optional(db)
    .await
}

pub async fn by_id<'e>(
    db: impl PgExecutor<'e>,
    id: Uuid,
) -> Result<Option<CouponRow>, sqlx::Error> {
    sqlx::query_as(&format!("select {COLUMNS} from coupons where id = $1"))
        .bind(id)
        .fetch_optional(db)
        .await
}

pub async fn by_code<'e>(
    db: impl PgExecutor<'e>,
    creator_id: Uuid,
    code: &str,
) -> Result<Option<CouponRow>, sqlx::Error> {
    sqlx::query_as(&format!(
        "select {COLUMNS} from coupons where creator_id = $1 and code = $2"
    ))
    .bind(creator_id)
    .bind(code)
    .fetch_optional(db)
    .await
}

pub async fn list_for_creator<'e>(
    db: impl PgExecutor<'e>,
    creator_id: Uuid,
    limit: i64,
) -> Result<Vec<CouponRow>, sqlx::Error> {
    sqlx::query_as(&format!(
        "select {COLUMNS} from coupons where creator_id = $1 \
         order by created_at desc, id desc limit $2"
    ))
    .bind(creator_id)
    .bind(limit)
    .fetch_all(db)
    .await
}

pub async fn count_for_creator<'e>(
    db: impl PgExecutor<'e>,
    creator_id: Uuid,
) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar("select count(*) from coupons where creator_id = $1")
        .bind(creator_id)
        .fetch_one(db)
        .await
}

/// Applies the changes; `None` if the coupon is gone or the new limit is below what is already in
/// use.
pub async fn update<'e>(
    db: impl PgExecutor<'e>,
    id: Uuid,
    changes: &CouponChanges,
) -> Result<Option<CouponRow>, sqlx::Error> {
    sqlx::query_as(&format!(
        "update coupons set active = coalesce($2, active), \
            valid_until = coalesce($3, valid_until), \
            max_redemptions = coalesce($4, max_redemptions), updated_at = now() \
         where id = $1 \
           and ($4::int is null or $4 >= redeemed_count) \
           and ($3::timestamptz is null or valid_from is null or $3 > valid_from) \
         returning {COLUMNS}"
    ))
    .bind(id)
    .bind(changes.active)
    .bind(changes.valid_until)
    .bind(changes.max_redemptions)
    .fetch_optional(db)
    .await
}

/// Whether a creator's own, not deleted, course exists.
pub async fn owns_course<'e>(
    db: impl PgExecutor<'e>,
    creator_id: Uuid,
    course_id: Uuid,
) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar(
        "select exists (select 1 from courses where id = $1 and creator_id = $2 \
                          and deleted_at is null)",
    )
    .bind(course_id)
    .bind(creator_id)
    .fetch_one(db)
    .await
}

/// How many redemptions of this coupon this person holds (released ones do not count).
///
/// `except_order` leaves out the redemption held by that order: asking for the coupon an open
/// order already carries is not a second use.
pub async fn uses_by<'e>(
    db: impl PgExecutor<'e>,
    coupon_id: Uuid,
    user_id: Uuid,
    except_order: Option<Uuid>,
) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar(
        "select count(*) from coupon_redemptions \
         where coupon_id = $1 and user_id = $2 and released_at is null \
           and ($3::uuid is null or order_id is distinct from $3)",
    )
    .bind(coupon_id)
    .bind(user_id)
    .bind(except_order)
    .fetch_one(db)
    .await
}

/// The coupon an open order is using, if any.
pub async fn coupon_of_order<'e>(
    db: impl PgExecutor<'e>,
    order_id: Uuid,
) -> Result<Option<Uuid>, sqlx::Error> {
    sqlx::query_scalar(
        "select coupon_id from coupon_redemptions where order_id = $1 and released_at is null",
    )
    .bind(order_id)
    .fetch_optional(db)
    .await
}

/// Takes one redemption, if the coupon is live and has one left. Returns the per-person limit on
/// success. This single conditional update is what makes `max_redemptions` hold under any amount
/// of concurrency: it takes a row lock, so redemptions of one coupon happen one at a time.
pub async fn take_redemption(
    conn: &mut PgConnection,
    coupon_id: Uuid,
) -> Result<Option<i32>, sqlx::Error> {
    sqlx::query_scalar(
        "update coupons set redeemed_count = redeemed_count + 1, updated_at = now() \
         where id = $1 and active \
           and (valid_from is null or valid_from <= now()) \
           and (valid_until is null or valid_until > now()) \
           and (max_redemptions is null or redeemed_count < max_redemptions) \
         returning per_user_limit",
    )
    .bind(coupon_id)
    .fetch_optional(conn)
    .await
}

pub async fn insert_redemption(
    conn: &mut PgConnection,
    coupon_id: Uuid,
    user_id: Uuid,
    order_id: Option<Uuid>,
    enrollment_id: Option<Uuid>,
    discount_paise: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "insert into coupon_redemptions \
            (coupon_id, user_id, order_id, enrollment_id, discount_paise) \
         values ($1,$2,$3,$4,$5)",
    )
    .bind(coupon_id)
    .bind(user_id)
    .bind(order_id)
    .bind(enrollment_id)
    .bind(discount_paise)
    .execute(conn)
    .await?;
    Ok(())
}
