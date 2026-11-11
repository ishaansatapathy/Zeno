//! The creator earnings ledger: the book of record for what Zeno owes each creator.
//!
//! Zeno collects every payment itself (decision D1) and takes no share, so the question this
//! module answers, to the paisa, is "how much of the money in the provider's account belongs to
//! whom". It is a **double-entry** book:
//!
//! * every event that moves money is a *journal*; its *entries* add up to exactly zero, which the
//!   database checks at commit, so a bug can move money between accounts but never create or lose
//!   any;
//! * journals and entries are **append-only** (database triggers refuse any edit or removal); a
//!   mistake is corrected by posting a new journal;
//! * a journal exists **once per event** (`kind` + `source_id`), so a retried webhook, a
//!   reconciliation pass and a repeated request all post the same thing once;
//! * a sale is posted **in the transaction that records the payment and grants the enrollment**,
//!   so a paid order without a ledger entry cannot exist.
//!
//! What a sale of `G` paise with provider fee `F` posts:
//!
//! | journal | customers | creator | gateway_fees |
//! | --- | --- | --- | --- |
//! | `sale` | `-G` | `+G` | |
//! | `gateway_fee` | | `-F` | `+F` |
//!
//! so the creator is owed `G - F`. A refund of `R` posts `customers +R, creator -R` (the provider
//! does not return its fee, so the creator bears it). A payout of `P` posts `creator -P,
//! payouts +P` the moment it is requested, so the same money cannot be requested twice.

use zeno_core::config::ConfigError;

pub mod csv;
pub mod dto;
pub mod fee;
pub mod integrity;
pub mod post;
pub mod repo;
pub mod routes;
pub mod service;

/// Settings of the book.
#[derive(Debug, Clone)]
pub struct LedgerConfig {
    /// The provider's fee, in basis points of the amount, used when a payment does not say what
    /// its fee was. Razorpay's standard rate with GST on the fee is about 2.36%.
    pub fallback_fee_bps: u32,
    /// The smallest payout a creator may request, in paise.
    pub payout_minimum_paise: i64,
}

impl LedgerConfig {
    pub const DEFAULT_FEE_BPS: u32 = 236;
    pub const DEFAULT_PAYOUT_MINIMUM_PAISE: i64 = 50_000;

    pub fn defaults() -> Self {
        Self {
            fallback_fee_bps: Self::DEFAULT_FEE_BPS,
            payout_minimum_paise: Self::DEFAULT_PAYOUT_MINIMUM_PAISE,
        }
    }

    pub fn from_env() -> Result<Self, ConfigError> {
        Self::from_lookup(|name| std::env::var(name).ok())
    }

    pub fn from_lookup(get: impl Fn(&str) -> Option<String>) -> Result<Self, ConfigError> {
        let value = |name: &str| {
            get(name)
                .map(|v| v.trim().to_owned())
                .filter(|v| !v.is_empty())
        };
        let invalid = |name: &'static str, reason: &str| ConfigError::Invalid {
            name,
            reason: reason.to_owned(),
        };
        let fallback_fee_bps = match value("GATEWAY_FEE_BPS") {
            None => Self::DEFAULT_FEE_BPS,
            Some(raw) => raw
                .parse::<u32>()
                .ok()
                .filter(|bps| *bps <= 1_000)
                .ok_or_else(|| invalid("GATEWAY_FEE_BPS", "a whole number from 0 to 1000"))?,
        };
        let payout_minimum_paise = match value("PAYOUT_MINIMUM_PAISE") {
            None => Self::DEFAULT_PAYOUT_MINIMUM_PAISE,
            Some(raw) => raw
                .parse::<i64>()
                .ok()
                .filter(|paise| (100..=10_000_000).contains(paise))
                .ok_or_else(|| {
                    invalid(
                        "PAYOUT_MINIMUM_PAISE",
                        "paise from 100 (Rs 1) to 10,000,000",
                    )
                })?,
        };
        Ok(Self {
            fallback_fee_bps,
            payout_minimum_paise,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        move |name| map.get(name).cloned()
    }

    #[test]
    fn defaults_are_the_standard_rate_and_rs_500() {
        let config = LedgerConfig::from_lookup(env(&[])).unwrap();
        assert_eq!(config.fallback_fee_bps, 236);
        assert_eq!(config.payout_minimum_paise, 50_000);
    }

    #[test]
    fn values_are_read_and_bounded() {
        let config = LedgerConfig::from_lookup(env(&[
            ("GATEWAY_FEE_BPS", "200"),
            ("PAYOUT_MINIMUM_PAISE", "100000"),
        ]))
        .unwrap();
        assert_eq!(
            (config.fallback_fee_bps, config.payout_minimum_paise),
            (200, 100_000)
        );
        for (name, value) in [
            ("GATEWAY_FEE_BPS", "1001"),
            ("GATEWAY_FEE_BPS", "-1"),
            ("GATEWAY_FEE_BPS", "two"),
            ("PAYOUT_MINIMUM_PAISE", "99"),
            ("PAYOUT_MINIMUM_PAISE", "0"),
            ("PAYOUT_MINIMUM_PAISE", "10000001"),
        ] {
            assert!(
                LedgerConfig::from_lookup(env(&[(name, value)])).is_err(),
                "{name}={value}"
            );
        }
    }
}
