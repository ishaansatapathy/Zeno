# Payments runbook

For whoever is on call when money and access disagree. Everything here is either a read-only query
or goes through a path that is safe to repeat. Where this says "never", the system relies on it.

**Ground rules**

1. The provider is the source of truth for whether a customer paid. Zeno's tables are the record of
   what we did about it. When they disagree, ask the provider (dashboard or reconciliation), do not
   guess from our side.
2. Nothing money-related is edited or deleted by hand. The ledger refuses it (triggers), and a
   mistake is corrected with a new entry: an adjustment, or a refund through the provider.
3. A payment, its order, the enrollment, the invoice number, the receipt email and the creator's
   ledger entry are written in one transaction. You will never find one without the others unless
   someone bypassed the application; if you do, that is an incident (see "The book disagrees").
4. Sending an event again is always safe. Handling is keyed on the provider's event id, so a
   resend is a no-op, and a payment is keyed on the provider's payment id.

## 1. What fires, and what to do

Alerts are structured log lines (`alert = "<name>"`). Each one is also a row in `webhook_events`
with `needs_review = true`, and an `order_events` row on the order.

| Alert | It means | First step |
| --- | --- | --- |
| `unknown_order` | A captured payment names no order of ours (or none at all) | §3: find the payment at the provider. A payment made outside checkout, or in another environment |
| `amount_mismatch` | The provider says it took a different amount than the order asked for. Nothing was granted | §3, then refund it from the provider dashboard (the buyer paid something we never agreed to) |
| `currency_mismatch` | Same, for the currency | As above |
| `duplicate_payment` | A second successful payment for an order that was already paid. The first one stands | Refund the duplicate from the dashboard; see §5 |
| `paid_access_blocked` | Paid, but a creator or admin had revoked this person's access, so nothing was granted | Decide with the creator: lift the block (§4) or refund (§5) |
| `refund_unknown_payment` | A refund event for a payment we cannot place | Look the payment up at the provider (§3). Usually a refund of a payment made outside Zeno |
| `refund_exceeds_payment` | A refund event whose total would be more than the payment. Nothing recorded, nothing posted | Almost certainly a provider-side problem; read the event (§2) and ask the provider. Never "fix" it by hand |
| `refund_failed` | The provider could not complete a refund. The order stays paid | §5 |
| `dispute` | A customer disputed a payment with their bank | §6 |
| `ledger_integrity` | The nightly check found the book disagreeing with the tables | §7 |

Anything with `needs_review = true` and `reviewed_at is null` is waiting for a person:

```sql
select received_at, event_type, outcome, order_id, provider_event_id
from webhook_events
where needs_review and reviewed_at is null
order by received_at;
```

When you have dealt with one, mark it (this is the only edit to that table that is meant to
happen by hand):

```sql
update webhook_events set reviewed_at = now() where id = '<id>';
```

## 2. Reading what happened to an order

```sql
-- the order, as the buyer was promised it
select id, user_id, course_id, amount_paise, status, provider_order_id, paid_at, expires_at,
       price_snapshot
from orders where id = '<order id>';

-- every payment the provider reported for it, and what we made of each
select id, provider_ref, status, amount_paise, fee_paise, created_at from payments
where order_id = '<order id>' order by created_at;

-- the story, in order
select created_at, kind, details from order_events
where order_id = '<order id>' order by created_at;

-- the raw events (exactly what the provider sent)
select received_at, provider_event_id, event_type, outcome, payload
from webhook_events where order_id = '<order id>' order by received_at;

-- what the buyer can do now: 'allowed', 'access_revoked', ...
select video_access_state('<user id>', '<a video id of the course>');
```

`payments.status`: `confirmed` paid for the order; `duplicate` a second payment for an
already-paid order; `failed` an attempt that failed; `refunded` returned in full; `pending` seen
but not decided. The order's status is `created` (ours only, the provider not yet asked),
`pending` (open, may be paid), `paid`, `failed`, `expired` or `refunded`.

