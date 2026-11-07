-- One row per time someone presses play. The playback token names a row here, and both the API
-- (playlists) and the key server (content keys) refuse a token whose session has ended, so ending
-- a session takes effect at once. The placeholder table from 0001 was never used.
drop table playback_sessions;

create table playback_sessions (
    id uuid primary key,
    user_id uuid not null references users(id) on delete cascade,
    video_id uuid not null references videos(id) on delete cascade,
    -- User agent, truncated; shown to the viewer when they hit the device limit.
    device text not null default '',
    ip inet,
    started_at timestamptz not null default now(),
    -- Moved forward by heartbeats, playlist loads and key fetches. A session not seen for a few
    -- minutes no longer counts toward the concurrent-stream limit.
    last_seen_at timestamptz not null default now(),
    expires_at timestamptz not null,
    ended_at timestamptz,
    end_reason text check (end_reason in ('stopped', 'idle', 'expired', 'password_reset', 'video_deleted')),
    last_position_seconds integer not null default 0 check (last_position_seconds >= 0),
    key_fetches integer not null default 0,
    check ((ended_at is null) = (end_reason is null))
);

create index playback_sessions_open_idx on playback_sessions (user_id) where ended_at is null;
create index playback_sessions_resume_idx on playback_sessions (user_id, video_id, last_seen_at desc);
create index playback_sessions_ended_idx on playback_sessions (ended_at) where ended_at is not null;
