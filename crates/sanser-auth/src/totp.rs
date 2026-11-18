//! Time-based one-time passwords (RFC 6238, HMAC-SHA1, 6 digits, 30 s) and
//! the encrypted-at-rest envelope used to store their shared secrets.

use std::fmt;

use chacha20poly1305::{
    XChaCha20Poly1305, XNonce,
    aead::{Aead, KeyInit, Payload},
};
use hmac::{Hmac, Mac};
use rand::{RngCore, rngs::OsRng};
use sha1::Sha1;
use sha2::Sha256;
use subtle::ConstantTimeEq;
use thiserror::Error;
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

pub const TOTP_DIGITS: u32 = 6;
pub const TOTP_PERIOD_SECONDS: i64 = 30;
/// Accept the previous and next step to tolerate small clock drift.
pub const TOTP_SKEW_STEPS: i64 = 1;
const SECRET_BYTES: usize = 20;
const BASE32_ALPHABET: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
/// Recovery codes avoid characters that are easy to misread (0/O, 1/I).
const RECOVERY_ALPHABET: &[u8; 32] = b"23456789ABCDEFGHJKLMNPQRSTUVWXYZ";
const ENVELOPE_VERSION: &str = "tfa1";

#[derive(Debug, Error, Eq, PartialEq)]
pub enum TotpError {
    #[error("the encrypted secret is malformed")]
    MalformedEnvelope,
    #[error("the encrypted secret could not be decrypted")]
    DecryptionFailed,
    #[error("the encryption key must be at least 32 bytes")]
    WeakKey,
}

/// A TOTP shared secret. Never printed or logged.
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct TotpSecret(Vec<u8>);

impl TotpSecret {
    #[must_use]
    pub fn generate() -> Self {
        let mut bytes = vec![0_u8; SECRET_BYTES];
        OsRng.fill_bytes(&mut bytes);
        Self(bytes)
    }

    #[must_use]
    pub fn from_bytes(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }

    /// Base32 (RFC 4648, no padding) form shown to the user for manual entry.
    #[must_use]
    pub fn to_base32(&self) -> Zeroizing<String> {
        Zeroizing::new(base32_encode(&self.0))
    }

    /// `otpauth://` URI understood by authenticator apps.
    #[must_use]
    pub fn otpauth_uri(&self, issuer: &str, account: &str) -> Zeroizing<String> {
        let label = format!("{}:{}", percent_encode(issuer), percent_encode(account));
        Zeroizing::new(format!(
            "otpauth://totp/{label}?secret={}&issuer={}&algorithm=SHA1&digits={TOTP_DIGITS}&period={TOTP_PERIOD_SECONDS}",
            self.to_base32().as_str(),
            percent_encode(issuer)
        ))
    }

    /// The code for one 30-second step.
    #[must_use]
    pub fn code_at_step(&self, step: i64) -> u32 {
        let Ok(mut mac) = <Hmac<Sha1> as Mac>::new_from_slice(&self.0) else {
            return u32::MAX;
        };
        mac.update(&step.to_be_bytes());
        let digest = mac.finalize().into_bytes();
        let offset = usize::from(digest[19] & 0x0f);
        let binary = (u32::from(digest[offset]) & 0x7f) << 24
            | u32::from(digest[offset + 1]) << 16
            | u32::from(digest[offset + 2]) << 8
            | u32::from(digest[offset + 3]);
        binary % 10_u32.pow(TOTP_DIGITS)
    }

    /// Verifies a user-supplied code and returns the matched step. Callers must
    /// persist the step and reject any later code whose step is not greater,
    /// so a code cannot be replayed within its validity window.
    #[must_use]
    pub fn verify(&self, code: &str, unix_time: i64, last_used_step: Option<i64>) -> Option<i64> {
        let code = normalize_totp_code(code)?;
        let current = unix_time.div_euclid(TOTP_PERIOD_SECONDS);
        let mut matched = None;
        // Evaluate every candidate step so timing does not reveal which matched.
        for step in (current - TOTP_SKEW_STEPS)..=(current + TOTP_SKEW_STEPS) {
            let expected = format!(
                "{:0width$}",
                self.code_at_step(step),
                width = TOTP_DIGITS as usize
            );
            let equal: bool = expected.as_bytes().ct_eq(code.as_bytes()).into();
            if equal && last_used_step.is_none_or(|last| step > last) && matched.is_none() {
                matched = Some(step);
            }
        }
        matched
    }
}

impl fmt::Debug for TotpSecret {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("TotpSecret([REDACTED])")
    }
}

/// Accepts "123456" and "123 456"; anything else is rejected before comparison.
#[must_use]
pub fn normalize_totp_code(code: &str) -> Option<String> {
    let digits: String = code
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect();
    (digits.len() == TOTP_DIGITS as usize && digits.bytes().all(|byte| byte.is_ascii_digit()))
        .then_some(digits)
}

