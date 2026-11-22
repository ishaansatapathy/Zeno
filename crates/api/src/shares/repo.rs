use chrono::{DateTime, Utc};
use sqlx::{PgConnection, PgExecutor};
use uuid::Uuid;
use zeno_core::model::VideoStatus;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct LinkRow {
    pub id: Uuid,
    pub code: String,
    pub recording_id: Uuid,
    pub password_hash: Option<String>,
    pub expires_at: Option<DateTime<Utc>>,
    pub revoked_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

const LINK_COLUMNS: &str =
    "id, code, recording_id, password_hash, expires_at, revoked_at, created_at";

pub async fn find_by_id<'e>(
    db: impl PgExecutor<'e>,
    id: Uuid,
) -> Result<Option<LinkRow>, sqlx::Error> {
    sqlx::query_as(&format!(
        "select {LINK_COLUMNS} from share_links where id = $1"
    ))
    .bind(id)
    .fetch_optional(db)
    .await
}

/// Makes a link. A unique violation means the code is taken (a one-in-2^128 event, but the caller
/// retries with a new one rather than assume).
pub async fn insert(
    db: &mut PgConnection,
    id: Uuid,
    code: &str,
    recording_id: Uuid,
    password_hash: Option<&str>,
    expires_at: Option<DateTime<Utc>>,
) -> Result<LinkRow, sqlx::Error> {
    sqlx::query_as(&format!(
        "insert into share_links (id, code, recording_id, password_hash, expires_at) \
         values ($1, $2, $3, $4, $5) returning {LINK_COLUMNS}"
    ))
    .bind(id)
    .bind(code)
    .bind(recording_id)
    .bind(password_hash)
    .bind(expires_at)
    .fetch_one(db)
    .await
}

/// A recording's links, newest first, up to `limit`.
pub async fn list_for_recording<'e>(
    db: impl PgExecutor<'e>,
    recording_id: Uuid,
    limit: i64,
) -> Result<Vec<LinkRow>, sqlx::Error> {
    sqlx::query_as(&format!(
        "select {LINK_COLUMNS} from share_links where recording_id = $1 \
         order by created_at desc, id desc limit $2"
    ))
    .bind(recording_id)
    .bind(limit)
    .fetch_all(db)
    .await
}

/// How many of a recording's links can still be used to watch.
pub async fn count_live<'e>(
    db: impl PgExecutor<'e>,
    recording_id: Uuid,
) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar(
        "select count(*) from share_links where recording_id = $1 \
           and revoked_at is null and (expires_at is null or expires_at > now())",
    )
    .bind(recording_id)
    .fetch_one(db)
    .await
}

/// Revokes a link of this recording. `false`: no such link here. Revoking twice is not an error
/// and keeps the first revocation time.
pub async fn revoke<'e>(
    db: impl PgExecutor<'e>,
    link_id: Uuid,
    recording_id: Uuid,
) -> Result<bool, sqlx::Error> {
    let found: Option<(Uuid,)> = sqlx::query_as(
        "update share_links set revoked_at = coalesce(revoked_at, now()) \
         where id = $1 and recording_id = $2 returning id",
    )
    .bind(link_id)
    .bind(recording_id)
    .fetch_optional(db)
    .await?;
    Ok(found.is_some())
}

/// Everything a visit to a link needs, in one indexed lookup by code.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Target {
    pub link_id: Uuid,
    pub recording_id: Uuid,
    pub password_hash: Option<String>,
    pub expires_at: Option<DateTime<Utc>>,
    pub revoked_at: Option<DateTime<Utc>>,
    pub title: String,
    pub visibility: String,
    pub recording_deleted: bool,
    pub video_id: Option<Uuid>,
    pub video_status: Option<VideoStatus>,
    pub duration_seconds: Option<i32>,
}

impl Target {
    /// Whether the link can be used at all right now: not revoked, not expired, and the recording
    /// still exists and is still shared. (The database makes the same call, per request, for a
    /// viewing already under way: `playback_access_state`.)
    pub fn is_live(&self, now: DateTime<Utc>) -> bool {
        self.revoked_at.is_none()
            && self.expires_at.is_none_or(|at| at > now)
            && !self.recording_deleted
            && matches!(self.visibility.as_str(), "link" | "public")
    }
}

pub async fn find_by_code<'e>(
    db: impl PgExecutor<'e>,
    code: &str,
) -> Result<Option<Target>, sqlx::Error> {
    sqlx::query_as(
        "select l.id as link_id, l.recording_id, l.password_hash, l.expires_at, l.revoked_at, \
                r.title, r.visibility, (r.deleted_at is not null) as recording_deleted, \
                r.video_id, v.status as video_status, v.duration_seconds \
         from share_links l \
         join recordings r on r.id = l.recording_id \
         left join videos v on v.id = r.video_id and v.deleted_at is null \
         where l.code = $1",
    )
    .bind(code)
    .fetch_optional(db)
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    fn target() -> Target {
        Target {
            link_id: Uuid::new_v4(),
            recording_id: Uuid::new_v4(),
            password_hash: None,
            expires_at: None,
            revoked_at: None,
            title: "t".into(),
            visibility: "link".into(),
            recording_deleted: false,
            video_id: None,
            video_status: None,
            duration_seconds: None,
        }
    }

    #[test]
    fn a_link_is_live_only_while_nothing_has_closed_it() {
        let now = Utc::now();
        assert!(target().is_live(now));
        let mut t = target();
        t.visibility = "public".into();
        assert!(t.is_live(now));

        let mut t = target();
        t.revoked_at = Some(now);
        assert!(!t.is_live(now), "revoked");
        let mut t = target();
        t.expires_at = Some(now - Duration::seconds(1));
        assert!(!t.is_live(now), "expired");
        t.expires_at = Some(now + Duration::seconds(1));
        assert!(t.is_live(now), "not yet");
        let mut t = target();
        t.visibility = "private".into();
        assert!(
            !t.is_live(now),
            "switched off by making the recording private"
        );
        let mut t = target();
        t.recording_deleted = true;
        assert!(!t.is_live(now), "recording deleted");
    }
}
