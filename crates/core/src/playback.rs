//! Playback tokens: what a player carries to fetch playlists from the API and the content key
//! from the key server.
//!
//! A token names one playback session, the video and user it belongs to, and when it expires,
//! signed with HMAC-SHA256 under `PLAYBACK_TOKEN_SECRET` (shared by the API and the key server).
//! The token alone is never enough: both services also check that the session it names is still
//! open, so ending a session takes effect at once rather than when the token expires.
//!
//! Format: `v1.` + base64url(payload) + `.` + base64url(tag), where the payload is the fixed
//! 56 bytes `session_id || video_id || user_id || expires_at (unix seconds, big endian)`. A fixed
//! binary layout leaves nothing to parse ambiguously.

use std::fmt;
use std::sync::Arc;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use chrono::{DateTime, TimeZone, Utc};
use ring::hmac;
use uuid::Uuid;

use crate::config::{required, ConfigError};

const PREFIX: &str = "v1.";
const PAYLOAD_LEN: usize = 16 * 3 + 8;
const TAG_LEN: usize = 32;
/// Domain separation: a tag made for anything else under the same secret never verifies here.
const CONTEXT: &[u8] = b"zeno-playback-token-v1";
/// Longest token accepted before any decoding is attempted.
const MAX_TOKEN_LEN: usize = 160;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlaybackClaims {
    pub session_id: Uuid,
    pub video_id: Uuid,
    pub user_id: Uuid,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum TokenError {
    #[error("malformed playback token")]
    Malformed,
    #[error("playback token signature is invalid")]
    BadSignature,
    #[error("playback token has expired")]
    Expired,
}

#[derive(Clone)]
pub struct PlaybackSigner {
    key: Arc<hmac::Key>,
}

impl fmt::Debug for PlaybackSigner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PlaybackSigner(..)")
    }
}

impl PlaybackSigner {
    pub fn from_env() -> Result<Self, ConfigError> {
        Self::new(&required("PLAYBACK_TOKEN_SECRET")?)
    }

    /// `secret` must be at least 32 bytes (`openssl rand -base64 48`).
    pub fn new(secret: &str) -> Result<Self, ConfigError> {
        if secret.len() < 32 {
            return Err(ConfigError::Invalid {
                name: "PLAYBACK_TOKEN_SECRET",
                reason: "must be at least 32 bytes".into(),
            });
        }
        Ok(Self {
            key: Arc::new(hmac::Key::new(hmac::HMAC_SHA256, secret.as_bytes())),
        })
    }

    fn tag(&self, payload: &[u8]) -> hmac::Tag {
        let mut ctx = hmac::Context::with_key(&self.key);
        ctx.update(CONTEXT);
        ctx.update(payload);
        ctx.sign()
    }

    pub fn sign(&self, claims: &PlaybackClaims) -> String {
        let payload = encode(claims);
        let tag = self.tag(&payload);
        format!(
            "{PREFIX}{}.{}",
            URL_SAFE_NO_PAD.encode(payload),
            URL_SAFE_NO_PAD.encode(tag.as_ref())
        )
    }

    /// Checks the signature first and only then looks at what the token claims.
    pub fn verify(&self, token: &str, now: DateTime<Utc>) -> Result<PlaybackClaims, TokenError> {
        if token.len() > MAX_TOKEN_LEN {
            return Err(TokenError::Malformed);
        }
        let rest = token.strip_prefix(PREFIX).ok_or(TokenError::Malformed)?;
        let (payload, tag) = rest.split_once('.').ok_or(TokenError::Malformed)?;
        let payload = URL_SAFE_NO_PAD
            .decode(payload)
            .map_err(|_| TokenError::Malformed)?;
        let tag = URL_SAFE_NO_PAD
            .decode(tag)
            .map_err(|_| TokenError::Malformed)?;
        if payload.len() != PAYLOAD_LEN || tag.len() != TAG_LEN {
            return Err(TokenError::Malformed);
        }
        let mut message = Vec::with_capacity(CONTEXT.len() + payload.len());
        message.extend_from_slice(CONTEXT);
        message.extend_from_slice(&payload);
        hmac::verify(&self.key, &message, &tag).map_err(|_| TokenError::BadSignature)?;

        let claims = decode(&payload).ok_or(TokenError::Malformed)?;
        if claims.expires_at <= now {
            return Err(TokenError::Expired);
        }
        Ok(claims)
    }
}

