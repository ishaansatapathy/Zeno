-- Keep the original Day 13 migration immutable for databases that have already applied it.
-- A later provider fee corrects an earlier estimate with a new balanced journal.

alter table ledger_journals drop constraint ledger_journals_kind_check;
alter table ledger_journals add constraint ledger_journals_kind_check check (
    kind in ('sale', 'gateway_fee', 'gateway_fee_correction', 'refund', 'payout',
             'payout_reversal', 'adjustment')
);
