use std::net::IpAddr;

use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::types::ipnetwork::IpNetwork;
use sqlx::{PgConnection, PgExecutor};
use uuid::Uuid;

/// Ends the user's sessions that have expired or have not been seen within `idle_secs`, so they
/// stop holding a slot. Returns how many were ended.
pub async fn end_stale(
    db: &mut PgConnection,
    user_id: Uuid,
    idle_secs: i64,
) -> Result<u64, sqlx::Error> {
    let expired = sqlx::query(
        "update playback_sessions set ended_at = now(), end_reason = 'expired' \
         where user_id = $1 and ended_at is null and expires_at <= now()",
    )
    .bind(user_id)
    .execute(&mut *db)
    .await?
    .rows_affected();
    let idle = sqlx::query(
        "update playback_sessions set ended_at = now(), end_reason = 'idle' \
         where user_id = $1 and ended_at is null \
           and last_seen_at < now() - make_interval(secs => $2)",
    )
    .bind(user_id)
    .bind(idle_secs as f64)
    .execute(&mut *db)
    .await?
    .rows_affected();
    Ok(expired + idle)
}

pub async fn count_open(db: &mut PgConnection, user_id: Uuid) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar(
        "select count(*) from playback_sessions where user_id = $1 and ended_at is null",
    )
    .bind(user_id)
    .fetch_one(db)
    .await
}

/// Serialises session starts for one user, so two devices pressing play at the same moment
/// cannot both take the last slot.
pub async fn lock_user(db: &mut PgConnection, user_id: Uuid) -> Result<(), sqlx::Error> {
    sqlx::query("select 1 from users where id = $1 for update")
        .bind(user_id)
        .execute(db)
        .await?;
    Ok(())
}

/// Where the user last was in this video, if they have watched it before.
pub async fn last_position(
    db: &mut PgConnection,
    user_id: Uuid,
    video_id: Uuid,
) -> Result<Option<i32>, sqlx::Error> {
    sqlx::query_scalar(
        "select last_position_seconds from playback_sessions \
         where user_id = $1 and video_id = $2 order by last_seen_at desc limit 1",
    )
    .bind(user_id)
    .bind(video_id)
    .fetch_optional(db)
    .await
}

pub struct NewSession<'a> {
    pub id: Uuid,
    /// The signed-in viewer; `None` when the viewing comes through a share link.
    pub user_id: Option<Uuid>,
    /// The share link a link viewing belongs to. Exactly one of this and `user_id` is set.
    pub share_link_id: Option<Uuid>,
    pub video_id: Uuid,
    pub device: &'a str,
    pub ip: Option<IpAddr>,
    pub expires_at: DateTime<Utc>,
    pub position: i32,
    /// The lesson being played, when it was started from one. Progress is recorded against it.
    pub lesson_id: Option<Uuid>,
}

pub async fn insert(db: &mut PgConnection, session: &NewSession<'_>) -> Result<(), sqlx::Error> {
    sqlx::query(
        "insert into playback_sessions (id, user_id, share_link_id, video_id, device, ip, expires_at, last_position_seconds, lesson_id) \
         values ($1, $2, $9, $3, $4, $5, $6, $7, $8)",
    )
    .bind(session.id)
    .bind(session.user_id)
    .bind(session.video_id)
    .bind(session.device)
    .bind(session.ip.map(IpNetwork::from))
    .bind(session.expires_at)
    .bind(session.position)
    .bind(session.lesson_id)
    .bind(session.share_link_id)
    .execute(db)
    .await?;
    Ok(())
}

