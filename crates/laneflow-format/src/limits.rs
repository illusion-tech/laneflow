//! 调用方可收紧的格式资源限制。

use laneflow_static_contract::{
    FORMAT_HARD_MAX_CANDIDATE_STAGING_BYTES, FORMAT_HARD_MAX_FIELDS_PER_ROW,
    FORMAT_HARD_MAX_IDENTITY_ASCII_BYTES, FORMAT_HARD_MAX_OBJECT_BYTES,
    FORMAT_HARD_MAX_RECORD_VECTOR_DEPTH, FORMAT_HARD_MAX_ROWS_PER_TABLE,
    FORMAT_HARD_MAX_SECTION_OR_TABLE_BYTES, FORMAT_HARD_MAX_SOURCE_LOCATION_ROWS,
    FORMAT_HARD_MAX_TOTAL_UTF8_BYTES, FORMAT_HARD_MAX_TOTAL_VECTOR_BYTES,
    FORMAT_HARD_MAX_UTF8_FIELD_BYTES, FORMAT_HARD_MAX_VECTOR_ITEMS,
};

use crate::{FormatError, LimitDimension};

/// 构造 [`FormatLimits`] 的调用方配置。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FormatLimitConfig {
    pub max_object_bytes: u64,
    pub max_section_or_table_bytes: u64,
    pub max_rows_per_table: u32,
    pub max_fields_per_row: u32,
    pub max_identity_ascii_bytes: u64,
    pub max_utf8_field_bytes: u64,
    pub max_total_utf8_bytes: u64,
    pub max_vector_items: u32,
    pub max_total_vector_bytes: u64,
    pub max_record_vector_depth: u8,
    pub max_source_location_rows: u32,
    pub max_candidate_staging_bytes: u64,
}

impl FormatLimitConfig {
    /// v1 格式天花板；调用方可以复制后只减小所需维度。
    pub const V1_HARD: Self = Self {
        max_object_bytes: FORMAT_HARD_MAX_OBJECT_BYTES,
        max_section_or_table_bytes: FORMAT_HARD_MAX_SECTION_OR_TABLE_BYTES,
        max_rows_per_table: FORMAT_HARD_MAX_ROWS_PER_TABLE,
        max_fields_per_row: FORMAT_HARD_MAX_FIELDS_PER_ROW,
        max_identity_ascii_bytes: FORMAT_HARD_MAX_IDENTITY_ASCII_BYTES,
        max_utf8_field_bytes: FORMAT_HARD_MAX_UTF8_FIELD_BYTES,
        max_total_utf8_bytes: FORMAT_HARD_MAX_TOTAL_UTF8_BYTES,
        max_vector_items: FORMAT_HARD_MAX_VECTOR_ITEMS,
        max_total_vector_bytes: FORMAT_HARD_MAX_TOTAL_VECTOR_BYTES,
        max_record_vector_depth: FORMAT_HARD_MAX_RECORD_VECTOR_DEPTH,
        max_source_location_rows: FORMAT_HARD_MAX_SOURCE_LOCATION_ROWS,
        max_candidate_staging_bytes: FORMAT_HARD_MAX_CANDIDATE_STAGING_BYTES,
    };
}

impl Default for FormatLimitConfig {
    fn default() -> Self {
        Self::V1_HARD
    }
}

/// 已证明没有扩大 v1 格式天花板的有效调用方限制。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FormatLimits(FormatLimitConfig);

impl FormatLimits {
    /// 直接使用全部 v1 格式安全天花板。
    pub const V1_HARD: Self = Self(FormatLimitConfig::V1_HARD);

