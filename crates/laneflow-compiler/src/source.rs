//! 官方合成来源模块头的受检构造。
//!
//! 调用方只提供不含内容摘要的来源描述信息；本模块验证文本和资源上限后取得所有权。
//! 规范来源记录及 `sourceContentDigest` 必须在完整领域声明已经规范化后由
//! `SyntheticModuleBuilder::finish` 派生，因此头不能与任意内容摘要自行配对。

use crate::diagnostic::DiagnosticCollector;
use crate::{
    CompileLimitDimension, CompileLimits, Diagnostic, DiagnosticBundle, SourceHeaderField,
    SourceTextViolation,
};
use std::sync::Arc;

/// 调用方提供、随后由编译器受检复制的合成来源模块头输入。
///
/// 字符串只在 [`SourceModuleHeader::new`] 成功后复制；失败不会留下部分头。两个摘要均
/// 是调用方声明的来源沿袭元数据，不是模块内容摘要，也不会替代后续
/// `sourceContentDigest` 的派生。
#[derive(Clone, Copy, Debug)]
pub struct SourceModuleHeaderInput<'a> {
    /// 模块拥有声明的稳定 ASCII 命名空间，也是声明身份前像的一部分。
    pub authoring_namespace_id: &'a str,
    /// 与机器路径无关的稳定 ASCII 来源文档键，用于诊断位置与规范排序。
    pub source_document_key: &'a str,
    /// 生成该来源的前端或生成器构建标识；仅作可审计来源沿袭。
    pub generator_build_id: &'a str,
    /// 调用参数与外部输入集合的 32 字节摘要；其算法由生成方契约负责。
    pub parameters_and_inputs_digest: [u8; 32],
    /// 影响前端输出的选项集合的 32 字节摘要；不是来源记录内容摘要。
    pub frontend_options_digest: [u8; 32],
    /// 生成过程实际使用的随机种子；若生成过程使用随机性，调用方必须提供 `Some`。
    /// 本构造器只能保存该声明，无法审计生成器是否暗中使用了未登记随机源。
    pub random_seed: Option<u64>,
    /// 供审计与诊断展示的可见 ASCII 来源沿袭说明。
    pub provenance: &'a str,
}

/// 已验证且由编译器拥有的来源模块头。
///
/// 本类型不包含 `sourceContentDigest`：内容摘要只能由官方前端在规范化来源记录完成后
/// 派生，调用方不能自报或把头与任意模块内容配对。
pub struct SourceModuleHeader {
    /// 已验证的声明身份命名空间。
    pub(crate) authoring_namespace_id: Arc<str>,
    /// 已验证、与机器路径无关的来源文档键。
    pub(crate) source_document_key: Arc<str>,
    /// 已验证的生成器构建标识。
    pub(crate) generator_build_id: Arc<str>,
    /// 原样保留的参数与输入摘要。
    pub(crate) parameters_and_inputs_digest: [u8; 32],
    /// 原样保留的前端选项摘要。
    pub(crate) frontend_options_digest: [u8; 32],
    /// 原样保留的可选随机种子。
    pub(crate) random_seed: Option<u64>,
    /// 已验证的来源沿袭说明。
    pub(crate) provenance: Arc<str>,
    /// 合成前端调用点；用于给模块级诊断提供稳定来源位置，不参与实体身份。
    pub(crate) declaration_span: crate::SourceSpan,
}

