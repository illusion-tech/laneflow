//! Checked little-endian primitive reads shared by framing and table preflight.

use crate::{FormatError, FormatStructure};

pub(crate) fn read_u8(
    bytes: &[u8],
    offset: u64,
    structure: FormatStructure,
) -> Result<u8, FormatError> {
    Ok(read_array::<1>(bytes, offset, structure)?[0])
}

pub(crate) fn read_u16(
    bytes: &[u8],
    offset: u64,
    structure: FormatStructure,
) -> Result<u16, FormatError> {
    Ok(u16::from_le_bytes(read_array(bytes, offset, structure)?))
}

pub(crate) fn read_u32(
    bytes: &[u8],
    offset: u64,
    structure: FormatStructure,
) -> Result<u32, FormatError> {
    Ok(u32::from_le_bytes(read_array(bytes, offset, structure)?))
}

pub(crate) fn read_u64(
    bytes: &[u8],
    offset: u64,
    structure: FormatStructure,
) -> Result<u64, FormatError> {
    Ok(u64::from_le_bytes(read_array(bytes, offset, structure)?))
}

pub(crate) fn read_array<const N: usize>(
    bytes: &[u8],
    offset: u64,
    structure: FormatStructure,
) -> Result<[u8; N], FormatError> {
    let slice = checked_slice(bytes, offset, N as u64, structure)?;
    let mut result = [0_u8; N];
    result.copy_from_slice(slice);
    Ok(result)
}

pub(crate) fn checked_slice(
    bytes: &[u8],
    offset: u64,
    length: u64,
    structure: FormatStructure,
) -> Result<&[u8], FormatError> {
    let end = offset
        .checked_add(length)
        .ok_or(FormatError::ArithmeticOverflow { structure })?;
    let available = bytes.len() as u64;
    if end > available {
        return Err(FormatError::Truncated {
            structure,
            offset,
            needed: length,
            available: available.saturating_sub(offset.min(available)),
        });
    }
    let start =
        usize::try_from(offset).map_err(|_| FormatError::ArithmeticOverflow { structure })?;
    let end = usize::try_from(end).map_err(|_| FormatError::ArithmeticOverflow { structure })?;
    Ok(&bytes[start..end])
}

/// 返回完全位于已声明父容器内的 slice，同时保留相对于完整对象的绝对 offset。
pub(crate) fn checked_slice_within(
    bytes: &[u8],
    offset: u64,
    length: u64,
    container_end: u64,
    structure: FormatStructure,
) -> Result<&[u8], FormatError> {
    let end = offset
        .checked_add(length)
        .ok_or(FormatError::ArithmeticOverflow { structure })?;
    if end > container_end {
        return Err(FormatError::LengthMismatch {
            structure,
            declared: length,
            actual: container_end.saturating_sub(offset),
        });
    }
    checked_slice(bytes, offset, length, structure)
}
