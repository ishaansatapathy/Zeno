//! Reading a provider event into the few facts we act on.
//!
//! Parsing is strict about the fields that matter (ids, amount as an integer, currency) and
//! ignores the rest. An event we cannot make sense of is stored and answered with `200`: the
//! provider would resend it for a day, and resending cannot fix it.

use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaymentEntity {
    pub id: String,
    /// The provider's order the payment belongs to; absent for payments made outside checkout.
    pub order_id: Option<String>,
    pub amount_paise: i64,
    pub currency: String,
    pub status: String,
    pub error_code: Option<String>,
    pub error_description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefundEntity {
    pub id: String,
    pub payment_id: String,
    pub amount_paise: i64,
    pub status: String,
    /// Our refund UUID, sent as Razorpay's receipt and as a note. Dashboard refunds have neither.
    pub receipt: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    PaymentCaptured(PaymentEntity),
    /// `order.paid` carries the payment too; it says the same thing as `payment.captured`.
    OrderPaid(PaymentEntity),
    PaymentFailed(PaymentEntity),
    RefundProcessed {
        refund: RefundEntity,
        payment: Option<PaymentEntity>,
    },
    RefundFailed {
        refund: RefundEntity,
        payment: Option<PaymentEntity>,
    },
    /// A customer disputed a payment with their bank (`payment.dispute.*`). Stage 1 handles
    /// these by hand: the event is recorded against the order and flagged.
    Dispute(DisputeEntity),
    /// Anything else (`payment.authorized`, settlements...): stored, not acted on.
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisputeEntity {
    pub id: String,
    pub payment_id: String,
    pub status: String,
    pub amount_paise: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parsed {
    /// The event's name, for the log of what was received.
    pub name: String,
    pub event: Result<Event, &'static str>,
}

fn text(value: &Value, field: &str) -> Option<String> {
    value[field]
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= 200)
        .map(str::to_owned)
}

fn payment(entity: &Value) -> Result<PaymentEntity, &'static str> {
    Ok(PaymentEntity {
        id: text(entity, "id").ok_or("payment id")?,
        order_id: text(entity, "order_id"),
        // An integer number of paise; "499.00" or 499.5 is not an amount.
        amount_paise: entity["amount"]
            .as_i64()
            .filter(|amount| *amount >= 0)
            .ok_or("payment amount")?,
        currency: text(entity, "currency").ok_or("payment currency")?,
        status: text(entity, "status").ok_or("payment status")?,
        error_code: text(entity, "error_code"),
        error_description: entity["error_description"]
            .as_str()
            .map(|s| s.chars().take(300).collect()),
    })
}

fn refund(entity: &Value) -> Result<RefundEntity, &'static str> {
    Ok(RefundEntity {
        id: text(entity, "id").ok_or("refund id")?,
        payment_id: text(entity, "payment_id").ok_or("refund payment id")?,
        amount_paise: entity["amount"]
            .as_i64()
            .filter(|amount| *amount >= 0)
            .ok_or("refund amount")?,
        status: text(entity, "status").ok_or("refund status")?,
        receipt: text(entity, "receipt").or_else(|| text(&entity["notes"], "zeno_refund_id")),
    })
}

fn dispute(entity: &Value) -> Result<DisputeEntity, &'static str> {
    Ok(DisputeEntity {
        id: text(entity, "id").ok_or("dispute id")?,
        payment_id: text(entity, "payment_id").ok_or("dispute payment id")?,
        status: text(entity, "status").unwrap_or_else(|| "unknown".to_owned()),
        amount_paise: entity["amount"].as_i64().filter(|amount| *amount >= 0),
    })
}

