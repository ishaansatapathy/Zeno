-- The creator earnings ledger and payouts. Expand-only.
--
-- A double-entry book: every event that moves money is a *journal* whose *entries* add up to
-- exactly zero, so money is never created or lost by a bug, only moved between accounts. Entries
-- are never edited or removed; a mistake is corrected by a new journal.
--
-- Accounts (an entry's `account`):
--   creator:<id>   what Zeno owes that creator (positive) or the creator owes Zeno (negative)
--   customers      the pool customers paid into (negative = money taken in)
--   gateway_fees   what the payment provider kept as its fee
--   payouts        what has been paid, or committed to be paid, out to creators
--   adjustments    manual corrections made by an admin
--   zeno_fees, tax reserved; unused while Zeno takes no share and issues no tax lines

create table ledger_journals (
    id uuid primary key default uuid_generate_v4(),
    kind text not null check (
        kind in ('sale', 'gateway_fee', 'refund', 'payout', 'payout_reversal', 'adjustment')
    ),
    -- The thing the journal is about: the payment (sale, gateway_fee), the refund, the payout or
    -- the adjustment. A journal exists once per (kind, source), so posting the same event twice,
    -- from a retried webhook or a reconciliation pass, is a no-op.
    source_id uuid not null,
    order_id uuid references orders(id),
    memo text check (memo is null or char_length(memo) between 1 and 300),
    created_at timestamptz not null default now(),
    constraint ledger_journals_source_unique unique (kind, source_id)
);

create index ledger_journals_order_idx on ledger_journals (order_id) where order_id is not null;

create table ledger_entries (
    id uuid primary key default uuid_generate_v4(),
    journal_id uuid not null references ledger_journals(id),
    account text not null,
    -- Set exactly for creator accounts, so a creator's entries are one index away.
    creator_id uuid references users(id),
    amount_paise bigint not null check (amount_paise <> 0),
    created_at timestamptz not null default now(),
    constraint ledger_entries_account_check check (
        (creator_id is null and account in
            ('customers', 'gateway_fees', 'payouts', 'adjustments', 'zeno_fees', 'tax'))
        or (creator_id is not null and account = 'creator:' || creator_id::text)
    )
);

create index ledger_entries_journal_idx on ledger_entries (journal_id);
create index ledger_entries_creator_idx
    on ledger_entries (creator_id, created_at desc, id desc)
    where creator_id is not null;

-- ---- the rules the book keeps for itself ---------------------------------------------------------

-- Never edited, never removed.
create function ledger_is_append_only() returns trigger
language plpgsql as $$
begin
    raise exception 'the ledger is append-only: post a correcting journal instead';
end;
$$;

create trigger ledger_journals_append_only
    before update or delete on ledger_journals
    for each row execute function ledger_is_append_only();
create trigger ledger_journals_no_truncate
    before truncate on ledger_journals
    for each statement execute function ledger_is_append_only();
create trigger ledger_entries_append_only
    before update or delete on ledger_entries
    for each row execute function ledger_is_append_only();
create trigger ledger_entries_no_truncate
    before truncate on ledger_entries
    for each statement execute function ledger_is_append_only();

-- Entries join a journal in the transaction that made it, never later: a journal that balanced
-- when it was posted cannot be quietly extended with another balanced pair.
create function ledger_entry_joins_new_journal() returns trigger
language plpgsql as $$
begin
    if (select xmin from ledger_journals where id = new.journal_id) <> pg_current_xact_id()::xid then
        raise exception 'entries can only be added to a journal in the transaction that created it';
    end if;
    return new;
end;
$$;

create trigger ledger_entries_same_transaction
    before insert on ledger_entries
    for each row execute function ledger_entry_joins_new_journal();

-- At commit, every journal that received entries adds up to zero. (Deferred, so a journal's
-- entries can be inserted one after the other.)
create function ledger_journal_must_balance() returns trigger
language plpgsql as $$
declare
    total bigint;
begin
    select coalesce(sum(amount_paise), 0) into total
    from ledger_entries where journal_id = new.journal_id;
    if total <> 0 then
        raise exception 'ledger journal % does not balance (off by % paise)', new.journal_id, total;
    end if;
    return null;
end;
$$;

create constraint trigger ledger_entries_balance
    after insert on ledger_entries
    deferrable initially deferred
    for each row execute function ledger_journal_must_balance();

-- What the payment provider said its fee was, when it said (null: we estimated).
alter table payments
    add column fee_paise bigint check (fee_paise is null or fee_paise >= 0);

-- ---- payouts ---------------------------------------------------------------------------------------

create type payout_status as enum ('requested', 'paid', 'cancelled');

create table payouts (
    id uuid primary key default uuid_generate_v4(),
    creator_id uuid not null references users(id),
    amount_paise bigint not null check (amount_paise > 0),
    status payout_status not null default 'requested',
    -- Filled in when it is paid (by an admin, after the bank transfer).
    method text check (method in ('bank_transfer', 'upi', 'other')),
    reference text check (reference is null or char_length(reference) between 3 and 100),
    paid_at timestamptz,
    cancelled_at timestamptz,
    cancel_reason text check (cancel_reason is null or char_length(cancel_reason) between 1 and 300),
    decided_by uuid references users(id) on delete set null,
    requested_at timestamptz not null default now(),
    updated_at timestamptz not null default now(),
    constraint payouts_paid_check check (
        status <> 'paid' or (method is not null and reference is not null and paid_at is not null)
    ),
    constraint payouts_cancelled_check check (status <> 'cancelled' or cancelled_at is not null)
);

-- One request at a time per creator: simple for them and for whoever pays.
create unique index payouts_one_open_per_creator
    on payouts (creator_id)
    where status = 'requested';
create index payouts_creator_idx on payouts (creator_id, requested_at desc, id desc);
create index payouts_open_idx on payouts (requested_at) where status = 'requested';

-- Manual corrections: who, how much, and why. Also append-only.
create table ledger_adjustments (
    id uuid primary key default uuid_generate_v4(),
    creator_id uuid not null references users(id),
    amount_paise bigint not null check (amount_paise <> 0),
    reason text not null check (char_length(reason) between 3 and 300),
    created_by uuid references users(id) on delete set null,
    created_at timestamptz not null default now()
);

create trigger ledger_adjustments_append_only
    before update or delete on ledger_adjustments
    for each row execute function ledger_is_append_only();
create trigger ledger_adjustments_no_truncate
    before truncate on ledger_adjustments
    for each statement execute function ledger_is_append_only();
