//! Shared SHA-256 hex helpers.

use sha2::{Digest, Sha256};

/// Lowercase hex SHA-256 without a `sha256:` prefix.
pub fn hex_sha256(bytes: &[u8]) -> String {
    hex_encode(Sha256::digest(bytes).as_slice())
}

/// Prefixed digest used in manifests (`sha256:<hex>`).
pub fn sha256_digest(bytes: &[u8]) -> String {
    format!("sha256:{}", hex_sha256(bytes))
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(char::from(HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    encoded
}
