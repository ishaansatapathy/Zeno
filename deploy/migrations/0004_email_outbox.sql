-- Emails are written here first and delivered afterwards, so a mail-provider outage delays
-- them instead of losing them. Bodies hold live one-time links, so they are blanked the
-- moment a row is sent or given up on, and rows stop retrying once the link would expire.
create table email_outbox (
    id uuid primary key default uuid_generate_v4(),
    to_address text not null,
    subject text not null,
    body text not null,
    attempts integer not null default 0,
    next_attempt_at timestamptz not null default now(),
    expires_at timestamptz not null,
    sent_at timestamptz,
    failed_at timestamptz,
    last_error text,
    created_at timestamptz not null default now()
);

create index email_outbox_due_idx on email_outbox (next_attempt_at)
    where sent_at is null and failed_at is null;
