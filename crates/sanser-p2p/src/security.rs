//! Security handshake and encryption.

use hmac::{Hmac, Mac};
use sha2::Sha256;
use serde::{Deserialize, Serialize};
use chacha20poly1305::{ChaCha20Poly1305, KeyInit, aead::Aead};
use x25519_dalek::{EphemeralSecret, PublicKey};
use base64::{Engine, engine::general_purpose::STANDARD};

type HmacSha256 = Hmac<Sha256>;

/// Keys derived from the HKDF key schedule.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DerivedKeys {
    /// Host-to-client encryption key.
    pub host_to_client: [u8; 32],
    /// Client-to-host encryption key.
    pub client_to_host: [u8; 32],
    /// Header protection key.
    pub header_protection: [u8; 32],
    /// Rekey derivation key.
    pub rekey: [u8; 32],
    /// Handshake confirmation key.
    pub handshake_confirmation: [u8; 32],
}

/// AEAD algorithm selection.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AeadAlgorithm {
    /// ChaCha20-Poly1305 (software-friendly).
    ChaCha20Poly1305,
    /// AES-256-GCM (hardware-accelerated on most CPUs).
    Aes256Gcm,
}

/// Conditions that trigger a rekey per plan section 18.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RekeyTrigger {
    /// Packet count limit reached.
    PacketLimit,
    /// Time limit reached.
    TimeLimit,
    /// Route changed.
    RouteChanged,
    /// Device resumed from sleep.
    ResumedFromSleep,
    /// Sequence number approaching overflow.
    SequenceNearOverflow,
}

/// Nonce construction: `key_id || stream_id || sequence_number`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NonceComponents {
    pub key_id: u8,
    pub stream_id: u8,
    pub sequence: u64,
}

impl NonceComponents {
    /// Builds a 12-byte nonce for AEAD.
    #[must_use]
    pub fn to_bytes(&self) -> [u8; 12] {
        let mut nonce = [0u8; 12];
        nonce[0] = self.key_id;
        nonce[1] = self.stream_id;
        let seq_bytes = self.sequence.to_le_bytes();
        nonce[2..10].copy_from_slice(&seq_bytes);
        nonce
    }
}

/// Security handshake state.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum HandshakeState {
    #[default]
    NotStarted,
    KeyExchangeSent,
    KeyExchangeReceived,
    KeysDerived,
    ConfirmationSent,
    ConfirmationReceived,
    Complete,
    Failed,
}

/// Generates a new ephemeral X25519 keypair.
pub fn generate_x25519_keypair() -> (EphemeralSecret, PublicKey) {
    let secret = EphemeralSecret::random_from_rng(rand::thread_rng());
    let public = PublicKey::from(&secret);
    (secret, public)
}

/// Serializes X25519 PublicKey to standard Base64.
pub fn public_key_to_base64(pk: &PublicKey) -> String {
    STANDARD.encode(pk.as_bytes())
}

/// Deserializes X25519 PublicKey from standard Base64.
///
/// # Errors
///
/// Returns an error string if decoding fails or length is incorrect.
pub fn public_key_from_base64(s: &str) -> Result<PublicKey, String> {
    let bytes = STANDARD.decode(s).map_err(|e| e.to_string())?;
    let array: [u8; 32] = bytes.try_into().map_err(|_| "invalid public key length".to_owned())?;
    Ok(PublicKey::from(array))
}

/// Derives standard RFC 5869 HKDF-SHA256 keying material.
///
/// # Errors
///
/// Returns an error string if HMAC setup fails.
pub fn hkdf_sha256(salt: &[u8], ikm: &[u8], info: &[u8], okm_len: usize) -> Result<Vec<u8>, String> {
    let prk = {
        let mut mac = <HmacSha256 as hmac::Mac>::new_from_slice(salt)
            .map_err(|error| error.to_string())?;
        mac.update(ikm);
        mac.finalize().into_bytes()
    };

    let mut okm = Vec::new();
    let mut t = Vec::new();
    let mut counter = 1u8;

    while okm.len() < okm_len {
        let mut mac = <HmacSha256 as hmac::Mac>::new_from_slice(&prk)
            .map_err(|error| error.to_string())?;
        mac.update(&t);
        mac.update(info);
        mac.update(&[counter]);
        let t_next = mac.finalize().into_bytes();
        okm.extend_from_slice(&t_next);
        t = t_next.to_vec();
        counter += 1;
    }

    okm.truncate(okm_len);
    Ok(okm)
}

