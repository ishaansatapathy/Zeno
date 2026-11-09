//! Payment settings, and turning them into a provider.
//!
//! Which provider runs is a deployment decision, made once at startup from the environment:
//!
//! * `PAYMENT_PROVIDER=razorpay` needs `RAZORPAY_KEY_ID`, `RAZORPAY_KEY_SECRET` and
//!   `RAZORPAY_WEBHOOK_SECRET` (and optionally `RAZORPAY_WEBHOOK_SECRET_PREVIOUS` while a
//!   webhook secret is being rotated, and `RAZORPAY_API_BASE` for a sandbox);
//! * `PAYMENT_PROVIDER=fake` is the in-memory provider for development. **Production refuses
//!   it**, and also refuses to start with no provider chosen, so a missing variable can never
//!   quietly turn into "everyone's payments succeed".

use std::sync::Arc;

use zeno_core::config::ConfigError;
use zeno_core::payments::{
    FakeProvider, PaymentProvider, ProviderError, RazorpayConfig, RazorpayProvider,
};
use zeno_core::secret::Secret;

use crate::config::parse_public_url;

/// Webhook secrets are chosen by a person in the provider's dashboard; a short one is guessable.
const MIN_WEBHOOK_SECRET_CHARS: usize = 16;

pub enum PaymentsConfig {
    Fake,
    Razorpay(RazorpayConfig),
}

impl PaymentsConfig {
    pub fn from_env(production: bool) -> Result<Self, ConfigError> {
        Self::from_lookup(|name| std::env::var(name).ok(), production)
    }

    /// `from_env` with the environment passed in, so every rule can be tested without touching
    /// the process environment.
    pub fn from_lookup(
        get: impl Fn(&str) -> Option<String>,
        production: bool,
    ) -> Result<Self, ConfigError> {
        let value = |name: &str| {
            get(name)
                .map(|v| v.trim().to_owned())
                .filter(|v| !v.is_empty())
        };
        let required = |name: &'static str| value(name).ok_or(ConfigError::Missing(name));
        let invalid = |name: &'static str, reason: &str| ConfigError::Invalid {
            name,
            reason: reason.to_owned(),
        };

        let provider = match value("PAYMENT_PROVIDER") {
            Some(chosen) => chosen.to_ascii_lowercase(),
            None if production => return Err(ConfigError::Missing("PAYMENT_PROVIDER")),
            None => "fake".to_owned(),
        };
        match provider.as_str() {
            "fake" if production => Err(invalid(
                "PAYMENT_PROVIDER",
                "the fake provider approves nothing real and is not allowed in production",
            )),
            "fake" => Ok(Self::Fake),
            "razorpay" => {
                let key_id = required("RAZORPAY_KEY_ID")?;
                if !is_key_id(&key_id) {
                    return Err(invalid(
                        "RAZORPAY_KEY_ID",
                        "expected rzp_test_… or rzp_live_… from the Razorpay dashboard",
                    ));
                }
                let key_secret = required("RAZORPAY_KEY_SECRET")?;
                let mut webhook_secrets = vec![required("RAZORPAY_WEBHOOK_SECRET")?];
                if let Some(previous) = value("RAZORPAY_WEBHOOK_SECRET_PREVIOUS") {
                    webhook_secrets.push(previous);
                }
                if webhook_secrets
                    .iter()
                    .any(|secret| secret.chars().count() < MIN_WEBHOOK_SECRET_CHARS)
                {
                    return Err(invalid(
                        "RAZORPAY_WEBHOOK_SECRET",
                        "must be at least 16 characters (also applies to the previous secret)",
                    ));
                }
                let mut config = RazorpayConfig::new(
                    key_id,
                    Secret::new(key_secret),
                    webhook_secrets.into_iter().map(Secret::new).collect(),
                );
                if let Some(base) = value("RAZORPAY_API_BASE") {
                    config.api_base = parse_public_url("RAZORPAY_API_BASE", &base)?;
                }
                if production && !config.api_base.starts_with("https://") {
                    return Err(invalid("RAZORPAY_API_BASE", "must use https in production"));
                }
                Ok(Self::Razorpay(config))
            }
            _ => Err(invalid("PAYMENT_PROVIDER", "expected razorpay or fake")),
        }
    }

    pub fn build(&self) -> Result<Arc<dyn PaymentProvider>, ProviderError> {
        Ok(match self {
            Self::Fake => Arc::new(FakeProvider::new()),
            Self::Razorpay(config) => Arc::new(RazorpayProvider::new(config.clone())?),
        })
    }

    /// True for Razorpay's `rzp_test_` keys, which move no real money.
    pub fn uses_test_keys(&self) -> bool {
        matches!(self, Self::Razorpay(config) if config.key_id.starts_with("rzp_test_"))
    }
}

