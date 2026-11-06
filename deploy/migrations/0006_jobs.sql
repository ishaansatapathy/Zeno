-- Durable job queue. Enqueueing happens in the same transaction as the state change that
-- needs the work, so a job is never lost or created for a change that rolled back.
create table jobs (
    id uuid primary key default uuid_generate_v4(),
    kind text not null,
    payload jsonb not null default '{}',
    attempts integer not null default 0,
    max_attempts integer not null default 5,
    run_at timestamptz not null default now(),
    locked_until timestamptz,
    completed_at timestamptz,
    failed_at timestamptz,
    last_error text,
    created_at timestamptz not null default now()
);

create index jobs_due_idx on jobs (run_at) where completed_at is null and failed_at is null;

-- A video can have at most one live transcode job, however many times completion is retried.
create unique index jobs_one_live_transcode_per_video
    on jobs ((payload ->> 'video_id'))
    where kind = 'transcode' and completed_at is null and failed_at is null;
