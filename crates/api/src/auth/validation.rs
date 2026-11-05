use std::collections::HashSet;
use std::sync::OnceLock;

use email_address::EmailAddress;

use crate::error::FieldError;

pub const PASSWORD_MIN_CHARS: usize = 10;
pub const PASSWORD_MAX_CHARS: usize = 128;
const MIN_DISTINCT_CHARS: usize = 5;
/// Local parts shorter than this are too likely to appear in a good passphrase by accident.
const MIN_LOCAL_PART_FOR_MATCH: usize = 6;

/// Passwords of 10+ characters from the UK NCSC's 100k most-used list (via SecLists).
/// Shorter entries are omitted because the length rule already rejects them.
static COMMON_PASSWORDS: &str = include_str!("../../data/common_passwords.txt");

fn common_passwords() -> &'static HashSet<&'static str> {
    static SET: OnceLock<HashSet<&'static str>> = OnceLock::new();
    SET.get_or_init(|| COMMON_PASSWORDS.lines().collect())
}

pub fn is_common_password(raw: &str) -> bool {
    common_passwords().contains(raw.trim().to_lowercase().as_str())
}

/// Trims and lower-cases. Emails are restricted to ASCII so that the application's
/// lower-casing and the database's `lower(email)` check can never disagree.
pub fn normalize_email(raw: &str) -> String {
    raw.trim().to_ascii_lowercase()
}

pub fn email(raw: &str) -> Result<String, FieldError> {
    let value = normalize_email(raw);
    if value.is_empty() {
        return Err(FieldError::new("email", "Email is required."));
    }
    let has_dotted_domain = value
        .rsplit_once('@')
        .is_some_and(|(_, domain)| domain.contains('.') && !domain.ends_with('.'));
    if !value.is_ascii()
        || value.len() > 254
        || !has_dotted_domain
        || !EmailAddress::is_valid(&value)
    {
        return Err(FieldError::new("email", "Enter a valid email address."));
    }
    Ok(value)
}

/// Length is what matters for password strength; composition rules mostly push people
/// towards predictable patterns like `Password1`. On top of length, reject the passwords
/// attackers try first: anything on a common-password list and trivially repetitive strings.
pub fn password(raw: &str, field: &'static str) -> Result<(), FieldError> {
    let length = raw.chars().count();
    if length < PASSWORD_MIN_CHARS {
        return Err(FieldError::new(
            field,
            format!("Password must be at least {PASSWORD_MIN_CHARS} characters."),
        ));
    }
    if length > PASSWORD_MAX_CHARS {
        return Err(FieldError::new(
            field,
            format!("Password must be at most {PASSWORD_MAX_CHARS} characters."),
        ));
    }
    if is_common_password(raw) {
        return Err(FieldError::new(
            field,
            "This password is too common. Choose something harder to guess.",
        ));
    }
    let distinct: HashSet<char> = raw.chars().collect();
    if distinct.len() < MIN_DISTINCT_CHARS {
        return Err(FieldError::new(field, "This password is too repetitive."));
    }
    Ok(())
}

/// A password built from the account's own email address is the first thing a targeted
/// attacker tries.
pub fn password_not_from_email(
    raw: &str,
    email: &str,
    field: &'static str,
) -> Result<(), FieldError> {
    let password = raw.trim().to_lowercase();
    let local = email.split('@').next().unwrap_or_default();
    let derived = password == email
        || password == local
        || (local.chars().count() >= MIN_LOCAL_PART_FOR_MATCH && password.contains(local));
    if derived {
        return Err(FieldError::new(
            field,
            "Password can't be based on your email address.",
        ));
    }
    Ok(())
}

pub fn name(raw: &str) -> Result<String, FieldError> {
    let value = raw.trim();
    let length = value.chars().count();
    if !(2..=80).contains(&length) {
        return Err(FieldError::new("name", "Name must be 2 to 80 characters."));
    }
    if value.chars().any(char::is_control) {
        return Err(FieldError::new("name", "Name contains invalid characters."));
    }
    Ok(value.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn email_is_trimmed_and_lowercased() {
        assert_eq!(
            email("  Teacher@Example.IN ").unwrap(),
            "teacher@example.in"
        );
    }

    #[test]
    fn email_rejects_malformed_values() {
        for bad in [
            "",
            "   ",
            "plain",
            "a@b",
            "a@b.",
            "@x.com",
            "a b@x.com",
            "a@@x.com",
            "tëst@x.com",
            "a@exämple.com",
        ] {
            assert!(email(bad).is_err(), "{bad:?} should be rejected");
        }
        let long = format!("{}@x.com", "a".repeat(250));
        assert!(email(&long).is_err());
    }

    #[test]
    fn password_length_bounds_count_characters_not_bytes() {
        assert!(password("123456789", "password").is_err());
        assert!(password("k3Vn8qZp5w", "password").is_ok());
        assert!(password(&"abcdefgh".repeat(16), "password").is_ok());
        assert!(password(&format!("{}x", "abcdefgh".repeat(16)), "password").is_err());
        assert!("पासवर्डपासव".len() > 10);
        assert!(password("पासवर्डपासव", "password").is_ok());
        assert!(password("पासवर्ड", "password").is_err());
    }

    #[test]
    fn the_embedded_list_is_sane() {
        let list = common_passwords();
        assert!(list.len() > 5_000, "list looks truncated: {}", list.len());
        assert!(list
            .iter()
            .all(|p| p.chars().count() >= PASSWORD_MIN_CHARS && *p == p.to_lowercase()));
    }

    #[test]
    fn common_passwords_are_rejected_in_any_case_and_with_padding() {
        for pw in [
            "password123456",
            "PASSWORD123456",
            "  iloveyou1234 ",
            "qwertyuiop",
        ] {
            let err = password(pw, "password").unwrap_err();
            assert!(
                err.message.contains("too common"),
                "{pw:?}: {}",
                err.message
            );
        }
    }

    #[test]
    fn repetitive_passwords_are_rejected() {
        for pw in [
            "aaaaaaaaaaaa",
            "abababababab",
            "1212121212121",
            "zzzzxxxxyyyy",
        ] {
            assert!(password(pw, "password").is_err(), "{pw:?}");
        }
    }

    #[test]
    fn real_passphrases_are_accepted() {
        for pw in [
            "correct horse battery staple",
            "a-long-enough-password",
            "tr0ub4dor&3-but-longer",
            "पासवर्डपासव",
        ] {
            assert!(password(pw, "password").is_ok(), "{pw:?}");
        }
    }

    #[test]
    fn passwords_derived_from_the_email_are_rejected() {
        let email = "asha.rao@example.com";
        assert!(password_not_from_email("asha.rao@example.com", email, "password").is_err());
        assert!(password_not_from_email("ASHA.RAO", email, "password").is_err());
        assert!(password_not_from_email("my-asha.rao-2024!", email, "password").is_err());
        // short local parts show up in good passphrases by chance, so they don't count
        assert!(password_not_from_email("a-sam-is-here-too", "sam@x.in", "password").is_ok());
        assert!(password_not_from_email("unrelated-passphrase", email, "password").is_ok());
    }

    #[test]
    fn name_rules() {
        assert_eq!(name("  Ishaan  ").unwrap(), "Ishaan");
        assert!(name("A").is_err());
        assert!(name(&"x".repeat(81)).is_err());
        assert!(name("bad\u{0}name").is_err());
        assert!(name("line\nbreak").is_err());
    }
}