/// Derives the 5 directional keys from the X25519 shared secret.
///
/// # Errors
///
/// Returns an error string if key derivation fails.
pub fn derive_keys(shared_secret: &[u8], session_token: &str) -> Result<DerivedKeys, String> {
    let info = b"Sanser P2P Key Derivation";
    let okm = hkdf_sha256(session_token.as_bytes(), shared_secret, info, 160)?;

    let mut keys = DerivedKeys {
        host_to_client: [0u8; 32],
        client_to_host: [0u8; 32],
        header_protection: [0u8; 32],
        rekey: [0u8; 32],
        handshake_confirmation: [0u8; 32],
    };

    keys.host_to_client.copy_from_slice(&okm[0..32]);
    keys.client_to_host.copy_from_slice(&okm[32..64]);
    keys.header_protection.copy_from_slice(&okm[64..96]);
    keys.rekey.copy_from_slice(&okm[96..128]);
    keys.handshake_confirmation.copy_from_slice(&okm[128..160]);

    Ok(keys)
}

/// Encrypts plaintext using ChaCha20-Poly1305 with the derived key and nonce.
///
/// # Errors
///
/// Returns an error string if encryption fails.
pub fn encrypt_payload(
    key: &[u8; 32],
    nonce: NonceComponents,
    plaintext: &[u8],
) -> Result<Vec<u8>, String> {
    let cipher = ChaCha20Poly1305::new(key.into());
    let nonce_bytes = nonce.to_bytes();
    cipher.encrypt(&nonce_bytes.into(), plaintext)
        .map_err(|e| format!("encryption failed: {e}"))
}

/// Decrypts ciphertext using ChaCha20-Poly1305.
///
/// # Errors
///
/// Returns an error string if decryption fails.
pub fn decrypt_payload(
    key: &[u8; 32],
    nonce: NonceComponents,
    ciphertext: &[u8],
) -> Result<Vec<u8>, String> {
    let cipher = ChaCha20Poly1305::new(key.into());
    let nonce_bytes = nonce.to_bytes();
    cipher.decrypt(&nonce_bytes.into(), ciphertext)
        .map_err(|e| format!("decryption failed: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nonce_has_correct_length() {
        let nc = NonceComponents {
            key_id: 1,
            stream_id: 5,
            sequence: 42,
        };
        let nonce = nc.to_bytes();
        assert_eq!(nonce.len(), 12);
        assert_eq!(nonce[0], 1);
        assert_eq!(nonce[1], 5);
    }

    #[test]
    fn different_sequences_produce_different_nonces() {
        let a = NonceComponents {
            key_id: 0,
            stream_id: 0,
            sequence: 1,
        };
        let b = NonceComponents {
            key_id: 0,
            stream_id: 0,
            sequence: 2,
        };
        assert_ne!(a.to_bytes(), b.to_bytes());
    }

    #[test]
    fn test_aead_roundtrip() {
        let key = [9u8; 32];
        let nonce = NonceComponents {
            key_id: 1,
            stream_id: 2,
            sequence: 100,
        };
        let plaintext = b"Hello, secure P2P!";
        let ciphertext = encrypt_payload(&key, nonce, plaintext).unwrap();
        let decrypted = decrypt_payload(&key, nonce, &ciphertext).unwrap();
        assert_eq!(plaintext, decrypted.as_slice());
    }

    #[test]
    fn test_x25519_key_exchange() {
        let (alice_secret, alice_public) = generate_x25519_keypair();
        let (bob_secret, bob_public) = generate_x25519_keypair();

        let alice_shared = alice_secret.diffie_hellman(&bob_public);
        let bob_shared = bob_secret.diffie_hellman(&alice_public);

        assert_eq!(alice_shared.as_bytes(), bob_shared.as_bytes());
    }
}
