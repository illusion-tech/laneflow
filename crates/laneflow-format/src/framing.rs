//! 对象前导与节目录的零拷贝预检。

use laneflow_static_contract::{
    OBJECT_PREAMBLE_V1_BYTE_LENGTH, PortableObjectKind, SECTION_DIRECTORY_ENTRY_V1_BYTE_LENGTH,
    SECTION_FORMAT_VERSION_V1,
};

use crate::{
    FormatError, FormatLimits, FormatStructure, LimitDimension,
    wire::{checked_slice, read_array, read_u16, read_u32, read_u64},
};

/// 只证明对象前导、节目录与连续节范围已经通过结构预检的借用视图。
///
/// 本类型不解释节内 Table/Row/Field，也不证明任何语义或信任绑定。调用方不得把它
/// 重命名或包装为 validated/trusted artifact view。
#[derive(Clone, Copy, Debug)]
pub struct ObjectFramingView<'a> {
    bytes: &'a [u8],
    kind: PortableObjectKind,
}

impl<'a> ObjectFramingView<'a> {
    /// 已与前导 magic 和 exact section shape 核对的对象种类。
    #[must_use]
    pub const fn kind(self) -> PortableObjectKind {
        self.kind
    }

    /// 完整对象的 exact bytes。
    #[must_use]
    pub const fn bytes(self) -> &'a [u8] {
        self.bytes
    }

    /// 精确节数。
    #[must_use]
    pub const fn section_count(self) -> u32 {
        self.kind.section_count()
    }

    /// 按零基目录 ordinal 取得只证明 framing 的节借用。
    #[must_use]
    pub fn section(self, ordinal: u32) -> Option<SectionFramingView<'a>> {
        if ordinal >= self.section_count() {
            return None;
        }
        let directory_offset = u64::from(OBJECT_PREAMBLE_V1_BYTE_LENGTH)
            + u64::from(ordinal) * SECTION_DIRECTORY_ENTRY_V1_BYTE_LENGTH;
        let kind = read_u16(
            self.bytes,
            directory_offset,
            FormatStructure::SectionDirectoryEntry,
        )
        .ok()?;
        let byte_offset = read_u64(
            self.bytes,
            directory_offset + 8,
            FormatStructure::SectionDirectoryEntry,
        )
        .ok()?;
        let byte_length = read_u64(
            self.bytes,
            directory_offset + 16,
            FormatStructure::SectionDirectoryEntry,
        )
        .ok()?;
        let bytes = checked_slice(
            self.bytes,
            byte_offset,
            byte_length,
            FormatStructure::Section,
        )
        .ok()?;
        Some(SectionFramingView { kind, bytes })
    }
}

/// 只证明目录范围连续、无越界的节借用。
#[derive(Clone, Copy, Debug)]
pub struct SectionFramingView<'a> {
    kind: u16,
    bytes: &'a [u8],
}

impl<'a> SectionFramingView<'a> {
    /// 对象专用、已按 wire order 核对的 section kind。
    #[must_use]
    pub const fn kind(self) -> u16 {
        self.kind
    }

    /// 尚未经过节内 registry/语义预检的原始节 bytes。
    #[must_use]
    pub const fn bytes(self) -> &'a [u8] {
        self.bytes
    }
}

