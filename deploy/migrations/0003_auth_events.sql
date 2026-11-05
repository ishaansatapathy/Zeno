-- Append-only record of security-relevant account activity. user_id survives as null if the
-- account is deleted so the trail stays. Emails are stored only as a SHA-256 so failed
-- attempts against unknown addresses can be correlated without keeping the raw address.
create table auth_events (
    id bigint generated always as identity primary key,
    user_id uuid references users(id) on delete set null,
    email_hash bytea,
    kind text not null,
    family_id uuid,
    ip inet,
    user_agent text,
    created_at timestamptz not null default now()
);

create index auth_events_user_idx on auth_events (user_id, created_at desc);
create index auth_events_email_idx on auth_events (email_hash, created_at desc);
create index auth_events_created_idx on auth_events (created_at);
