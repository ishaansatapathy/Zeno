-- Refunds, reconciliation bookkeeping and the refund window. Expand-only.

-- How long after paying a student may ask for their money back. A creator's choice per course;
-- the value in force at purchase time is copied into the order, so changing it later cannot
-- shorten (or lengthen) what a buyer was promised. 0 means no refunds.
alter table courses
    add column refund_window_days integer not null default 7,
    add constraint courses_refund_window_check check (refund_window_days between 0 and 365);

alter table orders
    -- When the order became paid. The refund window is counted from here.
    add column paid_at timestamptz,
    -- The reconciliation job looks at an order again no sooner than this (backoff, and a short
    -- claim while one instance is working on it).
    add column reconcile_after timestamptz,
    add column reconcile_failures integer not null default 0,
    add constraint orders_reconcile_failures_check check (reconcile_failures >= 0);

update orders set paid_at = updated_at where status in ('paid', 'refunded') and paid_at is null;

create index orders_reconcile_idx
    on orders (reconcile_after nulls first, created_at)
    where status in ('created', 'pending', 'expired');

create type refund_status as enum ('requested', 'declined', 'processing', 'succeeded', 'failed');

-- Every refund, asked for by a student, started by a creator or admin, or made by someone in the
-- provider's dashboard. A refund is final only when the provider says so.
create table refunds (
    id uuid primary key default uuid_generate_v4(),
    order_id uuid not null references orders(id),
    payment_id uuid not null references payments(id),
    amount_paise bigint not null check (amount_paise > 0),
    status refund_status not null,
    origin text not null check (origin in ('student', 'creator', 'admin', 'provider')),
    requested_by uuid references users(id) on delete set null,
    decided_by uuid references users(id) on delete set null,
    -- Why it was asked for or granted, in the asker's words.
    reason text check (reason is null or char_length(reason) between 1 and 300),
    -- Why it was declined, or why the provider could not complete it.
    decision_note text check (decision_note is null or char_length(decision_note) between 1 and 300),
    provider_refund_id text,
    -- Times we asked the provider to make it (the same idempotency key every time).
    attempts integer not null default 0 check (attempts >= 0),
    requested_at timestamptz not null default now(),
    decided_at timestamptz,
    completed_at timestamptz,
    updated_at timestamptz not null default now(),
    reconcile_after timestamptz,
    reconcile_failures integer not null default 0 check (reconcile_failures >= 0),
    constraint refunds_decision_check check (
        status <> 'declined' or (decision_note is not null and decided_at is not null)
    ),
    constraint refunds_completion_check check (
        (status = 'succeeded') = (completed_at is not null)
    )
);

-- One refund in flight per order: a request, or one the provider is making. Asking twice returns
-- the same one.
create unique index refunds_one_active_per_order
    on refunds (order_id)
    where status in ('requested', 'processing');

create unique index refunds_provider_refund_unique
    on refunds (provider_refund_id)
    where provider_refund_id is not null;

create index refunds_order_idx on refunds (order_id, requested_at desc);
create index refunds_reconcile_idx
    on refunds (reconcile_after nulls first, updated_at)
    where status = 'processing';