/// 对固定对象种类执行前导、目录、exact length 与连续范围预检。
pub fn preflight_object_framing(
    bytes: &[u8],
    expected_kind: PortableObjectKind,
    limits: FormatLimits,
) -> Result<ObjectFramingView<'_>, FormatError> {
    let config = limits.config();
    let actual_object_length = bytes.len() as u64;
    if actual_object_length > config.max_object_bytes {
        return Err(FormatError::LimitExceeded {
            dimension: LimitDimension::ObjectBytes,
            actual: actual_object_length,
            limit: config.max_object_bytes,
        });
    }

    let magic = read_array::<4>(bytes, 0, FormatStructure::ObjectPreamble)?;
    let actual_kind = PortableObjectKind::from_magic(magic).ok_or(FormatError::UnknownKind {
        structure: FormatStructure::ObjectPreamble,
        code: u64::from_le_bytes([magic[0], magic[1], magic[2], magic[3], 0, 0, 0, 0]),
    })?;
    if actual_kind != expected_kind {
        return Err(FormatError::BindingMismatch {
            structure: FormatStructure::ObjectPreamble,
        });
    }

    let format_version = read_u16(bytes, 4, FormatStructure::ObjectPreamble)?;
    if format_version != expected_kind.format_version() {
        return Err(FormatError::UnsupportedVersion {
            structure: FormatStructure::ObjectPreamble,
            actual: u64::from(format_version),
            expected: u64::from(expected_kind.format_version()),
        });
    }

    let header_byte_length = read_u16(bytes, 6, FormatStructure::ObjectPreamble)?;
    if header_byte_length != OBJECT_PREAMBLE_V1_BYTE_LENGTH {
        return Err(FormatError::NonCanonicalValue {
            structure: FormatStructure::ObjectPreamble,
            offset: 6,
        });
    }
    let flags = read_u32(bytes, 8, FormatStructure::ObjectPreamble)?;
    if flags != 0 {
        return Err(FormatError::NonCanonicalValue {
            structure: FormatStructure::ObjectPreamble,
            offset: 8,
        });
    }

    let section_count = read_u32(bytes, 12, FormatStructure::ObjectPreamble)?;
    if section_count != expected_kind.section_count() {
        return Err(FormatError::LengthMismatch {
            structure: FormatStructure::SectionDirectory,
            declared: u64::from(section_count),
            actual: u64::from(expected_kind.section_count()),
        });
    }
    let section_directory_offset = read_u64(bytes, 16, FormatStructure::ObjectPreamble)?;
    if section_directory_offset != u64::from(OBJECT_PREAMBLE_V1_BYTE_LENGTH) {
        return Err(FormatError::NonCanonicalValue {
            structure: FormatStructure::ObjectPreamble,
            offset: 16,
        });
    }
    let object_byte_length = read_u64(bytes, 24, FormatStructure::ObjectPreamble)?;
    if object_byte_length != actual_object_length {
        return Err(FormatError::LengthMismatch {
            structure: FormatStructure::ObjectPreamble,
            declared: object_byte_length,
            actual: actual_object_length,
        });
    }

    let directory_byte_length = u64::from(section_count)
        .checked_mul(SECTION_DIRECTORY_ENTRY_V1_BYTE_LENGTH)
        .ok_or(FormatError::ArithmeticOverflow {
            structure: FormatStructure::SectionDirectory,
        })?;
    checked_slice(
        bytes,
        section_directory_offset,
        directory_byte_length,
        FormatStructure::SectionDirectory,
    )?;

    let mut expected_offset = expected_kind.first_section_offset();
    for ordinal in 0..section_count {
        let entry_offset = section_directory_offset
            .checked_add(
                u64::from(ordinal)
                    .checked_mul(SECTION_DIRECTORY_ENTRY_V1_BYTE_LENGTH)
                    .ok_or(FormatError::ArithmeticOverflow {
                        structure: FormatStructure::SectionDirectory,
                    })?,
            )
            .ok_or(FormatError::ArithmeticOverflow {
                structure: FormatStructure::SectionDirectory,
            })?;
        let section_kind = read_u16(bytes, entry_offset, FormatStructure::SectionDirectoryEntry)?;
        let expected_section_kind =
            u16::try_from(ordinal + 1).map_err(|_| FormatError::ArithmeticOverflow {
                structure: FormatStructure::SectionDirectoryEntry,
            })?;
        if section_kind == 0 || u32::from(section_kind) > section_count {
            return Err(FormatError::UnknownKind {
                structure: FormatStructure::SectionDirectoryEntry,
                code: u64::from(section_kind),
            });
        }
        if section_kind != expected_section_kind {
            return Err(FormatError::NonCanonicalOrder {
                structure: FormatStructure::SectionDirectory,
                previous: u64::from(expected_section_kind),
                current: u64::from(section_kind),
            });
        }
        let section_version = read_u16(
            bytes,
            entry_offset + 2,
            FormatStructure::SectionDirectoryEntry,
        )?;
        if section_version != SECTION_FORMAT_VERSION_V1 {
            return Err(FormatError::UnsupportedVersion {
                structure: FormatStructure::SectionDirectoryEntry,
                actual: u64::from(section_version),
                expected: u64::from(SECTION_FORMAT_VERSION_V1),
            });
        }
        let section_flags = read_u32(
            bytes,
            entry_offset + 4,
            FormatStructure::SectionDirectoryEntry,
        )?;
        if section_flags != 0 {
            return Err(FormatError::NonCanonicalValue {
                structure: FormatStructure::SectionDirectoryEntry,
                offset: entry_offset + 4,
            });
        }
        let byte_offset = read_u64(
            bytes,
            entry_offset + 8,
            FormatStructure::SectionDirectoryEntry,
        )?;
        if byte_offset != expected_offset {
            return Err(FormatError::GapOrOverlap {
                expected_offset,
                actual_offset: byte_offset,
            });
        }
        let byte_length = read_u64(
            bytes,
            entry_offset + 16,
            FormatStructure::SectionDirectoryEntry,
        )?;
        if byte_length > config.max_section_or_table_bytes {
            return Err(FormatError::LimitExceeded {
                dimension: LimitDimension::SectionOrTableBytes,
                actual: byte_length,
                limit: config.max_section_or_table_bytes,
            });
        }
        checked_slice(bytes, byte_offset, byte_length, FormatStructure::Section)?;
        expected_offset =
            byte_offset
                .checked_add(byte_length)
                .ok_or(FormatError::ArithmeticOverflow {
                    structure: FormatStructure::SectionDirectoryEntry,
                })?;
    }

    if expected_offset != object_byte_length {
        return Err(FormatError::LengthMismatch {
            structure: FormatStructure::SectionDirectory,
            declared: expected_offset,
            actual: object_byte_length,
        });
    }

    Ok(ObjectFramingView {
        bytes,
        kind: expected_kind,
    })
}

