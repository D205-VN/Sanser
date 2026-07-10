use argon2::{
    Algorithm, Argon2, Params, Version,
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
};
use rand::rngs::OsRng;
use std::fmt;
use thiserror::Error;
use zeroize::{Zeroize, ZeroizeOnDrop};

const MAX_PASSWORD_BYTES: usize = 1_024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PasswordPolicy {
    pub minimum_characters: usize,
}

impl PasswordPolicy {
    pub fn validate(self, password: &str) -> Result<(), PasswordPolicyError> {
        if self.minimum_characters < 8 || self.minimum_characters > 128 {
            return Err(PasswordPolicyError::InvalidMinimum(self.minimum_characters));
        }
        if password.len() > MAX_PASSWORD_BYTES {
            return Err(PasswordPolicyError::TooLong);
        }
        if password.chars().count() < self.minimum_characters {
            return Err(PasswordPolicyError::TooShort {
                minimum: self.minimum_characters,
            });
        }
        if password.chars().all(char::is_whitespace) {
            return Err(PasswordPolicyError::OnlyWhitespace);
        }
        Ok(())
    }
}

impl Default for PasswordPolicy {
    fn default() -> Self {
        Self {
            minimum_characters: 10,
        }
    }
}

#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct PasswordHashString(String);

impl PasswordHashString {
    pub fn parse(encoded: String) -> Result<Self, PasswordError> {
        let parsed = PasswordHash::new(&encoded).map_err(|_| PasswordError::InvalidHash)?;
        if parsed.algorithm.as_str() != "argon2id" {
            return Err(PasswordError::UnsupportedHash);
        }
        Ok(Self(encoded))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for PasswordHashString {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PasswordHashString([REDACTED])")
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PasswordHasherService {
    policy: PasswordPolicy,
    memory_kib: u32,
    iterations: u32,
    lanes: u32,
}

impl PasswordHasherService {
    pub fn new(
        policy: PasswordPolicy,
        memory_kib: u32,
        iterations: u32,
        lanes: u32,
    ) -> Result<Self, PasswordError> {
        build_argon2(memory_kib, iterations, lanes)?;
        Ok(Self {
            policy,
            memory_kib,
            iterations,
            lanes,
        })
    }

    pub fn hash(&self, password: &str) -> Result<PasswordHashString, PasswordError> {
        self.policy.validate(password)?;
        let salt = SaltString::generate(&mut OsRng);
        let encoded = self
            .argon2()?
            .hash_password(password.as_bytes(), &salt)
            .map_err(|_| PasswordError::HashingFailed)?
            .to_string();
        PasswordHashString::parse(encoded)
    }

    /// Returns a generic false result for any malformed/incorrect credential.
    /// Callers should not expose parsing details to a login endpoint.
    pub fn verify(&self, password: &str, stored: &PasswordHashString) -> bool {
        let Ok(parsed) = PasswordHash::new(stored.as_str()) else {
            return false;
        };
        self.argon2()
            .and_then(|hasher| {
                hasher
                    .verify_password(password.as_bytes(), &parsed)
                    .map_err(|_| PasswordError::InvalidCredentials)
            })
            .is_ok()
    }

    fn argon2(&self) -> Result<Argon2<'static>, PasswordError> {
        build_argon2(self.memory_kib, self.iterations, self.lanes)
    }
}

impl Default for PasswordHasherService {
    fn default() -> Self {
        // OWASP's 19 MiB / 2 iteration Argon2id baseline.
        Self {
            policy: PasswordPolicy::default(),
            memory_kib: 19_456,
            iterations: 2,
            lanes: 1,
        }
    }
}

fn build_argon2(
    memory_kib: u32,
    iterations: u32,
    lanes: u32,
) -> Result<Argon2<'static>, PasswordError> {
    let params = Params::new(memory_kib, iterations, lanes, Some(32))
        .map_err(|_| PasswordError::InvalidParameters)?;
    Ok(Argon2::new(Algorithm::Argon2id, Version::V0x13, params))
}

#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum PasswordPolicyError {
    #[error("password policy minimum {0} is outside 8..=128 characters")]
    InvalidMinimum(usize),
    #[error("password does not meet the minimum of {minimum} characters")]
    TooShort { minimum: usize },
    #[error("password exceeds the maximum encoded length")]
    TooLong,
    #[error("password cannot contain only whitespace")]
    OnlyWhitespace,
}

#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum PasswordError {
    #[error(transparent)]
    Policy(#[from] PasswordPolicyError),
    #[error("invalid Argon2id parameters")]
    InvalidParameters,
    #[error("password hashing failed")]
    HashingFailed,
    #[error("stored password hash is invalid")]
    InvalidHash,
    #[error("stored password hash does not use Argon2id")]
    UnsupportedHash,
    #[error("invalid credentials")]
    InvalidCredentials,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashes_with_argon2id_and_never_debugs_the_hash() {
        let service = PasswordHasherService::new(PasswordPolicy::default(), 8_192, 1, 1)
            .unwrap_or_else(|error| panic!("hasher config failed: {error}"));
        let stored = service
            .hash("correct horse battery staple")
            .unwrap_or_else(|error| panic!("hash failed: {error}"));
        assert!(stored.as_str().starts_with("$argon2id$v=19$"));
        assert!(service.verify("correct horse battery staple", &stored));
        assert!(!service.verify("incorrect password", &stored));
        assert_eq!(format!("{stored:?}"), "PasswordHashString([REDACTED])");
    }

    #[test]
    fn policy_supports_unicode_characters_without_weak_empty_values() {
        let policy = PasswordPolicy {
            minimum_characters: 8,
        };
        assert!(policy.validate("mật-khẩu-ổn").is_ok());
        assert_eq!(
            policy.validate("        "),
            Err(PasswordPolicyError::OnlyWhitespace)
        );
    }
}