impl SourceModuleHeader {
    /// 校验并原子复制合成来源模块的非内容字段。
    ///
    /// 成功后返回的头拥有全部字符串，并记录此调用点作为模块声明位置。该位置使用
    /// `source_document_key`，不会记录宿主机器路径。
    ///
    /// # Errors
    ///
    /// 当必填文本为空、超过字符串上限、违反 ASCII/token 规则，或逻辑字符串与编译器
    /// 控制存续字节超过 `limits` 时，返回规范有序的 [`DiagnosticBundle`]。所有安全可
    /// 检查字段都会参与诊断收集；失败时不复制并返回部分头。
    #[track_caller]
    pub fn new(
        input: SourceModuleHeaderInput<'_>,
        limits: &CompileLimits,
    ) -> Result<Self, DiagnosticBundle> {
        let mut diagnostics =
            DiagnosticCollector::new(limits.value(CompileLimitDimension::DiagnosticCount));
        let single_string_limit = limits.value(CompileLimitDimension::SingleStringBytes);

        validate_external_token(
            input.authoring_namespace_id,
            SourceHeaderField::AuthoringNamespaceId,
            limits.identity_ascii_bytes_limit(),
            &mut diagnostics,
        );
        validate_external_token(
            input.source_document_key,
            SourceHeaderField::SourceDocumentKey,
            single_string_limit,
            &mut diagnostics,
        );
        validate_visible_ascii(
            input.generator_build_id,
            SourceHeaderField::GeneratorBuildId,
            single_string_limit,
            &mut diagnostics,
        );
        validate_visible_ascii(
            input.provenance,
            SourceHeaderField::Provenance,
            single_string_limit,
            &mut diagnostics,
        );

        // 校准契约只把规范模块名与来源文档键计入驻留字符串聚合；生成器标识和
        // 来源沿袭属于描述符元数据，但其复制字节仍计入编译器控制存续内存。
        let string_item_count = 2;
        check_limit(
            limits,
            CompileLimitDimension::StringItemCount,
            string_item_count,
            &mut diagnostics,
        );
        let total_string_bytes = [input.authoring_namespace_id, input.source_document_key]
            .into_iter()
            .try_fold(0_u64, |total, value| {
                total.checked_add(u64::try_from(value.len()).ok()?)
            })
            .unwrap_or(u64::MAX);
        check_limit(
            limits,
            CompileLimitDimension::TotalStringBytes,
            total_string_bytes,
            &mut diagnostics,
        );
        let controlled_live_bytes = [
            input.authoring_namespace_id,
            input.source_document_key,
            input.generator_build_id,
            input.provenance,
        ]
        .into_iter()
        .try_fold(0_u64, |total, value| {
            total.checked_add(u64::try_from(value.len()).ok()?)
        })
        .unwrap_or(u64::MAX);
        check_limit(
            limits,
            CompileLimitDimension::CompilerControlledLiveBytes,
            controlled_live_bytes,
            &mut diagnostics,
        );

        if !diagnostics.is_empty() {
            return Err(diagnostics.finish());
        }

        let source_document_key: Arc<str> = input.source_document_key.into();
        let declaration_span = crate::SourceSpan::at_caller(
            Arc::clone(&source_document_key),
            std::panic::Location::caller(),
        );
        Ok(Self {
            authoring_namespace_id: input.authoring_namespace_id.into(),
            source_document_key,
            generator_build_id: input.generator_build_id.into(),
            parameters_and_inputs_digest: input.parameters_and_inputs_digest,
            frontend_options_digest: input.frontend_options_digest,
            random_seed: input.random_seed,
            provenance: input.provenance.into(),
            declaration_span,
        })
    }

    /// 返回声明身份使用的 authoring namespace。
    #[must_use]
    pub fn authoring_namespace_id(&self) -> &str {
        &self.authoring_namespace_id
    }

    /// 返回与机器路径无关的来源文档键。
    #[must_use]
    pub fn source_document_key(&self) -> &str {
        &self.source_document_key
    }

    /// 返回生成器构建标识。
    #[must_use]
    pub fn generator_build_id(&self) -> &str {
        &self.generator_build_id
    }

    /// 返回调用方登记的参数与输入摘要；它不是模块内容摘要。
    #[must_use]
    pub const fn parameters_and_inputs_digest(&self) -> &[u8; 32] {
        &self.parameters_and_inputs_digest
    }

    /// 返回调用方登记的前端选项摘要；它不是模块内容摘要。
    #[must_use]
    pub const fn frontend_options_digest(&self) -> &[u8; 32] {
        &self.frontend_options_digest
    }

    /// 返回生成过程登记的随机种子。
    #[must_use]
    pub const fn random_seed(&self) -> Option<u64> {
        self.random_seed
    }

    /// 返回供审计使用的来源沿袭说明。
    #[must_use]
    pub fn provenance(&self) -> &str {
        &self.provenance
    }
}

fn check_limit(
    limits: &CompileLimits,
    dimension: CompileLimitDimension,
    observed: u64,
    diagnostics: &mut DiagnosticCollector,
) {
    let limit = limits.value(dimension);
    if observed > limit {
        diagnostics.push(Diagnostic::compile_limit_exceeded(
            dimension, limit, observed,
        ));
    }
}

