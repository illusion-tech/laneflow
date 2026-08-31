//! 调用方可收紧的格式资源限制。

use laneflow_static_contract::{
    FORMAT_HARD_MAX_FIELDS_PER_ROW, FORMAT_HARD_MAX_IDENTITY_ASCII_BYTES,
    FORMAT_HARD_MAX_RECORD_VECTOR_DEPTH, FORMAT_HARD_MAX_ROWS_PER_CHUNK,
    FORMAT_HARD_MAX_SOURCE_LOCATION_ROWS_PER_CHUNK, FORMAT_HARD_MAX_STAGED_CHUNK_BYTES,
    FORMAT_HARD_MAX_TABLE_CHUNK_BYTES, FORMAT_HARD_MAX_TOTAL_UTF8_BYTES,
    FORMAT_HARD_MAX_TOTAL_VECTOR_BYTES, FORMAT_HARD_MAX_UTF8_FIELD_BYTES,
    FORMAT_HARD_MAX_VECTOR_ITEMS, PortableObjectKind,
};

use crate::{FormatError, LimitDimension};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CanonicalRowMetrics {
    pub(crate) exact_byte_length: u64,
    pub(crate) total_utf8_bytes: u64,
    pub(crate) total_vector_bytes: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CanonicalChunkMetrics {
    pub(crate) row_count: u32,
    pub(crate) exact_byte_length: u64,
    pub(crate) total_utf8_bytes: u64,
    pub(crate) total_vector_bytes: u64,
}

impl CanonicalChunkMetrics {
    pub(crate) const fn empty(table_header_byte_length: u64) -> Self {
        Self {
            row_count: 0,
            exact_byte_length: table_header_byte_length,
            total_utf8_bytes: 0,
            total_vector_bytes: 0,
        }
    }
}

pub(crate) fn canonical_chunk_with_appended_row(
    object_kind: PortableObjectKind,
    section_kind: u16,
    table_kind: u16,
    chunk: CanonicalChunkMetrics,
    row: CanonicalRowMetrics,
) -> Option<CanonicalChunkMetrics> {
    let next = CanonicalChunkMetrics {
        row_count: chunk.row_count.checked_add(1)?,
        exact_byte_length: chunk.exact_byte_length.checked_add(row.exact_byte_length)?,
        total_utf8_bytes: chunk.total_utf8_bytes.checked_add(row.total_utf8_bytes)?,
        total_vector_bytes: chunk
            .total_vector_bytes
            .checked_add(row.total_vector_bytes)?,
    };
    let row_limit =
        if object_kind == PortableObjectKind::SourceMap && section_kind == 2 && table_kind == 3 {
            FORMAT_HARD_MAX_ROWS_PER_CHUNK.min(FORMAT_HARD_MAX_SOURCE_LOCATION_ROWS_PER_CHUNK)
        } else {
            FORMAT_HARD_MAX_ROWS_PER_CHUNK
        };
    (next.row_count <= row_limit
        && next.exact_byte_length <= FORMAT_HARD_MAX_TABLE_CHUNK_BYTES
        && next.total_utf8_bytes <= FORMAT_HARD_MAX_TOTAL_UTF8_BYTES
        && next.total_vector_bytes <= FORMAT_HARD_MAX_TOTAL_VECTOR_BYTES)
        .then_some(next)
}

/// 构造 [`FormatLimits`] 的调用方配置。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FormatLimitConfig {
    pub max_object_bytes: u64,
    pub max_chunks_per_section: u32,
    pub max_table_chunk_bytes: u64,
    pub max_rows_per_chunk: u32,
    pub max_fields_per_row: u32,
    pub max_identity_ascii_bytes: u64,
    pub max_utf8_field_bytes: u64,
    pub max_total_utf8_bytes: u64,
    pub max_vector_items: u32,
    pub max_total_vector_bytes: u64,
    pub max_record_vector_depth: u8,
    pub max_source_location_rows_per_chunk: u32,
    pub max_staged_chunk_bytes: u64,
}

