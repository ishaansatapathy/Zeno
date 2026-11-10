use chrono::{DateTime, Utc};
use sqlx::PgExecutor;
use uuid::Uuid;

use crate::catalog::dto::{Filter, Sort};

/// One course as the catalogue lists it. `creator_name` falls back to the account name for a
/// creator who has not made a profile.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct CardRow {
    pub id: Uuid,
    pub slug: String,
    pub title: String,
    pub short_description: String,
    pub price_paise: i64,
    pub currency: String,
    pub language: Option<String>,
    pub level: String,
    pub thumbnail_key: Option<String>,
    pub published_at: Option<DateTime<Utc>>,
    pub lesson_count: i64,
    pub total_seconds: i64,
    pub creator_handle: Option<String>,
    pub creator_name: String,
    pub creator_avatar_key: Option<String>,
}

#[derive(Debug, sqlx::FromRow)]
pub struct DetailRow {
    #[sqlx(flatten)]
    pub card: CardRow,
    pub description: String,
    pub refund_window_days: i32,
}

#[derive(Debug, sqlx::FromRow)]
pub struct LessonRow {
    pub id: Uuid,
    pub position: i32,
    pub title: String,
    pub is_free_preview: bool,
    pub duration_seconds: Option<i32>,
}

const CARD_COLUMNS: &str = "c.id, c.slug, c.title, c.short_description, c.price_paise, \
    c.currency, c.language, c.level, c.thumbnail_key, c.published_at, \
    coalesce(s.lesson_count, 0)::bigint as lesson_count, \
    coalesce(s.total_seconds, 0)::bigint as total_seconds, \
    p.handle as creator_handle, coalesce(p.display_name, u.name) as creator_name, \
    p.avatar_key as creator_avatar_key";

/// Joins shared by every catalogue query. Only published, undeleted courses are reachable.
const CARD_FROM: &str = "from courses c \
    join users u on u.id = c.creator_id \
    left join creator_profiles p on p.user_id = c.creator_id \
    left join lateral ( \
        select count(*) as lesson_count, \
               coalesce(sum(v.duration_seconds), 0) as total_seconds \
        from lessons l join videos v on v.id = l.video_id \
        where l.course_id = c.id and l.deleted_at is null \
    ) s on true \
    where c.status = 'published' and c.deleted_at is null";

pub async fn find_by_slug<'e>(
    db: impl PgExecutor<'e>,
    slug: &str,
) -> Result<Option<DetailRow>, sqlx::Error> {
    sqlx::query_as(&format!(
        "select {CARD_COLUMNS}, c.description, c.refund_window_days {CARD_FROM} and c.slug = $1"
    ))
    .bind(slug)
    .fetch_optional(db)
    .await
}

pub async fn lessons<'e>(
    db: impl PgExecutor<'e>,
    course_id: Uuid,
) -> Result<Vec<LessonRow>, sqlx::Error> {
    sqlx::query_as(
        "select l.id, l.position, l.title, l.is_free_preview, v.duration_seconds \
         from lessons l join videos v on v.id = l.video_id \
         where l.course_id = $1 and l.deleted_at is null order by l.position",
    )
    .bind(course_id)
    .fetch_all(db)
    .await
}

/// Search and browse. With text, a course matches when its weighted full-text vector matches the
/// query, or (for queries of four characters or more) some word of its title is close enough to
/// it to be a typo; relevance mixes the two. The typo check scans the published courses, which
/// is cheap at this scale; a trigram index would only help once the catalogue is far larger.
pub async fn search<'e>(
    db: impl PgExecutor<'e>,
    filter: &Filter,
    fetch: i64,
) -> Result<Vec<CardRow>, sqlx::Error> {
    let newest = "c.published_at desc, c.id desc";
    let order = match (filter.sort, filter.text.is_some()) {
        (Some(Sort::PriceAsc), _) => format!("c.price_paise asc, {newest}"),
        (Some(Sort::PriceDesc), _) => format!("c.price_paise desc, {newest}"),
        (Some(Sort::Newest), _) | (None, false) | (Some(Sort::Relevance), false) => {
            newest.to_owned()
        }
        (Some(Sort::Relevance) | None, true) => format!(
            "(ts_rank_cd(c.search_vector, websearch_to_tsquery('simple', $1)) \
              + word_similarity(lower($1), lower(c.title))) desc, {newest}"
        ),
    };
    sqlx::query_as(&format!(
        "select {CARD_COLUMNS} {CARD_FROM} \
           and ($1::text is null \
                or c.search_vector @@ websearch_to_tsquery('simple', $1) \
                or (char_length($1) >= 4 \
                    and word_similarity(lower($1), lower(c.title)) >= 0.4)) \
           and ($2::text is null or c.language = $2) \
           and ($3::text is null or c.level = $3) \
           and (not $4 or c.price_paise = 0) \
           and ($5::bigint is null or c.price_paise >= $5) \
           and ($6::bigint is null or c.price_paise <= $6) \
           and ($7::uuid is null or c.creator_id = $7) \
         order by {order} limit $8 offset $9"
    ))
    .bind(filter.text.as_deref())
    .bind(filter.language.as_deref())
    .bind(filter.level.as_deref())
    .bind(filter.free_only)
    .bind(filter.min_price)
    .bind(filter.max_price)
    .bind(filter.creator_id)
    .bind(fetch)
    .bind(i64::from(filter.offset))
    .fetch_all(db)
    .await
}
