//! Encryption of video content keys at rest.
//!
//! Every video's AES-128 segment key is stored in `video_keys` wrapped with AES-256-GCM under a
//! master secret that lives only in the environment of the services that need it. A copy of the
//! database (a backup, a SQL injection, a stray replica) therefore does not unlock the video
//! segments sitting in object storage.
//!
//! Stored blob: `key version (1 byte) || nonce (12) || ciphertext (16) || tag (16)`. The video id is
//! bound in as associated data, so a wrapped key copied onto another video's row fails to open.
//! New blobs use the active version; retained versions keep old blobs readable during rotation.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use base64::Engine;
use ring::aead::{Aad, LessSafeKey, Nonce, UnboundKey, AES_256_GCM, NONCE_LEN};
use ring::rand::{SecureRandom, SystemRandom};
use uuid::Uuid;

use crate::config::{required, ConfigError};

pub const CONTENT_KEY_LEN: usize = 16;
const TAG_LEN: usize = 16;
/// Size of a wrapped key as stored.
pub const WRAPPED_LEN: usize = 1 + NONCE_LEN + CONTENT_KEY_LEN + TAG_LEN;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum KeyVaultError {
    #[error("the stored key is malformed or was not sealed for this video with this secret")]
    CannotOpen,
    #[error("the system random number generator failed")]
    Random,
}

#[derive(Clone)]
pub struct KeyVault {
    active: u8,
    keys: Arc<BTreeMap<u8, LessSafeKey>>,
    rng: Arc<SystemRandom>,
}

impl fmt::Debug for KeyVault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("KeyVault(..)")
    }
}

impl KeyVault {
    /// Reads the versioned keyring when configured, falling back to the legacy single secret.
    pub fn from_env() -> Result<Self, ConfigError> {
        match std::env::var("CONTENT_KEY_SECRETS") {
            Ok(raw) if !raw.trim().is_empty() => {
                let active = required("CONTENT_KEY_ACTIVE")?
                    .trim()
                    .parse::<u8>()
                    .map_err(|_| ConfigError::Invalid {
                        name: "CONTENT_KEY_ACTIVE",
                        reason: "must be a key id from CONTENT_KEY_SECRETS".into(),
                    })?;
                Self::from_keyring(&raw, active)
            }
            _ => Self::from_secret(&required("CONTENT_KEY_SECRET")?),
        }
    }

    pub fn from_secret(encoded: &str) -> Result<Self, ConfigError> {
        let mut keys = BTreeMap::new();
        keys.insert(1, parse_key("CONTENT_KEY_SECRET", encoded)?);
        Ok(Self {
            active: 1,
            keys: Arc::new(keys),
            rng: Arc::new(SystemRandom::new()),
        })
    }

