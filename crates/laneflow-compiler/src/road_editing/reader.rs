use laneflow_road_editing_wire::generated::lane_flow::road_editing::v1 as wire;
use laneflow_road_editing_wire::runtime::{ErrorTraceDetail, InvalidFlatbuffer, VerifierOptions};

use super::RoadEditingModuleInput;
use super::location::RoadEditingLocationFactory;
use super::preflight::{RoadEditingPreflightCounts, preflight_source};
use crate::{
    CompileLimitDimension, CompileLimits, Diagnostic, DiagnosticBundle, RoadEditingByteRange,
    RoadEditingRootVectorKind, RoadEditingSourceViolation, RoadEditingTableKind, SourceLocation,
};

const FORMAT_VERSION: u32 = 4;
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
    preflight_counts: RoadEditingPreflightCounts,
}

impl<'a> VerifiedRoadEditingSource<'a> {
    pub(crate) const fn input(&self) -> RoadEditingModuleInput<'a> {
        self.input
    }

    pub(crate) const fn root(&self) -> wire::RoadEditingSource<'a> {
        self.root
    }

    #[cfg(test)]
    pub(crate) fn table_count(&self) -> u64 {
        table_count(self.root)
    }

    pub(crate) const fn typed_ast_record_count(&self) -> u64 {
        self.preflight_counts.typed_ast_record_count()
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
            expected_key,
        ));
    }
    let source_total_limit = limits.value(CompileLimitDimension::SourceBytesTotal);
    let source_total = source_bytes_already_admitted.saturating_add(source_len);
    if source_total > source_total_limit {
        return Err(limit_error(
            CompileLimitDimension::SourceBytesTotal,
            source_total_limit,
            source_total,
            expected_key,
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
    // 版本探针只读受界定的 root 标量。旧版本不要求携带 v4 新增的 required vector。
    if let Some(actual) = probe_format_version(bytes)
        && actual != FORMAT_VERSION
    {
        return Err(source_error(
            RoadEditingSourceViolation::UnsupportedFormatVersion {
                expected: FORMAT_VERSION,
                actual,
            },
            expected_key,
            None,
        ));
    }
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
        .map_err(|error| verifier_error(error, limits, expected_key, bytes.len()))?;

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
    // 物理 table 与 Typed AST 来源记录是两个口径；方向标量产生一个额外来源记录。
    let directions = root
        .movements()
        .iter()
        .filter(|movement| movement.turn_direction().is_some())
        .count() as u64;
    let typed_ast_record_count = table_count.saturating_sub(2).saturating_add(directions);
    let total_records = typed_ast_records_already_admitted.saturating_add(typed_ast_record_count);
    if total_records > typed_ast_limit {
        return Err(limit_error(
            CompileLimitDimension::TypedAstRecordCount,
            typed_ast_limit,
            total_records,
            expected_key,
        ));
    }

    let verified_header_location = || {
        RoadEditingLocationFactory::verified_module_header(
            root.module_header().authoring_namespace_id(),
            expected_key,
        )
    };
    let preflight_counts = preflight_source(root, limits, expected_key)
        .map_err(|bundle| bundle.with_fallback_primary_location(verified_header_location()))?;
    if preflight_counts.typed_ast_record_count() != typed_ast_record_count {
        return Err(semantic_error(
            "roadEditingSource.recordAccounting",
            crate::RoadEditingInputViolation::InvalidCombination,
            expected_key,
        )
        .with_fallback_primary_location(verified_header_location()));
    }

    Ok(VerifiedRoadEditingSource {
        input,
        root,
        preflight_counts,
    })
}

fn verifier_error(
    error: InvalidFlatbuffer,
    limits: &CompileLimits,
    expected_key: &str,
    source_len: usize,
) -> DiagnosticBundle {
    match error {
        InvalidFlatbuffer::TooManyTables => limit_error(
            CompileLimitDimension::TypedAstRecordCount,
            limits.value(CompileLimitDimension::TypedAstRecordCount),
            limits
                .value(CompileLimitDimension::TypedAstRecordCount)
                .saturating_add(1),
            expected_key,
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
        data_error => {
            let (trace, range) = verifier_data_site(&data_error, source_len);
            let field = trace.iter().find_map(|detail| match detail {
                ErrorTraceDetail::TableField { field_name, .. } => Some(field_name.as_ref()),
                _ => None,
            });
            let location = wire_location_from_trace(trace, expected_key, range);
            source_error_at(
                RoadEditingSourceViolation::MalformedWire,
                field,
                expected_key,
                None,
                location,
            )
        }
    }
}

fn verifier_data_site(
    error: &InvalidFlatbuffer,
    source_len: usize,
) -> (&[ErrorTraceDetail], Option<RoadEditingByteRange>) {
    let (trace, start, length) = match error {
        InvalidFlatbuffer::MissingRequiredField { error_trace, .. }
        | InvalidFlatbuffer::InconsistentUnion { error_trace, .. } => (error_trace, None, None),
        InvalidFlatbuffer::Utf8Error {
            range, error_trace, ..
        }
        | InvalidFlatbuffer::MissingNullTerminator { range, error_trace }
        | InvalidFlatbuffer::RangeOutOfBounds { range, error_trace } => (
            error_trace,
            u32::try_from(range.start).ok(),
            u32::try_from(range.end.saturating_sub(range.start)).ok(),
        ),
        InvalidFlatbuffer::Unaligned {
            position,
            error_trace,
            ..
        }
        | InvalidFlatbuffer::SignedOffsetOutOfBounds {
            position,
            error_trace,
            ..
        } => (error_trace, u32::try_from(*position).ok(), Some(1)),
        InvalidFlatbuffer::TooManyTables
        | InvalidFlatbuffer::ApparentSizeTooLarge
        | InvalidFlatbuffer::DepthLimitReached => {
            unreachable!("DoS verifier errors are handled before data-site extraction")
        }
    };
    let direct = start
        .zip(length)
        .and_then(|(start, length)| RoadEditingByteRange::checked(start, length, source_len));
    let traced = trace.as_ref().iter().find_map(|detail| {
        let position = match detail {
            ErrorTraceDetail::VectorElement { position, .. }
            | ErrorTraceDetail::TableField { position, .. }
            | ErrorTraceDetail::UnionVariant { position, .. } => *position,
        };
        RoadEditingByteRange::checked(u32::try_from(position).ok()?, 1, source_len)
    });
    (trace.as_ref(), direct.or(traced))
}

fn wire_location_from_trace(
    details: &[ErrorTraceDetail],
    expected_key: &str,
    range: Option<RoadEditingByteRange>,
) -> Option<SourceLocation> {
    for (field_position, detail) in details.iter().enumerate() {
        let ErrorTraceDetail::TableField { field_name, .. } = detail else {
            continue;
        };
        let Some((root_vector, table)) = root_vector_site(field_name.as_ref()) else {
            continue;
        };
        let physical_index =
            details[..field_position]
                .iter()
                .rev()
                .find_map(|detail| match detail {
                    ErrorTraceDetail::VectorElement { index, .. } => u32::try_from(*index).ok(),
                    _ => None,
                })?;
        return Some(RoadEditingLocationFactory::input_wire(
            expected_key,
            root_vector,
            physical_index,
            table,
            range,
        ));
    }
    None
}

fn root_vector_site(field_name: &str) -> Option<(RoadEditingRootVectorKind, RoadEditingTableKind)> {
    Some(match field_name {
        "right_of_way_policy_sets" => (
            RoadEditingRootVectorKind::RightOfWayPolicySet,
            RoadEditingTableKind::RightOfWayPolicySet,
        ),
        "road_alignments" => (
            RoadEditingRootVectorKind::RoadAlignment,
            RoadEditingTableKind::RoadAlignment,
        ),
        "road_corridors" => (
            RoadEditingRootVectorKind::RoadCorridor,
            RoadEditingTableKind::RoadCorridor,
        ),
        "road_sections" => (
            RoadEditingRootVectorKind::RoadSection,
            RoadEditingTableKind::RoadSection,
        ),
        "authoring_lanes" => (
            RoadEditingRootVectorKind::AuthoringLane,
            RoadEditingTableKind::AuthoringLane,
        ),
        "lane_edges" => (
            RoadEditingRootVectorKind::LaneEdge,
            RoadEditingTableKind::LaneEdge,
        ),
        "junctions" => (
            RoadEditingRootVectorKind::Junction,
            RoadEditingTableKind::Junction,
        ),
        "movements" => (
            RoadEditingRootVectorKind::Movement,
            RoadEditingTableKind::Movement,
        ),
        "maneuver_paths" => (
            RoadEditingRootVectorKind::ManeuverPath,
            RoadEditingTableKind::ManeuverPath,
        ),
        "maneuver_gates" => (
            RoadEditingRootVectorKind::ManeuverGate,
            RoadEditingTableKind::ManeuverGate,
        ),
        "waiting_zones" => (
            RoadEditingRootVectorKind::WaitingZone,
            RoadEditingTableKind::WaitingZone,
        ),
        "stop_lines" => (
            RoadEditingRootVectorKind::StopLine,
            RoadEditingTableKind::StopLine,
        ),
        "signal_groups" => (
            RoadEditingRootVectorKind::SignalGroup,
            RoadEditingTableKind::SignalGroup,
        ),
        "signal_controllers" => (
            RoadEditingRootVectorKind::SignalController,
            RoadEditingTableKind::SignalController,
        ),
        "signal_phases" => (
            RoadEditingRootVectorKind::SignalPhase,
            RoadEditingTableKind::SignalPhase,
        ),
        "parking_facilities" => (
            RoadEditingRootVectorKind::ParkingFacility,
            RoadEditingTableKind::ParkingFacility,
        ),
        "parking_spaces" => (
            RoadEditingRootVectorKind::ParkingSpace,
            RoadEditingTableKind::ParkingSpace,
        ),
        "lane_groups" => (
            RoadEditingRootVectorKind::LaneGroup,
            RoadEditingTableKind::LaneGroup,
        ),
        "facility_bands" => (
            RoadEditingRootVectorKind::FacilityBand,
            RoadEditingTableKind::FacilityBand,
        ),
        "participant_classes" => (
            RoadEditingRootVectorKind::ParticipantClass,
            RoadEditingTableKind::ParticipantClass,
        ),
        "access_rules" => (
            RoadEditingRootVectorKind::AccessRule,
            RoadEditingTableKind::AccessRule,
        ),
        "vehicle_profiles" => (
            RoadEditingRootVectorKind::VehicleProfile,
            RoadEditingTableKind::VehicleProfile,
        ),
        "canonical_frames" => (
            RoadEditingRootVectorKind::CanonicalFrame,
            RoadEditingTableKind::CanonicalFrame,
        ),
        _ => return None,
    })
}

fn source_error(
    violation: RoadEditingSourceViolation,
    expected_key: &str,
    actual_key: Option<&str>,
) -> DiagnosticBundle {
    source_error_at(
        violation,
        None,
        expected_key,
        actual_key,
        Some(RoadEditingLocationFactory::input_module_header(
            expected_key,
        )),
    )
}

fn source_error_at(
    violation: RoadEditingSourceViolation,
    field: Option<&str>,
    expected_key: &str,
    actual_key: Option<&str>,
    location: Option<SourceLocation>,
) -> DiagnosticBundle {
    DiagnosticBundle::single(Diagnostic::invalid_road_editing_source_at(
        violation,
        field,
        expected_key,
        actual_key,
        location.or_else(|| {
            Some(RoadEditingLocationFactory::input_module_header(
                expected_key,
            ))
        }),
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

fn limit_error(
    dimension: CompileLimitDimension,
    limit: u64,
    observed: u64,
    expected_key: &str,
) -> DiagnosticBundle {
    DiagnosticBundle::single(Diagnostic::compile_limit_exceeded_at(
        dimension,
        limit,
        observed,
        Some(RoadEditingLocationFactory::input_module_header(
            expected_key,
        )),
        None,
    ))
}

fn table_count(root: wire::RoadEditingSource<'_>) -> u64 {
    let mut count = 3_u64; // root + ModuleHeader + Provenance
    for policy in root.right_of_way_policy_sets() {
        count = count
            .saturating_add(2)
            .saturating_add(len_u64(policy.evidence()))
            .saturating_add(len_u64(policy.gap_profiles()))
            .saturating_add(len_u64(policy.stream_rules()))
            .saturating_add(len_u64(policy.gate_rules()));
    }
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
    for value in root.parking_facilities() {
        count = count
            .saturating_add(1)
            .saturating_add(len_u64(value.virtual_entries()))
            .saturating_add(len_u64(value.virtual_exits()));
    }
    count = count.saturating_add(len_u64(root.parking_spaces()).saturating_mul(4));
    count = count.saturating_add(len_u64(root.lane_groups()));
    count = count.saturating_add(len_u64(root.facility_bands()));
    count = count.saturating_add(len_u64(root.participant_classes()));
    for value in root.access_rules() {
        count = count.saturating_add(1 + u64::from(value.regulation().is_some()));
    }
    count = count.saturating_add(len_u64(root.vehicle_profiles()).saturating_mul(2));
    count = count.saturating_add(len_u64(root.canonical_frames()));
    count = count.saturating_add(len_u64(root.conflict_zones()));
    for value in root.participant_streams() {
        count = count
            .saturating_add(1)
            .saturating_add(len_u64(value.passages()).saturating_mul(3));
    }
    count.saturating_add(len_u64(root.conflict_zone_regions()))
}

fn curve_program_table_count(program: wire::CurveProgram<'_>) -> u64 {
    1_u64.saturating_add(len_u64(program.segments()).saturating_mul(2))
}

fn probe_format_version(bytes: &[u8]) -> Option<u32> {
    fn u16_at(bytes: &[u8], at: usize) -> Option<u16> {
        Some(u16::from_le_bytes(
            bytes.get(at..at.checked_add(2)?)?.try_into().ok()?,
        ))
    }
    fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
        Some(u32::from_le_bytes(
            bytes.get(at..at.checked_add(4)?)?.try_into().ok()?,
        ))
    }
    let table = 4_usize.checked_add(usize::try_from(u32_at(bytes, 4)?).ok()?)?;
    let offset = i32::from_le_bytes(bytes.get(table..table.checked_add(4)?)?.try_into().ok()?);
    let vtable =
        usize::try_from(i64::try_from(table).ok()?.checked_sub(i64::from(offset))?).ok()?;
    let vtable_len = usize::from(u16_at(bytes, vtable)?);
    bytes.get(vtable..vtable.checked_add(vtable_len)?)?;
    if vtable_len < 4 {
        return None;
    }
    if vtable_len < 6 {
        return Some(0);
    }
    let field = usize::from(u16_at(bytes, vtable.checked_add(4)?)?);
    if field == 0 {
        return Some(0);
    }
    let table_len = usize::from(u16_at(bytes, vtable.checked_add(2)?)?);
    if field < 4 || field.checked_add(4)? > table_len {
        return None;
    }
    u32_at(bytes, table.checked_add(field)?)
}

fn len_u64<T>(values: laneflow_road_editing_wire::runtime::Vector<'_, T>) -> u64 {
    u64::try_from(values.len()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::road_editing::{
        CanonicalFrameInput, CanonicalFrameReference, LaneEdgeInput, LaneEdgeReference,
        ParkingFacilityInput, RoadAlignmentInput, RoadEditingCurveProgram, RoadEditingCurveSegment,
        RoadEditingDeclaration, RoadEditingModuleHeader, RoadEditingPoint3, RoadEditingProvenance,
        RoadEditingSignalPhaseState, RoadEditingSourceModuleBuilder, RoadEditingSourceWriter,
        SignalControllerInput, SignalControllerReference, SignalGroupInput, SignalGroupReference,
        SignalPhaseInput,
    };
    use crate::{
        DiagnosticCode, DiagnosticPayload, GeometryAccuracyProfile, GeometryDirectionProfile,
        RoadEditingDocumentIdentity, RoadEditingRootVectorKind, RoadEditingSubject,
    };
    use laneflow_road_editing_wire::runtime::{self, ForwardsUOffset, WIPOffset};

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

    fn source_buffer_with_lane_successors(
        limits: &CompileLimits,
    ) -> super::super::OwnedRoadEditingSourceBuffer {
        let header = RoadEditingModuleHeader::try_new(
            "city",
            "roads/main",
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
            .add_declaration(RoadEditingDeclaration::LaneEdge(
                LaneEdgeInput::try_new(
                    "edge-a",
                    10.0,
                    vec![
                        LaneEdgeReference::local("edge-b").expect("successor"),
                        LaneEdgeReference::local("edge-c").expect("successor"),
                    ],
                    None,
                )
                .expect("edge"),
            ))
            .expect("edge declaration");
        for key in ["edge-b", "edge-c"] {
            builder
                .add_declaration(RoadEditingDeclaration::LaneEdge(
                    LaneEdgeInput::try_new(key, 10.0, Vec::new(), None).expect("edge"),
                ))
                .expect("edge declaration");
        }
        RoadEditingSourceWriter::new(limits)
            .write(builder.finish().expect("module"))
            .expect("buffer")
    }

    fn signal_source_buffer(limits: &CompileLimits) -> super::super::OwnedRoadEditingSourceBuffer {
        let header = RoadEditingModuleHeader::try_new(
            "city",
            "road-editing",
            vec!["signal".to_owned()],
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
        let group = SignalGroupReference::local("signal-group").expect("group");
        let controller = SignalControllerReference::local("controller").expect("controller");
        builder
            .add_declaration(RoadEditingDeclaration::SignalGroup(
                SignalGroupInput::try_new("signal-group").expect("group"),
            ))
            .expect("group declaration");
        builder
            .add_declaration(RoadEditingDeclaration::SignalController(
                SignalControllerInput::try_new(
                    "controller",
                    0,
                    vec![group.clone()],
                    vec![
                        super::super::SignalPhaseReference::owner_scoped(
                            vec!["controller".into()],
                            "phase",
                        )
                        .expect("phase reference"),
                    ],
                )
                .expect("controller"),
            ))
            .expect("controller declaration");
        builder
            .add_declaration(RoadEditingDeclaration::SignalPhase(
                SignalPhaseInput::try_new(
                    "phase",
                    1_000,
                    vec![
                        RoadEditingSignalPhaseState::try_new(
                            group,
                            laneflow_static_contract::SignalAspect::Green,
                        )
                        .expect("phase state"),
                    ],
                    controller,
                )
                .expect("phase"),
            ))
            .expect("phase declaration");
        RoadEditingSourceWriter::new(limits)
            .write(builder.finish().expect("module"))
            .expect("buffer")
    }

    fn parking_source_buffer(limits: &CompileLimits) -> super::super::OwnedRoadEditingSourceBuffer {
        let header = RoadEditingModuleHeader::try_new(
            "city",
            "road-editing",
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
        for key in ["facility-a", "facility-b"] {
            builder
                .add_declaration(RoadEditingDeclaration::ParkingFacility(
                    ParkingFacilityInput::try_new(key).expect("parking facility"),
                ))
                .expect("parking facility declaration");
        }
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

    fn overwrite_table_string_field(
        bytes: &mut [u8],
        table_position: usize,
        vtable_field: u16,
        replacement: &[u8],
    ) {
        let vtable_distance = i32::from_le_bytes(
            bytes[table_position..table_position + 4]
                .try_into()
                .expect("vtable offset"),
        );
        let vtable_is_before_table = vtable_distance.is_positive();
        let vtable_distance = usize::try_from(vtable_distance.unsigned_abs())
            .expect("u32 vtable distance fits usize on supported targets");
        let vtable_position = if vtable_is_before_table {
            table_position
                .checked_sub(vtable_distance)
                .expect("vtable position")
        } else {
            table_position
                .checked_add(vtable_distance)
                .expect("vtable position")
        };
        let entry = vtable_position + usize::from(vtable_field);
        let field_offset = u16::from_le_bytes(bytes[entry..entry + 2].try_into().expect("field"));
        assert_ne!(field_offset, 0, "test field must be present");
        let field_position = table_position + usize::from(field_offset);
        let string_offset = u32::from_le_bytes(
            bytes[field_position..field_position + 4]
                .try_into()
                .expect("string offset"),
        );
        let string_position = field_position + usize::try_from(string_offset).expect("string");
        let string_length = u32::from_le_bytes(
            bytes[string_position..string_position + 4]
                .try_into()
                .expect("string length"),
        );
        assert_eq!(
            usize::try_from(string_length).expect("string length"),
            replacement.len(),
            "replacement must preserve the verified FlatBuffers shape"
        );
        let value_start = string_position + 4;
        bytes[value_start..value_start + replacement.len()].copy_from_slice(replacement);
    }

    fn root_table_position(bytes: &[u8]) -> usize {
        let root_offset = u32::from_le_bytes(bytes[4..8].try_into().expect("root offset"));
        4_usize
            .checked_add(usize::try_from(root_offset).expect("root position"))
            .expect("root position")
    }

    fn table_vtable_position(bytes: &[u8], table_position: usize) -> usize {
        let vtable_distance = i32::from_le_bytes(
            bytes[table_position..table_position + 4]
                .try_into()
                .expect("vtable offset"),
        );
        let distance = usize::try_from(vtable_distance.unsigned_abs())
            .expect("u32 vtable distance fits usize on supported targets");
        if vtable_distance.is_positive() {
            table_position
                .checked_sub(distance)
                .expect("vtable position")
        } else {
            table_position
                .checked_add(distance)
                .expect("vtable position")
        }
    }

    fn table_field_position(bytes: &[u8], table_position: usize, field: u16) -> Option<usize> {
        let vtable_position = table_vtable_position(bytes, table_position);
        let vtable_length = u16::from_le_bytes(
            bytes[vtable_position..vtable_position + 2]
                .try_into()
                .expect("vtable length"),
        );
        if field >= vtable_length {
            return None;
        }
        let entry = vtable_position + usize::from(field);
        let field_offset =
            u16::from_le_bytes(bytes[entry..entry + 2].try_into().expect("field offset"));
        (field_offset > 0).then(|| table_position + usize::from(field_offset))
    }

    fn table_field_target_position(bytes: &[u8], table_position: usize, field: u16) -> usize {
        let field_position =
            table_field_position(bytes, table_position, field).expect("test field must be present");
        let target_offset = u32::from_le_bytes(
            bytes[field_position..field_position + 4]
                .try_into()
                .expect("target offset"),
        );
        field_position
            .checked_add(usize::try_from(target_offset).expect("target position"))
            .expect("target position")
    }

    fn clear_table_vtable_entry(bytes: &mut [u8], table_position: usize, field: u16) {
        let vtable_position = table_vtable_position(bytes, table_position);
        let entry = vtable_position + usize::from(field);
        bytes[entry..entry + 2].copy_from_slice(&0_u16.to_le_bytes());
    }

    fn empty_table_vector<'fbb, T>(
        fbb: &mut runtime::FlatBufferBuilder<'fbb>,
    ) -> WIPOffset<runtime::Vector<'fbb, ForwardsUOffset<T>>> {
        fbb.create_vector::<WIPOffset<T>>(&[])
    }

    /// 第三方 writer 视角的共享 DAG fixture：一条 alignment 的曲线在同一个
    /// `CurveSegment` 表上重复 `segments` 槽位引用。复制数放大 verifier 的
    /// apparent size 而几乎不放大缓冲区，用于探测 16 倍上限的精确边界。
    fn shared_segment_dag_buffer(segment_copies: usize) -> Vec<u8> {
        let mut fbb = runtime::FlatBufferBuilder::new();
        let generator_build_id = fbb.create_string("third-party-generator");
        let description = fbb.create_string("shared segment dag fixture");
        let digest = wire::Digest256::new(&[0; 32]);
        let provenance = wire::Provenance::create(
            &mut fbb,
            &wire::ProvenanceArgs {
                kind: wire::ProvenanceKind::Generated,
                generator_build_id: Some(generator_build_id),
                parameters_and_inputs_digest: Some(&digest),
                frontend_options_digest: Some(&digest),
                random_seed: None,
                description: Some(description),
            },
        );
        let namespace = fbb.create_string("city");
        let source_document_key = fbb.create_string("roads/main");
        let import = fbb.create_string("base");
        let imports = fbb.create_vector(&[import]);
        let module_header = wire::ModuleHeader::create(
            &mut fbb,
            &wire::ModuleHeaderArgs {
                authoring_namespace_id: Some(namespace),
                source_document_key: Some(source_document_key),
                imports: Some(imports),
                provenance: Some(provenance),
            },
        );
        let control_1 = wire::Vec3F64::new(1.0, 0.0, 0.0);
        let control_2 = wire::Vec3F64::new(2.0, 0.0, 0.0);
        let end = wire::Vec3F64::new(3.0, 0.0, 0.0);
        let geometry = wire::CubicBezierSegment::create(
            &mut fbb,
            &wire::CubicBezierSegmentArgs {
                control_1: Some(&control_1),
                control_2: Some(&control_2),
                end: Some(&end),
            },
        );
        let shared_segment = wire::CurveSegment::create(
            &mut fbb,
            &wire::CurveSegmentArgs {
                geometry_type: wire::CurveSegmentGeometry::CubicBezierSegment,
                geometry: Some(geometry.as_union_value()),
                canvas_selection: None,
            },
        );
        let segments = vec![shared_segment; segment_copies];
        let segments = fbb.create_vector(&segments);
        let start = wire::Vec3F64::new(0.0, 0.0, 0.0);
        let reference_line = wire::CurveProgram::create(
            &mut fbb,
            &wire::CurveProgramArgs {
                start: Some(&start),
                segments: Some(segments),
            },
        );
        let alignment_key = fbb.create_string("alignment");
        let frame = fbb.create_string("base::frame");
        let alignment = wire::RoadAlignment::create(
            &mut fbb,
            &wire::RoadAlignmentArgs {
                road_alignment_key: Some(alignment_key),
                canonical_frame: Some(frame),
                reference_line: Some(reference_line),
                canvas_selection: None,
            },
        );
        let road_alignments = fbb.create_vector(&[alignment]);
        let road_corridors = empty_table_vector::<wire::RoadCorridor>(&mut fbb);
        let road_sections = empty_table_vector::<wire::RoadSection>(&mut fbb);
        let authoring_lanes = empty_table_vector::<wire::AuthoringLane>(&mut fbb);
        let lane_edges = empty_table_vector::<wire::LaneEdge>(&mut fbb);
        let junctions = empty_table_vector::<wire::Junction>(&mut fbb);
        let movements = empty_table_vector::<wire::Movement>(&mut fbb);
        let maneuver_paths = empty_table_vector::<wire::ManeuverPath>(&mut fbb);
        let maneuver_gates = empty_table_vector::<wire::ManeuverGate>(&mut fbb);
        let waiting_zones = empty_table_vector::<wire::WaitingZone>(&mut fbb);
        let stop_lines = empty_table_vector::<wire::StopLine>(&mut fbb);
        let signal_groups = empty_table_vector::<wire::SignalGroup>(&mut fbb);
        let signal_controllers = empty_table_vector::<wire::SignalController>(&mut fbb);
        let signal_phases = empty_table_vector::<wire::SignalPhase>(&mut fbb);
        let parking_facilities = empty_table_vector::<wire::ParkingFacility>(&mut fbb);
        let parking_spaces = empty_table_vector::<wire::ParkingSpace>(&mut fbb);
        let lane_groups = empty_table_vector::<wire::LaneGroup>(&mut fbb);
        let facility_bands = empty_table_vector::<wire::FacilityBand>(&mut fbb);
        let participant_classes = empty_table_vector::<wire::ParticipantClass>(&mut fbb);
        let access_rules = empty_table_vector::<wire::AccessRule>(&mut fbb);
        let vehicle_profiles = empty_table_vector::<wire::VehicleProfile>(&mut fbb);
        let canonical_frames = empty_table_vector::<wire::CanonicalFrame>(&mut fbb);
        let conflict_zones = empty_table_vector::<wire::ConflictZone>(&mut fbb);
        let participant_streams = empty_table_vector::<wire::ParticipantStream>(&mut fbb);
        let conflict_zone_regions = empty_table_vector::<wire::ConflictZoneRegion>(&mut fbb);
        let right_of_way_policy_sets = empty_table_vector::<wire::RightOfWayPolicySet>(&mut fbb);
        let root = wire::RoadEditingSource::create(
            &mut fbb,
            &wire::RoadEditingSourceArgs {
                format_version: FORMAT_VERSION,
                module_header: Some(module_header),
                geometry_accuracy_profile: wire::GeometryAccuracyProfile::Balanced5Cm,
                geometry_direction_profile: wire::GeometryDirectionProfile::Balanced2Deg,
                road_alignments: Some(road_alignments),
                road_corridors: Some(road_corridors),
                road_sections: Some(road_sections),
                authoring_lanes: Some(authoring_lanes),
                lane_edges: Some(lane_edges),
                junctions: Some(junctions),
                movements: Some(movements),
                maneuver_paths: Some(maneuver_paths),
                maneuver_gates: Some(maneuver_gates),
                waiting_zones: Some(waiting_zones),
                stop_lines: Some(stop_lines),
                signal_groups: Some(signal_groups),
                signal_controllers: Some(signal_controllers),
                signal_phases: Some(signal_phases),
                parking_facilities: Some(parking_facilities),
                parking_spaces: Some(parking_spaces),
                lane_groups: Some(lane_groups),
                facility_bands: Some(facility_bands),
                participant_classes: Some(participant_classes),
                access_rules: Some(access_rules),
                vehicle_profiles: Some(vehicle_profiles),
                canonical_frames: Some(canonical_frames),
                conflict_zones: Some(conflict_zones),
                participant_streams: Some(participant_streams),
                conflict_zone_regions: Some(conflict_zone_regions),
                right_of_way_policy_sets: Some(right_of_way_policy_sets),
            },
        );
        wire::finish_size_prefixed_road_editing_source_buffer(&mut fbb, root);
        fbb.finished_data().to_vec()
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
        let primary = first_diagnostic(&error)
            .primary_location()
            .and_then(crate::SourceLocation::road_editing)
            .expect("semantic failures retain a verified binary source location");
        assert_eq!(primary.document_identity().module_namespace(), Some("city"));
        assert!(matches!(
            primary.subject(),
            crate::RoadEditingSubject::ModuleHeader
        ));
    }

    #[test]
    fn parking_preflight_uses_current_facility_field_paths() {
        let limits = CompileLimits::p100_initial_v1();
        let buffer = parking_source_buffer(&limits);
        let mut bytes = buffer.as_bytes().to_vec();
        let key = b"facility-a";
        let key_offset = bytes
            .windows(key.len())
            .position(|window| window == key)
            .expect("facility key");
        bytes[key_offset..key_offset + key.len()].copy_from_slice(b"facility!a");
        let input = RoadEditingModuleInput::try_new("road-editing", &bytes, None).expect("input");
        let error = verify_source(input, &limits, 0, 0).expect_err("invalid facility key");
        assert!(matches!(
            first_diagnostic(&error).payload(),
            DiagnosticPayload::InvalidRoadEditingSource {
                field: Some(field),
                ..
            } if field.as_ref() == "parkingFacility.parkingFacilityKey"
        ));

        let mut bytes = buffer.as_bytes().to_vec();
        let duplicate = b"facility-b";
        let duplicate_offset = bytes
            .windows(duplicate.len())
            .position(|window| window == duplicate)
            .expect("second facility key");
        bytes[duplicate_offset..duplicate_offset + duplicate.len()].copy_from_slice(key);
        let input = RoadEditingModuleInput::try_new("road-editing", &bytes, None).expect("input");
        let error = verify_source(input, &limits, 0, 0).expect_err("duplicate facility key");
        assert!(matches!(
            first_diagnostic(&error).payload(),
            DiagnosticPayload::InvalidRoadEditingSource {
                field: Some(field),
                ..
            } if field.as_ref() == "parkingFacilities.parkingFacilityKey"
        ));

        let buffer = RoadEditingSourceWriter::new(&limits)
            .write(super::super::writer::tests::module_with_every_declaration(
                &limits,
            ))
            .expect("complete parking source");
        let mut bytes = buffer.as_bytes().to_vec();
        let parking_space_position = {
            let root = wire::size_prefixed_root_as_road_editing_source(&bytes)
                .expect("writer output remains structurally valid");
            let parking_space = root.parking_spaces().get(0);
            assert_eq!(parking_space.parking_facility(), Some("parking-facility"));
            parking_space._tab.loc()
        };
        overwrite_table_string_field(
            &mut bytes,
            parking_space_position,
            wire::ParkingSpace::VT_PARKING_FACILITY,
            b"parking!facility",
        );
        let input = RoadEditingModuleInput::try_new("road-editing", &bytes, None).expect("input");
        let error = verify_source(input, &limits, 0, 0).expect_err("invalid facility reference");
        assert!(matches!(
            first_diagnostic(&error).payload(),
            DiagnosticPayload::InvalidRoadEditingSource {
                field: Some(field),
                ..
            } if field.as_ref() == "parkingSpace.parkingFacility"
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
    fn semantic_preflight_rejects_non_reciprocal_signal_phase_owner() {
        let limits = CompileLimits::p100_initial_v1();
        let buffer = RoadEditingSourceWriter::new(&limits)
            .write(super::super::writer::tests::module_with_every_declaration(
                &limits,
            ))
            .expect("all declarations buffer");
        let mut bytes = buffer.as_bytes().to_vec();
        let needle = b"controller>phase";
        let matches = bytes
            .windows(needle.len())
            .enumerate()
            .filter_map(|(index, value)| (value == needle).then_some(index))
            .collect::<Vec<_>>();
        assert_eq!(
            matches.len(),
            1,
            "fixture has one controller phase reference"
        );
        bytes[matches[0] + "controlle".len()] = b'z';
        let input = RoadEditingModuleInput::try_new("road-editing", &bytes, None).expect("input");

        let error = verify_source(input, &limits, 0, 0).expect_err("owner mismatch");

        assert!(matches!(
            first_diagnostic(&error).payload(),
            DiagnosticPayload::InvalidRoadEditingSource {
                violation: RoadEditingSourceViolation::InvalidSemanticValue(
                    crate::RoadEditingInputViolation::InvalidCombination
                ),
                field: Some(field),
                ..
            } if field.as_ref() == "signalController.signalPhases"
        ));
    }

    #[test]
    fn semantic_preflight_rejects_imported_signal_group_owner() {
        let limits = CompileLimits::p100_initial_v1();
        let buffer = signal_source_buffer(&limits);
        let mut bytes = buffer.as_bytes().to_vec();
        let reference_offset = {
            let input =
                RoadEditingModuleInput::try_new("road-editing", &bytes, None).expect("input");
            let verified = verify_source(input, &limits, 0, 0).expect("valid source");
            let controller = verified.root().signal_controllers().get(0);
            let reference = controller.signal_groups().get(0);
            (reference.as_ptr() as usize)
                .checked_sub(bytes.as_ptr() as usize)
                .expect("reference belongs to source buffer")
        };
        assert_eq!(
            &bytes[reference_offset..reference_offset + "signal-group".len()],
            b"signal-group"
        );
        bytes[reference_offset + "signal".len()..reference_offset + "signal::".len()]
            .copy_from_slice(b"::");
        let input = RoadEditingModuleInput::try_new("road-editing", &bytes, None).expect("input");

        let error = verify_source(input, &limits, 0, 0).expect_err("imported signal group owner");

        assert!(matches!(
            first_diagnostic(&error).payload(),
            DiagnosticPayload::InvalidRoadEditingSource {
                violation: RoadEditingSourceViolation::InvalidSemanticValue(
                    crate::RoadEditingInputViolation::InvalidCombination
                ),
                field: Some(field),
                ..
            } if field.as_ref() == "signalController.signalGroups"
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
        let primary = first_diagnostic(&error)
            .primary_location()
            .and_then(crate::SourceLocation::road_editing)
            .expect("wire failures retain the external input identity");
        assert_eq!(primary.document_identity().module_namespace(), None);
        assert!(matches!(
            primary.subject(),
            crate::RoadEditingSubject::ModuleHeader
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
    fn verifier_trace_preserves_root_vector_index_and_checked_byte_range() {
        let limits = CompileLimits::p100_initial_v1();
        let buffer = source_buffer(&limits, "roads/main");
        let mut bytes = buffer.as_bytes().to_vec();
        let key_position = bytes
            .windows(b"frame".len())
            .position(|window| window == b"frame")
            .expect("canonical-frame key bytes");
        bytes[key_position] = 0xff;
        let input = RoadEditingModuleInput::try_new("roads/main", &bytes, None).expect("input");

        let error = verify_source(input, &limits, 0, 0).expect_err("invalid utf-8");
        let Some(crate::SourceLocation::RoadEditing(location)) =
            first_diagnostic(&error).primary_location()
        else {
            panic!("verifier data error must retain a wire location");
        };
        assert!(matches!(
            location.document_identity(),
            RoadEditingDocumentIdentity::Input(_)
        ));
        assert!(matches!(
            location.subject(),
            RoadEditingSubject::Wire {
                root_vector: RoadEditingRootVectorKind::CanonicalFrame,
                physical_index: 0,
                table: RoadEditingTableKind::CanonicalFrame,
            }
        ));
        let range = location.byte_range().expect("verified byte range");
        assert!(range.start() <= u32::try_from(key_position).unwrap());
        assert!(
            range.start().saturating_add(range.length()) > u32::try_from(key_position).unwrap()
        );
    }

    #[test]
    fn rejects_unknown_format_version_before_current_schema_verification() {
        let limits = CompileLimits::p100_initial_v1();
        let buffer = source_buffer(&limits, "roads/main");
        let mut bytes = buffer.as_bytes().to_vec();
        overwrite_format_version(&mut bytes, 1);
        let input = RoadEditingModuleInput::try_new("roads/main", &bytes, None).expect("input");

        let error = verify_source(input, &limits, 0, 0).expect_err("unknown format version");

        assert!(matches!(
            first_diagnostic(&error).payload(),
            DiagnosticPayload::InvalidRoadEditingSource {
                violation: RoadEditingSourceViolation::UnsupportedFormatVersion {
                    expected: 4,
                    actual: 1
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
    fn semantic_preflight_rejects_duplicate_lane_edge_successors() {
        let limits = CompileLimits::p100_initial_v1();
        let buffer = source_buffer_with_lane_successors(&limits);
        let mut bytes = buffer.as_bytes().to_vec();
        let successor_offset = {
            let root = wire::size_prefixed_root_as_road_editing_source(&bytes)
                .expect("writer output must be structurally valid");
            let edge = root
                .lane_edges()
                .iter()
                .find(|value| value.lane_edge_key() == "edge-a")
                .expect("fixture edge");
            let successor = edge.successors().get(1);
            assert_eq!(successor, "edge-c");
            (successor.as_ptr() as usize)
                .checked_sub(bytes.as_ptr() as usize)
                .expect("successor belongs to source buffer")
        };
        bytes[successor_offset..successor_offset + "edge-c".len()].copy_from_slice(b"edge-b");
        let input = RoadEditingModuleInput::try_new("roads/main", &bytes, None).expect("input");

        let error = verify_source(input, &limits, 0, 0).expect_err("duplicate successors");

        assert!(matches!(
            first_diagnostic(&error).payload(),
            DiagnosticPayload::InvalidRoadEditingSource {
                violation: RoadEditingSourceViolation::InvalidSemanticValue(
                    crate::RoadEditingInputViolation::DuplicateValue
                ),
                field: Some(field),
                ..
            } if field.as_ref() == "laneEdge.successors"
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

    #[test]
    fn rejects_empty_input_and_every_truncated_length_before_verifier() {
        let limits = CompileLimits::p100_initial_v1();
        let buffer = source_buffer(&limits, "roads/main");
        let bytes = buffer.as_bytes();

        let input = RoadEditingModuleInput::try_new("roads/main", &[], None).expect("input");
        let error = verify_source(input, &limits, 0, 0).expect_err("empty input");
        assert!(matches!(
            first_diagnostic(&error).payload(),
            DiagnosticPayload::InvalidRoadEditingSource {
                violation: RoadEditingSourceViolation::TruncatedFraming,
                ..
            }
        ));

        for end in 0..bytes.len() {
            let input =
                RoadEditingModuleInput::try_new("roads/main", &bytes[..end], None).expect("input");
            let error = verify_source(input, &limits, 0, 0).expect_err("truncated source");
            let violation = match first_diagnostic(&error).payload() {
                DiagnosticPayload::InvalidRoadEditingSource { violation, .. } => violation,
                payload => panic!("truncation must fail closed as a source violation: {payload:?}"),
            };
            if end < MIN_SIZE_PREFIXED_LFRE_BYTES {
                assert!(
                    matches!(violation, RoadEditingSourceViolation::TruncatedFraming),
                    "length {end} must hit the minimum-framing check"
                );
            } else {
                assert!(
                    matches!(
                        violation,
                        RoadEditingSourceViolation::SizePrefixMismatch { .. }
                    ),
                    "length {end} must hit the exact-length check before the verifier"
                );
            }
        }
    }

    #[test]
    fn rejects_size_prefix_off_by_one_in_both_directions() {
        let limits = CompileLimits::p100_initial_v1();
        let buffer = source_buffer(&limits, "roads/main");
        let actual = u64::try_from(buffer.as_bytes().len() - 4).expect("payload length");
        for declared in [actual - 1, actual + 1] {
            let mut bytes = buffer.as_bytes().to_vec();
            let declared_u32 = u32::try_from(declared).expect("portable size prefix");
            bytes[..4].copy_from_slice(&declared_u32.to_le_bytes());
            let input = RoadEditingModuleInput::try_new("roads/main", &bytes, None).expect("input");

            let error = verify_source(input, &limits, 0, 0).expect_err("off-by-one size prefix");

            assert!(matches!(
                first_diagnostic(&error).payload(),
                DiagnosticPayload::InvalidRoadEditingSource {
                    violation: RoadEditingSourceViolation::SizePrefixMismatch {
                        declared: seen_declared,
                        actual: seen_actual,
                    },
                    ..
                } if *seen_declared == declared && *seen_actual == actual
            ));
        }
    }

    #[test]
    fn rejects_trailing_bytes_after_declared_end() {
        let limits = CompileLimits::p100_initial_v1();
        let buffer = source_buffer(&limits, "roads/main");
        let declared = u64::try_from(buffer.as_bytes().len() - 4).expect("declared length");
        let mut bytes = buffer.as_bytes().to_vec();
        bytes.resize(bytes.len() + 8, 0);
        let input = RoadEditingModuleInput::try_new("roads/main", &bytes, None).expect("input");

        let error = verify_source(input, &limits, 0, 0).expect_err("trailing bytes");

        assert!(matches!(
            first_diagnostic(&error).payload(),
            DiagnosticPayload::InvalidRoadEditingSource {
                violation: RoadEditingSourceViolation::SizePrefixMismatch {
                    declared: seen_declared,
                    actual: seen_actual,
                },
                ..
            } if *seen_declared == declared && *seen_actual == declared + 8
        ));
    }

    #[test]
    fn rejects_identifier_case_and_adjacent_byte_variants() {
        let limits = CompileLimits::p100_initial_v1();
        let buffer = source_buffer(&limits, "roads/main");
        for identifier in [b"lfre", b"LFRD", b"LFRF", b"LFRe"] {
            let mut bytes = buffer.as_bytes().to_vec();
            bytes[8..12].copy_from_slice(identifier);
            let input = RoadEditingModuleInput::try_new("roads/main", &bytes, None).expect("input");

            let error = verify_source(input, &limits, 0, 0).expect_err("identifier variant");

            assert!(matches!(
                first_diagnostic(&error).payload(),
                DiagnosticPayload::InvalidRoadEditingSource {
                    violation: RoadEditingSourceViolation::FileIdentifierMismatch,
                    ..
                }
            ));
        }
    }

    #[test]
    fn rejects_root_offset_pointing_at_buffer_end() {
        let limits = CompileLimits::p100_initial_v1();
        let buffer = source_buffer(&limits, "roads/main");
        let mut bytes = buffer.as_bytes().to_vec();
        let root_offset = u32::try_from(bytes.len() - 4).expect("portable root offset");
        bytes[4..8].copy_from_slice(&root_offset.to_le_bytes());
        let input = RoadEditingModuleInput::try_new("roads/main", &bytes, None).expect("input");

        let error = verify_source(input, &limits, 0, 0).expect_err("root offset at buffer end");

        assert!(matches!(
            first_diagnostic(&error).payload(),
            DiagnosticPayload::InvalidRoadEditingSource {
                violation: RoadEditingSourceViolation::MalformedWire,
                ..
            }
        ));
    }

    #[test]
    fn rejects_misaligned_root_table_offset() {
        let limits = CompileLimits::p100_initial_v1();
        let buffer = source_buffer(&limits, "roads/main");
        let mut bytes = buffer.as_bytes().to_vec();
        bytes[4..8].copy_from_slice(&5_u32.to_le_bytes());
        let input = RoadEditingModuleInput::try_new("roads/main", &bytes, None).expect("input");

        let error = verify_source(input, &limits, 0, 0).expect_err("misaligned root table");

        assert!(matches!(
            first_diagnostic(&error).payload(),
            DiagnosticPayload::InvalidRoadEditingSource {
                violation: RoadEditingSourceViolation::MalformedWire,
                ..
            }
        ));
    }

    #[test]
    fn rejects_out_of_bounds_vtable_offset() {
        let limits = CompileLimits::p100_initial_v1();
        let buffer = source_buffer(&limits, "roads/main");
        let mut bytes = buffer.as_bytes().to_vec();
        let root_position = root_table_position(&bytes);
        bytes[root_position..root_position + 4].copy_from_slice(&i32::MAX.to_le_bytes());
        let input = RoadEditingModuleInput::try_new("roads/main", &bytes, None).expect("input");

        let error = verify_source(input, &limits, 0, 0).expect_err("out-of-bounds vtable");

        assert!(matches!(
            first_diagnostic(&error).payload(),
            DiagnosticPayload::InvalidRoadEditingSource {
                violation: RoadEditingSourceViolation::MalformedWire,
                ..
            }
        ));
    }

    #[test]
    fn rejects_truncated_vtable_beyond_buffer() {
        let limits = CompileLimits::p100_initial_v1();
        let buffer = source_buffer(&limits, "roads/main");
        let mut bytes = buffer.as_bytes().to_vec();
        let vtable_position = table_vtable_position(&bytes, root_table_position(&bytes));
        bytes[vtable_position..vtable_position + 2].copy_from_slice(&0xFFFE_u16.to_le_bytes());
        let input = RoadEditingModuleInput::try_new("roads/main", &bytes, None).expect("input");

        let error = verify_source(input, &limits, 0, 0).expect_err("truncated vtable");

        assert!(matches!(
            first_diagnostic(&error).payload(),
            DiagnosticPayload::InvalidRoadEditingSource {
                violation: RoadEditingSourceViolation::MalformedWire,
                ..
            }
        ));
    }

    #[test]
    fn rejects_vector_length_beyond_buffer() {
        let limits = CompileLimits::p100_initial_v1();
        let buffer = source_buffer(&limits, "roads/main");
        let mut bytes = buffer.as_bytes().to_vec();
        let vector_position = table_field_target_position(
            &bytes,
            root_table_position(&bytes),
            wire::RoadEditingSource::VT_CANONICAL_FRAMES,
        );
        bytes[vector_position..vector_position + 4].copy_from_slice(&0x4000_0000_u32.to_le_bytes());
        let input = RoadEditingModuleInput::try_new("roads/main", &bytes, None).expect("input");

        let error = verify_source(input, &limits, 0, 0).expect_err("vector length overflow");

        assert!(matches!(
            first_diagnostic(&error).payload(),
            DiagnosticPayload::InvalidRoadEditingSource {
                violation: RoadEditingSourceViolation::MalformedWire,
                ..
            }
        ));
    }

    #[test]
    fn rejects_string_missing_null_terminator() {
        let limits = CompileLimits::p100_initial_v1();
        let buffer = source_buffer(&limits, "roads/main");
        let mut bytes = buffer.as_bytes().to_vec();
        let key_position = bytes
            .windows(b"frame".len())
            .position(|window| window == b"frame")
            .expect("canonical-frame key bytes");
        let terminator = key_position + b"frame".len();
        assert_eq!(bytes[terminator], 0, "writer strings are NUL-terminated");
        bytes[terminator] = b'!';
        let input = RoadEditingModuleInput::try_new("roads/main", &bytes, None).expect("input");

        let error = verify_source(input, &limits, 0, 0).expect_err("missing NUL terminator");

        assert!(matches!(
            first_diagnostic(&error).payload(),
            DiagnosticPayload::InvalidRoadEditingSource {
                violation: RoadEditingSourceViolation::MalformedWire,
                ..
            }
        ));
    }

    #[test]
    fn rejects_string_offset_beyond_buffer() {
        let limits = CompileLimits::p100_initial_v1();
        let buffer = source_buffer(&limits, "roads/main");
        let mut bytes = buffer.as_bytes().to_vec();
        let frame_position = {
            let root = wire::size_prefixed_root_as_road_editing_source(&bytes)
                .expect("writer output must be structurally valid");
            root.canonical_frames().get(0)._tab.loc()
        };
        let field_position = table_field_position(
            &bytes,
            frame_position,
            wire::CanonicalFrame::VT_CANONICAL_FRAME_KEY,
        )
        .expect("key field must be present");
        bytes[field_position..field_position + 4].copy_from_slice(&u32::MAX.to_le_bytes());
        let input = RoadEditingModuleInput::try_new("roads/main", &bytes, None).expect("input");

        let error = verify_source(input, &limits, 0, 0).expect_err("string offset out of bounds");

        assert!(matches!(
            first_diagnostic(&error).payload(),
            DiagnosticPayload::InvalidRoadEditingSource {
                violation: RoadEditingSourceViolation::MalformedWire,
                ..
            }
        ));
    }

    #[test]
    fn rejects_union_value_without_discriminant_at_verifier() {
        let limits = CompileLimits::p100_initial_v1();
        let buffer = source_buffer_with_imported_frame(&limits);
        let mut bytes = buffer.as_bytes().to_vec();
        let segment_position = {
            let root = wire::size_prefixed_root_as_road_editing_source(&bytes)
                .expect("writer output must be structurally valid");
            root.road_alignments()
                .get(0)
                .reference_line()
                .segments()
                .get(0)
                ._tab
                .loc()
        };
        clear_table_vtable_entry(
            &mut bytes,
            segment_position,
            wire::CurveSegment::VT_GEOMETRY_TYPE,
        );
        let input = RoadEditingModuleInput::try_new("roads/main", &bytes, None).expect("input");

        let error = verify_source(input, &limits, 0, 0).expect_err("inconsistent union");

        assert!(matches!(
            first_diagnostic(&error).payload(),
            DiagnosticPayload::InvalidRoadEditingSource {
                violation: RoadEditingSourceViolation::MalformedWire,
                ..
            }
        ));
    }

    #[test]
    fn rejects_unknown_union_discriminant_after_verification() {
        let limits = CompileLimits::p100_initial_v1();
        let buffer = source_buffer_with_imported_frame(&limits);
        let mut bytes = buffer.as_bytes().to_vec();
        let segment_position = {
            let root = wire::size_prefixed_root_as_road_editing_source(&bytes)
                .expect("writer output must be structurally valid");
            root.road_alignments()
                .get(0)
                .reference_line()
                .segments()
                .get(0)
                ._tab
                .loc()
        };
        let field_position = table_field_position(
            &bytes,
            segment_position,
            wire::CurveSegment::VT_GEOMETRY_TYPE,
        )
        .expect("discriminant field must be present");
        bytes[field_position] = 200;
        let input = RoadEditingModuleInput::try_new("roads/main", &bytes, None).expect("input");

        let error = verify_source(input, &limits, 0, 0).expect_err("unknown union discriminant");

        assert!(matches!(
            first_diagnostic(&error).payload(),
            DiagnosticPayload::InvalidRoadEditingSource {
                violation: RoadEditingSourceViolation::InvalidSemanticValue(
                    crate::RoadEditingInputViolation::InvalidCombination
                ),
                field: Some(field),
                ..
            } if field.as_ref() == "curveSegment.geometry"
        ));
    }

    #[test]
    fn rejects_absent_required_field_at_verifier() {
        let limits = CompileLimits::p100_initial_v1();
        let buffer = source_buffer(&limits, "roads/main");
        let mut bytes = buffer.as_bytes().to_vec();
        let frame_position = {
            let root = wire::size_prefixed_root_as_road_editing_source(&bytes)
                .expect("writer output must be structurally valid");
            root.canonical_frames().get(0)._tab.loc()
        };
        clear_table_vtable_entry(
            &mut bytes,
            frame_position,
            wire::CanonicalFrame::VT_CANONICAL_FRAME_KEY,
        );
        let input = RoadEditingModuleInput::try_new("roads/main", &bytes, None).expect("input");

        let error = verify_source(input, &limits, 0, 0).expect_err("missing required field");

        assert!(matches!(
            first_diagnostic(&error).payload(),
            DiagnosticPayload::InvalidRoadEditingSource {
                violation: RoadEditingSourceViolation::MalformedWire,
                ..
            }
        ));
    }

    #[test]
    fn rejects_unknown_geometry_profile_enum_after_verification() {
        let limits = CompileLimits::p100_initial_v1();
        let buffer = source_buffer(&limits, "roads/main");
        let mut bytes = buffer.as_bytes().to_vec();
        overwrite_root_u8_field(&mut bytes, 2, 200);
        let input = RoadEditingModuleInput::try_new("roads/main", &bytes, None).expect("input");

        let error = verify_source(input, &limits, 0, 0).expect_err("unknown enum value");

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
    fn verifier_depth_admits_schema_longest_path() {
        let limits = CompileLimits::p100_initial_v1();
        let buffer = source_buffer_with_imported_frame(&limits);
        let input =
            RoadEditingModuleInput::try_new("roads/main", buffer.as_bytes(), None).expect("input");

        let verified = verify_source(input, &limits, 0, 0).expect("depth-five schema path");

        let alignment = verified.root().road_alignments().get(0);
        assert_eq!(alignment.reference_line().segments().len(), 1);
    }

    #[test]
    fn verifier_dos_errors_map_to_closed_violations() {
        let limits = CompileLimits::p100_initial_v1();

        let error = verifier_error(
            InvalidFlatbuffer::DepthLimitReached,
            &limits,
            "roads/main",
            64,
        );
        assert!(matches!(
            first_diagnostic(&error).payload(),
            DiagnosticPayload::InvalidRoadEditingSource {
                violation: RoadEditingSourceViolation::VerifierDepthExceeded,
                ..
            }
        ));

        let error = verifier_error(
            InvalidFlatbuffer::ApparentSizeTooLarge,
            &limits,
            "roads/main",
            64,
        );
        assert!(matches!(
            first_diagnostic(&error).payload(),
            DiagnosticPayload::InvalidRoadEditingSource {
                violation: RoadEditingSourceViolation::VerifierApparentSizeExceeded,
                ..
            }
        ));

        let error = verifier_error(InvalidFlatbuffer::TooManyTables, &limits, "roads/main", 64);
        assert!(matches!(
            first_diagnostic(&error).payload(),
            DiagnosticPayload::CompileLimitExceeded {
                dimension: CompileLimitDimension::TypedAstRecordCount,
                limit,
                observed,
            } if *limit == 58_387 && *observed == 58_388
        ));
    }

    #[test]
    fn verifier_apparent_size_admits_boundary_and_rejects_boundary_plus_one() {
        let limits = CompileLimits::p100_initial_v1();
        let mut accepted_copies = 0_usize;
        let mut rejection = None;
        for copies in 1..=5_000 {
            let bytes = shared_segment_dag_buffer(copies);
            let input = RoadEditingModuleInput::try_new("roads/main", &bytes, None).expect("input");
            match verify_source(input, &limits, 0, 0) {
                Ok(_) => accepted_copies = copies,
                Err(error) => {
                    rejection = Some((copies, error));
                    break;
                }
            }
        }
        let (rejected_copies, error) =
            rejection.expect("shared-DAG apparent size must cross the 16x budget");
        assert!(matches!(
            first_diagnostic(&error).payload(),
            DiagnosticPayload::InvalidRoadEditingSource {
                violation: RoadEditingSourceViolation::VerifierApparentSizeExceeded,
                ..
            }
        ));
        assert!(
            accepted_copies >= 1,
            "single-copy fixture must stay accepted"
        );
        assert_eq!(
            accepted_copies + 1,
            rejected_copies,
            "apparent-size rejection must flip exactly at the boundary"
        );
    }
}