/// Starts a viewing through a share link, unless the link has closed. `false`: it has.
///
/// The check and the insert are one statement, and the statement takes a shared lock on the link
/// and on its recording. Closing a link (revoking it, making the recording private, deleting it)
/// updates one of those rows and then ends the viewings it can see. A viewing that started after
/// the closer looked, but before it committed, would be missed by that sweep and stay open on a
/// closed link. With the lock, either the closer committed first (this statement waits, re-reads
/// the row, finds it closed and inserts nothing) or this commits first (and the closer's sweep,
/// which runs after, sees and ends it).
pub async fn insert_for_link(
    db: &mut PgConnection,
    session: &NewSession<'_>,
) -> Result<bool, sqlx::Error> {
    let Some(link) = session.share_link_id else {
        return Ok(false);
    };
    let inserted = sqlx::query(
        "with open as (
             select l.id from share_links l join recordings r on r.id = l.recording_id
             where l.id = $9 and r.video_id = $3 and r.deleted_at is null
               and r.visibility in ('link', 'public')
               and l.revoked_at is null and (l.expires_at is null or l.expires_at > now())
             for share of l, r
         )
         insert into playback_sessions
             (id, user_id, share_link_id, video_id, device, ip, expires_at, last_position_seconds, lesson_id)
         select $1::uuid, $2::uuid, open.id, $3::uuid, $4::text, $5::inet, $6::timestamptz, $7::int4, $8::uuid
         from open",
    )
    .bind(session.id)
    .bind(session.user_id)
    .bind(session.video_id)
    .bind(session.device)
    .bind(session.ip.map(IpNetwork::from))
    .bind(session.expires_at)
    .bind(session.position)
    .bind(session.lesson_id)
    .bind(link)
    .execute(db)
    .await?;
    Ok(inserted.rows_affected() == 1)
}

#[derive(sqlx::FromRow)]
pub struct Viewer {
    pub name: String,
    pub email: Option<String>,
    pub phone: Option<String>,
}

pub async fn viewer<'e>(
    db: impl PgExecutor<'e>,
    user_id: Uuid,
) -> Result<Option<Viewer>, sqlx::Error> {
    sqlx::query_as("select name, email, phone from users where id = $1")
        .bind(user_id)
        .fetch_optional(db)
        .await
}

/// A session that may still be served, with what serving it needs.
#[derive(sqlx::FromRow)]
pub struct Playable {
    /// `None` for a share-link viewing.
    pub user_id: Option<Uuid>,
    pub video_id: Uuid,
    pub expires_at: DateTime<Utc>,
    pub renditions: Value,
}

/// Marks an authorised session as seen. A session whose entitlement disappeared is atomically
/// ended instead, so the very request that observes revocation cannot refresh its lifetime.
pub async fn touch_playable<'e>(
    db: impl PgExecutor<'e>,
    session_id: Uuid,
) -> Result<Option<Playable>, sqlx::Error> {
    sqlx::query_as(
        "with candidate as materialized ( \
             select s.id, s.user_id, s.video_id, s.expires_at, v.renditions, \
                case \
                    when s.expires_at <= now() then 'expired' \
                    when v.deleted_at is not null then 'video_deleted' \
                    when v.status <> 'ready' then 'access_revoked' \
                    else playback_access_state(s.user_id, s.video_id, s.share_link_id) \
                end as access_state \
             from playback_sessions s join videos v on v.id=s.video_id \
             where s.id=$1 and s.ended_at is null \
         ), ended as ( \
             update playback_sessions s set ended_at=now(), end_reason=c.access_state \
             from candidate c where s.id=c.id and c.access_state <> 'allowed' \
         ) \
         update playback_sessions s set last_seen_at=now() \
         from candidate c where s.id=c.id and c.access_state='allowed' \
         returning c.user_id, c.video_id, c.expires_at, c.renditions",
    )
    .bind(session_id)
    .fetch_optional(db)
    .await
}

#[derive(sqlx::FromRow)]
pub struct SessionState {
    pub ended: bool,
    pub end_reason: Option<String>,
    pub expired: bool,
    pub duration_seconds: Option<i32>,
    pub lesson_id: Option<Uuid>,
    pub video_id: Uuid,
}

pub async fn state_for_user<'e>(
    db: impl PgExecutor<'e>,
    session_id: Uuid,
    user_id: Uuid,
) -> Result<Option<SessionState>, sqlx::Error> {
    sqlx::query_as(
        "select s.ended_at is not null as ended, s.end_reason, s.expires_at <= now() as expired, \
                v.duration_seconds, s.lesson_id, s.video_id \
         from playback_sessions s join videos v on v.id = s.video_id \
         where s.id = $1 and s.user_id = $2",
    )
    .bind(session_id)
    .bind(user_id)
    .fetch_optional(db)
    .await
}

