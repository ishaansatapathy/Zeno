-- View counts for recordings, written in batches, never one row per view.
--
-- The hot path counts in Redis (see zeno_core::views); a flush moves the counts here once a
-- minute, one row per recording and day. Nothing is written when a person presses play.
--
-- Expand-only. Nothing existing changes (the new column has a constant default).

-- All-time views, kept on the recording so a library page shows them with no join. Written only
-- by the flush, once per recording per batch, and it deliberately does not touch `version` or
-- `updated_at`: a view is not an edit, and must never make an editor's copy stale.
alter table recordings
    add column view_count bigint not null default 0,
    add constraint recordings_view_count_check check (view_count >= 0);

-- One row per recording per UTC day. `viewers` is the day's unique viewers: an estimate
-- (HyperLogLog, about 1% off), and per day, because unique counts cannot be added across days.
create table recording_views (
    recording_id uuid not null references recordings(id) on delete cascade,
    day date not null,
    views bigint not null default 0,
    viewers bigint not null default 0,
    primary key (recording_id, day),
    constraint recording_views_nonnegative check (views >= 0 and viewers >= 0)
);

-- The ids of the batches already applied. A flush records its batch id in the same transaction
-- as the counts, so a batch found again after a crash (or picked up by two instances at once)
-- is applied once, never twice.
create table view_flushes (
    batch_id uuid primary key,
    applied_at timestamptz not null default now()
);

create index view_flushes_applied_idx on view_flushes (applied_at);
