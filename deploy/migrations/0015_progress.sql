-- Per-lesson progress. Playback sessions are per video and are swept after 90 days, but a
-- student's place in a lesson has to outlive them, and the same video can be a lesson in several
-- courses, so progress is kept per lesson.
alter table playback_sessions
    add column lesson_id uuid references lessons(id) on delete set null;

create table lesson_progress (
    user_id uuid not null references users(id) on delete cascade,
    lesson_id uuid not null references lessons(id) on delete cascade,
    -- Where the viewer was at the last heartbeat: what "resume" restores.
    last_position_seconds integer not null default 0 check (last_position_seconds >= 0),
    -- The furthest point reached, however it was reached.
    furthest_seconds integer not null default 0 check (furthest_seconds >= 0),
    -- Credible watch time: forward movement the clock can account for. Seeking ahead earns
    -- nothing, so completion cannot be claimed by dragging the slider.
    watched_seconds integer not null default 0 check (watched_seconds >= 0),
    completed_at timestamptz,
    last_watched_at timestamptz not null default now(),
    created_at timestamptz not null default now(),
    primary key (user_id, lesson_id)
);

-- "Continue watching": a user's unfinished lessons, most recent first.
create index lesson_progress_recent_idx
    on lesson_progress (user_id, last_watched_at desc)
    where completed_at is null;
