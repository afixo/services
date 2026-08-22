//! Opaque tokens and secrets. Every bearer token, refresh token and client
//! secret in Afixo is 256 bits from the OS CSPRNG, base64url-encoded, and stored
//! only as its SHA-256 digest. Opaque beats signed here: the only claim anyone
//! needs is "which principal", revocation is a row delete, and there is no key
//! to manage or rotate.

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

pub const TOKEN_BYTES: usize = 32;

/// A fresh random token, 43 chars of base64url.
pub fn random_token() -> String {
    let mut bytes = [0u8; TOKEN_BYTES];
    getrandom::fill(&mut bytes).expect("OS randomness available");
    URL_SAFE_NO_PAD.encode(bytes)
}

/// Client ids are shorter and prefixed so they are recognisable in logs/configs.
pub fn random_client_id() -> String {
    let mut bytes = [0u8; 12];
    getrandom::fill(&mut bytes).expect("OS randomness available");
    format!("afx_{}", hex::encode(bytes))
}

/// The storage form of a token or secret.
pub fn sha256(value: &str) -> [u8; 32] {
    Sha256::digest(value.as_bytes()).into()
}

/// Constant-time equality for digests (FINAL_REPORT NF2).
pub fn digest_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && bool::from(a.ct_eq(b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_unique_and_urlsafe() {
        let a = random_token();
        let b = random_token();
        assert_ne!(a, b);
        assert_eq!(a.len(), 43);
        assert!(
            a.bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
        );
    }

    #[test]
    fn digest_compare() {
        let d = sha256("secret");
        assert!(digest_eq(&d, &sha256("secret")));
        assert!(!digest_eq(&d, &sha256("Secret")));
        assert!(!digest_eq(&d, &d[..31]));
    }

    #[test]
    fn client_id_shape() {
        let id = random_client_id();
        assert!(id.starts_with("afx_"));
        assert_eq!(id.len(), 4 + 24);
    }
}
