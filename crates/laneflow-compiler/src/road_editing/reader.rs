use laneflow_road_editing_wire::generated::lane_flow::road_editing::v1 as wire;
use laneflow_road_editing_wire::runtime::{InvalidFlatbuffer, VerifierOptions};

use super::RoadEditingModuleInput;
use super::preflight::{RoadEditingPreflightCounts, preflight_source};
use crate::{
    CompileLimitDimension, CompileLimits, Diagnostic, DiagnosticBundle, RoadEditingSourceViolation,
};

const FORMAT_VERSION: u32 = 1;
const MIN_SIZE_PREFIXED_LFRE_BYTES: usize = 12;
const MAX_SCHEMA_TABLE_DEPTH: usize = 5;
const APPARENT_SIZE_MULTIPLIER: usize = 16;

/// 已通过 framing、显式 verifier 上限、exact version 与外部文档身份绑定的借用 view。
///
/// 本类型保持 crate-private，不能绕过后续语义预检和共同 admission 成为编译输入。
#[derive(Debug)]
pub(crate) struct VerifiedRoadEditingSource<'a> {
    input: RoadEditingModuleInput<'a>,
    root: wire::RoadEditingSource<'a>,
    table_count: u64,
    preflight_counts: RoadEditingPreflightCounts,
}

impl<'a> VerifiedRoadEditingSource<'a> {
    pub(crate) const fn input(&self) -> RoadEditingModuleInput<'a> {
        self.input
    }

    pub(crate) const fn root(&self) -> wire::RoadEditingSource<'a> {
        self.root
    }

    pub(crate) const fn table_count(&self) -> u64 {
        self.table_count
    }

    pub(crate) const fn typed_ast_record_count(&self) -> u64 {
        // v1 只有 root 与 Provenance 不进入 Typed AST record 计数。
        self.table_count - 2
    }

    pub(crate) const fn preflight_counts(&self) -> RoadEditingPreflightCounts {
        self.preflight_counts
    }
}

