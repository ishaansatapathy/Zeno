-- Share links: anyone with the link can watch a recording, no account needed.
--
-- A link is a capability: its code is 22 characters of URL-safe base64 (128 random bits), so it
-- cannot be guessed, and whoever holds it may watch until it is revoked, expires, or the owner
-- switches the recording back to private. An optional password is checked (Argon2id) before a
-- viewing session starts.
create table share_links (
    id uuid primary key,
    code text not null,
    recording_id uuid not null references recordings(id) on delete cascade,
    password_hash text,
    expires_at timestamptz,
    revoked_at timestamptz,
    created_at timestamptz not null default now(),
    constraint share_links_code_format check (code ~ '^[A-Za-z0-9_-]{22}$')
);

-- The one lookup every visit makes.
create unique index share_links_code_unique on share_links (code);
-- A recording's links, newest first (the owner's list, and the cap on how many are live).
create index share_links_recording_idx on share_links (recording_id, created_at desc);

-- A viewer with no account has a playback session too, tied to the link instead of a user. The
-- same session machinery (token, playlists, key server, revocation) then serves both.
--
-- Rolling deploys: until every API and key-server instance runs the new code, an old instance
-- that meets an anonymous session cannot read its null user and refuses it. Roll both services
-- out before anyone creates a link.
alter table playback_sessions
    alter column user_id drop not null,
    add column share_link_id uuid references share_links(id) on delete cascade,
    add constraint playback_sessions_viewer_check
        check ((user_id is not null) <> (share_link_id is not null));

create index playback_sessions_link_open_idx
    on playback_sessions (share_link_id)
    where share_link_id is not null and ended_at is null;

-- The single source of truth for "may this session keep playing?". For a signed-in viewer it is
-- exactly the existing course rule. For a link viewer: the link must be live, the recording must
-- still exist, still be shared (not private) and still be made of this video.
create function playback_access_state(p_user_id uuid, p_video_id uuid, p_link_id uuid)
returns text
language sql
stable
security invoker
set search_path = public
as $$
    select case
        when p_link_id is null then video_access_state(p_user_id, p_video_id)
        when exists (
            select 1
            from share_links l
            join recordings r on r.id = l.recording_id
            where l.id = p_link_id
              and r.video_id = p_video_id
              and r.deleted_at is null
              and r.visibility in ('link', 'public')
              and l.revoked_at is null
              and (l.expires_at is null or l.expires_at > now())
        ) then 'allowed'
        when exists (
            select 1 from share_links l
            where l.id = p_link_id and l.revoked_at is null and l.expires_at <= now()
        ) then 'expired'
        else 'access_revoked'
    end
$$;