impl FormatLimitConfig {
    /// 百万单路网默认接收预算与线格式原语天花板。
    ///
    /// `max_object_bytes` 与 `max_chunks_per_section` 是调用方预算，不是 wire hard limit；
    /// 其余字段不得高于格式原语天花板。
    pub const HARD: Self = Self {
        max_object_bytes: 4_294_967_296,
        max_chunks_per_section: 65_536,
        max_table_chunk_bytes: FORMAT_HARD_MAX_TABLE_CHUNK_BYTES,
        max_rows_per_chunk: FORMAT_HARD_MAX_ROWS_PER_CHUNK,
        max_fields_per_row: FORMAT_HARD_MAX_FIELDS_PER_ROW,
        max_identity_ascii_bytes: FORMAT_HARD_MAX_IDENTITY_ASCII_BYTES,
        max_utf8_field_bytes: FORMAT_HARD_MAX_UTF8_FIELD_BYTES,
        max_total_utf8_bytes: FORMAT_HARD_MAX_TOTAL_UTF8_BYTES,
        max_vector_items: FORMAT_HARD_MAX_VECTOR_ITEMS,
        max_total_vector_bytes: FORMAT_HARD_MAX_TOTAL_VECTOR_BYTES,
        max_record_vector_depth: FORMAT_HARD_MAX_RECORD_VECTOR_DEPTH,
        max_source_location_rows_per_chunk: FORMAT_HARD_MAX_SOURCE_LOCATION_ROWS_PER_CHUNK,
        max_staged_chunk_bytes: FORMAT_HARD_MAX_STAGED_CHUNK_BYTES,
    };
}

impl Default for FormatLimitConfig {
    fn default() -> Self {
        Self::HARD
    }
}

/// 已证明没有扩大格式天花板的有效调用方限制。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FormatLimits(FormatLimitConfig);

impl FormatLimits {
    /// 直接使用全部格式安全天花板。
    pub const HARD: Self = Self(FormatLimitConfig::HARD);

    /// 校验调用方配置；原语维度高于格式天花板时失败，而不是静默 clamp。
    pub fn try_new(config: FormatLimitConfig) -> Result<Self, FormatError> {
        if config.max_object_bytes == 0 {
            return Err(FormatError::InvalidLimitConfiguration {
                dimension: LimitDimension::ObjectBytes,
                requested: 0,
                hard_limit: u64::MAX,
            });
        }
        if config.max_chunks_per_section == 0 {
            return Err(FormatError::InvalidLimitConfiguration {
                dimension: LimitDimension::ChunksPerSection,
                requested: 0,
                hard_limit: u64::from(u32::MAX),
            });
        }
        if config.max_rows_per_chunk == 0 {
            return Err(FormatError::InvalidLimitConfiguration {
                dimension: LimitDimension::RowsPerChunk,
                requested: 0,
                hard_limit: u64::from(FORMAT_HARD_MAX_ROWS_PER_CHUNK),
            });
        }
        if config.max_source_location_rows_per_chunk == 0 {
            return Err(FormatError::InvalidLimitConfiguration {
                dimension: LimitDimension::SourceLocationRowsPerChunk,
                requested: 0,
                hard_limit: u64::from(FORMAT_HARD_MAX_SOURCE_LOCATION_ROWS_PER_CHUNK),
            });
        }
        check_limit(
            LimitDimension::TableChunkBytes,
            config.max_table_chunk_bytes,
            FORMAT_HARD_MAX_TABLE_CHUNK_BYTES,
        )?;
        check_limit(
            LimitDimension::RowsPerChunk,
            u64::from(config.max_rows_per_chunk),
            u64::from(FORMAT_HARD_MAX_ROWS_PER_CHUNK),
        )?;
        check_limit(
            LimitDimension::FieldsPerRow,
            u64::from(config.max_fields_per_row),
            u64::from(FORMAT_HARD_MAX_FIELDS_PER_ROW),
        )?;
        check_limit(
            LimitDimension::IdentityAsciiBytes,
            config.max_identity_ascii_bytes,
            FORMAT_HARD_MAX_IDENTITY_ASCII_BYTES,
        )?;
        check_limit(
            LimitDimension::Utf8FieldBytes,
            config.max_utf8_field_bytes,
            FORMAT_HARD_MAX_UTF8_FIELD_BYTES,
        )?;
        check_limit(
            LimitDimension::TotalUtf8Bytes,
            config.max_total_utf8_bytes,
            FORMAT_HARD_MAX_TOTAL_UTF8_BYTES,
        )?;
        check_limit(
            LimitDimension::VectorItems,
            u64::from(config.max_vector_items),
            u64::from(FORMAT_HARD_MAX_VECTOR_ITEMS),
        )?;
        check_limit(
            LimitDimension::TotalVectorBytes,
            config.max_total_vector_bytes,
            FORMAT_HARD_MAX_TOTAL_VECTOR_BYTES,
        )?;
        check_limit(
            LimitDimension::RecordVectorDepth,
            u64::from(config.max_record_vector_depth),
            u64::from(FORMAT_HARD_MAX_RECORD_VECTOR_DEPTH),
        )?;
        check_limit(
            LimitDimension::SourceLocationRowsPerChunk,
            u64::from(config.max_source_location_rows_per_chunk),
            u64::from(FORMAT_HARD_MAX_SOURCE_LOCATION_ROWS_PER_CHUNK),
        )?;
        check_limit(
            LimitDimension::StagedChunkBytes,
            config.max_staged_chunk_bytes,
            FORMAT_HARD_MAX_STAGED_CHUNK_BYTES,
        )?;
        Ok(Self(config))
    }