/// 在任何领域对象分配前验证一份完整 size-prefixed `LFRE` buffer。
pub(crate) fn verify_source<'a>(
    input: RoadEditingModuleInput<'a>,
    limits: &CompileLimits,
    source_bytes_already_admitted: u64,
    typed_ast_records_already_admitted: u64,
) -> Result<VerifiedRoadEditingSource<'a>, DiagnosticBundle> {
    let expected_key = input.expected_source_document_key();
    let bytes = input.source_bytes();
    let source_len = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    let per_module_limit = limits.value(CompileLimitDimension::SourceBytesPerModule);
    if source_len > per_module_limit {
        return Err(limit_error(
            CompileLimitDimension::SourceBytesPerModule,
            per_module_limit,
            source_len,
        ));
    }
    let source_total_limit = limits.value(CompileLimitDimension::SourceBytesTotal);
    let source_total = source_bytes_already_admitted.saturating_add(source_len);
    if source_total > source_total_limit {
        return Err(limit_error(
            CompileLimitDimension::SourceBytesTotal,
            source_total_limit,
            source_total,
        ));
    }
    if bytes.len() < MIN_SIZE_PREFIXED_LFRE_BYTES {
        return Err(source_error(
            RoadEditingSourceViolation::TruncatedFraming,
            expected_key,
            None,
        ));
    }

    let declared_len = u32::from_le_bytes(
        bytes[..4]
            .try_into()
            .expect("minimum framing length includes size prefix"),
    );
    let actual_len = u64::try_from(bytes.len() - 4).unwrap_or(u64::MAX);
    if u64::from(declared_len) != actual_len {
        return Err(source_error(
            RoadEditingSourceViolation::SizePrefixMismatch {
                declared: u64::from(declared_len),
                actual: actual_len,
            },
            expected_key,
            None,
        ));
    }
    if !wire::road_editing_source_size_prefixed_buffer_has_identifier(bytes) {
        return Err(source_error(
            RoadEditingSourceViolation::FileIdentifierMismatch,
            expected_key,
            None,
        ));
    }

    let typed_ast_limit = limits.value(CompileLimitDimension::TypedAstRecordCount);
    let remaining_records = typed_ast_limit.saturating_sub(typed_ast_records_already_admitted);
    let max_tables_u64 = remaining_records.checked_add(2).ok_or_else(|| {
        source_error(
            RoadEditingSourceViolation::VerifierTableBudgetExceeded,
            expected_key,
            None,
        )
    })?;
    let max_tables = usize::try_from(max_tables_u64).map_err(|_| {
        source_error(
            RoadEditingSourceViolation::VerifierTableBudgetExceeded,
            expected_key,
            None,
        )
    })?;
    let max_apparent_size = bytes
        .len()
        .checked_mul(APPARENT_SIZE_MULTIPLIER)
        .ok_or_else(|| {
            source_error(
                RoadEditingSourceViolation::VerifierApparentSizeExceeded,
                expected_key,
                None,
            )
        })?;
    let options = VerifierOptions {
        max_depth: MAX_SCHEMA_TABLE_DEPTH,
        max_tables,
        max_apparent_size,
        ignore_missing_null_terminator: false,
    };
    let root = wire::size_prefixed_root_as_road_editing_source_with_opts(&options, bytes)
        .map_err(|error| verifier_error(error, limits, expected_key))?;

    if root.format_version() != FORMAT_VERSION {
        return Err(source_error(
            RoadEditingSourceViolation::UnsupportedFormatVersion {
                expected: FORMAT_VERSION,
                actual: root.format_version(),
            },
            expected_key,
            None,
        ));
    }
    let actual_key = root.module_header().source_document_key();
    if actual_key != expected_key {
        let bounded_actual_key = (u64::try_from(actual_key.len()).unwrap_or(u64::MAX)
            <= limits.value(CompileLimitDimension::SingleStringBytes))
        .then_some(actual_key);
        return Err(source_error(
            RoadEditingSourceViolation::SourceDocumentKeyMismatch,
            expected_key,
            bounded_actual_key,
        ));
    }

    let table_count = table_count(root);
    let typed_ast_record_count = table_count.saturating_sub(2);
    let total_records = typed_ast_records_already_admitted.saturating_add(typed_ast_record_count);
    if total_records > typed_ast_limit {
        return Err(limit_error(
            CompileLimitDimension::TypedAstRecordCount,
            typed_ast_limit,
            total_records,
        ));
    }

    let preflight_counts = preflight_source(root, limits, expected_key)?;
    if preflight_counts.typed_ast_record_count() != typed_ast_record_count {
        return Err(semantic_error(
            "roadEditingSource.tableAccounting",
            crate::RoadEditingInputViolation::InvalidCombination,
            expected_key,
        ));
    }

    Ok(VerifiedRoadEditingSource {
        input,
        root,
        table_count,
        preflight_counts,
    })
}

fn verifier_error(
    error: InvalidFlatbuffer,
    limits: &CompileLimits,
    expected_key: &str,
) -> DiagnosticBundle {
    match error {
        InvalidFlatbuffer::TooManyTables => limit_error(
            CompileLimitDimension::TypedAstRecordCount,
            limits.value(CompileLimitDimension::TypedAstRecordCount),
            limits
                .value(CompileLimitDimension::TypedAstRecordCount)
                .saturating_add(1),
        ),
        InvalidFlatbuffer::ApparentSizeTooLarge => source_error(
            RoadEditingSourceViolation::VerifierApparentSizeExceeded,
            expected_key,
            None,
        ),
        InvalidFlatbuffer::DepthLimitReached => source_error(
            RoadEditingSourceViolation::VerifierDepthExceeded,
            expected_key,
            None,
        ),
        InvalidFlatbuffer::MissingRequiredField { .. }
        | InvalidFlatbuffer::InconsistentUnion { .. }
        | InvalidFlatbuffer::Utf8Error { .. }
        | InvalidFlatbuffer::MissingNullTerminator { .. }
        | InvalidFlatbuffer::Unaligned { .. }
        | InvalidFlatbuffer::RangeOutOfBounds { .. }
        | InvalidFlatbuffer::SignedOffsetOutOfBounds { .. } => source_error(
            RoadEditingSourceViolation::MalformedWire,
            expected_key,
            None,
        ),
    }
}