/// Records a heartbeat only while both the session and the viewer's entitlement are live.
pub async fn heartbeat<'e>(
    db: impl PgExecutor<'e>,
    session_id: Uuid,
    user_id: Uuid,
    position: Option<i32>,
) -> Result<bool, sqlx::Error> {
    let result = sqlx::query(
        "with candidate as materialized ( \
             select s.id, case \
                 when s.expires_at <= now() then 'expired' \
                 when v.deleted_at is not null then 'video_deleted' \
                 when v.status <> 'ready' then 'access_revoked' \
                 else playback_access_state(s.user_id, s.video_id, s.share_link_id) \
             end as access_state \
             from playback_sessions s join videos v on v.id=s.video_id \
             where s.id=$1 and s.user_id=$2 and s.ended_at is null \
         ), ended as ( \
             update playback_sessions s set ended_at=now(), end_reason=c.access_state \
             from candidate c where s.id=c.id and c.access_state <> 'allowed' \
         ) \
         update playback_sessions s set last_seen_at=now(), \
             last_position_seconds=coalesce($3, last_position_seconds) \
         from candidate c where s.id=c.id and c.access_state='allowed'",
    )
    .bind(session_id)
    .bind(user_id)
    .bind(position)
    .execute(db)
    .await?;
    Ok(result.rows_affected() == 1)
}

pub async fn end<'e>(
    db: impl PgExecutor<'e>,
    session_id: Uuid,
    user_id: Uuid,
    reason: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "update playback_sessions set ended_at = now(), end_reason = $3 \
         where id = $1 and user_id = $2 and ended_at is null",
    )
    .bind(session_id)
    .bind(user_id)
    .bind(reason)
    .execute(db)
    .await?;
    Ok(())
}

pub async fn end_all_for_user<'e>(
    db: impl PgExecutor<'e>,
    user_id: Uuid,
    reason: &str,
) -> Result<u64, sqlx::Error> {
    Ok(sqlx::query(
        "update playback_sessions set ended_at = now(), end_reason = $2 \
         where user_id = $1 and ended_at is null",
    )
    .bind(user_id)
    .bind(reason)
    .execute(db)
    .await?
    .rows_affected())
}

pub async fn end_all_for_video<'e>(
    db: impl PgExecutor<'e>,
    video_id: Uuid,
    reason: &str,
) -> Result<u64, sqlx::Error> {
    Ok(sqlx::query(
        "update playback_sessions set ended_at = now(), end_reason = $2 \
         where video_id = $1 and ended_at is null",
    )
    .bind(video_id)
    .bind(reason)
    .execute(db)
    .await?
    .rows_affected())
}

/// Ends every open viewing through one share link (it was revoked).
pub async fn end_for_link<'e>(
    db: impl PgExecutor<'e>,
    link_id: Uuid,
    reason: &str,
) -> Result<u64, sqlx::Error> {
    Ok(sqlx::query(
        "update playback_sessions set ended_at = now(), end_reason = $2 \
         where share_link_id = $1 and ended_at is null",
    )
    .bind(link_id)
    .bind(reason)
    .execute(db)
    .await?
    .rows_affected())
}

/// Ends every open viewing through any link of a recording (it went private, or was deleted).
pub async fn end_for_recording_links<'e>(
    db: impl PgExecutor<'e>,
    recording_id: Uuid,
    reason: &str,
) -> Result<u64, sqlx::Error> {
    Ok(sqlx::query(
        "update playback_sessions set ended_at = now(), end_reason = $2 \
         where ended_at is null and share_link_id in \
             (select id from share_links where recording_id = $1)",
    )
    .bind(recording_id)
    .bind(reason)
    .execute(db)
    .await?
    .rows_affected())
}

/// Hourly housekeeping: closes expired sessions and forgets sessions that ended long ago.
pub async fn sweep<'e, E>(db: E, keep_days: i32) -> Result<(u64, u64), sqlx::Error>
where
    E: PgExecutor<'e> + Copy,
{
    let expired = sqlx::query(
        "update playback_sessions set ended_at = now(), end_reason = 'expired' \
         where ended_at is null and expires_at <= now()",
    )
    .execute(db)
    .await?
    .rows_affected();
    // Sessions of signed-in viewers are kept for resume points and abuse investigations. A link
    // viewing has neither, and a popular link makes a great many of them: keep those a week.
    let forgotten = sqlx::query(
        "delete from playback_sessions where ended_at < now() - \
             case when share_link_id is null then make_interval(days => $1) else interval '7 days' end",
    )
    .bind(keep_days)
    .execute(db)
    .await?
    .rows_affected();
    Ok((expired, forgotten))
}
