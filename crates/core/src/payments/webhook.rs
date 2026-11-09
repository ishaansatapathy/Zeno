//! Webhook signatures: hex(HMAC-SHA256(secret, raw body)).
//!
//! The signature covers the exact bytes the provider sent, so verification must run on the raw
//! request body before anything parses it. Comparison is constant-time (`ring` does it), and
//! every configured secret is tried so a secret can be rotated without dropping events: deploy
//! with the new secret first and the old one as the previous.

use ring::hmac;

use crate::secret::Secret;

/// Lower-case hex of the HMAC-SHA256 of `body` under `secret`.
pub fn sign(secret: &str, body: &[u8]) -> String {
    let key = hmac::Key::new(hmac::HMAC_SHA256, secret.as_bytes());
    hex(hmac::sign(&key, body).as_ref())
}

/// True when `signature` is a valid signature of `body` under any of `secrets`.
pub fn verify(secrets: &[Secret], body: &[u8], signature: &str) -> bool {
    let Some(expected) = unhex(signature) else {
        return false;
    };
    let mut valid = false;
    for secret in secrets {
        if secret.expose().is_empty() {
            continue;
        }
        let key = hmac::Key::new(hmac::HMAC_SHA256, secret.expose().as_bytes());
        // `|=` rather than an early return, so the time taken does not say which secret matched.
        valid |= hmac::verify(&key, body, &expected).is_ok();
    }
    valid
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(DIGITS[(byte >> 4) as usize] as char);
        out.push(DIGITS[(byte & 0x0f) as usize] as char);
    }
    out
}

/// Decodes exactly 64 hex digits (either case) into 32 bytes; anything else is not a signature.
fn unhex(signature: &str) -> Option<Vec<u8>> {
    let bytes = signature.as_bytes();
    if bytes.len() != 64 {
        return None;
    }
    let nibble = |c: u8| match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    };
    bytes
        .chunks(2)
        .map(|pair| Some(nibble(pair[0])? << 4 | nibble(pair[1])?))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const BODY: &[u8] =
        br#"{"event":"payment.captured","payload":{"payment":{"entity":{"id":"pay_1"}}}}"#;

    fn secrets(values: &[&str]) -> Vec<Secret> {
        values.iter().map(|v| Secret::new(*v)).collect()
    }

    #[test]
    fn matches_the_published_hmac_sha256_test_vector() {
        // RFC 4231 test case 2.
        assert_eq!(
            sign("Jefe", b"what do ya want for nothing?"),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
    }

    #[test]
    fn accepts_the_right_signature_in_either_case() {
        let signature = sign("s3cret", BODY);
        assert!(verify(&secrets(&["s3cret"]), BODY, &signature));
        assert!(verify(
            &secrets(&["s3cret"]),
            BODY,
            &signature.to_uppercase()
        ));
    }

    #[test]
    fn rejects_everything_else() {
        let good = sign("s3cret", BODY);
        let all = secrets(&["s3cret"]);
        // Wrong secret, modified body, truncated body, appended byte.
        assert!(!verify(&secrets(&["other"]), BODY, &good));
        assert!(!verify(&all, b"{\"event\":\"payment.failed\"}", &good));
        assert!(!verify(&all, &BODY[..BODY.len() - 1], &good));
        assert!(!verify(&all, &[BODY, b" "].concat(), &good));
        // Malformed signatures: empty, short, long, not hex, a flipped digit.
        assert!(!verify(&all, BODY, ""));
        assert!(!verify(&all, BODY, &good[..63]));
        assert!(!verify(&all, BODY, &format!("{good}0")));
        assert!(!verify(&all, BODY, &format!("{}zz", &good[..62])));
        let mut flipped = good.clone().into_bytes();
        flipped[10] = if flipped[10] == b'0' { b'1' } else { b'0' };
        assert!(!verify(&all, BODY, &String::from_utf8(flipped).unwrap()));
        // A signature with whitespace around it is not the signature.
        assert!(!verify(&all, BODY, &format!(" {good}")));
    }

    #[test]
    fn no_secrets_or_empty_secrets_verify_nothing() {
        let empty_key_signature = sign("", BODY);
        assert!(!verify(&[], BODY, &empty_key_signature));
        assert!(!verify(&secrets(&[""]), BODY, &empty_key_signature));
    }

    #[test]
    fn a_rotated_secret_stays_valid_for_the_previous_one() {
        let old = sign("old-secret", BODY);
        let new = sign("new-secret", BODY);
        let both = secrets(&["new-secret", "old-secret"]);
        assert!(verify(&both, BODY, &old));
        assert!(verify(&both, BODY, &new));
        assert!(!verify(&secrets(&["new-secret"]), BODY, &old));
    }
}
