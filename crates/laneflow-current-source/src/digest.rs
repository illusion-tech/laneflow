//! SHA-256 摘要计算与 `sha256:<64 lowercase hex>` 词法校验。

use sha2::{Digest, Sha256};

/// 制品 size 的 JSON portable integer 上限（2^53 - 1）。
pub(crate) const MAX_PORTABLE_ARTIFACT_SIZE: u64 = 9_007_199_254_740_991;

/// 对原始制品 bytes 执行一次 SHA-256 线性扫描。
pub(crate) fn sha256_digest(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

/// 解析 `sha256:<64 lowercase hex>`；词法不符时返回 `None`，由调用方构造错误。
pub(crate) fn parse_digest(value: &str) -> Option<[u8; 32]> {
    let hex = value.strip_prefix("sha256:")?;
    if hex.len() != 64
        || !hex
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return None;
    }

    let mut digest = [0_u8; 32];
    for (index, chunk) in hex.as_bytes().chunks_exact(2).enumerate() {
        digest[index] = (hex_value(chunk[0]) << 4) | hex_value(chunk[1]);
    }
    Some(digest)
}

fn hex_value(byte: u8) -> u8 {
    match byte {
        b'0'..=b'9' => byte - b'0',
        b'a'..=b'f' => byte - b'a' + 10,
        _ => unreachable!("parse_digest validates lowercase hexadecimal input"),
    }
}

/// 把 32 字节摘要编码回 `sha256:<64 lowercase hex>` 展示形式。
pub(crate) fn encode_digest(digest: &[u8; 32]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";

    let mut encoded = String::with_capacity("sha256:".len() + digest.len() * 2);
    encoded.push_str("sha256:");
    for byte in digest {
        encoded.push(char::from(HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    encoded
}
