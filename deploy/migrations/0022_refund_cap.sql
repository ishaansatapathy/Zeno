-- A payment cannot be refunded for more than it was. Expand-only.
--
-- The provider will not do it, and the webhook handler checks before it records anything (an event
-- that says otherwise is flagged for a person, not applied). This is the last line: whatever
-- reaches the table, a payment's succeeded refunds never add up to more than the payment, so the
-- book can never be made to hand a creator's money to a buyer twice.

create function refund_must_fit_the_payment() returns trigger
language plpgsql as $$
declare
    paid bigint;
    returned bigint;
begin
    if new.status <> 'succeeded' then
        return new;
    end if;
    -- Nothing about an already-counted refund changed: no need to count again.
    if tg_op = 'UPDATE' and old.status = 'succeeded'
       and old.amount_paise = new.amount_paise and old.payment_id = new.payment_id then
        return new;
    end if;
    -- The payment's row is the turnstile: two refunds of one payment count one after the other.
    select amount_paise into paid from payments where id = new.payment_id for update;
    select coalesce(sum(amount_paise), 0) into returned
    from refunds
    where payment_id = new.payment_id and status = 'succeeded' and id <> new.id;
    if returned + new.amount_paise > paid then
        raise exception 'refunds of payment % would total % paise, more than the % paise it was',
            new.payment_id, returned + new.amount_paise, paid;
    end if;
    return new;
end;
$$;

create trigger refunds_fit_the_payment
    before insert or update on refunds
    for each row execute function refund_must_fit_the_payment();