pub fn parse(body: &[u8]) -> Parsed {
    let unreadable = |reason| Parsed {
        name: "unparseable".to_owned(),
        event: Err(reason),
    };
    let Ok(root) = serde_json::from_slice::<Value>(body) else {
        return unreadable("body is not JSON");
    };
    let Some(name) = root["event"].as_str().filter(|n| n.len() <= 100) else {
        return unreadable("no event name");
    };
    let entity = |kind: &str| root["payload"][kind]["entity"].clone();
    let optional_payment = || {
        let value = entity("payment");
        (!value.is_null()).then(|| payment(&value))
    };
    let event = match name {
        "payment.captured" => payment(&entity("payment")).map(Event::PaymentCaptured),
        "payment.failed" => payment(&entity("payment")).map(Event::PaymentFailed),
        "order.paid" => payment(&entity("payment")).and_then(|payment| {
            // The payment and the order in one event must agree about which order this is.
            match (&payment.order_id, text(&entity("order"), "id")) {
                (Some(on_payment), Some(on_order)) if *on_payment != on_order => {
                    Err("payment and order disagree")
                }
                _ => Ok(Event::OrderPaid(payment)),
            }
        }),
        "refund.processed" => refund(&entity("refund")).and_then(|refund| {
            Ok(Event::RefundProcessed {
                refund,
                payment: optional_payment().transpose()?,
            })
        }),
        "refund.failed" => refund(&entity("refund")).and_then(|refund| {
            Ok(Event::RefundFailed {
                refund,
                payment: optional_payment().transpose()?,
            })
        }),
        other if other.starts_with("payment.dispute.") => {
            dispute(&entity("dispute")).map(Event::Dispute)
        }
        _ => Ok(Event::Other),
    };
    Parsed {
        name: name.to_owned(),
        event,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn payment_json(status: &str) -> Value {
        json!({"id": "pay_1", "order_id": "order_1", "amount": 49900, "currency": "INR",
               "status": status, "method": "upi", "email": "a@b.in"})
    }

    fn body(event: &str, payload: Value) -> Vec<u8> {
        serde_json::to_vec(&json!({"entity": "event", "event": event, "payload": payload})).unwrap()
    }

    #[test]
    fn a_captured_payment_is_read_with_the_fields_we_act_on() {
        let parsed = parse(&body(
            "payment.captured",
            json!({"payment": {"entity": payment_json("captured")}}),
        ));
        assert_eq!(parsed.name, "payment.captured");
        let Ok(Event::PaymentCaptured(p)) = parsed.event else {
            panic!("{:?}", parsed.event);
        };
        assert_eq!(p.id, "pay_1");
        assert_eq!(p.order_id.as_deref(), Some("order_1"));
        assert_eq!(p.amount_paise, 49_900);
        assert_eq!(p.currency, "INR");
        assert_eq!(p.status, "captured");
    }

    #[test]
    fn order_paid_must_agree_with_its_payment() {
        let ok = parse(&body(
            "order.paid",
            json!({"payment": {"entity": payment_json("captured")}, "order": {"entity": {"id": "order_1"}}}),
        ));
        assert!(matches!(ok.event, Ok(Event::OrderPaid(_))));
        let crossed = parse(&body(
            "order.paid",
            json!({"payment": {"entity": payment_json("captured")}, "order": {"entity": {"id": "order_2"}}}),
        ));
        assert_eq!(crossed.event, Err("payment and order disagree"));
    }

    #[test]
    fn amounts_must_be_whole_non_negative_paise() {
        for amount in [json!("49900"), json!(499.5), json!(-1), json!(null)] {
            let mut payment = payment_json("captured");
            payment["amount"] = amount.clone();
            let parsed = parse(&body(
                "payment.captured",
                json!({"payment": {"entity": payment}}),
            ));
            assert_eq!(parsed.event, Err("payment amount"), "{amount}");
        }
    }

    #[test]
    fn refunds_are_read_with_or_without_their_payment() {
        let refund =
            json!({"id": "rfnd_1", "payment_id": "pay_1", "amount": 49900, "status": "processed"});
        let alone = parse(&body(
            "refund.processed",
            json!({"refund": {"entity": refund.clone()}}),
        ));
        assert!(matches!(
            alone.event,
            Ok(Event::RefundProcessed { payment: None, .. })
        ));
        let with = parse(&body(
            "refund.processed",
            json!({"refund": {"entity": refund}, "payment": {"entity": payment_json("refunded")}}),
        ));
        assert!(matches!(
            with.event,
            Ok(Event::RefundProcessed {
                payment: Some(_),
                ..
            })
        ));
    }

    #[test]
    fn disputes_are_read_with_their_payment() {
        let parsed = parse(&body(
            "payment.dispute.created",
            json!({"dispute": {"entity": {"id": "disp_1", "payment_id": "pay_1", "amount": 49900, "status": "open"}}}),
        ));
        let Ok(Event::Dispute(dispute)) = parsed.event else {
            panic!("{:?}", parsed.event);
        };
        assert_eq!(
            (
                dispute.id.as_str(),
                dispute.payment_id.as_str(),
                dispute.status.as_str()
            ),
            ("disp_1", "pay_1", "open")
        );
        assert_eq!(dispute.amount_paise, Some(49_900));
        let nameless = parse(&body("payment.dispute.won", json!({})));
        assert_eq!(nameless.event, Err("dispute id"));
    }

    #[test]
    fn events_we_do_not_act_on_are_other_and_garbage_is_unreadable() {
        let other = parse(&body("payment.authorized", json!({})));
        assert_eq!(
            (other.name.as_str(), other.event),
            ("payment.authorized", Ok(Event::Other))
        );
        for garbage in [&b"not json"[..], b"[]", b"{}", br#"{"event": 7}"#, b""] {
            let parsed = parse(garbage);
            assert!(parsed.event.is_err(), "{garbage:?}");
            assert_eq!(parsed.name, "unparseable");
        }
        let missing_payload = parse(&body("payment.captured", json!({})));
        assert_eq!(missing_payload.event, Err("payment id"));
    }
}