#[cfg(test)]
mod tests {
    use std::vec;
    use std::vec::Vec;

    use super::*;
    use crate::{FormatErrorClass, FormatLimitConfig};

    fn object_bytes(kind: PortableObjectKind, section_lengths: &[u64]) -> Vec<u8> {
        assert_eq!(section_lengths.len(), kind.section_count() as usize);
        let total = section_lengths
            .iter()
            .copied()
            .try_fold(kind.first_section_offset(), u64::checked_add)
            .unwrap();
        let mut bytes = vec![0_u8; usize::try_from(total).unwrap()];
        bytes[0..4].copy_from_slice(&kind.magic());
        bytes[4..6].copy_from_slice(&kind.format_version().to_le_bytes());
        bytes[6..8].copy_from_slice(&OBJECT_PREAMBLE_V1_BYTE_LENGTH.to_le_bytes());
        bytes[12..16].copy_from_slice(&kind.section_count().to_le_bytes());
        bytes[16..24].copy_from_slice(&u64::from(OBJECT_PREAMBLE_V1_BYTE_LENGTH).to_le_bytes());
        bytes[24..32].copy_from_slice(&total.to_le_bytes());

        let mut section_offset = kind.first_section_offset();
        for (ordinal, byte_length) in section_lengths.iter().copied().enumerate() {
            let entry = usize::from(OBJECT_PREAMBLE_V1_BYTE_LENGTH)
                + ordinal * usize::try_from(SECTION_DIRECTORY_ENTRY_V1_BYTE_LENGTH).unwrap();
            bytes[entry..entry + 2]
                .copy_from_slice(&u16::try_from(ordinal + 1).unwrap().to_le_bytes());
            bytes[entry + 2..entry + 4].copy_from_slice(&SECTION_FORMAT_VERSION_V1.to_le_bytes());
            bytes[entry + 8..entry + 16].copy_from_slice(&section_offset.to_le_bytes());
            bytes[entry + 16..entry + 24].copy_from_slice(&byte_length.to_le_bytes());
            section_offset += byte_length;
        }
        bytes
    }