fn validate_external_token(
    value: &str,
    field: SourceHeaderField,
    limit: u64,
    diagnostics: &mut DiagnosticCollector,
) {
    if let Some(violation) = external_token_violation(value, limit) {
        diagnostics.push(Diagnostic::invalid_source_header_field(field, violation));
    }
}

fn validate_visible_ascii(
    value: &str,
    field: SourceHeaderField,
    limit: u64,
    diagnostics: &mut DiagnosticCollector,
) {
    if let Some(violation) = common_text_violation(value, limit) {
        diagnostics.push(Diagnostic::invalid_source_header_field(field, violation));
        return;
    }

    if let Some((byte_index, byte)) = value
        .bytes()
        .enumerate()
        .find(|(_, byte)| !byte.is_ascii_graphic() && *byte != b' ')
    {
        diagnostics.push(Diagnostic::invalid_source_header_field(
            field,
            SourceTextViolation::ControlByte {
                byte_index: u64::try_from(byte_index).unwrap_or(u64::MAX),
                byte,
            },
        ));
    }
}

pub(crate) fn external_token_violation(value: &str, limit: u64) -> Option<SourceTextViolation> {
    if value.is_empty() {
        return Some(SourceTextViolation::Empty);
    }

    let observed = u64::try_from(value.len()).unwrap_or(u64::MAX);
    if observed > limit {
        return Some(SourceTextViolation::TooLong { limit, observed });
    }

    if let Some(byte_index) = value.bytes().position(|byte| !byte.is_ascii()) {
        return Some(SourceTextViolation::NonAscii {
            byte_index: u64::try_from(byte_index).unwrap_or(u64::MAX),
        });
    }

    // 空串已在上方排除，因此读取首字节不会 panic。首字节规则阻止仅由标点构成的键，
    // 后续字符仍允许模块/文档键需要的 `. _ : / -`。
    let bytes = value.as_bytes();
    if !bytes[0].is_ascii_alphanumeric() {
        return Some(SourceTextViolation::InvalidFirstByte { byte: bytes[0] });
    }

    bytes
        .iter()
        .copied()
        .enumerate()
        .find(|(_, byte)| {
            !byte.is_ascii_alphanumeric() && !matches!(byte, b'.' | b'_' | b':' | b'/' | b'-')
        })
        .map(|(byte_index, byte)| SourceTextViolation::InvalidTokenByte {
            byte_index: u64::try_from(byte_index).unwrap_or(u64::MAX),
            byte,
        })
}

