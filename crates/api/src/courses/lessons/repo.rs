use chrono::{DateTime, Utc};
use sqlx::PgConnection;
use uuid::Uuid;
use zeno_core::model::VideoStatus;

#[derive(Debug, sqlx::FromRow)]
pub struct LockedCourse {
    pub id: Uuid,
    pub creator_id: Uuid,
    pub version: i32,
}

#[derive(Debug, sqlx::FromRow)]
pub struct LockedLesson {
    pub title: String,
    pub description: String,
    pub video_id: Uuid,
    pub is_free_preview: bool,
    pub position: i32,
}

#[derive(Debug, sqlx::FromRow)]
pub struct VideoForLesson {
    pub id: Uuid,
    pub status: VideoStatus,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct LessonRow {
    pub id: Uuid,
    pub course_id: Uuid,
    pub title: String,
    pub description: String,
    pub position: i32,
    pub video_id: Uuid,
    pub is_free_preview: bool,
    pub video_status: VideoStatus,
    pub video_deleted_at: Option<DateTime<Utc>>,
    pub duration_seconds: Option<i32>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

const LESSON_COLUMNS: &str = "l.id, l.course_id, l.title, l.description, l.position, \
    l.video_id, l.is_free_preview, v.status as video_status, v.deleted_at as video_deleted_at, \
    v.duration_seconds, \
    l.created_at, l.updated_at";

pub async fn lock_course(
    db: &mut PgConnection,
    course_id: Uuid,
    actor_id: Uuid,
    admin: bool,
) -> Result<Option<LockedCourse>, sqlx::Error> {
    sqlx::query_as(
        "select id, creator_id, version from courses \
         where id=$1 and deleted_at is null and ($2 or creator_id=$3) for update",
    )
    .bind(course_id)
    .bind(admin)
    .bind(actor_id)
    .fetch_optional(db)
    .await
}

pub async fn find_course(
    db: &mut PgConnection,
    course_id: Uuid,
    actor_id: Uuid,
    admin: bool,
) -> Result<Option<LockedCourse>, sqlx::Error> {
    sqlx::query_as(
        "select id, creator_id, version from courses \
         where id=$1 and deleted_at is null and ($2 or creator_id=$3)",
    )
    .bind(course_id)
    .bind(admin)
    .bind(actor_id)
    .fetch_optional(db)
    .await
}

pub async fn lock_course_for_lesson(
    db: &mut PgConnection,
    lesson_id: Uuid,
    actor_id: Uuid,
    admin: bool,
) -> Result<Option<LockedCourse>, sqlx::Error> {
    sqlx::query_as(
        "select c.id, c.creator_id, c.version \
         from courses c join lessons l on l.course_id=c.id \
         where l.id=$1 and l.deleted_at is null and c.deleted_at is null \
           and ($2 or c.creator_id=$3) for update of c",
    )
    .bind(lesson_id)
    .bind(admin)
    .bind(actor_id)
    .fetch_optional(db)
    .await
}

pub async fn lock_lesson(
    db: &mut PgConnection,
    lesson_id: Uuid,
    course_id: Uuid,
) -> Result<Option<LockedLesson>, sqlx::Error> {
    sqlx::query_as(
        "select l.title, l.description, \
                l.video_id, l.is_free_preview, l.position \
         from lessons l where l.id=$1 and l.course_id=$2 and l.deleted_at is null \
         for update of l",
    )
    .bind(lesson_id)
    .bind(course_id)
    .fetch_optional(db)
    .await
}

pub async fn find_video(
    db: &mut PgConnection,
    video_id: Uuid,
    owner_id: Uuid,
) -> Result<Option<VideoForLesson>, sqlx::Error> {
    sqlx::query_as(
        "select id, status from videos \
         where id=$1 and owner_id=$2 and deleted_at is null",
    )
    .bind(video_id)
    .bind(owner_id)
    .fetch_optional(db)
    .await
}

pub async fn active_count(db: &mut PgConnection, course_id: Uuid) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar("select count(*) from lessons where course_id=$1 and deleted_at is null")
        .bind(course_id)
        .fetch_one(db)
        .await
}

pub async fn defer_positions(db: &mut PgConnection) -> Result<(), sqlx::Error> {
    sqlx::query("set constraints lessons_course_position_unique deferred")
        .execute(db)
        .await?;
    Ok(())
}

pub async fn shift_for_insert(
    db: &mut PgConnection,
    course_id: Uuid,
    position: i32,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "update lessons set position=position+1, updated_at=now() \
         where course_id=$1 and deleted_at is null and position >= $2",
    )
    .bind(course_id)
    .bind(position)
    .execute(db)
    .await?;
    Ok(())
}

pub struct NewLessonRow<'a> {
    pub course_id: Uuid,
    pub title: &'a str,
    pub description: &'a str,
    pub position: i32,
    pub video_id: Uuid,
    pub is_free_preview: bool,
}