    /// 返回调用方选择的单对象读取预算。
    #[must_use]
    pub const fn max_object_bytes(self) -> u64 {
        self.0.max_object_bytes
    }

    /// 返回 Identity v1 ASCII 值的有效上限。
    #[must_use]
    pub const fn max_identity_ascii_bytes(self) -> u64 {
        self.0.max_identity_ascii_bytes
    }

    /// 返回每个 section 允许的最大物理 chunk 数。
    #[must_use]
    pub const fn max_chunks_per_section(self) -> u32 {
        self.0.max_chunks_per_section
    }

    /// 返回单 LFSM SourceLocation chunk 的有效行数上限。
    #[must_use]
    pub const fn max_source_location_rows_per_chunk(self) -> u32 {
        self.0.max_source_location_rows_per_chunk
    }

    /// 返回三个对象同时暂存当前 chunk 的有效内存上限。
    #[must_use]
    pub const fn max_staged_chunk_bytes(self) -> u64 {
        self.0.max_staged_chunk_bytes
    }

    pub(crate) const fn config(self) -> FormatLimitConfig {
        self.0
    }
}

fn check_limit(
    dimension: LimitDimension,
    requested: u64,
    hard_limit: u64,
) -> Result<(), FormatError> {
    if requested > hard_limit {
        return Err(FormatError::InvalidLimitConfiguration {
            dimension,
            requested,
            hard_limit,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::FormatErrorClass;

    #[test]
    fn caller_can_choose_a_nonzero_object_budget() {
        let mut config = FormatLimitConfig::HARD;
        config.max_object_bytes -= 1;
        assert_eq!(FormatLimits::try_new(config).unwrap().config(), config);

        config.max_object_bytes = u64::MAX;
        assert_eq!(FormatLimits::try_new(config).unwrap().config(), config);

        config.max_object_bytes = 0;
        assert_eq!(
            FormatLimits::try_new(config).unwrap_err().class(),
            FormatErrorClass::InvalidLimitConfiguration
        );
    }

    #[test]
    fn every_configurable_dimension_rejects_hard_limit_plus_one() {
        let cases = [
            (
                LimitDimension::TableChunkBytes,
                FORMAT_HARD_MAX_TABLE_CHUNK_BYTES + 1,
                FORMAT_HARD_MAX_TABLE_CHUNK_BYTES,
            ),
            (
                LimitDimension::RowsPerChunk,
                u64::from(FORMAT_HARD_MAX_ROWS_PER_CHUNK) + 1,
                u64::from(FORMAT_HARD_MAX_ROWS_PER_CHUNK),
            ),
            (
                LimitDimension::FieldsPerRow,
                u64::from(FORMAT_HARD_MAX_FIELDS_PER_ROW) + 1,
                u64::from(FORMAT_HARD_MAX_FIELDS_PER_ROW),
            ),
            (
                LimitDimension::IdentityAsciiBytes,
                FORMAT_HARD_MAX_IDENTITY_ASCII_BYTES + 1,
                FORMAT_HARD_MAX_IDENTITY_ASCII_BYTES,
            ),
            (
                LimitDimension::Utf8FieldBytes,
                FORMAT_HARD_MAX_UTF8_FIELD_BYTES + 1,
                FORMAT_HARD_MAX_UTF8_FIELD_BYTES,
            ),
            (
                LimitDimension::TotalUtf8Bytes,
                FORMAT_HARD_MAX_TOTAL_UTF8_BYTES + 1,
                FORMAT_HARD_MAX_TOTAL_UTF8_BYTES,
            ),
            (
                LimitDimension::VectorItems,
                u64::from(FORMAT_HARD_MAX_VECTOR_ITEMS) + 1,
                u64::from(FORMAT_HARD_MAX_VECTOR_ITEMS),
            ),
            (
                LimitDimension::TotalVectorBytes,
                FORMAT_HARD_MAX_TOTAL_VECTOR_BYTES + 1,
                FORMAT_HARD_MAX_TOTAL_VECTOR_BYTES,
            ),
            (
                LimitDimension::RecordVectorDepth,
                u64::from(FORMAT_HARD_MAX_RECORD_VECTOR_DEPTH) + 1,
                u64::from(FORMAT_HARD_MAX_RECORD_VECTOR_DEPTH),
            ),
            (
                LimitDimension::SourceLocationRowsPerChunk,
                u64::from(FORMAT_HARD_MAX_SOURCE_LOCATION_ROWS_PER_CHUNK) + 1,
                u64::from(FORMAT_HARD_MAX_SOURCE_LOCATION_ROWS_PER_CHUNK),
            ),
            (
                LimitDimension::StagedChunkBytes,
                FORMAT_HARD_MAX_STAGED_CHUNK_BYTES + 1,
                FORMAT_HARD_MAX_STAGED_CHUNK_BYTES,
            ),
        ];

        for (dimension, requested, hard_limit) in cases {
            let mut config = FormatLimitConfig::HARD;
            match dimension {
                LimitDimension::TableChunkBytes => {
                    config.max_table_chunk_bytes = requested;
                }
                LimitDimension::RowsPerChunk => {
                    config.max_rows_per_chunk = u32::try_from(requested).unwrap();
                }
                LimitDimension::FieldsPerRow => {
                    config.max_fields_per_row = u32::try_from(requested).unwrap();
                }
                LimitDimension::IdentityAsciiBytes => {
                    config.max_identity_ascii_bytes = requested;
                }
                LimitDimension::Utf8FieldBytes => config.max_utf8_field_bytes = requested,
                LimitDimension::TotalUtf8Bytes => config.max_total_utf8_bytes = requested,
                LimitDimension::VectorItems => {
                    config.max_vector_items = u32::try_from(requested).unwrap();
                }
                LimitDimension::TotalVectorBytes => config.max_total_vector_bytes = requested,
                LimitDimension::RecordVectorDepth => {
                    config.max_record_vector_depth = u8::try_from(requested).unwrap();
                }
                LimitDimension::SourceLocationRowsPerChunk => {
                    config.max_source_location_rows_per_chunk = u32::try_from(requested).unwrap();
                }
                LimitDimension::StagedChunkBytes => {
                    config.max_staged_chunk_bytes = requested;
                }
                LimitDimension::ObjectBytes | LimitDimension::ChunksPerSection => unreachable!(),
            }

            assert_eq!(
                FormatLimits::try_new(config),
                Err(FormatError::InvalidLimitConfiguration {
                    dimension,
                    requested,
                    hard_limit,
                })
            );
        }
    }

    #[test]
    fn row_chunking_limits_must_be_nonzero() {
        for dimension in [
            LimitDimension::RowsPerChunk,
            LimitDimension::SourceLocationRowsPerChunk,
        ] {
            let mut config = FormatLimitConfig::HARD;
            match dimension {
                LimitDimension::RowsPerChunk => config.max_rows_per_chunk = 0,
                LimitDimension::SourceLocationRowsPerChunk => {
                    config.max_source_location_rows_per_chunk = 0;
                }
                _ => unreachable!(),
            }
            assert!(matches!(
                FormatLimits::try_new(config),
                Err(FormatError::InvalidLimitConfiguration {
                    dimension: actual,
                    requested: 0,
                    ..
                }) if actual == dimension
            ));
        }
    }
}