fn common_text_violation(value: &str, limit: u64) -> Option<SourceTextViolation> {
    if value.is_empty() {
        return Some(SourceTextViolation::Empty);
    }

    let observed = u64::try_from(value.len()).unwrap_or(u64::MAX);
    if observed > limit {
        return Some(SourceTextViolation::TooLong { limit, observed });
    }

    value
        .bytes()
        .position(|byte| !byte.is_ascii())
        .map(|byte_index| SourceTextViolation::NonAscii {
            byte_index: u64::try_from(byte_index).unwrap_or(u64::MAX),
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DiagnosticPayload, DiagnosticSeverity};

    fn valid_input<'a>() -> SourceModuleHeaderInput<'a> {
        SourceModuleHeaderInput {
            authoring_namespace_id: "city.example/corridor",
            source_document_key: "generator.main",
            generator_build_id: "git:0123456789abcdef",
            parameters_and_inputs_digest: [0x11; 32],
            frontend_options_digest: [0x22; 32],
            random_seed: Some(42),
            provenance: "repository:laneflow",
        }
    }

    #[test]
    fn valid_header_is_owned_without_a_self_reported_content_digest() {
        let input = valid_input();
        let header = SourceModuleHeader::new(input, &CompileLimits::p100_initial_v1()).unwrap();

        assert_eq!(
            header.authoring_namespace_id(),
            input.authoring_namespace_id
        );
        assert_eq!(header.source_document_key(), input.source_document_key);
        assert_eq!(header.generator_build_id(), input.generator_build_id);
        assert_eq!(
            header.parameters_and_inputs_digest(),
            &input.parameters_and_inputs_digest
        );
        assert_eq!(
            header.frontend_options_digest(),
            &input.frontend_options_digest
        );
        assert_eq!(header.random_seed(), Some(42));
        assert_eq!(header.provenance(), input.provenance);
    }

    #[test]
    fn invalid_header_returns_all_safe_candidates_in_canonical_order() {
        let input = SourceModuleHeaderInput {
            authoring_namespace_id: "_bad",
            source_document_key: "bad key",
            generator_build_id: "",
            provenance: "bad\nprovenance",
            ..valid_input()
        };

        let bundle = match SourceModuleHeader::new(input, &CompileLimits::p100_initial_v1()) {
            Ok(_) => panic!("invalid source module header must fail"),
            Err(bundle) => bundle,
        };
        assert!(!bundle.diagnostics_truncated());
        assert!(bundle.has_errors());
        assert_eq!(bundle.diagnostics().len(), 4);
        assert!(
            bundle
                .diagnostics()
                .windows(2)
                .all(|pair| pair[0] < pair[1])
        );
        assert!(
            bundle
                .diagnostics()
                .iter()
                .all(|diagnostic| diagnostic.severity() == DiagnosticSeverity::Error)
        );
        assert!(bundle.diagnostics().iter().any(|diagnostic| matches!(
            diagnostic.payload(),
            DiagnosticPayload::InvalidSourceHeaderField {
                field: SourceHeaderField::GeneratorBuildId,
                violation: SourceTextViolation::Empty,
            }
        )));
    }

    #[test]
    fn fifty_three_bytes_is_inclusive_and_plus_one_is_rejected_before_copy() {
        let at_bound = "a".repeat(53);
        let over_bound = "a".repeat(54);
        let mut input = valid_input();
        input.generator_build_id = &at_bound;
        assert!(SourceModuleHeader::new(input, &CompileLimits::p100_initial_v1()).is_ok());

        input.generator_build_id = &over_bound;
        let bundle = match SourceModuleHeader::new(input, &CompileLimits::p100_initial_v1()) {
            Ok(_) => panic!("over-limit source module header must fail"),
            Err(bundle) => bundle,
        };
        assert!(matches!(
            bundle.diagnostics()[0].payload(),
            DiagnosticPayload::InvalidSourceHeaderField {
                field: SourceHeaderField::GeneratorBuildId,
                violation: SourceTextViolation::TooLong {
                    limit: 53,
                    observed: 54,
                },
            }
        ));
    }

    #[test]
    fn million_profile_keeps_identity_namespace_at_53_without_narrowing_metadata() {
        let at_bound = "a".repeat(53);
        let over_bound = "a".repeat(54);
        let limits = CompileLimits::single_network_1m_v2();
        let mut input = valid_input();
        input.authoring_namespace_id = &at_bound;
        input.provenance = &over_bound;
        assert!(SourceModuleHeader::new(input, &limits).is_ok());

        input.authoring_namespace_id = &over_bound;
        let bundle = match SourceModuleHeader::new(input, &limits) {
            Ok(_) => panic!("over-limit identity namespace must fail"),
            Err(bundle) => bundle,
        };
        assert!(bundle.diagnostics().iter().any(|diagnostic| matches!(
            diagnostic.payload(),
            DiagnosticPayload::InvalidSourceHeaderField {
                field: SourceHeaderField::AuthoringNamespaceId,
                violation: SourceTextViolation::TooLong {
                    limit: 53,
                    observed: 54,
                },
            }
        )));
        assert!(!bundle.diagnostics().iter().any(|diagnostic| matches!(
            diagnostic.payload(),
            DiagnosticPayload::InvalidSourceHeaderField {
                field: SourceHeaderField::Provenance,
                ..
            }
        )));
    }

    #[test]
    fn aggregate_limit_payload_keeps_dimension_limit_and_observed_order() {
        let limits = CompileLimits::p100_initial_v1().with_test_string_limits(1, 1);
        let bundle = match SourceModuleHeader::new(valid_input(), &limits) {
            Ok(_) => panic!("aggregate over-limit source module header must fail"),
            Err(bundle) => bundle,
        };

        assert!(bundle.diagnostics().iter().any(|diagnostic| matches!(
            diagnostic.payload(),
            DiagnosticPayload::CompileLimitExceeded {
                dimension: CompileLimitDimension::TotalStringBytes,
                limit: 1,
                observed,
            } if *observed > 1
        )));
        assert!(bundle.diagnostics().iter().any(|diagnostic| matches!(
            diagnostic.payload(),
            DiagnosticPayload::CompileLimitExceeded {
                dimension: CompileLimitDimension::CompilerControlledLiveBytes,
                limit: 1,
                observed,
            } if *observed > 1
        )));
    }
}