/// Encrypts TOTP secrets at rest with XChaCha20-Poly1305. The account id is
/// authenticated as associated data so a ciphertext cannot be moved to
/// another account row.
pub struct SecretBox {
    cipher: XChaCha20Poly1305,
    key_id: &'static str,
}

impl SecretBox {
    /// Derives a dedicated key from `master` with a fixed domain label.
    ///
    /// # Errors
    ///
    /// Returns [`TotpError::WeakKey`] when `master` is shorter than 32 bytes.
    pub fn derive(master: &[u8], key_id: &'static str) -> Result<Self, TotpError> {
        if master.len() < 32 {
            return Err(TotpError::WeakKey);
        }
        let mut mac =
            <Hmac<Sha256> as Mac>::new_from_slice(master).map_err(|_| TotpError::WeakKey)?;
        mac.update(b"sanser/two-factor-secret/v1");
        let mut key: [u8; 32] = mac.finalize().into_bytes().into();
        let cipher = XChaCha20Poly1305::new(&key.into());
        key.zeroize();
        Ok(Self { cipher, key_id })
    }

    #[must_use]
    pub fn key_id(&self) -> &'static str {
        self.key_id
    }

    /// # Errors
    ///
    /// Returns [`TotpError::DecryptionFailed`] only if the AEAD rejects the input.
    pub fn seal(&self, secret: &TotpSecret, account_id: &str) -> Result<String, TotpError> {
        let mut nonce = [0_u8; 24];
        OsRng.fill_bytes(&mut nonce);
        let ciphertext = self
            .cipher
            .encrypt(
                XNonce::from_slice(&nonce),
                Payload {
                    msg: &secret.0,
                    aad: account_id.as_bytes(),
                },
            )
            .map_err(|_| TotpError::DecryptionFailed)?;
        let mut sealed = nonce.to_vec();
        sealed.extend_from_slice(&ciphertext);
        Ok(format!(
            "{ENVELOPE_VERSION}.{}.{}",
            self.key_id,
            hex::encode(sealed)
        ))
    }

    /// Returns the key id a stored envelope was sealed with.
    #[must_use]
    pub fn envelope_key_id(envelope: &str) -> Option<&str> {
        let mut parts = envelope.splitn(3, '.');
        (parts.next()? == ENVELOPE_VERSION).then_some(())?;
        parts.next()
    }

    /// # Errors
    ///
    /// Returns [`TotpError::MalformedEnvelope`] for an unknown format or key id,
    /// or [`TotpError::DecryptionFailed`] if authentication fails.
    pub fn open(&self, envelope: &str, account_id: &str) -> Result<TotpSecret, TotpError> {
        let mut parts = envelope.splitn(3, '.');
        if parts.next() != Some(ENVELOPE_VERSION) || parts.next() != Some(self.key_id) {
            return Err(TotpError::MalformedEnvelope);
        }
        let sealed = hex::decode(parts.next().ok_or(TotpError::MalformedEnvelope)?)
            .map_err(|_| TotpError::MalformedEnvelope)?;
        if sealed.len() <= 24 {
            return Err(TotpError::MalformedEnvelope);
        }
        let (nonce, ciphertext) = sealed.split_at(24);
        let plaintext = self
            .cipher
            .decrypt(
                XNonce::from_slice(nonce),
                Payload {
                    msg: ciphertext,
                    aad: account_id.as_bytes(),
                },
            )
            .map_err(|_| TotpError::DecryptionFailed)?;
        Ok(TotpSecret(plaintext))
    }
}

/// Generates a single-use recovery code formatted as `XXXXX-XXXXX` (50 bits).
#[must_use]
pub fn generate_recovery_code() -> Zeroizing<String> {
    let mut bytes = [0_u8; 10];
    OsRng.fill_bytes(&mut bytes);
    let mut code = String::with_capacity(11);
    for (index, byte) in bytes.iter().enumerate() {
        if index == 5 {
            code.push('-');
        }
        code.push(char::from(RECOVERY_ALPHABET[usize::from(byte & 31)]));
    }
    bytes.zeroize();
    Zeroizing::new(code)
}

/// Canonical form used for hashing: upper-case, no separators or spaces.
#[must_use]
pub fn normalize_recovery_code(code: &str) -> Option<String> {
    let normalized: String = code
        .chars()
        .filter(|character| !matches!(character, '-' | ' '))
        .map(|character| character.to_ascii_uppercase())
        .collect();
    (normalized.len() == 10
        && normalized
            .bytes()
            .all(|byte| RECOVERY_ALPHABET.contains(&byte)))
    .then_some(normalized)
}

fn base32_encode(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len().div_ceil(5) * 8);
    let mut buffer = 0_u32;
    let mut bits = 0_u32;
    for byte in bytes {
        buffer = (buffer << 8) | u32::from(*byte);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            output.push(char::from(
                BASE32_ALPHABET[((buffer >> bits) & 31) as usize],
            ));
        }
    }
    if bits > 0 {
        output.push(char::from(
            BASE32_ALPHABET[((buffer << (5 - bits)) & 31) as usize],
        ));
    }
    buffer.zeroize();
    output
}