fn source_error(
    violation: RoadEditingSourceViolation,
    expected_key: &str,
    actual_key: Option<&str>,
) -> DiagnosticBundle {
    DiagnosticBundle::single(Diagnostic::invalid_road_editing_source(
        violation,
        None,
        expected_key,
        actual_key,
    ))
}

fn semantic_error(
    field: &'static str,
    violation: crate::RoadEditingInputViolation,
    expected_key: &str,
) -> DiagnosticBundle {
    DiagnosticBundle::single(Diagnostic::invalid_road_editing_source(
        RoadEditingSourceViolation::InvalidSemanticValue(violation),
        Some(field),
        expected_key,
        Some(expected_key),
    ))
}

fn limit_error(dimension: CompileLimitDimension, limit: u64, observed: u64) -> DiagnosticBundle {
    DiagnosticBundle::single(Diagnostic::compile_limit_exceeded(
        dimension, limit, observed,
    ))
}

fn table_count(root: wire::RoadEditingSource<'_>) -> u64 {
    let mut count = 3_u64; // root + ModuleHeader + Provenance
    for value in root.road_alignments() {
        count = count.saturating_add(1 + curve_program_table_count(value.reference_line()));
    }
    for value in root.road_corridors() {
        count = count.saturating_add(1 + len_u64(value.elements()));
    }
    count = count.saturating_add(len_u64(root.road_sections()));
    count = count.saturating_add(len_u64(root.authoring_lanes()));
    for value in root.lane_edges() {
        count = count.saturating_add(1);
        if let Some(program) = value.explicit_geometry() {
            count = count.saturating_add(curve_program_table_count(program));
        }
    }
    count = count.saturating_add(len_u64(root.junctions()));
    count = count.saturating_add(len_u64(root.movements()));
    count = count.saturating_add(len_u64(root.maneuver_paths()));
    count = count.saturating_add(len_u64(root.maneuver_gates()));
    count = count.saturating_add(len_u64(root.waiting_zones()));
    count = count.saturating_add(len_u64(root.stop_lines()));
    count = count.saturating_add(len_u64(root.signal_groups()));
    count = count.saturating_add(len_u64(root.signal_controllers()));
    for value in root.signal_phases() {
        count = count.saturating_add(1 + len_u64(value.states()));
    }
    count = count.saturating_add(len_u64(root.parking_areas()));
    count = count.saturating_add(len_u64(root.parking_spaces()).saturating_mul(4));
    count = count.saturating_add(len_u64(root.lane_groups()));
    count = count.saturating_add(len_u64(root.facility_bands()));
    count = count.saturating_add(len_u64(root.participant_classes()));
    for value in root.access_rules() {
        count = count.saturating_add(1 + u64::from(value.regulation().is_some()));
    }
    count = count.saturating_add(len_u64(root.vehicle_profiles()).saturating_mul(2));
    count = count.saturating_add(len_u64(root.static_routes()));
    count.saturating_add(len_u64(root.canonical_frames()))
}

fn curve_program_table_count(program: wire::CurveProgram<'_>) -> u64 {
    1_u64.saturating_add(len_u64(program.segments()).saturating_mul(2))
}

