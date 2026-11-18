//! Share-link codes: unguessable, URL-safe, fixed length.
//!
//! 16 random bytes (128 bits), base64url without padding: 22 characters. Nobody can find a link
//! by trying codes, so the code itself is the credential for a link without a password.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use rand::rngs::OsRng;
use rand::RngCore;

pub const CODE_LEN: usize = 22;

pub fn generate() -> String {
    let mut bytes = [0u8; 16];
    OsRng.fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

/// Whether `raw` could be a code at all. Anything else is "no such link" without a database call,
/// so junk paths and scans cost nothing.
pub fn is_well_formed(raw: &str) -> bool {
    raw.len() == CODE_LEN
        && raw
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn codes_are_22_url_safe_characters_and_do_not_repeat() {
        let codes: HashSet<String> = (0..2000).map(|_| generate()).collect();
        assert_eq!(codes.len(), 2000);
        for code in &codes {
            assert_eq!(code.len(), CODE_LEN);
            assert!(is_well_formed(code), "{code}");
        }
    }

    #[test]
    fn only_codes_of_the_right_shape_are_looked_up() {
        assert!(is_well_formed("abcdefghijklmnopqrstuv"));
        assert!(is_well_formed("AbC-_dEfGhIjKlMnOpQrSt"));
        for bad in [
            "",
            "short",
            "abcdefghijklmnopqrstuvw",
            "abcdefghijklmnopqrstu+",
            "abcdefghijklmnopqrstu/",
            "abcdefghijklmnopqrstu=",
            "abcdefghijklmnopqrstu ",
            "abcdefghijklmnopqrstu\u{e9}",
            "abcdefghijklmnopqrstu'",
        ] {
            assert!(!is_well_formed(bad), "{bad:?}");
        }
    }
}