fn percent_encode(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~' | b'@') {
            output.push(char::from(byte));
        } else {
            const HEX: &[u8; 16] = b"0123456789ABCDEF";
            output.push('%');
            output.push(char::from(HEX[usize::from(byte >> 4)]));
            output.push(char::from(HEX[usize::from(byte & 15)]));
        }
    }
    output
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn rfc_secret() -> TotpSecret {
        TotpSecret::from_bytes(b"12345678901234567890".to_vec())
    }

    #[test]
    fn matches_rfc_6238_sha1_vectors() {
        // RFC 6238 Appendix B, truncated to the 6 digits authenticator apps use.
        let secret = rfc_secret();
        for (time, expected) in [
            (59_i64, 287_082_u32),
            (1_111_111_109, 81_804),
            (1_111_111_111, 50_471),
            (1_234_567_890, 5_924),
            (2_000_000_000, 279_037),
            (20_000_000_000, 353_130),
        ] {
            assert_eq!(secret.code_at_step(time / 30), expected, "time {time}");
        }
    }

    #[test]
    fn base32_matches_rfc_4648() {
        assert_eq!(base32_encode(b"foobar"), "MZXW6YTBOI");
        assert_eq!(
            rfc_secret().to_base32().as_str(),
            "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ"
        );
    }

    #[test]
    fn verify_accepts_drift_and_rejects_replay_and_garbage() {
        let secret = rfc_secret();
        let now = 1_111_111_111;
        let step = now / 30;
        let current = format!("{:06}", secret.code_at_step(step));
        assert_eq!(secret.verify(&current, now, None), Some(step));
        assert_eq!(
            secret.verify(&format!("{} {}", &current[..3], &current[3..]), now, None),
            Some(step)
        );
        // Same step cannot be used twice.
        assert_eq!(secret.verify(&current, now, Some(step)), None);
        let previous = format!("{:06}", secret.code_at_step(step - 1));
        assert_eq!(secret.verify(&previous, now, None), Some(step - 1));
        let stale = format!("{:06}", secret.code_at_step(step - 2));
        assert_eq!(secret.verify(&stale, now, None), None);
        for garbage in ["", "12345", "1234567", "abcdef", "12345a"] {
            assert_eq!(secret.verify(garbage, now, None), None, "{garbage}");
        }
    }

    #[test]
    fn otpauth_uri_contains_issuer_and_encoded_account() {
        let uri = rfc_secret().otpauth_uri("Sanser", "a b@example.test");
        assert_eq!(
            uri.as_str(),
            "otpauth://totp/Sanser:a%20b@example.test?secret=GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ&issuer=Sanser&algorithm=SHA1&digits=6&period=30"
        );
    }

    #[test]
    fn secret_box_round_trips_and_binds_account_and_key() {
        let sealer = SecretBox::derive(&[7_u8; 32], "d").unwrap();
        let secret = TotpSecret::generate();
        let envelope = sealer.seal(&secret, "account-1").unwrap();
        assert!(!envelope.contains(secret.to_base32().as_str()));
        assert_eq!(SecretBox::envelope_key_id(&envelope), Some("d"));
        assert_eq!(sealer.open(&envelope, "account-1").unwrap().0, secret.0);
        assert_eq!(
            sealer.open(&envelope, "account-2").unwrap_err(),
            TotpError::DecryptionFailed
        );
        let other = SecretBox::derive(&[8_u8; 32], "d").unwrap();
        assert_eq!(
            other.open(&envelope, "account-1").unwrap_err(),
            TotpError::DecryptionFailed
        );
        let other_id = SecretBox::derive(&[7_u8; 32], "k").unwrap();
        assert_eq!(
            other_id.open(&envelope, "account-1").unwrap_err(),
            TotpError::MalformedEnvelope
        );
        assert!(SecretBox::derive(&[1_u8; 16], "d").is_err());
        let mut tampered = envelope.clone();
        tampered.pop();
        tampered.push(if envelope.ends_with('0') { '1' } else { '0' });
        assert!(sealer.open(&tampered, "account-1").is_err());
    }

    #[test]
    fn recovery_codes_are_formatted_and_normalized() {
        let code = generate_recovery_code();
        assert_eq!(code.len(), 11);
        assert_eq!(&code[5..6], "-");
        let normalized = normalize_recovery_code(&code.to_ascii_lowercase()).unwrap();
        assert_eq!(normalized, code.replace('-', ""));
        assert!(normalize_recovery_code("ABCDE-FGHI").is_none());
        assert!(normalize_recovery_code("ABCDE-FGHI0").is_none());
        assert_ne!(generate_recovery_code().as_str(), code.as_str());
    }
}
