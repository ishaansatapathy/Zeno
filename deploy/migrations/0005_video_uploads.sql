-- A video row is created when an upload starts and tracks it through processing.
-- size and part_size are fixed up front: every part's exact length is signed into its upload
-- URL, so the finished object can only be exactly size_bytes.
alter table videos
    add column owner_id uuid not null references users(id) on delete cascade,
    add column title text not null default '',
    add column original_filename text not null,
    add column content_type text not null,
    add column size_bytes bigint not null check (size_bytes > 0),
    add column part_size bigint not null check (part_size >= 5242880),
    add column source_key text not null,
    add column upload_id text,
    add column error_code text,
    add column width integer,
    add column height integer,
    add column updated_at timestamptz not null default now(),
    add column uploaded_at timestamptz,
    add column ready_at timestamptz;

create index videos_owner_idx on videos (owner_id, created_at desc);
create index videos_stale_upload_idx on videos (created_at) where status = 'uploading';

-- AES-128 content keys. Kept apart from the segments they protect, and never served except
-- through the key server.
create table video_keys (
    video_id uuid primary key references videos(id) on delete cascade,
    key bytea not null check (octet_length(key) = 16),
    created_at timestamptz not null default now()
);