## 3. A customer says they paid and have no access

1. Find their order (by email):
   ```sql
   select o.id, o.status, o.amount_paise, o.provider_order_id, o.created_at, o.expires_at
   from orders o join users u on u.id = o.user_id
   where u.email = '<email>' order by o.created_at desc limit 5;
   ```
2. **Order is `paid`** and `video_access_state` says `allowed`: they have access; the problem is on
   their side (wrong account, not signed in, cached page). If the state is `access_revoked` for a
   paid order, read `order_events` for `access_blocked` (§4).
3. **Order is `pending` or `expired`**: the money may not have reached us. In the provider
   dashboard, search the payment by the `provider_order_id`.
   - *Captured there*: the webhook was lost or is late. Do nothing by hand. Reconciliation asks
     the provider about every open order 15 minutes after creation (then every 5 minutes while it
     is pending), and about an expired order every two hours for a day after it expires, because a
     payment can land at the last moment. It applies the payment through exactly the code the webhook uses. To hurry it,
     clear the backoff and wait at most one pass (`RECONCILE_INTERVAL_SECS`, default 300):
     ```sql
     update orders set reconcile_after = null where id = '<order id>';
     ```
     Or resend the `payment.captured` event from the dashboard (Webhooks, the delivery log):
     either is safe, and whichever arrives second changes nothing.
   - *Failed or not there*: they were not charged. Tell them to try again; the order is still open
     until `expires_at`, and a new checkout after that makes a new one.
   - *Captured for a different amount*: `amount_mismatch` (§1). Nothing was granted on purpose.
4. **No order at all**: the checkout never got that far. Nothing to repair.

Reconciliation, the webhook and the buyer's own return to the site all converge on the same
state, so "the webhook was lost" is a delay of minutes, not a reason to intervene.

## 4. Replaying an event, and granting access by hand

**Replaying a webhook.** Resend it from the provider dashboard. It is signed again by the
provider, verified, matched by its event id, and answered `duplicate` if we already handled it.
You cannot cause a double enrollment, a double credit or a double refund this way; that is tested
(`webhooks`, `payments_chaos`). Do not try to "re-run" an event by deleting its `webhook_events`
row: the table is the record that it was handled, and the ledger and invoice numbers have moved
on.

If an event was recorded but its outcome was a refusal (`unknown_order`, `amount_mismatch`) and
the cause is now fixed, do not replay it; the fix is reconciliation (§3) or a new, correct
payment.

**Granting access by hand** (a customer who is owed it, and the automatic route will not give
it). Use the product, not SQL, so that it leaves its own record:

- As an admin or the course's creator: `POST /v1/courses/{id}/grants` with the person's email or
  id. It records who granted it, in the enrollment's own event history.
- If their access was revoked by a person (`access_blocked`), lifting that is a decision for
  whoever revoked it. An admin can grant again through the same call.

Never insert into `enrollments` by hand: nothing would say why, and a later refund would not find
it to end it.

## 5. Refunds

- A buyer asks inside the course's refund window: `POST /v1/orders/{id}/request-refund`; the
  creator or an admin approves (`POST /v1/orders/{id}/refund`) or declines
  (`.../refund/decline`). Approving asks the provider; the order stays paid until the provider
  says the refund is processed, and only then is access ended and the creator charged.
- A refund made directly in the provider dashboard is picked up from its `refund.processed`
  event, applied the same way, and recorded as made outside Zeno.
- **A refund the provider could not complete** (`refund_failed`): the order stays paid and access
  stays. Read `refunds.decision_note`; usually the buyer's bank rejected it. Reconciliation keeps
  asking about refunds still pending (every 30 minutes). Decide with the buyer whether to pay
  them another way and then record the money that left with an **adjustment** (§7), not by editing
  anything.
- **A duplicate payment**: refund the duplicate in the dashboard. The event arrives as a refund of
  a payment that is `duplicate`, ends nothing on the real purchase and charges no creator.
