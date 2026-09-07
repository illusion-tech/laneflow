//! 准入与路权共用的法规身份值；宿主负责业务时间和策略选择。

use crate::{Diagnostic, DiagnosticBundle, RoadEditingInputViolation};

/// 固定规则含义的法域、版本与可选来源，不是独立稳定实体。
///
/// Synthetic 输入借用 `&str`；拥有型编制输入使用 `Box<str>`，私有阶段使用 `Arc<str>`。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RegulationIdentity<S = Box<str>> {
    pub jurisdiction: S,
    pub version: S,
    pub source: Option<S>,
}

impl RegulationIdentity {
    /// 受检构造仅含法域与版本的法规身份。
    pub fn try_new(
        jurisdiction: impl Into<String>,
        version: impl Into<String>,
    ) -> Result<Self, DiagnosticBundle> {
        let jurisdiction = jurisdiction.into();
        let version = version.into();
        validate_text(&jurisdiction, "regulation.jurisdiction")?;
        validate_text(&version, "regulation.version")?;
        Ok(Self {
            jurisdiction: jurisdiction.into_boxed_str(),
            version: version.into_boxed_str(),
            source: None,
        })
    }

    /// 为法规身份附加可选来源，同样受检。
    pub fn with_source(mut self, source: impl Into<String>) -> Result<Self, DiagnosticBundle> {
        let source = source.into();
        validate_text(&source, "regulation.source")?;
        self.source = Some(source.into_boxed_str());
        Ok(self)
    }
}

impl<S: AsRef<str>> RegulationIdentity<S> {
    /// 返回法域。
    #[must_use]
    pub fn jurisdiction(&self) -> &str {
        self.jurisdiction.as_ref()
    }
    /// 返回规则含义的版本。
    #[must_use]
    pub fn version(&self) -> &str {
        self.version.as_ref()
    }
    /// 返回可选法规来源。
    #[must_use]
    pub fn source(&self) -> Option<&str> {
        self.source.as_ref().map(AsRef::as_ref)
    }

    /// 校验法域、版本与可选来源的文本约束。
    pub(crate) fn validate(&self) -> Result<(), DiagnosticBundle> {
        validate_text(self.jurisdiction(), "regulation.jurisdiction")?;
        validate_text(self.version(), "regulation.version")?;
        if let Some(source) = self.source() {
            validate_text(source, "regulation.source")?;
        }
        Ok(())
    }
}

/// 判断文本是否满足法规字段的长度约束（1 至 128 字符）。
pub(crate) fn valid_text(value: &str) -> bool {
    (1..=128).contains(&value.chars().count())
}

fn validate_text(value: &str, field: &str) -> Result<(), DiagnosticBundle> {
    if valid_text(value) {
        Ok(())
    } else {
        Err(DiagnosticBundle::single(
            Diagnostic::invalid_road_editing_input(
                field,
                RoadEditingInputViolation::InvalidCombination,
            ),
        ))
    }
}