    /// 校验调用方配置；任一维度高于格式天花板都失败，而不是静默 clamp。
    pub fn try_new(config: FormatLimitConfig) -> Result<Self, FormatError> {
        check_limit(
            LimitDimension::ObjectBytes,
            config.max_object_bytes,
            FORMAT_HARD_MAX_OBJECT_BYTES,
        )?;
        check_limit(
            LimitDimension::SectionOrTableBytes,
            config.max_section_or_table_bytes,
            FORMAT_HARD_MAX_SECTION_OR_TABLE_BYTES,
        )?;
        check_limit(
            LimitDimension::RowsPerTable,
            u64::from(config.max_rows_per_table),
            u64::from(FORMAT_HARD_MAX_ROWS_PER_TABLE),
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
            LimitDimension::SourceLocationRows,
            u64::from(config.max_source_location_rows),
            u64::from(FORMAT_HARD_MAX_SOURCE_LOCATION_ROWS),
        )?;
        check_limit(
            LimitDimension::CandidateStagingBytes,
            config.max_candidate_staging_bytes,
            FORMAT_HARD_MAX_CANDIDATE_STAGING_BYTES,
        )?;
        Ok(Self(config))
    }

    /// 返回调用方已经证明不高于 v1 天花板的单对象读取上限。
    #[must_use]
    pub const fn max_object_bytes(self) -> u64 {
        self.0.max_object_bytes
    }

    /// 返回 Identity v1 ASCII 值的有效上限。
    #[must_use]
    pub const fn max_identity_ascii_bytes(self) -> u64 {
        self.0.max_identity_ascii_bytes
    }

    /// 返回单 LFSM 来源位置记录的有效上限。
    #[must_use]
    pub const fn max_source_location_rows(self) -> u32 {
        self.0.max_source_location_rows
    }

    /// 返回一次候选暂存 exact bytes 的有效上限。
    #[must_use]
    pub const fn max_candidate_staging_bytes(self) -> u64 {
        self.0.max_candidate_staging_bytes
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

    #[test]
    fn caller_can_only_reduce_hard_limits() {
        let mut config = FormatLimitConfig::V1_HARD;
        config.max_object_bytes -= 1;
        assert_eq!(FormatLimits::try_new(config).unwrap().config(), config);

        config.max_object_bytes = FORMAT_HARD_MAX_OBJECT_BYTES + 1;
        assert_eq!(
            FormatLimits::try_new(config),
            Err(FormatError::InvalidLimitConfiguration {
                dimension: LimitDimension::ObjectBytes,
                requested: FORMAT_HARD_MAX_OBJECT_BYTES + 1,
                hard_limit: FORMAT_HARD_MAX_OBJECT_BYTES,
            })
        );
    }

    #[test]
    fn every_configurable_dimension_rejects_hard_limit_plus_one() {
        let cases = [
            (
                LimitDimension::SectionOrTableBytes,
                FORMAT_HARD_MAX_SECTION_OR_TABLE_BYTES + 1,
                FORMAT_HARD_MAX_SECTION_OR_TABLE_BYTES,
            ),
            (
                LimitDimension::RowsPerTable,
                u64::from(FORMAT_HARD_MAX_ROWS_PER_TABLE) + 1,
                u64::from(FORMAT_HARD_MAX_ROWS_PER_TABLE),
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
                LimitDimension::SourceLocationRows,
                u64::from(FORMAT_HARD_MAX_SOURCE_LOCATION_ROWS) + 1,
                u64::from(FORMAT_HARD_MAX_SOURCE_LOCATION_ROWS),
            ),
            (
                LimitDimension::CandidateStagingBytes,
                FORMAT_HARD_MAX_CANDIDATE_STAGING_BYTES + 1,
                FORMAT_HARD_MAX_CANDIDATE_STAGING_BYTES,
            ),
        ];

        for (dimension, requested, hard_limit) in cases {
            let mut config = FormatLimitConfig::V1_HARD;
            match dimension {
                LimitDimension::SectionOrTableBytes => {
                    config.max_section_or_table_bytes = requested;
                }
                LimitDimension::RowsPerTable => {
                    config.max_rows_per_table = u32::try_from(requested).unwrap();
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
                LimitDimension::SourceLocationRows => {
                    config.max_source_location_rows = u32::try_from(requested).unwrap();
                }
                LimitDimension::CandidateStagingBytes => {
                    config.max_candidate_staging_bytes = requested;
                }
                LimitDimension::ObjectBytes => unreachable!(),
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
}