- A refund can never return more than the payment. A refund event that says it does is refused
  and flagged (`refund_exceeds_payment`), and the database refuses to hold such a row at all.

## 6. Disputes (chargebacks)

A `payment.dispute.*` event is recorded against the order and flagged; nothing moves
automatically. The bank's clock is short.

1. Read the event (§2) and open the dispute in the provider dashboard. Answer it there, with the
   evidence the provider asks for (the order, the invoice, the access log).
2. While it is open, access stands. To end access, revoke through the product
   (`DELETE /v1/courses/{id}/grants/{user}`), which leaves a record.
3. If the dispute is **lost**, the provider takes the money back without a refund event. Charge
   the creator what they received with an adjustment (a negative amount, reason naming the
   dispute id): `POST /v1/admin/ledger/adjustments`. If it is **won**, nothing to do.
4. Mark the `webhook_events` row reviewed.

## 7. The book disagrees

`GET /v1/admin/ledger/integrity` (also run nightly, and logged as `ledger_integrity`) returns
`{"ok": true, "problems": []}` when the ledger agrees with payments, refunds, payouts and
adjustments. Each problem names its `kind` and the record to look at.

| `kind` | Meaning |
| --- | --- |
| `unbalanced_journal` | A journal whose entries do not add to zero. The database should make this impossible; someone bypassed it |
| `missing_sale` | A payment paid an order but has no sale in the book |
| `sale_amount_mismatch`, `fee_mismatch`, `order_net_mismatch` | The book and the payment, its fee and its refunds disagree about what the creator is owed |
| `refund_without_refund`, `refund_not_posted` | A refund and its journal do not match one-to-one |
| `refund_exceeds_payment` | A payment's succeeded refunds add up to more than the payment |
| `payout_not_posted`, `payout_not_reversed`, `payout_without_payout` | A payout and its journal do not match |
| `adjustment_mismatch` | An adjustment and its journal do not match |

Do not repair by editing: the journal and entry tables refuse updates and deletes. The repair is
always a new, explained entry:

- **Sales and refunds that predate the ledger, or a missing sale/refund**: `POST
  /v1/admin/ledger/backfill` posts what is missing, once each, and is safe to repeat.
- **Anything else a person decided** (a goodwill credit, a chargeback lost, a refund paid by
  another route): `POST /v1/admin/ledger/adjustments` with a signed amount and a reason of at
  least three characters. It records who and why.
- A finding with no explanation you can find is an incident: stop paying out
  (`GET /v1/admin/payouts`, leave requests unpaid), keep the findings, and find out who or what
  wrote around the application before changing anything.

## 8. When the database or the provider is down

- **Database down or failing over**: the webhook answers `5xx`, which is what makes the provider
  retry it for a day with backoff. Nothing is lost and nothing is applied twice when it returns
  (the failover chaos tests do exactly this). If an outage will outlast the provider's retry
  window, reconciliation covers the rest, since it asks the provider for every order older than
  fifteen minutes that is still open.
- **Provider API down**: checkout answers a clear error and leaves no half-made order (it looks
  up an order whose creation timed out before making another). Webhooks stop; resume when it is
  back. Reconciliation backs off per order and retries.
- **Webhook secret rotated**: deploy with the new secret first and the old one as previous (both
  are accepted), then remove the old one. A flood of `invalid_signature` from the provider's own
  addresses means the secret is wrong; they are rate limited per address only for failures, so
  real events still pass.
- **A burst of `invalid_signature` from other addresses**: someone is guessing. They are slowed
  per address and change nothing.

## 9. Things that look wrong and are not

- A paid order whose creator balance is *lower* than the price: the provider's fee is the
  creator's, and it stays with them on a refunded sale (the provider does not return it).
- A creator whose *available* balance is zero after a sale: sales are held for the course's
  refund window before they can be paid out.
- Two payments rows for one order, one `duplicate`: see §5.
- `payments.fee_paise` null: the provider had not reported a fee yet; the book used an estimate
  and corrects it once, when the provider says (`gateway_fee_correction`).
