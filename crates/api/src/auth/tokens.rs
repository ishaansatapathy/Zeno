use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use chrono::{DateTime, Duration, Utc};
use jsonwebtoken::errors::Error as JwtError;
use jsonwebtoken::{decode, encode, Algorithm, DecodingKey, EncodingKey, Header, Validation};
use rand::rngs::OsRng;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;
use zeno_core::model::Role;

pub const ISSUER: &str = "zeno";
pub const AUDIENCE: &str = "zeno-api";
pub const ACCESS_TTL_MINUTES: i64 = 15;

#[derive(Debug, Serialize, Deserialize)]
pub struct AccessClaims {
    pub sub: Uuid,
    pub role: Role,
    /// Session (refresh-token row) this access token was minted from.
    pub sid: Uuid,
    pub iss: String,
    pub aud: String,
    pub iat: i64,
    pub exp: i64,
}

/// Signs and verifies short-lived access tokens. Only HS256 is ever accepted, and issuer,
/// audience and expiry are all required, so a token minted for another purpose or with a
/// different algorithm header is rejected.
pub struct TokenKeys {
    encoding: EncodingKey,
    decoding: DecodingKey,
    validation: Validation,
}

impl TokenKeys {
    pub fn new(secret: &[u8]) -> Self {
        let mut validation = Validation::new(Algorithm::HS256);
        validation.set_issuer(&[ISSUER]);
        validation.set_audience(&[AUDIENCE]);
        validation.set_required_spec_claims(&["exp", "sub", "iss", "aud"]);
        validation.leeway = 5;
        Self {
            encoding: EncodingKey::from_secret(secret),
            decoding: DecodingKey::from_secret(secret),
            validation,
        }
    }

    pub fn issue_access(
        &self,
        user_id: Uuid,
        role: Role,
        session_id: Uuid,
        now: DateTime<Utc>,
    ) -> Result<(String, DateTime<Utc>), JwtError> {
        let expires_at = now + Duration::minutes(ACCESS_TTL_MINUTES);
        let claims = AccessClaims {
            sub: user_id,
            role,
            sid: session_id,
            iss: ISSUER.into(),
            aud: AUDIENCE.into(),
            iat: now.timestamp(),
            exp: expires_at.timestamp(),
        };
        let token = encode(&Header::new(Algorithm::HS256), &claims, &self.encoding)?;
        Ok((token, expires_at))
    }

    pub fn verify_access(&self, token: &str) -> Result<AccessClaims, JwtError> {
        decode::<AccessClaims>(token, &self.decoding, &self.validation).map(|data| data.claims)
    }
}

/// 256 bits from the OS CSPRNG, URL-safe, for refresh and emailed one-time tokens.
pub fn generate_opaque_token() -> String {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

/// Only this hash is stored. The inputs are high-entropy random strings, so a fast hash
/// is correct here (slow hashes are for human-chosen passwords).
pub fn hash_opaque_token(token: &str) -> Vec<u8> {
    Sha256::digest(token.as_bytes()).to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &[u8] = b"0123456789abcdef0123456789abcdef";

    fn keys() -> TokenKeys {
        TokenKeys::new(SECRET)
    }

    #[test]
    fn issued_token_round_trips() {
        let (user, session) = (Uuid::new_v4(), Uuid::new_v4());
        let (token, _) = keys()
            .issue_access(user, Role::Creator, session, Utc::now())
            .unwrap();
        let claims = keys().verify_access(&token).unwrap();
        assert_eq!(claims.sub, user);
        assert_eq!(claims.sid, session);
        assert_eq!(claims.role, Role::Creator);
    }

    #[test]
    fn expired_token_is_rejected() {
        let long_ago = Utc::now() - Duration::hours(2);
        let (token, _) = keys()
            .issue_access(Uuid::new_v4(), Role::Student, Uuid::new_v4(), long_ago)
            .unwrap();
        assert!(keys().verify_access(&token).is_err());
    }

    #[test]
    fn token_signed_with_another_secret_is_rejected() {
        let (token, _) = TokenKeys::new(b"ffffffffffffffffffffffffffffffff")
            .issue_access(Uuid::new_v4(), Role::Admin, Uuid::new_v4(), Utc::now())
            .unwrap();
        assert!(keys().verify_access(&token).is_err());
    }

    #[test]
    fn tampered_payload_is_rejected() {
        let (token, _) = keys()
            .issue_access(Uuid::new_v4(), Role::Student, Uuid::new_v4(), Utc::now())
            .unwrap();
        let mut parts: Vec<String> = token.split('.').map(str::to_owned).collect();
        let forged = URL_SAFE_NO_PAD.encode(
            br#"{"sub":"00000000-0000-0000-0000-000000000000","role":"admin","sid":"00000000-0000-0000-0000-000000000000","iss":"zeno","aud":"zeno-api","iat":0,"exp":9999999999}"#,
        );
        parts[1] = forged;
        assert!(keys().verify_access(&parts.join(".")).is_err());
    }

    #[test]
    fn other_algorithms_are_rejected_even_with_the_right_secret() {
        let claims = AccessClaims {
            sub: Uuid::new_v4(),
            role: Role::Admin,
            sid: Uuid::new_v4(),
            iss: ISSUER.into(),
            aud: AUDIENCE.into(),
            iat: Utc::now().timestamp(),
            exp: (Utc::now() + Duration::minutes(5)).timestamp(),
        };
        let token = encode(
            &Header::new(Algorithm::HS512),
            &claims,
            &EncodingKey::from_secret(SECRET),
        )
        .unwrap();
        assert!(keys().verify_access(&token).is_err());
    }

    #[test]
    fn unsigned_none_algorithm_token_is_rejected() {
        let header = URL_SAFE_NO_PAD.encode(br#"{"alg":"none","typ":"JWT"}"#);
        let payload = URL_SAFE_NO_PAD.encode(
            br#"{"sub":"00000000-0000-0000-0000-000000000000","role":"admin","sid":"00000000-0000-0000-0000-000000000000","iss":"zeno","aud":"zeno-api","iat":0,"exp":9999999999}"#,
        );
        assert!(keys()
            .verify_access(&format!("{header}.{payload}."))
            .is_err());
    }

    #[test]
    fn wrong_audience_or_issuer_is_rejected() {
        for (iss, aud) in [("zeno", "someone-else"), ("someone-else", "zeno-api")] {
            let claims = AccessClaims {
                sub: Uuid::new_v4(),
                role: Role::Student,
                sid: Uuid::new_v4(),
                iss: iss.into(),
                aud: aud.into(),
                iat: Utc::now().timestamp(),
                exp: (Utc::now() + Duration::minutes(5)).timestamp(),
            };
            let token = encode(
                &Header::new(Algorithm::HS256),
                &claims,
                &EncodingKey::from_secret(SECRET),
            )
            .unwrap();
            assert!(keys().verify_access(&token).is_err(), "{iss}/{aud}");
        }
    }

    #[test]
    fn garbage_is_rejected_without_panicking() {
        for junk in ["", "a.b.c", "....", "Bearer x", "\u{0}"] {
            assert!(keys().verify_access(junk).is_err());
        }
    }

    #[test]
    fn opaque_tokens_are_long_unique_and_hash_deterministically() {
        let a = generate_opaque_token();
        let b = generate_opaque_token();
        assert_eq!(a.len(), 43);
        assert_ne!(a, b);
        assert_eq!(hash_opaque_token(&a), hash_opaque_token(&a));
        assert_ne!(hash_opaque_token(&a), hash_opaque_token(&b));
        assert_eq!(hash_opaque_token(&a).len(), 32);
    }
}