    #[test]
    fn all_object_kinds_use_frozen_directory_anchors() {
        for kind in PortableObjectKind::ALL {
            let section_lengths = vec![4_u64; kind.section_count() as usize];
            let bytes = object_bytes(kind, &section_lengths);
            let view = preflight_object_framing(&bytes, kind, FormatLimits::V1_HARD).unwrap();

            assert_eq!(view.kind(), kind);
            assert_eq!(view.bytes(), bytes);
            assert_eq!(view.section_count(), kind.section_count());
            assert_eq!(view.section(0).unwrap().kind(), 1);
            assert_eq!(view.section(0).unwrap().bytes().len(), 4);
            assert!(view.section(kind.section_count()).is_none());
        }
    }

    #[test]
    fn framing_rejects_length_count_gap_and_kind_mismatches() {
        let kind = PortableObjectKind::CanonicalPublicationDescriptor;
        let original = object_bytes(kind, &[4, 4, 4, 4]);

        let mut bytes = original.clone();
        let wrong_length = bytes.len() as u64 + 1;
        bytes[24..32].copy_from_slice(&wrong_length.to_le_bytes());
        assert_eq!(
            preflight_object_framing(&bytes, kind, FormatLimits::V1_HARD)
                .unwrap_err()
                .class(),
            FormatErrorClass::LengthMismatch
        );

        let mut bytes = original.clone();
        bytes[12..16].copy_from_slice(&3_u32.to_le_bytes());
        assert_eq!(
            preflight_object_framing(&bytes, kind, FormatLimits::V1_HARD)
                .unwrap_err()
                .class(),
            FormatErrorClass::LengthMismatch
        );

        let mut bytes = original.clone();
        let first_entry_offset = usize::from(OBJECT_PREAMBLE_V1_BYTE_LENGTH);
        let wrong_offset = kind.first_section_offset() + 1;
        bytes[first_entry_offset + 8..first_entry_offset + 16]
            .copy_from_slice(&wrong_offset.to_le_bytes());
        assert_eq!(
            preflight_object_framing(&bytes, kind, FormatLimits::V1_HARD)
                .unwrap_err()
                .class(),
            FormatErrorClass::GapOrOverlap
        );

        assert_eq!(
            preflight_object_framing(
                &original,
                PortableObjectKind::SemanticDiff,
                FormatLimits::V1_HARD,
            )
            .unwrap_err()
            .class(),
            FormatErrorClass::BindingMismatch
        );
    }

    #[test]
    fn framing_applies_caller_limit_before_parsing() {
        let kind = PortableObjectKind::CanonicalPublicationDescriptor;
        let bytes = object_bytes(kind, &[4, 4, 4, 4]);
        let mut config = FormatLimitConfig::V1_HARD;
        config.max_object_bytes = bytes.len() as u64 - 1;
        let limits = FormatLimits::try_new(config).unwrap();

        assert_eq!(
            preflight_object_framing(&bytes, kind, limits).unwrap_err(),
            FormatError::LimitExceeded {
                dimension: LimitDimension::ObjectBytes,
                actual: bytes.len() as u64,
                limit: bytes.len() as u64 - 1,
            }
        );
    }

    #[test]
    fn framing_distinguishes_unknown_section_kind_from_noncanonical_order() {
        let kind = PortableObjectKind::CanonicalPublicationDescriptor;
        let original = object_bytes(kind, &[4, 4, 4, 4]);
        let first_entry_offset = usize::from(OBJECT_PREAMBLE_V1_BYTE_LENGTH);

        let mut unknown = original.clone();
        unknown[first_entry_offset..first_entry_offset + 2].copy_from_slice(&5_u16.to_le_bytes());
        assert_eq!(
            preflight_object_framing(&unknown, kind, FormatLimits::V1_HARD)
                .unwrap_err()
                .class(),
            FormatErrorClass::UnknownKind
        );

        let mut duplicate = original;
        let second_entry_offset =
            first_entry_offset + usize::try_from(SECTION_DIRECTORY_ENTRY_V1_BYTE_LENGTH).unwrap();
        duplicate[second_entry_offset..second_entry_offset + 2]
            .copy_from_slice(&1_u16.to_le_bytes());
        assert_eq!(
            preflight_object_framing(&duplicate, kind, FormatLimits::V1_HARD)
                .unwrap_err()
                .class(),
            FormatErrorClass::NonCanonicalOrder
        );
    }
}