fn encode(claims: &PlaybackClaims) -> Vec<u8> {
    let mut out = Vec::with_capacity(PAYLOAD_LEN);
    out.extend_from_slice(claims.session_id.as_bytes());
    out.extend_from_slice(claims.video_id.as_bytes());
    out.extend_from_slice(claims.user_id.as_bytes());
    out.extend_from_slice(&claims.expires_at.timestamp().to_be_bytes());
    out
}

fn decode(payload: &[u8]) -> Option<PlaybackClaims> {
    let uuid = |range: std::ops::Range<usize>| Uuid::from_slice(&payload[range]).ok();
    let seconds = i64::from_be_bytes(payload[48..56].try_into().ok()?);
    Some(PlaybackClaims {
        session_id: uuid(0..16)?,
        video_id: uuid(16..32)?,
        user_id: uuid(32..48)?,
        expires_at: Utc.timestamp_opt(seconds, 0).single()?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    const SECRET: &str = "a-test-secret-that-is-long-enough-to-use";

    fn signer() -> PlaybackSigner {
        PlaybackSigner::new(SECRET).unwrap()
    }

    fn claims(expires_in: Duration) -> PlaybackClaims {
        PlaybackClaims {
            session_id: Uuid::new_v4(),
            video_id: Uuid::new_v4(),
            user_id: Uuid::new_v4(),
            // Whole seconds: that is the precision the token carries.
            expires_at: Utc
                .timestamp_opt((Utc::now() + expires_in).timestamp(), 0)
                .unwrap(),
        }
    }

    #[test]
    fn a_signed_token_verifies_back_to_the_same_claims() {
        let c = claims(Duration::hours(1));
        let token = signer().sign(&c);
        assert!(token.starts_with("v1."));
        assert!(token.len() <= MAX_TOKEN_LEN);
        assert!(
            token
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || "-_.".contains(ch)),
            "safe to put in a URL unescaped: {token}"
        );
        assert_eq!(signer().verify(&token, Utc::now()), Ok(c));
    }

    #[test]
    fn an_expired_token_is_refused_even_though_its_signature_is_good() {
        let c = claims(Duration::seconds(10));
        let token = signer().sign(&c);
        assert_eq!(
            signer().verify(&token, c.expires_at),
            Err(TokenError::Expired)
        );
        assert!(signer()
            .verify(&token, c.expires_at - Duration::seconds(1))
            .is_ok());
    }

    #[test]
    fn any_change_to_the_token_breaks_it() {
        let token = signer().sign(&claims(Duration::hours(1)));
        let bytes = token.as_bytes();
        for index in 0..bytes.len() {
            for replacement in *b"Ax-." {
                if bytes[index] == replacement {
                    continue;
                }
                let mut changed = bytes.to_vec();
                changed[index] = replacement;
                let changed = String::from_utf8(changed).unwrap();
                assert!(
                    signer().verify(&changed, Utc::now()).is_err(),
                    "{changed} still verified"
                );
            }
        }
    }

    #[test]
    fn swapping_in_another_payload_fails_the_signature() {
        let a = signer().sign(&claims(Duration::hours(1)));
        let b = signer().sign(&claims(Duration::hours(1)));
        let (a_payload, _) = a[3..].split_once('.').unwrap();
        let (_, b_tag) = b[3..].split_once('.').unwrap();
        let spliced = format!("v1.{a_payload}.{b_tag}");
        assert_eq!(
            signer().verify(&spliced, Utc::now()),
            Err(TokenError::BadSignature)
        );
    }

    #[test]
    fn a_token_from_another_secret_is_refused() {
        let other = PlaybackSigner::new("another-secret-that-is-also-long-enough").unwrap();
        let token = other.sign(&claims(Duration::hours(1)));
        assert_eq!(
            signer().verify(&token, Utc::now()),
            Err(TokenError::BadSignature)
        );
    }

    #[test]
    fn garbage_is_refused_without_panicking() {
        let huge = "v1.".to_string() + &"A".repeat(10_000);
        for bad in [
            "",
            "v1.",
            "v1..",
            "v2.abc.def",
            "v1.abc",
            "v1.!!!.???",
            "v1.AAAA.AAAA",
            huge.as_str(),
        ] {
            assert!(signer().verify(bad, Utc::now()).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn short_secrets_are_rejected_and_debug_hides_the_key() {
        assert!(PlaybackSigner::new("short").is_err());
        assert_eq!(format!("{:?}", signer()), "PlaybackSigner(..)");
    }
}
