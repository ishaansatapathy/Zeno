-- Orders: the intention to buy a course, priced by the server at checkout time. `payments` (from
-- the first schema) stays what the provider tells us happened, and now points at its order.
-- Expand-only: nothing is dropped or renamed.

create type order_status as enum ('created', 'pending', 'paid', 'failed', 'expired', 'refunded');

create table orders (
    id uuid primary key default uuid_generate_v4(),
    user_id uuid not null references users(id),
    course_id uuid not null references courses(id),
    -- What the customer is asked to pay, in paise, fixed when the order is made. The course's
    -- price may change afterwards; this order does not.
    amount_paise bigint not null,
    currency text not null default 'INR',
    -- Everything shown on the receipt, copied at order time (title, list price, discount).
    price_snapshot jsonb not null,
    -- created: our row exists, the provider has not been asked yet (or we do not know the answer).
    -- pending: the provider order exists and the customer may pay.
    status order_status not null default 'created',
    provider text not null,
    provider_order_id text,
    -- The caller's Idempotency-Key for the checkout request, scoped to the user.
    idempotency_key text,
    -- How many times we asked the provider to create the order. More than one means an earlier
    -- attempt ended without an answer, so look the order up before creating again.
    provider_attempts integer not null default 0,
    -- While set and in the future, one request is talking to the provider for this order.
    attempt_lease_until timestamptz,
    expires_at timestamptz not null,
    created_at timestamptz not null default now(),
    updated_at timestamptz not null default now(),
    constraint orders_amount_check check (amount_paise between 100 and 100000000),
    constraint orders_currency_check check (currency = 'INR'),
    constraint orders_snapshot_check check (jsonb_typeof(price_snapshot) = 'object'),
    constraint orders_key_check check (
        idempotency_key is null or char_length(idempotency_key) between 1 and 64
    ),
    -- A paid, pending or refunded order always names the provider's order it is about.
    constraint orders_provider_order_check check (
        status in ('created', 'failed', 'expired') or provider_order_id is not null
    ),
    constraint orders_attempts_check check (provider_attempts >= 0)
);

-- A provider's order id belongs to exactly one of our orders.
create unique index orders_provider_order_unique
    on orders (provider, provider_order_id)
    where provider_order_id is not null;

-- A user has at most one open order per course: double clicks and second tabs share it.
create unique index orders_one_open_per_user_course
    on orders (user_id, course_id)
    where status in ('created', 'pending');

create unique index orders_idempotency_unique
    on orders (user_id, idempotency_key)
    where idempotency_key is not null;

create index orders_user_list_idx on orders (user_id, created_at desc, id desc);
create index orders_course_idx on orders (course_id);
-- For the job that expires stale orders.
create index orders_open_expiry_idx on orders (expires_at) where status in ('created', 'pending');

alter table payments
    add column order_id uuid references orders(id),
    add column provider text,
    add column currency text not null default 'INR',
    add column updated_at timestamptz not null default now();

create index payments_order_idx on payments (order_id) where order_id is not null;
