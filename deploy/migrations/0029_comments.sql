-- Comments and reactions on a recording, left by people watching it through a share link.
--
-- Two kinds. A comment is words, optionally pinned to a moment of the video. A reaction is one emoji from
-- a short list, always pinned to a moment (that is the point of it). Anyone who may watch can read them;
-- only a signed-in person can leave one, so there is someone to hold to account, and the owner (or the
-- author) can remove any of them. They belong to the recording, not to a link: revoking a link does not
-- take the conversation with it.
--
-- Expand-only: an old binary does not know the column or the table and never touches them.
alter table recordings add column comments_enabled boolean not null default true;

create table recording_comments (
    id uuid primary key,
    recording_id uuid not null references recordings(id) on delete cascade,
    author_id uuid not null references users(id) on delete cascade,
    kind text not null check (kind in ('comment', 'reaction')),
    body text,
    emoji text,
    -- Where in the video, in milliseconds. A comment may be about the whole video (null); a reaction may not.
    at_ms integer check (at_ms is null or at_ms >= 0),
    created_at timestamptz not null default now(),
    constraint recording_comments_shape check (
        (kind = 'comment' and body is not null and char_length(body) between 1 and 1000 and emoji is null)
        or (kind = 'reaction' and body is null and emoji is not null and char_length(emoji) <= 16 and at_ms is not null)
    )
);

-- The feed: newest first, a page at a time (keyset on the same two columns).
create index recording_comments_feed_idx on recording_comments (recording_id, created_at desc, id desc);
-- How many one person has left on one recording (the cap that stops one account filling a page).
create index recording_comments_author_idx on recording_comments (recording_id, author_id);
