use std::sync::Arc;

use anyhow::anyhow;
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::{Algorithm, Argon2, Params, Version};
use rand::rngs::OsRng;
use rand::RngCore;
use tokio::sync::Semaphore;

use crate::error::ApiError;

#[derive(Debug, Clone, Copy)]
pub struct PasswordParams {
    pub memory_kib: u32,
    pub iterations: u32,
    pub parallelism: u32,
}

impl Default for PasswordParams {
    /// OWASP's minimum recommended Argon2id configuration (19 MiB, t=2, p=1).
    fn default() -> Self {
        Self {
            memory_kib: 19_456,
            iterations: 2,
            parallelism: 1,
        }
    }
}

impl PasswordParams {
    /// Deliberately weak so tests stay fast. Never use outside tests.
    pub fn for_tests() -> Self {
        Self {
            memory_kib: 8,
            iterations: 1,
            parallelism: 1,
        }
    }
}

struct Inner {
    argon2: Argon2<'static>,
    /// Verified against when the account does not exist, so "unknown email" and
    /// "wrong password" cost the same and cannot be told apart by timing.
    dummy_hash: String,
    permits: Arc<Semaphore>,
}

/// Argon2id hashing that runs off the async runtime and never uses more blocking threads
/// than there are CPUs, so a burst of login attempts cannot exhaust memory.
#[derive(Clone)]
pub struct Passwords(Arc<Inner>);

impl Passwords {
    pub fn new(params: PasswordParams) -> anyhow::Result<Self> {
        let params = Params::new(
            params.memory_kib,
            params.iterations,
            params.parallelism,
            None,
        )
        .map_err(|e| anyhow!("invalid argon2 parameters: {e}"))?;
        let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);

        let mut filler = [0u8; 24];
        OsRng.fill_bytes(&mut filler);
        let dummy_hash = hash_with(&argon2, &filler)?;

        let workers = std::thread::available_parallelism().map_or(2, |n| n.get());
        Ok(Self(Arc::new(Inner {
            argon2,
            dummy_hash,
            permits: Arc::new(Semaphore::new(workers)),
        })))
    }

    pub async fn hash(&self, password: String) -> Result<String, ApiError> {
        let inner = self.0.clone();
        let permit = inner
            .permits
            .clone()
            .acquire_owned()
            .await
            .map_err(|e| anyhow!(e))?;
        let hash = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            hash_with(&inner.argon2, password.as_bytes())
        })
        .await??;
        Ok(hash)
    }

    /// `stored_hash` is `None` when the account does not exist; the check still runs
    /// against a dummy hash and always returns `false`.
    pub async fn verify(
        &self,
        password: String,
        stored_hash: Option<String>,
    ) -> Result<bool, ApiError> {
        let inner = self.0.clone();
        let account_exists = stored_hash.is_some();
        let hash = stored_hash.unwrap_or_else(|| inner.dummy_hash.clone());
        let permit = inner
            .permits
            .clone()
            .acquire_owned()
            .await
            .map_err(|e| anyhow!(e))?;
        let matches = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            verify_with(&inner.argon2, password.as_bytes(), &hash)
        })
        .await?;
        Ok(matches && account_exists)
    }
}

fn hash_with(argon2: &Argon2<'_>, password: &[u8]) -> anyhow::Result<String> {
    let mut salt_bytes = [0u8; 16];
    OsRng.fill_bytes(&mut salt_bytes);
    let salt = SaltString::encode_b64(&salt_bytes).map_err(|e| anyhow!("salt encoding: {e}"))?;
    let hash = argon2
        .hash_password(password, &salt)
        .map_err(|e| anyhow!("password hashing: {e}"))?;
    Ok(hash.to_string())
}

fn verify_with(argon2: &Argon2<'_>, password: &[u8], hash: &str) -> bool {
    match PasswordHash::new(hash) {
        Ok(parsed) => argon2.verify_password(password, &parsed).is_ok(),
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn passwords() -> Passwords {
        Passwords::new(PasswordParams::for_tests()).unwrap()
    }

    #[tokio::test]
    async fn correct_password_verifies_and_wrong_one_does_not() {
        let p = passwords();
        let hash = p.hash("correct horse battery".into()).await.unwrap();
        assert!(p
            .verify("correct horse battery".into(), Some(hash.clone()))
            .await
            .unwrap());
        assert!(!p
            .verify("Correct horse battery".into(), Some(hash))
            .await
            .unwrap());
    }

    #[tokio::test]
    async fn hashes_are_argon2id_and_salted() {
        let p = passwords();
        let a = p.hash("same password".into()).await.unwrap();
        let b = p.hash("same password".into()).await.unwrap();
        assert!(a.starts_with("$argon2id$v=19$"));
        assert_ne!(a, b);
    }

    #[tokio::test]
    async fn missing_account_never_verifies() {
        let p = passwords();
        assert!(!p.verify("anything".into(), None).await.unwrap());
    }

    #[tokio::test]
    async fn garbage_stored_hash_does_not_verify_or_panic() {
        let p = passwords();
        assert!(!p
            .verify("x".into(), Some("not-a-phc-string".into()))
            .await
            .unwrap());
    }

    #[tokio::test]
    async fn old_hashes_still_verify_after_params_change() {
        let old = Passwords::new(PasswordParams {
            memory_kib: 8,
            iterations: 1,
            parallelism: 1,
        })
        .unwrap();
        let hash = old.hash("pw-from-the-past".into()).await.unwrap();
        let newer = Passwords::new(PasswordParams {
            memory_kib: 16,
            iterations: 2,
            parallelism: 1,
        })
        .unwrap();
        assert!(newer
            .verify("pw-from-the-past".into(), Some(hash))
            .await
            .unwrap());
    }

    #[test]
    fn invalid_params_are_rejected_at_startup() {
        assert!(Passwords::new(PasswordParams {
            memory_kib: 1,
            iterations: 0,
            parallelism: 0
        })
        .is_err());
    }
}
