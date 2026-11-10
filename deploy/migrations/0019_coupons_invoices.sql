-- Coupons, and invoices with gapless numbering. Expand-only.

create type coupon_kind as enum ('percent', 'flat');

create table coupons (
    id uuid primary key default uuid_generate_v4(),
    creator_id uuid not null references users(id),
    -- Stored in one canonical form (upper case, no spaces), so "launch50", " Launch50 " and
    -- "LAUNCH50" are the same code.
    code text not null,
    kind coupon_kind not null,
    -- A whole percent for `percent`; paise for `flat`.
    value bigint not null,
    -- One of the creator's courses, or null for all of them.
    course_id uuid references courses(id),
    valid_from timestamptz,
    valid_until timestamptz,
    max_redemptions integer,
    per_user_limit integer not null default 1,
    -- Redemptions in use (not released). Incremented by a conditional update, so the limit holds
    -- however many buyers arrive at once.
    redeemed_count integer not null default 0,
    active boolean not null default true,
    created_at timestamptz not null default now(),
    updated_at timestamptz not null default now(),
    constraint coupons_code_format check (code ~ '^[A-Z0-9][A-Z0-9_-]{2,31}$'),
    constraint coupons_value_check check (
        (kind = 'percent' and value between 1 and 100)
        or (kind = 'flat' and value between 100 and 100000000)
    ),
    constraint coupons_window_check check (
        valid_from is null or valid_until is null or valid_until > valid_from
    ),
    constraint coupons_max_check check (max_redemptions is null or max_redemptions >= 1),
    constraint coupons_per_user_check check (per_user_limit between 1 and 100),
    constraint coupons_count_check check (redeemed_count >= 0),
    constraint coupons_unique_code unique (creator_id, code)
);

create index coupons_creator_idx on coupons (creator_id, created_at desc, id desc);

-- Who used a coupon, on which order (or on which free enrollment, when the coupon made the course
-- free). A redemption on an order that expires unpaid is released and can be used again.
create table coupon_redemptions (
    id uuid primary key default uuid_generate_v4(),
    coupon_id uuid not null references coupons(id),
    user_id uuid not null references users(id),
    order_id uuid unique references orders(id),
    enrollment_id uuid references enrollments(id),
    discount_paise bigint not null check (discount_paise >= 0),
    released_at timestamptz,
    created_at timestamptz not null default now(),
    constraint coupon_redemptions_target_check check ((order_id is null) <> (enrollment_id is null))
);

create index coupon_redemptions_use_idx
    on coupon_redemptions (coupon_id, user_id)
    where released_at is null;

-- Invoices are numbered 1, 2, 3 ... within each financial year (April to March, Indian time),
-- with no gaps: the counter is advanced in the same transaction that writes the invoice, so a
-- rolled-back invoice gives its number back.
create table invoice_counters (
    fy text primary key,
    last_sequence integer not null check (last_sequence >= 0)
);

create table invoices (
    id uuid primary key default uuid_generate_v4(),
    order_id uuid not null unique references orders(id),
    user_id uuid not null references users(id),
    fy text not null,
    sequence integer not null check (sequence > 0),
    number text not null unique,
    amount_paise bigint not null check (amount_paise > 0),
    currency text not null default 'INR',
    -- Everything printed on it, copied when it was issued: the seller, the buyer, the lines.
    data jsonb not null,
    issued_at timestamptz not null default now(),
    constraint invoices_fy_sequence_unique unique (fy, sequence)
);

create index invoices_user_idx on invoices (user_id, issued_at desc);

-- An invoice, once issued, is never edited or removed. A correction is a new document.
create function reject_invoice_change() returns trigger
language plpgsql as $$
begin
    raise exception 'invoices cannot be changed or removed';
end;
$$;

create trigger invoices_immutable
    before update or delete on invoices
    for each row execute function reject_invoice_change();

create trigger invoices_no_truncate
    before truncate on invoices
    for each statement execute function reject_invoice_change();
