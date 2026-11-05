use email_address::EmailAddress;

use crate::error::FieldError;

pub const PASSWORD_MIN_CHARS: usize = 10;
pub const PASSWORD_MAX_CHARS: usize = 128;

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
/// towards predictable patterns like `Password1`.
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
        assert!(password("1234567890", "password").is_ok());
        assert!(password(&"a".repeat(128), "password").is_ok());
        assert!(password(&"a".repeat(129), "password").is_err());
        assert!("पासवर्डपासव".len() > 10);
        assert!(password("पासवर्डपासव", "password").is_ok());
        assert!(password("पासवर्ड", "password").is_err());
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
