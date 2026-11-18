-- Recordings: what a person sees in their library, whatever made it. A recording is a thin row
-- on top of a video: the extension, the studio, a live stream and a meeting all produce one, so
-- everything that is later said about "my recordings" (sharing, comments, editing) hangs here.
--
-- Expand-only. Nothing existing changes.
create table recordings (
    -- The client may choose it (a random UUID), which makes creating a recording safe to retry
    -- after a dropped connection: the second attempt finds the first.
    id uuid primary key,
    owner_id uuid not null references users(id) on delete cascade,
    -- Empty while the recording is still being captured; set once an upload exists. If the video
    -- row is ever removed outright (an aborted upload) the recording goes back to "capturing".
    video_id uuid references videos(id) on delete set null,
    source text not null,
    title text not null,
    -- Who may watch. Only the owner can until share links exist: nothing reads this yet.
    visibility text not null default 'private',
    version integer not null default 1,
    created_at timestamptz not null default now(),
    updated_at timestamptz not null default now(),
    deleted_at timestamptz,
    constraint recordings_source_check check (
        source in ('upload', 'extension', 'studio', 'stream', 'meeting')
    ),
    constraint recordings_visibility_check check (visibility in ('private', 'link', 'public')),
    constraint recordings_title_length check (char_length(title) between 1 and 120),
    constraint recordings_version_positive check (version > 0)
);

-- A video belongs to at most one live recording.
create unique index recordings_video_unique
    on recordings (video_id)
    where video_id is not null and deleted_at is null;

-- The library: one person's recordings, newest first, keyset-paginated.
create index recordings_owner_list_idx
    on recordings (owner_id, created_at desc, id desc)
    where deleted_at is null;
