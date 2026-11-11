//! What the payment provider kept from a payment.
//!
//! When the provider says what its fee was, that is the fee. When it does not (or says something
//! impossible), the fee is estimated from a configured rate, in integer paise, rounded half up:
//! the book never holds fractions of a paisa.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeeSource {
    /// The provider told us.
    Reported,
    /// We worked it out from the configured rate.
    Estimated,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fee {
    pub paise: i64,
    pub source: FeeSource,
}

/// `gross` paise at `bps` basis points, rounded half up.
pub fn estimate(gross_paise: i64, bps: u32) -> i64 {
    let gross = i128::from(gross_paise.max(0));
    let fee = (gross * i128::from(bps) + 5_000) / 10_000;
    i64::try_from(fee)
        .unwrap_or(i64::MAX)
        .min(gross_paise.max(0))
}

/// The fee on a payment of `gross_paise`: what the provider reported if that is possible (not
/// negative, not more than the payment), otherwise the estimate.
pub fn resolve(gross_paise: i64, reported: Option<i64>, fallback_bps: u32) -> Fee {
    match reported {
        Some(paise) if (0..=gross_paise).contains(&paise) => Fee {
            paise,
            source: FeeSource::Reported,
        },
        _ => Fee {
            paise: estimate(gross_paise, fallback_bps),
            source: FeeSource::Estimated,
        },
    }
}

#[cfg(test)]
mod tests {
    use rand::rngs::StdRng;
    use rand::{Rng, SeedableRng};

    use super::*;

    #[test]
    fn estimates_round_half_up() {
        // 2.36% of Rs 499.00 is 1177.64 paise.
        assert_eq!(estimate(49_900, 236), 1_178);
        // Exactly half a paisa rounds up: 1% of 150 paise is 1.5.
        assert_eq!(estimate(150, 100), 2);
        // 1% of 149 paise is 1.49.
        assert_eq!(estimate(149, 100), 1);
        assert_eq!(estimate(49_900, 0), 0);
        assert_eq!(estimate(0, 236), 0);
        assert_eq!(estimate(100_000_000, 1_000), 10_000_000);
    }

    #[test]
    fn what_the_provider_reports_wins_when_it_is_possible() {
        let fee = resolve(49_900, Some(1_000), 236);
        assert_eq!((fee.paise, fee.source), (1_000, FeeSource::Reported));
        let zero = resolve(49_900, Some(0), 236);
        assert_eq!(
            (zero.paise, zero.source),
            (0, FeeSource::Reported),
            "a waived fee is a fee of 0"
        );
        let all = resolve(49_900, Some(49_900), 236);
        assert_eq!(all.paise, 49_900);
    }

    #[test]
    fn nothing_reported_or_something_impossible_is_estimated() {
        for reported in [None, Some(-1), Some(49_901), Some(i64::MAX)] {
            let fee = resolve(49_900, reported, 236);
            assert_eq!(
                (fee.paise, fee.source),
                (1_178, FeeSource::Estimated),
                "{reported:?}"
            );
        }
    }

    #[test]
    fn a_fee_is_never_negative_and_never_more_than_the_payment() {
        let mut rng = StdRng::seed_from_u64(13);
        for _ in 0..20_000 {
            let gross = rng.gen_range(100..=100_000_000i64);
            let bps = rng.gen_range(0..=1_000u32);
            let reported = match rng.gen_range(0..4) {
                0 => None,
                1 => Some(rng.gen_range(-5..=gross + 5)),
                2 => Some(rng.gen_range(0..=gross)),
                _ => Some(i64::MAX),
            };
            let fee = resolve(gross, reported, bps);
            assert!(
                (0..=gross).contains(&fee.paise),
                "{gross} {bps} {reported:?} -> {}",
                fee.paise
            );
            if fee.source == FeeSource::Estimated {
                // Within half a paisa of the exact figure.
                let exact = gross as f64 * f64::from(bps) / 10_000.0;
                assert!(
                    (fee.paise as f64 - exact).abs() <= 0.5 + 1e-6,
                    "{gross} {bps}"
                );
            }
        }
    }
}
