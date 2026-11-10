//! What a coupon does to a price. Pure integer arithmetic in paise, so the rules can be tested
//! exhaustively and can never drift by a rounding error.
//!
//! * A percent discount is rounded **down** (the buyer never gets more than the exact figure).
//! * A discount that covers the whole price makes the course free.
//! * Otherwise the buyer pays at least Rs 1 (the provider's minimum): a discount that would leave
//!   less is cut back so the price lands on Rs 1.
//! * A coupon that would change nothing is refused rather than used up.

use zeno_core::model::CouponKind;

/// The least the payment provider will charge.
pub const MIN_CHARGE_PAISE: i64 = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Priced {
    /// Pay `amount_paise`, which is `list_paise - discount_paise`.
    Charge {
        list_paise: i64,
        discount_paise: i64,
        amount_paise: i64,
    },
    /// The coupon covers everything: enroll without taking payment.
    Free { list_paise: i64 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NoEffect;

/// Prices a course at `list_paise` (at least 1) under a coupon of this kind and value.
pub fn price(list_paise: i64, kind: CouponKind, value: i64) -> Result<Priced, NoEffect> {
    if list_paise <= 0 || value <= 0 {
        return Err(NoEffect);
    }
    let wanted = match kind {
        // i128: a price near the limit times 100 must not overflow.
        CouponKind::Percent => (i128::from(list_paise) * i128::from(value.min(100)) / 100) as i64,
        CouponKind::Flat => value,
    };
    if wanted >= list_paise {
        return Ok(Priced::Free { list_paise });
    }
    let amount_paise = (list_paise - wanted).max(MIN_CHARGE_PAISE).min(list_paise);
    let discount_paise = list_paise - amount_paise;
    if discount_paise == 0 {
        return Err(NoEffect);
    }
    Ok(Priced::Charge {
        list_paise,
        discount_paise,
        amount_paise,
    })
}

#[cfg(test)]
mod tests {
    use rand::rngs::StdRng;
    use rand::{Rng, SeedableRng};

    use super::*;

    fn charge(list: i64, kind: CouponKind, value: i64) -> (i64, i64) {
        match price(list, kind, value) {
            Ok(Priced::Charge {
                discount_paise,
                amount_paise,
                ..
            }) => (discount_paise, amount_paise),
            other => panic!("expected a charge, got {other:?}"),
        }
    }

    #[test]
    fn percent_discounts_round_down() {
        // 10% of Rs 499 is Rs 49.90 exactly; 15% of Rs 99.99 is 1499.85 paise, rounded down.
        assert_eq!(charge(49_900, CouponKind::Percent, 10), (4_990, 44_910));
        assert_eq!(charge(9_999, CouponKind::Percent, 15), (1_499, 8_500));
        assert_eq!(charge(100_000, CouponKind::Percent, 1), (1_000, 99_000));
    }

    #[test]
    fn flat_discounts_come_off_as_stated() {
        assert_eq!(charge(49_900, CouponKind::Flat, 10_000), (10_000, 39_900));
    }

    #[test]
    fn a_discount_that_covers_the_price_makes_it_free() {
        assert_eq!(
            price(49_900, CouponKind::Percent, 100),
            Ok(Priced::Free { list_paise: 49_900 })
        );
        assert_eq!(
            price(49_900, CouponKind::Flat, 49_900),
            Ok(Priced::Free { list_paise: 49_900 })
        );
        assert_eq!(
            price(49_900, CouponKind::Flat, 500_000),
            Ok(Priced::Free { list_paise: 49_900 })
        );
    }

    #[test]
    fn the_buyer_never_pays_less_than_one_rupee_unless_it_is_free() {
        // 99.5% off Rs 499 would leave 249.5 paise; it is held at Rs 1.
        assert_eq!(charge(49_900, CouponKind::Flat, 49_850), (49_800, 100));
        assert_eq!(charge(49_900, CouponKind::Percent, 99), (49_401, 499));
        assert_eq!(charge(150, CouponKind::Flat, 100), (50, 100));
    }

    #[test]
    fn a_coupon_that_changes_nothing_is_refused() {
        // 1% of Rs 1 is nothing; and Rs 1 cannot go lower.
        assert_eq!(price(100, CouponKind::Percent, 1), Err(NoEffect));
        assert_eq!(price(100, CouponKind::Percent, 50), Err(NoEffect));
        // But 1% of Rs 1.50 is a real paisa off, and that is honoured.
        assert_eq!(charge(150, CouponKind::Percent, 1), (1, 149));
        // Less than a paisa rounds to nothing.
        assert_eq!(price(199, CouponKind::Percent, 0), Err(NoEffect));
        assert_eq!(price(0, CouponKind::Percent, 10), Err(NoEffect));
        assert_eq!(price(49_900, CouponKind::Flat, 0), Err(NoEffect));
    }

    #[test]
    fn the_largest_price_does_not_overflow() {
        let top = 100_000_000;
        assert_eq!(
            charge(top, CouponKind::Percent, 99),
            (99_000_000, 1_000_000)
        );
    }

    #[test]
    fn whatever_the_inputs_the_laws_hold() {
        let mut rng = StdRng::seed_from_u64(0x20_26_11_12);
        for _ in 0..20_000 {
            let list = rng.gen_range(100..=100_000_000i64);
            let (kind, value) = if rng.gen_bool(0.5) {
                (CouponKind::Percent, rng.gen_range(1..=100i64))
            } else {
                (CouponKind::Flat, rng.gen_range(100..=120_000_000i64))
            };
            match price(list, kind, value) {
                Ok(Priced::Charge {
                    list_paise,
                    discount_paise,
                    amount_paise,
                }) => {
                    assert_eq!(list_paise, list);
                    assert_eq!(
                        list - discount_paise,
                        amount_paise,
                        "{list} {kind:?} {value}"
                    );
                    assert!(discount_paise > 0, "a used coupon takes something off");
                    assert!(amount_paise >= MIN_CHARGE_PAISE, "{list} {kind:?} {value}");
                    assert!(amount_paise < list);
                    // Never more than what was asked for.
                    let asked = match kind {
                        CouponKind::Percent => list * value / 100,
                        CouponKind::Flat => value,
                    };
                    assert!(discount_paise <= asked, "{list} {kind:?} {value}");
                }
                Ok(Priced::Free { list_paise }) => {
                    assert_eq!(list_paise, list);
                    let asked = match kind {
                        CouponKind::Percent => list * value / 100,
                        CouponKind::Flat => value,
                    };
                    assert!(
                        asked >= list,
                        "free only when the discount covers the price"
                    );
                }
                Err(NoEffect) => {
                    // Only when nothing was asked for, or the price is already Rs 1.
                    let asked = match kind {
                        CouponKind::Percent => list * value / 100,
                        CouponKind::Flat => value,
                    };
                    assert!(
                        asked == 0 || list == MIN_CHARGE_PAISE,
                        "{list} {kind:?} {value}"
                    );
                }
            }
        }
    }

    #[test]
    fn a_bigger_percent_never_discounts_less() {
        let mut rng = StdRng::seed_from_u64(7);
        for _ in 0..5_000 {
            let list = rng.gen_range(100..=100_000_000i64);
            let mut last = 0;
            for percent in 1..=100 {
                let discount = match price(list, CouponKind::Percent, percent) {
                    Ok(Priced::Charge { discount_paise, .. }) => discount_paise,
                    Ok(Priced::Free { list_paise }) => list_paise,
                    Err(NoEffect) => 0,
                };
                assert!(
                    discount >= last,
                    "{list}: {percent}% gave {discount} after {last}"
                );
                last = discount;
            }
        }
    }
}