fn len_u64<T>(values: laneflow_road_editing_wire::runtime::Vector<'_, T>) -> u64 {
    u64::try_from(values.len()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::road_editing::{
        CanonicalFrameInput, CanonicalFrameReference, RoadAlignmentInput, RoadEditingCurveProgram,
        RoadEditingCurveSegment, RoadEditingDeclaration, RoadEditingModuleHeader,
        RoadEditingPoint3, RoadEditingProvenance, RoadEditingSourceModuleBuilder,
        RoadEditingSourceWriter,
    };
    use crate::{
        DiagnosticCode, DiagnosticPayload, GeometryAccuracyProfile, GeometryDirectionProfile,
    };

    fn source_buffer(
        limits: &CompileLimits,
        source_document_key: &str,
    ) -> super::super::OwnedRoadEditingSourceBuffer {
        let header = RoadEditingModuleHeader::try_new(
            "city",
            source_document_key,
            Vec::new(),
            RoadEditingProvenance::direct("editor save").expect("provenance"),
        )
        .expect("header");
        let mut builder = RoadEditingSourceModuleBuilder::new(
            header,
            GeometryAccuracyProfile::Balanced5Cm,
            GeometryDirectionProfile::Balanced2Deg,
            limits,
        )
        .expect("builder");
        builder
            .add_declaration(RoadEditingDeclaration::CanonicalFrame(
                CanonicalFrameInput::try_new("frame").expect("frame"),
            ))
            .expect("frame declaration");
        RoadEditingSourceWriter::new(limits)
            .write(builder.finish().expect("module"))
            .expect("buffer")
    }

    fn source_buffer_with_imported_frame(
        limits: &CompileLimits,
    ) -> super::super::OwnedRoadEditingSourceBuffer {
        let header = RoadEditingModuleHeader::try_new(
            "city",
            "roads/main",
            vec!["base".into()],
            RoadEditingProvenance::direct("editor save").expect("provenance"),
        )
        .expect("header");
        let mut builder = RoadEditingSourceModuleBuilder::new(
            header,
            GeometryAccuracyProfile::Balanced5Cm,
            GeometryDirectionProfile::Balanced2Deg,
            limits,
        )
        .expect("builder");
        let curve = RoadEditingCurveProgram::try_new(
            RoadEditingPoint3::try_new(0.0, 0.0, 0.0).expect("start"),
            vec![RoadEditingCurveSegment::line(
                RoadEditingPoint3::try_new(10.0, 0.0, 0.0).expect("end"),
            )],
        )
        .expect("curve");
        builder
            .add_alignment(
                RoadAlignmentInput::try_new(
                    "alignment",
                    CanonicalFrameReference::imported("base", Vec::new(), "frame")
                        .expect("imported frame"),
                    curve,
                )
                .expect("alignment"),
            )
            .expect("add alignment");
        RoadEditingSourceWriter::new(limits)
            .write(builder.finish().expect("module"))
            .expect("buffer")
    }

    fn first_diagnostic(error: &DiagnosticBundle) -> &crate::Diagnostic {
        error.diagnostics().first().expect("diagnostic")
    }

    fn overwrite_format_version(bytes: &mut [u8], version: u32) {
        let root_offset = u32::from_le_bytes(bytes[4..8].try_into().expect("root offset"));
        let root_position = 4_usize + usize::try_from(root_offset).expect("root position");
        let vtable_distance = i32::from_le_bytes(
            bytes[root_position..root_position + 4]
                .try_into()
                .expect("vtable offset"),
        );
        let vtable_position = root_position
            .checked_sub(usize::try_from(vtable_distance).expect("positive vtable distance"))
            .expect("vtable position");
        let field_offset = u16::from_le_bytes(
            bytes[vtable_position + 4..vtable_position + 6]
                .try_into()
                .expect("format field offset"),
        );
        let field_position = root_position + usize::from(field_offset);
        bytes[field_position..field_position + 4].copy_from_slice(&version.to_le_bytes());
    }

    fn overwrite_root_u8_field(bytes: &mut [u8], field_id: usize, value: u8) {
        let root_offset = u32::from_le_bytes(bytes[4..8].try_into().expect("root offset"));
        let root_position = 4_usize + usize::try_from(root_offset).expect("root position");
        let vtable_distance = i32::from_le_bytes(
            bytes[root_position..root_position + 4]
                .try_into()
                .expect("vtable offset"),
        );
        let vtable_position = root_position
            .checked_sub(usize::try_from(vtable_distance).expect("positive vtable distance"))
            .expect("vtable position");
        let entry = vtable_position + 4 + field_id * 2;
        let field_offset = u16::from_le_bytes(bytes[entry..entry + 2].try_into().expect("field"));
        assert_ne!(field_offset, 0, "test field must be present");
        bytes[root_position + usize::from(field_offset)] = value;
    }

    #[test]
    fn verifies_writer_output_with_explicit_limits_and_exact_table_count() {
        let limits = CompileLimits::p100_initial_v1();
        let buffer = source_buffer(&limits, "roads/main");
        let input =
            RoadEditingModuleInput::try_new("roads/main", buffer.as_bytes(), None).expect("input");

        let verified = verify_source(input, &limits, 0, 0).expect("verified source");

        assert_eq!(
            verified.input().expected_source_document_key(),
            "roads/main"
        );
        assert_eq!(
            verified.root().module_header().authoring_namespace_id(),
            "city"
        );
        assert_eq!(verified.table_count(), 4);
        assert_eq!(verified.typed_ast_record_count(), 2);
        assert_eq!(verified.preflight_counts().typed_ast_record_count(), 2);
    }

    #[test]
    fn semantic_preflight_accepts_every_first_party_declaration_shape() {
        let limits = CompileLimits::p100_initial_v1();
        let buffer = RoadEditingSourceWriter::new(&limits)
            .write(super::super::writer::tests::module_with_every_declaration(
                &limits,
            ))
            .expect("all declarations buffer");
        let input = RoadEditingModuleInput::try_new("road-editing", buffer.as_bytes(), None)
            .expect("input");

        let verified = verify_source(input, &limits, 0, 0).expect("semantic preflight");

        assert_eq!(
            verified.preflight_counts().typed_ast_record_count(),
            verified.typed_ast_record_count()
        );
    }

    #[test]
    fn semantic_preflight_rejects_unspecified_profile_after_wire_verification() {
        let limits = CompileLimits::p100_initial_v1();
        let buffer = source_buffer(&limits, "roads/main");
        let mut bytes = buffer.as_bytes().to_vec();
        overwrite_root_u8_field(&mut bytes, 2, 0);
        let input = RoadEditingModuleInput::try_new("roads/main", &bytes, None).expect("input");

        let error = verify_source(input, &limits, 0, 0).expect_err("unspecified profile");

        assert!(matches!(
            first_diagnostic(&error).payload(),
            DiagnosticPayload::InvalidRoadEditingSource {
                violation: RoadEditingSourceViolation::InvalidSemanticValue(
                    crate::RoadEditingInputViolation::InvalidCombination
                ),
                field: Some(field),
                ..
            } if field.as_ref() == "roadEditingSource.geometryAccuracyProfile"
        ));
    }

    #[test]
    fn semantic_preflight_applies_exact_string_item_budget() {
        let normal_limits = CompileLimits::p100_initial_v1();
        let buffer = source_buffer(&normal_limits, "roads/main");
        let input =
            RoadEditingModuleInput::try_new("roads/main", buffer.as_bytes(), None).expect("input");
        let verified = verify_source(input, &normal_limits, 0, 0).expect("normal limits");
        let observed = verified.preflight_counts().string_item_count();
        let limits = normal_limits.with_test_admission_limit(
            CompileLimitDimension::StringItemCount,
            u32::try_from(observed - 1).expect("small fixture"),
        );

        let error = verify_source(input, &limits, 0, 0).expect_err("string item budget");

        assert!(matches!(
            first_diagnostic(&error).payload(),
            DiagnosticPayload::CompileLimitExceeded {
                dimension: CompileLimitDimension::StringItemCount,
                limit,
                observed: actual,
            } if *limit == observed - 1 && *actual == observed
        ));
    }

    #[test]
    fn rejects_prefix_mismatch_before_flatbuffers_verifier() {
        let limits = CompileLimits::p100_initial_v1();
        let buffer = source_buffer(&limits, "roads/main");
        let mut bytes = buffer.as_bytes().to_vec();
        bytes[0] ^= 1;
        let input = RoadEditingModuleInput::try_new("roads/main", &bytes, None).expect("input");

        let error = verify_source(input, &limits, 0, 0).expect_err("prefix mismatch");

        assert!(matches!(
            first_diagnostic(&error).payload(),
            DiagnosticPayload::InvalidRoadEditingSource {
                violation: RoadEditingSourceViolation::SizePrefixMismatch { .. },
                ..
            }
        ));
    }

    #[test]
    fn rejects_file_identifier_before_verifier() {
        let limits = CompileLimits::p100_initial_v1();
        let buffer = source_buffer(&limits, "roads/main");
        let mut bytes = buffer.as_bytes().to_vec();
        bytes[8..12].copy_from_slice(b"NOPE");
        let input = RoadEditingModuleInput::try_new("roads/main", &bytes, None).expect("input");

        let error = verify_source(input, &limits, 0, 0).expect_err("identifier mismatch");

        assert_eq!(
            first_diagnostic(&error).code(),
            DiagnosticCode::InvalidRoadEditingSource
        );
        assert!(matches!(
            first_diagnostic(&error).payload(),
            DiagnosticPayload::InvalidRoadEditingSource {
                violation: RoadEditingSourceViolation::FileIdentifierMismatch,
                ..
            }
        ));
    }

    #[test]
    fn rejects_structural_corruption_after_identifier_check() {
        let limits = CompileLimits::p100_initial_v1();
        let buffer = source_buffer(&limits, "roads/main");
        let mut bytes = buffer.as_bytes().to_vec();
        bytes[4..8].copy_from_slice(&u32::MAX.to_le_bytes());
        let input = RoadEditingModuleInput::try_new("roads/main", &bytes, None).expect("input");

        let error = verify_source(input, &limits, 0, 0).expect_err("malformed root offset");

        assert!(matches!(
            first_diagnostic(&error).payload(),
            DiagnosticPayload::InvalidRoadEditingSource {
                violation: RoadEditingSourceViolation::MalformedWire,
                ..
            }
        ));
    }

    #[test]
    fn rejects_unknown_format_version_after_verification() {
        let limits = CompileLimits::p100_initial_v1();
        let buffer = source_buffer(&limits, "roads/main");
        let mut bytes = buffer.as_bytes().to_vec();
        overwrite_format_version(&mut bytes, 2);
        let input = RoadEditingModuleInput::try_new("roads/main", &bytes, None).expect("input");

        let error = verify_source(input, &limits, 0, 0).expect_err("unknown format version");

        assert!(matches!(
            first_diagnostic(&error).payload(),
            DiagnosticPayload::InvalidRoadEditingSource {
                violation: RoadEditingSourceViolation::UnsupportedFormatVersion {
                    expected: 1,
                    actual: 2
                },
                ..
            }
        ));
    }

    #[test]
    fn binds_verified_document_key_to_external_expected_identity() {
        let limits = CompileLimits::p100_initial_v1();
        let buffer = source_buffer(&limits, "roads/actual");
        let input = RoadEditingModuleInput::try_new("roads/expected", buffer.as_bytes(), None)
            .expect("input");

        let error = verify_source(input, &limits, 0, 0).expect_err("document mismatch");

        assert!(matches!(
            first_diagnostic(&error).payload(),
            DiagnosticPayload::InvalidRoadEditingSource {
                violation: RoadEditingSourceViolation::SourceDocumentKeyMismatch,
                actual_source_document_key: Some(actual),
                ..
            } if actual.as_ref() == "roads/actual"
        ));
    }

    #[test]
    fn document_key_mismatch_does_not_copy_an_unbounded_wire_key() {
        let normal_limits = CompileLimits::p100_initial_v1();
        let actual_key = "a".repeat(53);
        let buffer = source_buffer(&normal_limits, &actual_key);
        let limits = normal_limits.with_test_single_string_limit(8);
        let input =
            RoadEditingModuleInput::try_new("roads/x", buffer.as_bytes(), None).expect("input");

        let error = verify_source(input, &limits, 0, 0).expect_err("document mismatch");

        assert!(matches!(
            first_diagnostic(&error).payload(),
            DiagnosticPayload::InvalidRoadEditingSource {
                violation: RoadEditingSourceViolation::SourceDocumentKeyMismatch,
                actual_source_document_key: None,
                ..
            }
        ));
        assert!(!error.to_string().contains(&actual_key));
    }

    #[test]
    fn semantic_preflight_rejects_a_self_qualified_reference() {
        let limits = CompileLimits::p100_initial_v1();
        let buffer = source_buffer_with_imported_frame(&limits);
        let mut bytes = buffer.as_bytes().to_vec();
        let valid = b"base::frame";
        let invalid = b"city::frame";
        let positions = bytes
            .windows(valid.len())
            .enumerate()
            .filter_map(|(index, value)| (value == valid).then_some(index))
            .collect::<Vec<_>>();
        assert_eq!(positions.len(), 1, "one imported reference spelling");
        bytes[positions[0]..positions[0] + invalid.len()].copy_from_slice(invalid);
        let input = RoadEditingModuleInput::try_new("roads/main", &bytes, None).expect("input");

        let error = verify_source(input, &limits, 0, 0).expect_err("self-qualified reference");

        assert!(matches!(
            first_diagnostic(&error).payload(),
            DiagnosticPayload::InvalidRoadEditingSource {
                violation: RoadEditingSourceViolation::InvalidSemanticValue(
                    crate::RoadEditingInputViolation::InvalidCombination
                ),
                field: Some(field),
                ..
            } if field.as_ref() == "roadAlignment.canonicalFrame"
        ));
    }

    #[test]
    fn verifier_table_limit_maps_to_typed_ast_resource_dimension() {
        let normal_limits = CompileLimits::p100_initial_v1();
        let buffer = source_buffer(&normal_limits, "roads/main");
        let limits =
            normal_limits.with_test_admission_limit(CompileLimitDimension::TypedAstRecordCount, 1);
        let input =
            RoadEditingModuleInput::try_new("roads/main", buffer.as_bytes(), None).expect("input");

        let error = verify_source(input, &limits, 0, 0).expect_err("table budget");

        assert!(matches!(
            first_diagnostic(&error).payload(),
            DiagnosticPayload::CompileLimitExceeded {
                dimension: CompileLimitDimension::TypedAstRecordCount,
                ..
            }
        ));
    }

    #[test]
    fn source_byte_limits_fail_before_verifier() {
        let normal_limits = CompileLimits::p100_initial_v1();
        let buffer = source_buffer(&normal_limits, "roads/main");
        let source_len = u32::try_from(buffer.as_bytes().len()).expect("portable source len");
        let input =
            RoadEditingModuleInput::try_new("roads/main", buffer.as_bytes(), None).expect("input");

        let per_module_limits = normal_limits.clone().with_test_source_byte_limits(
            source_len - 1,
            u32::try_from(normal_limits.value(CompileLimitDimension::SourceBytesTotal))
                .expect("configured total limit"),
        );
        let error = verify_source(input, &per_module_limits, 0, 0).expect_err("per-module limit");
        assert!(matches!(
            first_diagnostic(&error).payload(),
            DiagnosticPayload::CompileLimitExceeded {
                dimension: CompileLimitDimension::SourceBytesPerModule,
                ..
            }
        ));

        let total_limits = normal_limits.with_test_source_byte_limits(source_len, source_len);
        let error = verify_source(input, &total_limits, 1, 0).expect_err("total limit");
        assert!(matches!(
            first_diagnostic(&error).payload(),
            DiagnosticPayload::CompileLimitExceeded {
                dimension: CompileLimitDimension::SourceBytesTotal,
                ..
            }
        ));
    }
}