pub async fn insert(db: &mut PgConnection, lesson: &NewLessonRow<'_>) -> Result<Uuid, sqlx::Error> {
    sqlx::query_scalar(
        "insert into lessons \
            (course_id, title, description, position, video_id, is_free_preview) \
         values ($1,$2,$3,$4,$5,$6) returning id",
    )
    .bind(lesson.course_id)
    .bind(lesson.title)
    .bind(lesson.description)
    .bind(lesson.position)
    .bind(lesson.video_id)
    .bind(lesson.is_free_preview)
    .fetch_one(db)
    .await
}

pub struct LessonValues<'a> {
    pub title: &'a str,
    pub description: &'a str,
    pub video_id: Uuid,
    pub is_free_preview: bool,
}

pub async fn update(
    db: &mut PgConnection,
    lesson_id: Uuid,
    values: &LessonValues<'_>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "update lessons set title=$2, description=$3, video_id=$4, is_free_preview=$5, \
            updated_at=now() where id=$1 and deleted_at is null",
    )
    .bind(lesson_id)
    .bind(values.title)
    .bind(values.description)
    .bind(values.video_id)
    .bind(values.is_free_preview)
    .execute(db)
    .await?;
    Ok(())
}

pub async fn soft_delete(
    db: &mut PgConnection,
    lesson_id: Uuid,
    course_id: Uuid,
    old_position: i32,
) -> Result<(), sqlx::Error> {
    let tombstone: i32 = sqlx::query_scalar(
        "select coalesce(min(position), 0) - 1 from lessons \
         where course_id=$1 and deleted_at is not null",
    )
    .bind(course_id)
    .fetch_one(&mut *db)
    .await?;
    sqlx::query(
        "update lessons set deleted_at=now(), deleted_position=position, position=$2, \
            updated_at=now() where id=$1 and deleted_at is null",
    )
    .bind(lesson_id)
    .bind(tombstone)
    .execute(&mut *db)
    .await?;
    sqlx::query(
        "update lessons set position=position-1, updated_at=now() \
         where course_id=$1 and deleted_at is null and position > $2",
    )
    .bind(course_id)
    .bind(old_position)
    .execute(db)
    .await?;
    Ok(())
}

pub async fn active_ids(db: &mut PgConnection, course_id: Uuid) -> Result<Vec<Uuid>, sqlx::Error> {
    sqlx::query_scalar(
        "select id from lessons where course_id=$1 and deleted_at is null order by position",
    )
    .bind(course_id)
    .fetch_all(db)
    .await
}

pub async fn reorder(
    db: &mut PgConnection,
    course_id: Uuid,
    lesson_ids: &[Uuid],
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "update lessons l set position=ordered.position, updated_at=now() \
         from (select id, ordinality::integer as position \
               from unnest($2::uuid[]) with ordinality as ids(id, ordinality)) ordered \
         where l.course_id=$1 and l.deleted_at is null and l.id=ordered.id",
    )
    .bind(course_id)
    .bind(lesson_ids)
    .execute(db)
    .await?;
    Ok(())
}

pub async fn bump_course_version(
    db: &mut PgConnection,
    course_id: Uuid,
) -> Result<i32, sqlx::Error> {
    sqlx::query_scalar(
        "update courses c set \
            status=case when c.status='published' and not ( \
                exists (select 1 from lessons l where l.course_id=c.id and l.deleted_at is null) \
                and not exists ( \
                    select 1 from lessons l left join videos v on v.id=l.video_id \
                    where l.course_id=c.id and l.deleted_at is null \
                      and (l.video_id is null or v.status <> 'ready' or v.deleted_at is not null) \
                ) \
            ) then 'draft'::course_status else c.status end, \
            published_at=case when c.status='published' and not ( \
                exists (select 1 from lessons l where l.course_id=c.id and l.deleted_at is null) \
                and not exists ( \
                    select 1 from lessons l left join videos v on v.id=l.video_id \
                    where l.course_id=c.id and l.deleted_at is null \
                      and (l.video_id is null or v.status <> 'ready' or v.deleted_at is not null) \
                ) \
            ) then null else c.published_at end, \
            version=c.version+1, updated_at=now() \
         where c.id=$1 returning version",
    )
    .bind(course_id)
    .fetch_one(db)
    .await
}

pub async fn find_one(db: &mut PgConnection, lesson_id: Uuid) -> Result<LessonRow, sqlx::Error> {
    sqlx::query_as(&format!(
        "select {LESSON_COLUMNS} from lessons l join videos v on v.id=l.video_id \
         where l.id=$1 and l.deleted_at is null"
    ))
    .bind(lesson_id)
    .fetch_one(db)
    .await
}

pub async fn list_in_transaction(
    db: &mut PgConnection,
    course_id: Uuid,
) -> Result<Vec<LessonRow>, sqlx::Error> {
    sqlx::query_as(&format!(
        "select {LESSON_COLUMNS} from lessons l join videos v on v.id=l.video_id \
         where l.course_id=$1 and l.deleted_at is null order by l.position"
    ))
    .bind(course_id)
    .fetch_all(db)
    .await
}
