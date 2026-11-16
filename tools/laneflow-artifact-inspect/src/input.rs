//! 输入外层：受限读入与 hex 解码。
//!
//! 读入是唯一的文件 I/O 入口：以 `limit + 1` 字节为读入上限，多出的一个
//! 字节用于当场探测超限——分配受限于读入本身，而不是「先无界读完再检查」。
//! raw 预检失败不自动回退 hex：外层编码由调用方显式声明。

use std::io::Read as _;
use std::path::Path;

use crate::session::InputLimits;

/// 输入外层编码。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InputEncoding {
    /// 原始对象字节。
    Raw,
    /// 忽略 ASCII 空白的十六进制文本。
    Hex,
}

/// 输入外层错误。
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum InputError {
    /// 打开或读取文件失败。
    Io(String),
    /// 读入过程中发现内容超过限额。
    InputTooLarge {
        /// 实际读入字节数（至少 `limit + 1`）。
        actual: u64,
        /// 允许的最大值。
        limit: u64,
    },
    /// 解码后的对象超过对象预算。
    ObjectTooLarge {
        /// 解码后的字节数。
        actual: u64,
        /// 允许的最大值。
        limit: u64,
    },
    /// hex 文本位数不是偶数。
    OddHexDigits {
        /// 过滤空白后的字符数。
        digits: usize,
    },
    /// hex 文本包含非 hex 字符。
    InvalidHexCharacter {
        /// 违规字符在过滤后文本中的零基序号。
        index: usize,
        /// 违规字节值。
        byte: u8,
    },
}

impl core::fmt::Display for InputError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Io(message) => write!(f, "读取输入失败：{message}"),
            Self::InputTooLarge { actual, limit } => {
                write!(f, "输入超过读入限额：读到 {actual} 字节（上限 {limit}）")
            }
            Self::ObjectTooLarge { actual, limit } => {
                write!(f, "解码后对象超过预算：{actual} > {limit}")
            }
            Self::OddHexDigits { digits } => {
                write!(f, "hex 输入为奇数位（{digits} 个字符）")
            }
            Self::InvalidHexCharacter { index, byte } => {
                write!(f, "hex 输入第 {index} 个字符非法（0x{byte:02x}）")
            }
        }
    }
}

impl std::error::Error for InputError {}

/// 按声明的编码受限读入并解码一个对象输入。
///
/// # Errors
///
/// 文件系统错误、读入或解码超限、hex 位数或字符非法时返回 [`InputError`]。
pub fn read_object_input(
    path: &Path,
    encoding: InputEncoding,
    limits: InputLimits,
) -> Result<Vec<u8>, InputError> {
    match encoding {
        InputEncoding::Raw => {
            let bytes = read_bounded(path, limits.max_input_bytes)?;
            enforce_object_budget(bytes, limits.max_object_bytes)
        }
        InputEncoding::Hex => {
            let text_limit = limits.hex_text_limit();
            let text = read_bounded(path, text_limit)?;
            let text = strip_ascii_whitespace(&text);
            decode_hex(&text, limits)
        }
    }
}

/// 对已读入的内存字节按声明的编码解码（测试与调用方复用同一外层规则）。
///
/// # Errors
///
/// 解码超限、hex 位数或字符非法时返回 [`InputError`]。
pub fn decode_object_input(
    bytes: &[u8],
    encoding: InputEncoding,
    limits: InputLimits,
) -> Result<Vec<u8>, InputError> {
    match encoding {
        InputEncoding::Raw => enforce_object_budget(bytes.to_vec(), limits.max_object_bytes),
        InputEncoding::Hex => {
            let text = strip_ascii_whitespace(bytes);
            decode_hex(&text, limits)
        }
    }
}

fn enforce_object_budget(bytes: Vec<u8>, limit: u64) -> Result<Vec<u8>, InputError> {
    let actual = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    if actual > limit {
        return Err(InputError::ObjectTooLarge { actual, limit });
    }
    Ok(bytes)
}

/// 读入期限界：最多读 `limit + 1` 字节，读到超限即报错。
fn read_bounded(path: &Path, limit: u64) -> Result<Vec<u8>, InputError> {
    let mut file = std::fs::File::open(path).map_err(|error| InputError::Io(error.to_string()))?;
    let mut bytes = Vec::new();
    let capped = limit.saturating_add(1);
    let mut reader = file.by_ref().take(capped);
    reader
        .read_to_end(&mut bytes)
        .map_err(|error| InputError::Io(error.to_string()))?;
    let actual = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    if actual > limit {
        return Err(InputError::InputTooLarge { actual, limit });
    }
    Ok(bytes)
}

fn strip_ascii_whitespace(bytes: &[u8]) -> Vec<u8> {
    let mut stripped = Vec::with_capacity(bytes.len());
    stripped.extend(
        bytes
            .iter()
            .copied()
            .filter(|byte| !byte.is_ascii_whitespace()),
    );
    stripped
}

fn decode_hex(text: &[u8], limits: InputLimits) -> Result<Vec<u8>, InputError> {
    if !text.len().is_multiple_of(2) {
        return Err(InputError::OddHexDigits { digits: text.len() });
    }
    let decoded_len = text.len() / 2;
    if u64::try_from(decoded_len).unwrap_or(u64::MAX) > limits.max_object_bytes {
        return Err(InputError::ObjectTooLarge {
            actual: u64::try_from(decoded_len).unwrap_or(u64::MAX),
            limit: limits.max_object_bytes,
        });
    }
    let mut decoded = Vec::with_capacity(decoded_len);
    for (index, pair) in text.as_chunks::<2>().0.iter().enumerate() {
        let high = hex_digit(pair[0], index * 2)?;
        let low = hex_digit(pair[1], index * 2 + 1)?;
        decoded.push(high << 4 | low);
    }
    Ok(decoded)
}

fn hex_digit(byte: u8, index: usize) -> Result<u8, InputError> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => Err(InputError::InvalidHexCharacter { index, byte }),
    }
}
