use crate::diagnostic::DiagnosticCollector;
use crate::{
    CompileLimitDimension, CompileLimits, Diagnostic, DiagnosticBundle, SourceHeaderField,
    SourceTextViolation,
};

/// 调用方提供、随后由编译器受检复制的合成来源模块头输入。
#[derive(Clone, Copy, Debug)]
pub struct SourceModuleHeaderInput<'a> {
    pub authoring_namespace_id: &'a str,
    pub source_document_key: &'a str,
    pub generator_build_id: &'a str,
    pub parameters_and_inputs_digest: [u8; 32],
    pub frontend_options_digest: [u8; 32],
    pub random_seed: Option<u64>,
    pub provenance: &'a str,
}

/// 已验证且由编译器拥有的来源模块头。
///
/// 本类型不包含 `sourceContentDigest`：内容摘要只能由官方前端在规范化来源记录完成后
/// 派生，调用方不能自报或把头与任意模块内容配对。
pub struct SourceModuleHeader {
    authoring_namespace_id: Box<str>,
    source_document_key: Box<str>,
    generator_build_id: Box<str>,
    parameters_and_inputs_digest: [u8; 32],
    frontend_options_digest: [u8; 32],
    random_seed: Option<u64>,
    provenance: Box<str>,
}

impl SourceModuleHeader {
    /// 校验并原子复制合成来源模块的非内容字段。
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
            single_string_limit,
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

        let string_item_count = 4;
        check_limit(
            limits,
            CompileLimitDimension::StringItemCount,
            string_item_count,
            &mut diagnostics,
        );
        let total_string_bytes = [
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
            CompileLimitDimension::TotalStringBytes,
            total_string_bytes,
            &mut diagnostics,
        );
        check_limit(
            limits,
            CompileLimitDimension::CompilerControlledLiveBytes,
            total_string_bytes,
            &mut diagnostics,
        );

        if !diagnostics.is_empty() {
            return Err(diagnostics.finish());
        }

        Ok(Self {
            authoring_namespace_id: input.authoring_namespace_id.into(),
            source_document_key: input.source_document_key.into(),
            generator_build_id: input.generator_build_id.into(),
            parameters_and_inputs_digest: input.parameters_and_inputs_digest,
            frontend_options_digest: input.frontend_options_digest,
            random_seed: input.random_seed,
            provenance: input.provenance.into(),
        })
    }

    #[must_use]
    pub fn authoring_namespace_id(&self) -> &str {
        &self.authoring_namespace_id
    }

    #[must_use]
    pub fn source_document_key(&self) -> &str {
        &self.source_document_key
    }

    #[must_use]
    pub fn generator_build_id(&self) -> &str {
        &self.generator_build_id
    }

    #[must_use]
    pub const fn parameters_and_inputs_digest(&self) -> &[u8; 32] {
        &self.parameters_and_inputs_digest
    }

    #[must_use]
    pub const fn frontend_options_digest(&self) -> &[u8; 32] {
        &self.frontend_options_digest
    }

    #[must_use]
    pub const fn random_seed(&self) -> Option<u64> {
        self.random_seed
    }

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
    if let Some(violation) = common_text_violation(value, limit) {
        diagnostics.push(Diagnostic::invalid_source_header_field(field, violation));
        return;
    }

    let bytes = value.as_bytes();
    if !bytes[0].is_ascii_alphanumeric() {
        diagnostics.push(Diagnostic::invalid_source_header_field(
            field,
            SourceTextViolation::InvalidFirstByte { byte: bytes[0] },
        ));
        return;
    }

    if let Some((byte_index, byte)) = bytes.iter().copied().enumerate().find(|(_, byte)| {
        !byte.is_ascii_alphanumeric() && !matches!(byte, b'.' | b'_' | b':' | b'/' | b'-')
    }) {
        diagnostics.push(Diagnostic::invalid_source_header_field(
            field,
            SourceTextViolation::InvalidTokenByte {
                byte_index: u64::try_from(byte_index).unwrap_or(u64::MAX),
                byte,
            },
        ));
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
