use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use hmac::{Hmac, Mac};
use rand::{RngCore, rngs::OsRng};
use sha2::Sha256;
use std::fmt;
use subtle::ConstantTimeEq;
use thiserror::Error;
use uuid::Uuid;
use zeroize::{Zeroize, ZeroizeOnDrop};

type HmacSha256 = Hmac<Sha256>;
const TOKEN_BYTES: usize = 32;
const MAX_TOKEN_TTL_SECONDS: u64 = 31_536_000;

#[derive(Zeroize, ZeroizeOnDrop)]
pub struct SecretToken(String);

impl SecretToken {
    /// Parses a URL-safe, unpadded opaque token.
    ///
    /// # Errors
    ///
    /// Returns [`TokenError::Malformed`] when `encoded` is not valid URL-safe
    /// Base64 or does not decode to the required token length.
    pub fn parse(encoded: String) -> Result<Self, TokenError> {
        let decoded = URL_SAFE_NO_PAD
            .decode(encoded.as_bytes())
            .map_err(|_| TokenError::Malformed)?;
        if decoded.len() != TOKEN_BYTES {
            return Err(TokenError::Malformed);
        }
        Ok(Self(encoded))
    }

    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for SecretToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SecretToken([REDACTED])")
    }
}

#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct TokenKey([u8; 32]);

impl TokenKey {
    #[must_use]
    pub const fn new(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }
}

impl fmt::Debug for TokenKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("TokenKey([REDACTED])")
    }
}

#[derive(Clone, Copy, Eq, Hash, PartialEq)]
pub struct TokenDigest([u8; 32]);

impl TokenDigest {
    #[must_use]
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Debug for TokenDigest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("TokenDigest([REDACTED])")
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TokenKind {
    Access,
    Refresh,
    PasswordReset,
}

#[derive(Debug)]
pub struct IssuedToken {
    pub secret: SecretToken,
    pub record: TokenRecord,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TokenRecord {
    pub digest: TokenDigest,
    pub subject: Uuid,
    pub kind: TokenKind,
    pub issued_at: u64,
    pub expires_at: u64,
    pub revoked_at: Option<u64>,
}

impl TokenRecord {
    #[must_use]
    pub fn is_active_at(&self, now: u64) -> bool {
        self.revoked_at.is_none() && now >= self.issued_at && now < self.expires_at
    }

    pub fn revoke(&mut self, at: u64) -> bool {
        if self.revoked_at.is_some() {
            return false;
        }
        self.revoked_at = Some(at);
        true
    }
}

#[derive(Clone, Debug)]
pub struct TokenService {
    key: TokenKey,
}

impl TokenService {
    #[must_use]
    pub const fn new(key: TokenKey) -> Self {
        Self { key }
    }

    /// Issues a random opaque token and its persistable digest record.
    ///
    /// # Errors
    ///
    /// Returns [`TokenError::InvalidTtl`] when `ttl_seconds` is outside the
    /// supported range or its expiry would overflow, and propagates digest
    /// initialization failures.
    pub fn issue(
        &self,
        subject: Uuid,
        kind: TokenKind,
        now: u64,
        ttl_seconds: u64,
    ) -> Result<IssuedToken, TokenError> {
        if ttl_seconds == 0 || ttl_seconds > MAX_TOKEN_TTL_SECONDS {
            return Err(TokenError::InvalidTtl(ttl_seconds));
        }
        let expires_at = now
            .checked_add(ttl_seconds)
            .ok_or(TokenError::InvalidTtl(ttl_seconds))?;
        let mut raw = [0_u8; TOKEN_BYTES];
        OsRng.fill_bytes(&mut raw);
        let encoded = URL_SAFE_NO_PAD.encode(raw);
        raw.zeroize();
        let secret = SecretToken(encoded);
        let digest = self.digest(&secret)?;
        Ok(IssuedToken {
            secret,
            record: TokenRecord {
                digest,
                subject,
                kind,
                issued_at: now,
                expires_at,
                revoked_at: None,
            },
        })
    }

    /// Computes the keyed digest used to persist and compare a token.
    ///
    /// # Errors
    ///
    /// Returns [`TokenError::DigestInitialization`] if the HMAC state cannot be
    /// initialized from the service key.
    pub fn digest(&self, token: &SecretToken) -> Result<TokenDigest, TokenError> {
        let mut mac = HmacSha256::new_from_slice(&self.key.0)
            .map_err(|_| TokenError::DigestInitialization)?;
        mac.update(token.expose().as_bytes());
        Ok(TokenDigest(mac.finalize().into_bytes().into()))
    }

    /// Returns whether the token is active and matches the stored digest.
    #[must_use]
    pub fn verify(&self, token: &SecretToken, record: &TokenRecord, now: u64) -> bool {
        if !record.is_active_at(now) {
            return false;
        }
        self.digest(token)
            .is_ok_and(|candidate| candidate.as_bytes().ct_eq(record.digest.as_bytes()).into())
    }
}

#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum TokenError {
    #[error("token is malformed")]
    Malformed,
    #[error("token TTL {0}s is outside 1..=31536000s")]
    InvalidTtl(u64),
    #[error("token digest initialization failed")]
    DigestInitialization,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opaque_token_is_digest_only_revocable_and_expiring() {
        let service = TokenService::new(TokenKey::new([7; 32]));
        let mut issued = service
            .issue(Uuid::new_v4(), TokenKind::Refresh, 100, 30)
            .unwrap_or_else(|error| panic!("token issue failed: {error}"));
        assert!(service.verify(&issued.secret, &issued.record, 100));
        assert!(!service.verify(&issued.secret, &issued.record, 130));
        assert!(issued.record.revoke(110));
        assert!(!service.verify(&issued.secret, &issued.record, 111));
        assert!(!format!("{:?}", issued.secret).contains(issued.secret.expose()));
    }

    #[test]
    fn another_server_key_cannot_validate_digest() {
        let issuer = TokenService::new(TokenKey::new([1; 32]));
        let verifier = TokenService::new(TokenKey::new([2; 32]));
        let issued_token = issuer
            .issue(Uuid::new_v4(), TokenKind::Access, 1, 10)
            .unwrap_or_else(|error| panic!("token issue failed: {error}"));
        assert!(!verifier.verify(&issued_token.secret, &issued_token.record, 2));
    }
}