    /// Parses `id:base64,id:base64`. The id is written into every wrapped blob, allowing old
    /// keys to remain readable while new keys are sealed under the active version.
    pub fn from_keyring(raw: &str, active: u8) -> Result<Self, ConfigError> {
        let invalid = |reason: &str| ConfigError::Invalid {
            name: "CONTENT_KEY_SECRETS",
            reason: reason.into(),
        };
        if active == 0 {
            return Err(ConfigError::Invalid {
                name: "CONTENT_KEY_ACTIVE",
                reason: "version 0 is reserved".into(),
            });
        }
        let mut keys = BTreeMap::new();
        for entry in raw.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            let (id, encoded) = entry
                .split_once(':')
                .ok_or_else(|| invalid("expected comma-separated id:base64 entries"))?;
            let id = id
                .parse::<u8>()
                .map_err(|_| invalid("key ids must be integers from 1 to 255"))?;
            if id == 0 {
                return Err(invalid("version 0 is reserved"));
            }
            if keys
                .insert(id, parse_key("CONTENT_KEY_SECRETS", encoded)?)
                .is_some()
            {
                return Err(invalid("key ids must be unique"));
            }
        }
        if !keys.contains_key(&active) {
            return Err(ConfigError::Invalid {
                name: "CONTENT_KEY_ACTIVE",
                reason: "does not name a key in CONTENT_KEY_SECRETS".into(),
            });
        }
        Ok(Self {
            active,
            keys: Arc::new(keys),
            rng: Arc::new(SystemRandom::new()),
        })
    }

    pub fn active_version(&self) -> u8 {
        self.active
    }

    pub fn seal(
        &self,
        video_id: Uuid,
        content_key: &[u8; CONTENT_KEY_LEN],
    ) -> Result<Vec<u8>, KeyVaultError> {
        let mut nonce = [0u8; NONCE_LEN];
        self.rng
            .fill(&mut nonce)
            .map_err(|_| KeyVaultError::Random)?;
        let mut out = Vec::with_capacity(WRAPPED_LEN);
        out.push(self.active);
        out.extend_from_slice(&nonce);
        out.extend_from_slice(content_key);
        let tag = self
            .keys
            .get(&self.active)
            .expect("active key was validated")
            .seal_in_place_separate_tag(
                Nonce::assume_unique_for_key(nonce),
                Aad::from(video_id.as_bytes()),
                &mut out[1 + NONCE_LEN..],
            )
            .map_err(|_| KeyVaultError::Random)?;
        out.extend_from_slice(tag.as_ref());
        Ok(out)
    }

    pub fn open(
        &self,
        video_id: Uuid,
        wrapped: &[u8],
    ) -> Result<[u8; CONTENT_KEY_LEN], KeyVaultError> {
        if wrapped.len() != WRAPPED_LEN {
            return Err(KeyVaultError::CannotOpen);
        }
        let key = self
            .keys
            .get(&wrapped[0])
            .ok_or(KeyVaultError::CannotOpen)?;
        let nonce: [u8; NONCE_LEN] = wrapped[1..1 + NONCE_LEN]
            .try_into()
            .map_err(|_| KeyVaultError::CannotOpen)?;
        let mut buffer = wrapped[1 + NONCE_LEN..].to_vec();
        let plain = key
            .open_in_place(
                Nonce::assume_unique_for_key(nonce),
                Aad::from(video_id.as_bytes()),
                &mut buffer,
            )
            .map_err(|_| KeyVaultError::CannotOpen)?;
        plain.try_into().map_err(|_| KeyVaultError::CannotOpen)
    }

    /// Opens a blob under any retained version and seals it under the active one. Safe to retry.
    pub fn rewrap(&self, video_id: Uuid, wrapped: &[u8]) -> Result<Vec<u8>, KeyVaultError> {
        if wrapped.first() == Some(&self.active) {
            // Authenticate before returning it: a corrupt active-version blob must not be kept.
            self.open(video_id, wrapped)?;
            return Ok(wrapped.to_vec());
        }
        let plain = self.open(video_id, wrapped)?;
        self.seal(video_id, &plain)
    }
}

