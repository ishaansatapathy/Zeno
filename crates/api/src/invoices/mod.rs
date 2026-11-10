//! Invoices (decision D8): numbered, immutable, and off until someone has advised on the tax side.
//!
//! What this builds, behind `INVOICES_ENABLED`:
//!
//! * a **gapless number** per financial year (`ZNO/2026-27/000001`), taken from a counter that is
//!   advanced in the same transaction that writes the invoice, so a number is never skipped and
//!   never reused;
//! * an **immutable record** of what was printed (a database trigger refuses any change), kept
//!   exactly as issued even if the course or the buyer's name changes later;
//! * a **plain PDF** and a JSON form, for the buyer.
//!
//! What it deliberately does not do: tax. There is no GST number, no tax split and no credit note
//! for a refund. Those depend on how Zeno is registered and on what a chartered accountant says,
//! so the data carries an empty `tax` slot and nothing more. A plain receipt email goes out for
//! every payment regardless of this flag.

use zeno_core::config::ConfigError;

pub mod dto;
pub mod fy;
pub mod pdf;
pub mod repo;
pub mod routes;
pub mod service;

const MAX_NAME_CHARS: usize = 120;
const MAX_ADDRESS_CHARS: usize = 300;

#[derive(Debug, Clone)]
pub struct InvoicesConfig {
    pub enabled: bool,
    /// Who issues the invoice (the legal name of the business).
    pub seller_name: String,
    pub seller_address: Option<String>,
    /// The letters before the year in the number, e.g. `ZNO`.
    pub prefix: String,
}

impl InvoicesConfig {
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            seller_name: String::new(),
            seller_address: None,
            prefix: "ZNO".to_owned(),
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
        let enabled = match value("INVOICES_ENABLED").as_deref() {
            None | Some("false" | "0" | "no" | "off") => false,
            Some("true" | "1" | "yes" | "on") => true,
            Some(_) => return Err(invalid("INVOICES_ENABLED", "expected true or false")),
        };
        let prefix = value("INVOICE_PREFIX").unwrap_or_else(|| "ZNO".to_owned());
        if !(2..=8).contains(&prefix.len())
            || !prefix
                .chars()
                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
        {
            return Err(invalid(
                "INVOICE_PREFIX",
                "2 to 8 upper-case letters or digits",
            ));
        }
        let seller_name = value("INVOICE_SELLER_NAME").unwrap_or_default();
        let seller_address = value("INVOICE_SELLER_ADDRESS");
        if enabled && seller_name.is_empty() {
            return Err(ConfigError::Missing("INVOICE_SELLER_NAME"));
        }
        if seller_name.chars().count() > MAX_NAME_CHARS {
            return Err(invalid("INVOICE_SELLER_NAME", "too long"));
        }
        if seller_address
            .as_deref()
            .is_some_and(|a| a.chars().count() > MAX_ADDRESS_CHARS)
        {
            return Err(invalid("INVOICE_SELLER_ADDRESS", "too long"));
        }
        Ok(Self {
            enabled,
            seller_name,
            seller_address,
            prefix,
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
    fn invoices_are_off_unless_asked_for() {
        let config = InvoicesConfig::from_lookup(env(&[])).unwrap();
        assert!(!config.enabled);
        assert_eq!(config.prefix, "ZNO");
        let off = InvoicesConfig::from_lookup(env(&[("INVOICES_ENABLED", "false")])).unwrap();
        assert!(!off.enabled);
    }

    #[test]
    fn turning_them_on_needs_a_seller() {
        assert!(matches!(
            InvoicesConfig::from_lookup(env(&[("INVOICES_ENABLED", "true")])),
            Err(ConfigError::Missing("INVOICE_SELLER_NAME"))
        ));
        let on = InvoicesConfig::from_lookup(env(&[
            ("INVOICES_ENABLED", "true"),
            ("INVOICE_SELLER_NAME", "Zeno Learning Pvt Ltd"),
            ("INVOICE_SELLER_ADDRESS", "12 MG Road, Pune"),
            ("INVOICE_PREFIX", "ZL"),
        ]))
        .unwrap();
        assert!(on.enabled);
        assert_eq!(on.seller_name, "Zeno Learning Pvt Ltd");
        assert_eq!(on.seller_address.as_deref(), Some("12 MG Road, Pune"));
        assert_eq!(on.prefix, "ZL");
    }

    #[test]
    fn bad_values_are_refused() {
        for (name, value) in [
            ("INVOICES_ENABLED", "maybe"),
            ("INVOICE_PREFIX", "z"),
            ("INVOICE_PREFIX", "toolongprefix"),
            ("INVOICE_PREFIX", "lower"),
            ("INVOICE_PREFIX", "A/B"),
        ] {
            assert!(
                InvoicesConfig::from_lookup(env(&[(name, value)])).is_err(),
                "{name}={value}"
            );
        }
    }
}
