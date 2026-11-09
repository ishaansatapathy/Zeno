-- Webhooks: every event the provider sends is stored once, and what happened to money is
-- recorded in a trail nobody edits. Expand-only.

-- A second successful payment for an order that is already paid is real money that has to be
-- found and returned; it gets its own status rather than pretending to be a normal payment.
-- (Added here, used only by later statements, as PostgreSQL requires.)
alter type payment_status add value if not exists 'duplicate';

-- At most one confirmed payment per order: the database refuses a second even if code is wrong.
create unique index payments_one_confirmed_per_order
    on payments (order_id)
    where status = 'confirmed' and order_id is not null;

create table webhook_events (
    id uuid primary key default uuid_generate_v4(),
    provider text not null,
    -- The provider's id for the event; the same event redelivered carries the same id.
    provider_event_id text not null,
    event_type text not null,
    payload jsonb not null,
    -- The order the event was about, when we could tell.
    order_id uuid references orders(id),
    -- What was done with it: enrolled, duplicate_payment, amount_mismatch, ignored, ...
    outcome text not null,
    -- Money moved in a way a person has to look at (a payment that does not match its order,
    -- a second payment, a refund we did not make, a payment for access that was blocked).
    needs_review boolean not null default false,
    reviewed_at timestamptz,
    received_at timestamptz not null default now(),
    processed_at timestamptz not null default now(),
    constraint webhook_events_id_check check (char_length(provider_event_id) between 1 and 200),
    constraint webhook_events_unique unique (provider, provider_event_id)
);

create index webhook_events_review_idx
    on webhook_events (received_at)
    where needs_review and reviewed_at is null;
create index webhook_events_order_idx on webhook_events (order_id) where order_id is not null;

-- What happened to an order, in order. Rows are only ever added.
create table order_events (
    id uuid primary key default uuid_generate_v4(),
    order_id uuid not null references orders(id) on delete cascade,
    kind text not null check (char_length(kind) between 1 and 40),
    payment_id uuid references payments(id),
    details jsonb not null default '{}'::jsonb,
    created_at timestamptz not null default now()
);

create index order_events_order_idx on order_events (order_id, created_at);
