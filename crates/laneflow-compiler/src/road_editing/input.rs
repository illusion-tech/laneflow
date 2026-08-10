use core::fmt;

use crate::SourceTextViolation;
use crate::source::external_token_violation;

const SOURCE_DOCUMENT_KEY_MAX_BYTES: u64 = 53;

/// 建立道路编辑来源借用输入时发现的外部文档身份错误。
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct InvalidRoadEditingModuleInput {
    violation: SourceTextViolation,
}

impl InvalidRoadEditingModuleInput {
    /// 返回 expected source-document key 违反的精确 token 规则。
    #[must_use]
    pub const fn violation(self) -> SourceTextViolation {
        self.violation
    }
}

impl fmt::Display for InvalidRoadEditingModuleInput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "道路编辑来源的 expected source-document key 非法：{:?}",
            self.violation
        )
    }
}

impl std::error::Error for InvalidRoadEditingModuleInput {}

/// 一份待受检加入编译单元的 size-prefixed `LFRE` 来源借用。
///
/// 本类型只验证来自 wire 外部、损坏输入也能使用的稳定文档身份。它不读取、哈希或验证
/// `source_bytes`；真正的长度、identifier、FlatBuffers verifier、版本和语义检查只发生在
/// `CompilationUnitBuilder::add_road_editing_module` 的原子事务内。
#[derive(Clone, Copy, Debug)]
pub struct RoadEditingModuleInput<'a> {
    expected_source_document_key: &'a str,
    source_bytes: &'a [u8],
    display_source: Option<&'a str>,
}

impl<'a> RoadEditingModuleInput<'a> {
    /// 组装待受检来源；不会访问 `source_bytes`。
    ///
    /// # Errors
    ///
    /// 当 expected key 为空、超过 53 bytes、包含非 ASCII、控制字节或保留的 `::`
    /// 分隔符时失败。
    pub fn try_new(
        expected_source_document_key: &'a str,
        source_bytes: &'a [u8],
        display_source: Option<&'a str>,
    ) -> Result<Self, InvalidRoadEditingModuleInput> {
        if let Some(violation) =
            external_token_violation(expected_source_document_key, SOURCE_DOCUMENT_KEY_MAX_BYTES)
        {
            return Err(InvalidRoadEditingModuleInput { violation });
        }
        if let Some(byte_index) = expected_source_document_key
            .as_bytes()
            .windows(2)
            .position(|pair| pair == b"::")
        {
            return Err(InvalidRoadEditingModuleInput {
                violation: SourceTextViolation::ReservedDelimiter {
                    byte_index: u64::try_from(byte_index).unwrap_or(u64::MAX),
                },
            });
        }
        Ok(Self {
            expected_source_document_key,
            source_bytes,
            display_source,
        })
    }

    /// 返回 wire 外部提供的稳定文档键。
    #[must_use]
    pub const fn expected_source_document_key(&self) -> &'a str {
        self.expected_source_document_key
    }

    /// 返回完整 size-prefixed 来源 bytes。
    #[must_use]
    pub const fn source_bytes(&self) -> &'a [u8] {
        self.source_bytes
    }

    /// 返回不参与稳定身份或摘要的显示/审计来源。
    #[must_use]
    pub const fn display_source(&self) -> Option<&'a str> {
        self.display_source
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_only_validates_the_external_document_identity() {
        let bytes = [0xff, 0x00];
        let input = RoadEditingModuleInput::try_new("roads/main", &bytes, Some("save slot"))
            .expect("valid expected key");

        assert_eq!(input.expected_source_document_key(), "roads/main");
        assert_eq!(input.source_bytes(), bytes);
        assert_eq!(input.display_source(), Some("save slot"));
    }

    #[test]
    fn input_rejects_reserved_owner_qualification_delimiter() {
        let error = RoadEditingModuleInput::try_new("roads::main", &[], None)
            .expect_err("reserved delimiter must fail");

        assert!(matches!(
            error.violation(),
            SourceTextViolation::ReservedDelimiter { byte_index: 5 }
        ));
    }
}