fn is_key_id(key_id: &str) -> bool {
    key_id
        .strip_prefix("rzp_test_")
        .or_else(|| key_id.strip_prefix("rzp_live_"))
        .is_some_and(|rest| !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_alphanumeric()))
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

    const RAZORPAY: &[(&str, &str)] = &[
        ("PAYMENT_PROVIDER", "razorpay"),
        ("RAZORPAY_KEY_ID", "rzp_test_AbC123"),
        ("RAZORPAY_KEY_SECRET", "key-secret"),
        ("RAZORPAY_WEBHOOK_SECRET", "a-long-enough-webhook-secret"),
    ];

    fn with(extra: &[(&'static str, &'static str)]) -> Vec<(&'static str, &'static str)> {
        let mut all: Vec<_> = RAZORPAY.to_vec();
        for (name, value) in extra {
            all.retain(|(n, _)| n != name);
            all.push((name, value));
        }
        all
    }

    fn error_name(result: Result<PaymentsConfig, ConfigError>) -> String {
        match result {
            Err(ConfigError::Missing(name)) => name.to_owned(),
            Err(ConfigError::Invalid { name, .. }) => name.to_owned(),
            Ok(_) => panic!("expected an error"),
        }
    }

    #[test]
    fn development_defaults_to_the_fake_provider() {
        assert!(matches!(
            PaymentsConfig::from_lookup(env(&[]), false),
            Ok(PaymentsConfig::Fake)
        ));
    }

    #[test]
    fn production_must_choose_and_cannot_choose_fake() {
        assert_eq!(
            error_name(PaymentsConfig::from_lookup(env(&[]), true)),
            "PAYMENT_PROVIDER"
        );
        assert_eq!(
            error_name(PaymentsConfig::from_lookup(
                env(&[("PAYMENT_PROVIDER", "fake")]),
                true
            )),
            "PAYMENT_PROVIDER"
        );
        assert_eq!(
            error_name(PaymentsConfig::from_lookup(
                env(&[("PAYMENT_PROVIDER", "  ")]),
                true
            )),
            "PAYMENT_PROVIDER"
        );
    }

    #[test]
    fn an_unknown_provider_is_refused() {
        assert_eq!(
            error_name(PaymentsConfig::from_lookup(
                env(&[("PAYMENT_PROVIDER", "stripe")]),
                false
            )),
            "PAYMENT_PROVIDER"
        );
    }

    #[test]
    fn razorpay_needs_all_three_credentials() {
        assert!(PaymentsConfig::from_lookup(env(RAZORPAY), true).is_ok());
        for missing in [
            "RAZORPAY_KEY_ID",
            "RAZORPAY_KEY_SECRET",
            "RAZORPAY_WEBHOOK_SECRET",
        ] {
            let pairs: Vec<_> = RAZORPAY
                .iter()
                .filter(|(n, _)| *n != missing)
                .copied()
                .collect();
            assert_eq!(
                error_name(PaymentsConfig::from_lookup(env(&pairs), false)),
                missing
            );
        }
    }

    #[test]
    fn the_key_id_must_look_like_a_razorpay_key() {
        for bad in [
            "",
            "key",
            "rzp_test_",
            "rzp_prod_abc",
            "rzp_live_a b",
            "rzp_test_a/b",
        ] {
            let pairs = with(&[("RAZORPAY_KEY_ID", bad)]);
            assert!(
                PaymentsConfig::from_lookup(env(&pairs), false).is_err(),
                "{bad:?}"
            );
        }
        let live = with(&[("RAZORPAY_KEY_ID", "rzp_live_XyZ789")]);
        let config = PaymentsConfig::from_lookup(env(&live), true).unwrap();
        assert!(!config.uses_test_keys());
        assert!(PaymentsConfig::from_lookup(env(RAZORPAY), true)
            .unwrap()
            .uses_test_keys());
    }

    #[test]
    fn weak_webhook_secrets_are_refused_current_or_previous() {
        let short = with(&[("RAZORPAY_WEBHOOK_SECRET", "short")]);
        assert_eq!(
            error_name(PaymentsConfig::from_lookup(env(&short), false)),
            "RAZORPAY_WEBHOOK_SECRET"
        );
        let short_previous = with(&[("RAZORPAY_WEBHOOK_SECRET_PREVIOUS", "short")]);
        assert_eq!(
            error_name(PaymentsConfig::from_lookup(env(&short_previous), false)),
            "RAZORPAY_WEBHOOK_SECRET"
        );
    }

    #[test]
    fn the_previous_webhook_secret_is_kept_after_the_current_one() {
        let rotating = with(&[(
            "RAZORPAY_WEBHOOK_SECRET_PREVIOUS",
            "the-previous-webhook-secret",
        )]);
        let Ok(PaymentsConfig::Razorpay(config)) =
            PaymentsConfig::from_lookup(env(&rotating), true)
        else {
            panic!("expected razorpay");
        };
        let exposed: Vec<&str> = config.webhook_secrets.iter().map(Secret::expose).collect();
        assert_eq!(
            exposed,
            [
                "a-long-enough-webhook-secret",
                "the-previous-webhook-secret"
            ]
        );
    }

    #[test]
    fn a_custom_api_base_must_be_https_in_production() {
        let sandbox = with(&[("RAZORPAY_API_BASE", "http://127.0.0.1:9999/")]);
        let config = PaymentsConfig::from_lookup(env(&sandbox), false).unwrap();
        let PaymentsConfig::Razorpay(config) = config else {
            panic!("expected razorpay");
        };
        assert_eq!(config.api_base, "http://127.0.0.1:9999");
        assert_eq!(
            error_name(PaymentsConfig::from_lookup(env(&sandbox), true)),
            "RAZORPAY_API_BASE"
        );
    }

    #[test]
    fn the_provider_name_matches_the_configuration() {
        assert_eq!(PaymentsConfig::Fake.build().unwrap().name(), "fake");
        let config = PaymentsConfig::from_lookup(env(RAZORPAY), true).unwrap();
        assert_eq!(config.build().unwrap().name(), "razorpay");
    }
}