fn parse_key(name: &'static str, encoded: &str) -> Result<LessSafeKey, ConfigError> {
    let invalid = |reason: &str| ConfigError::Invalid {
        name,
        reason: reason.into(),
    };
    let raw = base64::engine::general_purpose::STANDARD
        .decode(encoded.trim())
        .map_err(|_| invalid("must be base64 (openssl rand -base64 32)"))?;
    if raw.len() != 32 {
        return Err(invalid("must decode to exactly 32 bytes"));
    }
    if raw.iter().all(|byte| *byte == raw[0]) {
        return Err(invalid("must be random, not a repeated byte"));
    }
    let unbound =
        UnboundKey::new(&AES_256_GCM, &raw).map_err(|_| invalid("not a usable AES-256 key"))?;
    Ok(LessSafeKey::new(unbound))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secret(byte_seed: u8) -> String {
        let raw: Vec<u8> = (0..32u8)
            .map(|i| i.wrapping_mul(7).wrapping_add(byte_seed))
            .collect();
        base64::engine::general_purpose::STANDARD.encode(raw)
    }

    fn vault(seed: u8) -> KeyVault {
        KeyVault::from_secret(&secret(seed)).unwrap()
    }

    const KEY: [u8; 16] = *b"0123456789abcdef";

    #[test]
    fn a_sealed_key_opens_for_the_same_video_and_secret() {
        let id = Uuid::new_v4();
        let blob = vault(1).seal(id, &KEY).unwrap();
        assert_eq!(blob.len(), WRAPPED_LEN);
        assert_eq!(vault(1).open(id, &blob).unwrap(), KEY);
    }

    #[test]
    fn the_stored_blob_does_not_contain_the_key() {
        let blob = vault(1).seal(Uuid::new_v4(), &KEY).unwrap();
        assert!(!blob.windows(KEY.len()).any(|window| window == KEY));
    }

    #[test]
    fn sealing_twice_gives_different_blobs() {
        let id = Uuid::new_v4();
        let v = vault(1);
        assert_ne!(v.seal(id, &KEY).unwrap(), v.seal(id, &KEY).unwrap());
    }

    #[test]
    fn a_key_copied_to_another_videos_row_does_not_open() {
        let blob = vault(1).seal(Uuid::new_v4(), &KEY).unwrap();
        assert_eq!(
            vault(1).open(Uuid::new_v4(), &blob),
            Err(KeyVaultError::CannotOpen)
        );
    }

    #[test]
    fn the_wrong_master_secret_does_not_open() {
        let id = Uuid::new_v4();
        let blob = vault(1).seal(id, &KEY).unwrap();
        assert_eq!(vault(2).open(id, &blob), Err(KeyVaultError::CannotOpen));
    }

    #[test]
    fn any_flipped_bit_is_detected() {
        let id = Uuid::new_v4();
        let v = vault(1);
        let blob = v.seal(id, &KEY).unwrap();
        for index in 0..blob.len() {
            let mut tampered = blob.clone();
            tampered[index] ^= 0x01;
            assert_eq!(
                v.open(id, &tampered),
                Err(KeyVaultError::CannotOpen),
                "byte {index}"
            );
        }
    }

    #[test]
    fn malformed_blobs_are_refused_without_panicking() {
        let id = Uuid::new_v4();
        let v = vault(1);
        let blob = v.seal(id, &KEY).unwrap();
        for bad in [
            &[][..],
            &[1][..],
            &blob[..blob.len() - 1],
            &[blob.clone(), vec![0]].concat()[..],
        ] {
            assert_eq!(v.open(id, bad), Err(KeyVaultError::CannotOpen));
        }
        // A raw, unwrapped 16-byte key (the old format) is never mistaken for a wrapped one.
        assert_eq!(v.open(id, &KEY), Err(KeyVaultError::CannotOpen));
    }

    #[test]
    fn weak_or_malformed_secrets_are_rejected() {
        let engine = base64::engine::general_purpose::STANDARD;
        for bad in [
            "".to_string(),
            "not base64!!".to_string(),
            engine.encode([7u8; 31]),
            engine.encode([7u8; 33]),
            engine.encode([0u8; 32]),
            engine.encode([7u8; 32]),
        ] {
            assert!(KeyVault::from_secret(&bad).is_err(), "{bad:?}");
        }
        assert!(
            KeyVault::from_secret(&format!("  {}\n", secret(3))).is_ok(),
            "whitespace is trimmed"
        );
    }

    #[test]
    fn debug_output_never_shows_key_material() {
        assert_eq!(format!("{:?}", vault(1)), "KeyVault(..)");
    }

    #[test]
    fn rotation_reads_old_blobs_and_rewraps_them_under_the_active_key() {
        let id = Uuid::new_v4();
        let old = vault(1).seal(id, &KEY).unwrap();
        let ring = KeyVault::from_keyring(&format!("1:{},2:{}", secret(1), secret(2)), 2).unwrap();
        assert_eq!(ring.open(id, &old).unwrap(), KEY);
        let moved = ring.rewrap(id, &old).unwrap();
        assert_eq!(moved[0], 2);
        assert_eq!(ring.open(id, &moved).unwrap(), KEY);
        assert_eq!(ring.rewrap(id, &moved).unwrap(), moved);
        assert_eq!(vault(1).open(id, &moved), Err(KeyVaultError::CannotOpen));
    }

    #[test]
    fn keyring_configuration_is_strict() {
        assert!(KeyVault::from_keyring(&format!("1:{}", secret(1)), 2).is_err());
        assert!(KeyVault::from_keyring(&format!("1:{},1:{}", secret(1), secret(2)), 1).is_err());
        assert!(KeyVault::from_keyring(&format!("0:{}", secret(1)), 0).is_err());
    }
}
