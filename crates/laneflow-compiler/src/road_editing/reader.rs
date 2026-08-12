use laneflow_road_editing_wire::generated::lane_flow::road_editing::v1 as wire;
use laneflow_road_editing_wire::runtime::{ErrorTraceDetail, InvalidFlatbuffer, VerifierOptions};

use super::RoadEditingModuleInput;
use super::location::RoadEditingLocationFactory;
use super::preflight::{RoadEditingPreflightCounts, preflight_source};
use crate::{
    CompileLimitDimension, CompileLimits, Diagnostic, DiagnosticBundle, FailureLiveBudget,
    RoadEditingByteRange, RoadEditingRootVectorKind, RoadEditingSourceViolation,
    RoadEditingTableKind, SourceLocation,
};

const FORMAT_VERSION: u32 = 1;
const MIN_SIZE_PREFIXED_LFRE_BYTES: usize = 12;
const MAX_SCHEMA_TABLE_DEPTH: usize = 5;
const APPARENT_SIZE_MULTIPLIER: usize = 16;
// 单项 reader failure 的保守物化 ceiling：最长闭合字段名、至多三份受 SingleStringBytes
// 约束的 payload/stable token、一条 related-location handle，以及最小 Input/Verified location。
// 这是失败路径的窄 headroom 取舍，不从成功输入中预留；下面的最大形状测试防止构造漂移。
const INPUT_FAILURE_UPPER_BYTES: u64 = 53 * 3 + core::mem::size_of::<SourceLocation>() as u64 + 256;
// pinned flatbuffers 25.12.19 的 ErrorTrace 使用 Vec。冻结 schema 的唯一最长路径为：
// RoadEditingSource.road_alignments[i].reference_line.segments[j].geometry(LineSegment).end，
// 即 4 个 table field、2 个 vector element 和 1 个 union variant，共 7 项。16/24 不是
// Rust Vec 的语言级保证，而是受支持工具链上由下方 schema-path + capacity-growth 测试锁定的
// retained/reallocation ceiling；Cargo.lock 的 flatbuffers 与 CI Rust toolchain 变化都必须重跑。
#[cfg(test)]
const MAX_VERIFIER_TRACE_DETAILS: usize = 7;
const VERIFIER_TRACE_RETAINED_UPPER_BYTES: u64 =
    core::mem::size_of::<ErrorTraceDetail>() as u64 * 16;
const VERIFIER_TRACE_PEAK_UPPER_BYTES: u64 = core::mem::size_of::<ErrorTraceDetail>() as u64 * 24;

fn materialize_input_failure(
    budget: FailureLiveBudget,
    rich: impl FnOnce() -> DiagnosticBundle,
) -> DiagnosticBundle {
    budget.materialize(INPUT_FAILURE_UPPER_BYTES, rich)
}

/// 已通过来源字节、size prefix 与 file identifier 前门的借用输入。
#[derive(Debug)]
pub(crate) struct FramedRoadEditingSource<'a> {
    input: RoadEditingModuleInput<'a>,
}

impl<'a> FramedRoadEditingSource<'a> {
    pub(crate) const fn input(&self) -> RoadEditingModuleInput<'a> {
        self.input
    }
}

/// 已由固定上限 FlatBuffers verifier 产生安全 root、尚未读取 LaneFlow 语义的借用 view。
#[derive(Debug)]
pub(crate) struct WireVerifiedRoadEditingSource<'a> {
    input: RoadEditingModuleInput<'a>,
    root: wire::RoadEditingSource<'a>,
}

impl<'a> WireVerifiedRoadEditingSource<'a> {
    pub(crate) const fn input(&self) -> RoadEditingModuleInput<'a> {
        self.input
    }

    pub(crate) const fn root(&self) -> wire::RoadEditingSource<'a> {
        self.root
    }
}

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

/// 在任何 wire traversal 前校验来源字节限额、size prefix 与 `LFRE` identifier。
pub(crate) fn preflight_source_framing<'a>(
    input: RoadEditingModuleInput<'a>,
    limits: &CompileLimits,
    source_bytes_already_admitted: u64,
    failure_live_budget: FailureLiveBudget,
) -> Result<FramedRoadEditingSource<'a>, DiagnosticBundle> {
    let expected_key = input.expected_source_document_key();
    let bytes = input.source_bytes();
    let source_len = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    let per_module_limit = limits.value(CompileLimitDimension::SourceBytesPerModule);
    if source_len > per_module_limit {
        return Err(materialize_input_failure(failure_live_budget, || {
            limit_error(
                CompileLimitDimension::SourceBytesPerModule,
                per_module_limit,
                source_len,
                expected_key,
            )
        }));
    }
    let source_total_limit = limits.value(CompileLimitDimension::SourceBytesTotal);
    let source_total = source_bytes_already_admitted.saturating_add(source_len);
    if source_total > source_total_limit {
        return Err(materialize_input_failure(failure_live_budget, || {
            limit_error(
                CompileLimitDimension::SourceBytesTotal,
                source_total_limit,
                source_total,
                expected_key,
            )
        }));
    }
    if bytes.len() < MIN_SIZE_PREFIXED_LFRE_BYTES {
        return Err(materialize_input_failure(failure_live_budget, || {
            source_error(
                RoadEditingSourceViolation::TruncatedFraming,
                expected_key,
                None,
            )
        }));
    }

    let declared_len = u32::from_le_bytes(
        bytes[..4]
            .try_into()
            .expect("minimum framing length includes size prefix"),
    );
    let actual_len = u64::try_from(bytes.len() - 4).unwrap_or(u64::MAX);
    if u64::from(declared_len) != actual_len {
        return Err(materialize_input_failure(failure_live_budget, || {
            source_error(
                RoadEditingSourceViolation::SizePrefixMismatch {
                    declared: u64::from(declared_len),
                    actual: actual_len,
                },
                expected_key,
                None,
            )
        }));
    }
    if !wire::road_editing_source_size_prefixed_buffer_has_identifier(bytes) {
        return Err(materialize_input_failure(failure_live_budget, || {
            source_error(
                RoadEditingSourceViolation::FileIdentifierMismatch,
                expected_key,
                None,
            )
        }));
    }

    Ok(FramedRoadEditingSource { input })
}

/// 使用冻结 verifier 上限产生唯一安全 FlatBuffers root，不读取 LaneFlow 领域语义。
pub(crate) fn verify_source_wire<'a>(
    framed: FramedRoadEditingSource<'a>,
    limits: &CompileLimits,
    typed_ast_records_already_admitted: u64,
    failure_live_budget: FailureLiveBudget,
) -> Result<WireVerifiedRoadEditingSource<'a>, DiagnosticBundle> {
    let input = framed.input();
    let expected_key = input.expected_source_document_key();
    let bytes = input.source_bytes();
    let verifier_failure_budget = failure_live_budget.with_transient(
        VERIFIER_TRACE_RETAINED_UPPER_BYTES,
        VERIFIER_TRACE_PEAK_UPPER_BYTES,
    )?;
    let scratch_limit = limits.value(CompileLimitDimension::StageScratchBytes);
    if VERIFIER_TRACE_PEAK_UPPER_BYTES > scratch_limit {
        return Err(
            failure_live_budget.materialize(INPUT_FAILURE_UPPER_BYTES, || {
                limit_error(
                    CompileLimitDimension::StageScratchBytes,
                    scratch_limit,
                    VERIFIER_TRACE_PEAK_UPPER_BYTES,
                    expected_key,
                )
            }),
        );
    }

    let typed_ast_limit = limits.value(CompileLimitDimension::TypedAstRecordCount);
    let remaining_records = typed_ast_limit.saturating_sub(typed_ast_records_already_admitted);
    let max_tables_u64 = remaining_records.checked_add(2).ok_or_else(|| {
        materialize_input_failure(failure_live_budget, || {
            source_error(
                RoadEditingSourceViolation::VerifierTableBudgetExceeded,
                expected_key,
                None,
            )
        })
    })?;
    let max_tables = usize::try_from(max_tables_u64).map_err(|_| {
        materialize_input_failure(failure_live_budget, || {
            source_error(
                RoadEditingSourceViolation::VerifierTableBudgetExceeded,
                expected_key,
                None,
            )
        })
    })?;
    let max_apparent_size = bytes
        .len()
        .checked_mul(APPARENT_SIZE_MULTIPLIER)
        .ok_or_else(|| {
            materialize_input_failure(failure_live_budget, || {
                source_error(
                    RoadEditingSourceViolation::VerifierApparentSizeExceeded,
                    expected_key,
                    None,
                )
            })
        })?;
    let options = VerifierOptions {
        max_depth: MAX_SCHEMA_TABLE_DEPTH,
        max_tables,
        max_apparent_size,
        ignore_missing_null_terminator: false,
    };
    let root = wire::size_prefixed_root_as_road_editing_source_with_opts(&options, bytes).map_err(
        |error| {
            materialize_input_failure(verifier_failure_budget, || {
                verifier_error(error, limits, expected_key, bytes.len())
            })
        },
    )?;

    Ok(WireVerifiedRoadEditingSource { input, root })
}

/// 在安全 root 上执行 exact version/identity、table 计量与完整 LaneFlow 语义预检。
pub(crate) fn preflight_verified_source<'a>(
    wire_verified: WireVerifiedRoadEditingSource<'a>,
    limits: &CompileLimits,
    typed_ast_records_already_admitted: u64,
    failure_live_budget: FailureLiveBudget,
) -> Result<VerifiedRoadEditingSource<'a>, DiagnosticBundle> {
    let input = wire_verified.input();
    let expected_key = input.expected_source_document_key();
    let root = wire_verified.root();

    if root.format_version() != FORMAT_VERSION {
        return Err(materialize_input_failure(failure_live_budget, || {
            source_error(
                RoadEditingSourceViolation::UnsupportedFormatVersion {
                    expected: FORMAT_VERSION,
                    actual: root.format_version(),
                },
                expected_key,
                None,
            )
        }));
    }
    let actual_key = root.module_header().source_document_key();
    if actual_key != expected_key {
        let bounded_actual_key = (u64::try_from(actual_key.len()).unwrap_or(u64::MAX)
            <= limits.value(CompileLimitDimension::SingleStringBytes))
        .then_some(actual_key);
        return Err(materialize_input_failure(failure_live_budget, || {
            source_error(
                RoadEditingSourceViolation::SourceDocumentKeyMismatch,
                expected_key,
                bounded_actual_key,
            )
        }));
    }

    let table_count = table_count(root);
    let typed_ast_record_count = table_count.saturating_sub(2);
    let typed_ast_limit = limits.value(CompileLimitDimension::TypedAstRecordCount);
    let total_records = typed_ast_records_already_admitted.saturating_add(typed_ast_record_count);
    if total_records > typed_ast_limit {
        return Err(materialize_input_failure(failure_live_budget, || {
            limit_error(
                CompileLimitDimension::TypedAstRecordCount,
                typed_ast_limit,
                total_records,
                expected_key,
            )
        }));
    }

    let preflight_counts = preflight_source(root, limits, expected_key, failure_live_budget)?;
    if preflight_counts.typed_ast_record_count() != typed_ast_record_count {
        return Err(
            failure_live_budget.materialize(INPUT_FAILURE_UPPER_BYTES, || {
                semantic_error(
                    "roadEditingSource.tableAccounting",
                    crate::RoadEditingInputViolation::InvalidCombination,
                    expected_key,
                )
                .with_fallback_primary_location(
                    RoadEditingLocationFactory::verified_module_header(
                        root.module_header().authoring_namespace_id(),
                        expected_key,
                    ),
                )
            }),
        );
    }

    Ok(VerifiedRoadEditingSource {
        input,
        root,
        table_count,
        preflight_counts,
    })
}

/// 组合生产 reader 的三段边界；供不需要独立计时的内部调用与测试复用。
#[cfg(test)]
pub(crate) fn verify_source<'a>(
    input: RoadEditingModuleInput<'a>,
    limits: &CompileLimits,
    source_bytes_already_admitted: u64,
    typed_ast_records_already_admitted: u64,
) -> Result<VerifiedRoadEditingSource<'a>, DiagnosticBundle> {
    let framed = preflight_source_framing(
        input,
        limits,
        source_bytes_already_admitted,
        FailureLiveBudget::unlimited(),
    )?;
    let wire_verified = verify_source_wire(
        framed,
        limits,
        typed_ast_records_already_admitted,
        FailureLiveBudget::unlimited(),
    )?;
    preflight_verified_source(
        wire_verified,
        limits,
        typed_ast_records_already_admitted,
        FailureLiveBudget::unlimited(),
    )
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
            let field = verifier_field_hint(&data_error, trace);
            let location = wire_location_from_trace(trace, expected_key, range).or_else(|| {
                range.map(|range| {
                    RoadEditingLocationFactory::input_module_header_with_range(
                        expected_key,
                        Some(range),
                    )
                })
            });
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

fn verifier_field_hint<'a>(
    error: &'a InvalidFlatbuffer,
    trace: &'a [ErrorTraceDetail],
) -> Option<&'a str> {
    match error {
        InvalidFlatbuffer::MissingRequiredField { required, .. } => Some(required.as_ref()),
        InvalidFlatbuffer::InconsistentUnion { field, .. } => Some(field.as_ref()),
        _ => trace.iter().find_map(|detail| match detail {
            ErrorTraceDetail::TableField { field_name, .. } => Some(field_name.as_ref()),
            _ => None,
        }),
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
    // FlatBuffers 从内层向外层追加 trace；必须选择最外层已知 root vector，避免
    // `SignalController.signal_groups` 等嵌套字段被误报为根 `signal_groups`。
    for (field_position, detail) in details.iter().enumerate().rev() {
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
        "parking_areas" => (
            RoadEditingRootVectorKind::ParkingArea,
            RoadEditingTableKind::ParkingArea,
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
        "static_routes" => (
            RoadEditingRootVectorKind::StaticRoute,
            RoadEditingTableKind::StaticRoute,
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
    use std::borrow::Cow;

    use super::*;
    use crate::road_editing::{
        CanonicalFrameInput, CanonicalFrameReference, RoadAlignmentInput, RoadEditingCurveProgram,
        RoadEditingCurveSegment, RoadEditingDeclaration, RoadEditingModuleHeader,
        RoadEditingPoint3, RoadEditingProvenance, RoadEditingSignalPhaseState,
        RoadEditingSourceModuleBuilder, RoadEditingSourceWriter, SignalControllerInput,
        SignalControllerReference, SignalGroupInput, SignalGroupReference, SignalPhaseInput,
    };
    use crate::{
        DiagnosticCode, DiagnosticPayload, GeometryAccuracyProfile, GeometryDirectionProfile,
        RoadEditingDocumentIdentity, RoadEditingPropertyStep, RoadEditingRootVectorKind,
        RoadEditingSubject,
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
        let other_group = SignalGroupReference::local("signal-group-b").expect("other group");
        let controller = SignalControllerReference::local("controller").expect("controller");
        builder
            .add_declaration(RoadEditingDeclaration::SignalGroup(
                SignalGroupInput::try_new("signal-group").expect("group"),
            ))
            .expect("group declaration");
        builder
            .add_declaration(RoadEditingDeclaration::SignalGroup(
                SignalGroupInput::try_new("signal-group-b").expect("other group"),
            ))
            .expect("other group declaration");
        builder
            .add_declaration(RoadEditingDeclaration::SignalController(
                SignalControllerInput::try_new(
                    "controller",
                    0,
                    vec![group.clone(), other_group.clone()],
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
                        RoadEditingSignalPhaseState::try_new(
                            other_group,
                            laneflow_static_contract::SignalAspect::Red,
                        )
                        .expect("other phase state"),
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

    fn deepest_trace_source_buffer(
        limits: &CompileLimits,
    ) -> super::super::OwnedRoadEditingSourceBuffer {
        let header = RoadEditingModuleHeader::try_new(
            "city",
            "road-editing",
            Vec::new(),
            RoadEditingProvenance::direct("trace ceiling").expect("provenance"),
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
        builder
            .add_alignment(
                RoadAlignmentInput::try_new(
                    "alignment",
                    CanonicalFrameReference::local("frame").expect("frame reference"),
                    RoadEditingCurveProgram::try_new(
                        RoadEditingPoint3::try_new(0.0, 0.0, 0.0).expect("start"),
                        vec![RoadEditingCurveSegment::line(
                            RoadEditingPoint3::try_new(10.0, 0.0, 0.0).expect("end"),
                        )],
                    )
                    .expect("curve"),
                )
                .expect("alignment"),
            )
            .expect("alignment declaration");
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

        let framed = preflight_source_framing(input, &limits, 0, FailureLiveBudget::unlimited())
            .expect("framing preflight");
        let wire_verified = verify_source_wire(framed, &limits, 0, FailureLiveBudget::unlimited())
            .expect("unspecified profile remains structurally valid wire");
        let error =
            preflight_verified_source(wire_verified, &limits, 0, FailureLiveBudget::unlimited())
                .expect_err("unspecified profile is a semantic failure");

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
        let location = first_diagnostic(&error)
            .primary_location()
            .and_then(SourceLocation::road_editing)
            .expect("semantic preflight location");
        assert!(matches!(
            location.subject(),
            RoadEditingSubject::ModuleHeader
        ));
        assert_eq!(
            location.property_path().expect("profile property").steps(),
            &[RoadEditingPropertyStep::TableField {
                table: RoadEditingTableKind::RoadEditingSource,
                field_id: 2,
            }]
        );
    }

    #[test]
    fn wire_trace_selects_outer_root_when_nested_field_has_root_name() {
        let trace = [
            ErrorTraceDetail::VectorElement {
                index: 3,
                position: 24,
            },
            ErrorTraceDetail::TableField {
                field_name: Cow::Borrowed("signal_groups"),
                position: 20,
            },
            ErrorTraceDetail::VectorElement {
                index: 1,
                position: 12,
            },
            ErrorTraceDetail::TableField {
                field_name: Cow::Borrowed("signal_controllers"),
                position: 8,
            },
        ];

        let Some(SourceLocation::RoadEditing(location)) =
            wire_location_from_trace(&trace, "roads/main", None)
        else {
            panic!("outer root wire location expected");
        };
        assert!(matches!(
            location.document_identity(),
            RoadEditingDocumentIdentity::Input(_)
        ));
        assert!(matches!(
            location.subject(),
            RoadEditingSubject::Wire {
                root_vector: RoadEditingRootVectorKind::SignalController,
                physical_index: 1,
                table: RoadEditingTableKind::SignalController,
            }
        ));
        assert!(location.property_path().is_none());
        assert!(location.byte_range().is_none());
    }

    #[test]
    fn verifier_field_hint_prefers_terminal_error_metadata() {
        let trace = [ErrorTraceDetail::TableField {
            field_name: Cow::Borrowed("road_alignments"),
            position: 8,
        }];
        let missing = InvalidFlatbuffer::MissingRequiredField {
            required: Cow::Borrowed("reference_line"),
            error_trace: Default::default(),
        };
        let inconsistent = InvalidFlatbuffer::InconsistentUnion {
            field: Cow::Borrowed("geometry"),
            field_type: Cow::Borrowed("geometry_type"),
            error_trace: Default::default(),
        };

        assert_eq!(
            verifier_field_hint(&missing, &trace),
            Some("reference_line")
        );
        assert_eq!(verifier_field_hint(&inconsistent, &trace), Some("geometry"));
    }

    #[test]
    fn pinned_schema_and_toolchain_bound_the_deepest_verifier_trace() {
        let limits = CompileLimits::p100_initial_v1();
        let buffer = deepest_trace_source_buffer(&limits);
        let mut bytes = buffer.as_bytes().to_vec();
        let root = wire::size_prefixed_root_as_road_editing_source(&bytes)
            .expect("writer output is valid before one required vtable slot is cleared");
        let line = root
            .road_alignments()
            .get(0)
            .reference_line()
            .segments()
            .get(0)
            .geometry_as_line_segment()
            .expect("line segment");
        let line_position = line._tab.loc();
        let vtable_distance = i32::from_le_bytes(
            bytes[line_position..line_position + 4]
                .try_into()
                .expect("line vtable offset"),
        );
        let vtable_position = line_position
            .checked_sub(usize::try_from(vtable_distance).expect("positive vtable distance"))
            .expect("line vtable position");
        bytes[vtable_position + 4..vtable_position + 6].copy_from_slice(&0_u16.to_le_bytes());

        let error = wire::size_prefixed_root_as_road_editing_source_with_opts(
            &VerifierOptions {
                max_depth: MAX_SCHEMA_TABLE_DEPTH,
                max_tables: 64,
                max_apparent_size: bytes.len() * APPARENT_SIZE_MULTIPLIER,
                ignore_missing_null_terminator: false,
            },
            &bytes,
        )
        .expect_err("missing LineSegment.end");
        let (trace, _) = verifier_data_site(&error, bytes.len());
        assert_eq!(trace.len(), MAX_VERIFIER_TRACE_DETAILS);
        assert!(matches!(
            trace,
            [
                ErrorTraceDetail::UnionVariant { .. },
                ErrorTraceDetail::TableField { field_name: geometry, .. },
                ErrorTraceDetail::VectorElement { index: 0, .. },
                ErrorTraceDetail::TableField { field_name: segments, .. },
                ErrorTraceDetail::TableField { field_name: reference_line, .. },
                ErrorTraceDetail::VectorElement { index: 0, .. },
                ErrorTraceDetail::TableField { field_name: road_alignments, .. },
            ] if geometry.as_ref() == "geometry"
                && segments.as_ref() == "segments"
                && reference_line.as_ref() == "reference_line"
                && road_alignments.as_ref() == "road_alignments"
        ));

        // flatbuffers 25.12.19 uses Vec::new()+push for ErrorTrace. Lock the supported
        // MSRV/stable capacity-growth ceiling used by the precharge above.
        let mut capacity_probe = Vec::new();
        let mut previous_capacity = 0_usize;
        let mut maximum_coexisting_capacity = 0_usize;
        for detail in trace.iter().cloned() {
            capacity_probe.push(detail);
            let capacity = capacity_probe.capacity();
            if capacity != previous_capacity {
                maximum_coexisting_capacity =
                    maximum_coexisting_capacity.max(previous_capacity.saturating_add(capacity));
                previous_capacity = capacity;
            }
        }
        assert!(capacity_probe.capacity() <= MAX_VERIFIER_TRACE_DETAILS * 2);
        assert!(maximum_coexisting_capacity <= MAX_VERIFIER_TRACE_DETAILS * 3);
    }

    #[test]
    fn reader_failure_ceiling_preserves_rich_or_falls_back_without_allocation() {
        let expected = "roads/main";
        let rich = || source_error(RoadEditingSourceViolation::TruncatedFraming, expected, None);
        let budget = FailureLiveBudget {
            limit: u64::MAX,
            historical_peak_bytes: 0,
            current_live_bytes: 0,
            existing_context_bytes: 0,
            caller_transferred_context_bytes: 0,
        };
        let error = materialize_input_failure(budget, rich);
        assert_eq!(
            first_diagnostic(&error).code(),
            DiagnosticCode::InvalidRoadEditingSource
        );
        assert!(error.failure_retained_bytes() > 0);

        let rejected = materialize_input_failure(
            FailureLiveBudget {
                limit: INPUT_FAILURE_UPPER_BYTES - 1,
                ..budget
            },
            || panic!("rich failure must not be materialized beyond its ceiling"),
        );
        assert!(matches!(
            first_diagnostic(&rejected).payload(),
            DiagnosticPayload::CompileLimitExceeded {
                dimension: CompileLimitDimension::CompilerControlledLiveBytes,
                ..
            }
        ));
        assert!(first_diagnostic(&rejected).primary_location().is_none());
    }

    #[test]
    fn semantic_preflight_uses_verified_wire_fallback_for_invalid_declaration_key() {
        let limits = CompileLimits::p100_initial_v1();
        let buffer = source_buffer(&limits, "roads/main");
        let mut bytes = buffer.as_bytes().to_vec();
        let key_position = bytes
            .windows(b"frame".len())
            .position(|window| window == b"frame")
            .expect("canonical frame key");
        bytes[key_position] = b'_';
        let input = RoadEditingModuleInput::try_new("roads/main", &bytes, None).expect("input");

        let error = verify_source(input, &limits, 0, 0).expect_err("invalid declaration key");
        let location = first_diagnostic(&error)
            .primary_location()
            .and_then(SourceLocation::road_editing)
            .expect("semantic preflight location");
        assert!(matches!(
            location.document_identity(),
            RoadEditingDocumentIdentity::Verified(_)
        ));
        assert!(matches!(
            location.subject(),
            RoadEditingSubject::Wire {
                root_vector: RoadEditingRootVectorKind::CanonicalFrame,
                physical_index: 0,
                table: RoadEditingTableKind::CanonicalFrame,
            }
        ));
        assert!(location.property_path().is_none());
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
    fn malformed_canonical_reference_uses_the_root_wire_site() {
        let limits = CompileLimits::p100_initial_v1();
        let buffer = signal_source_buffer(&limits);
        let mut bytes = buffer.as_bytes().to_vec();
        let (reference_offset, reference_len) = {
            let input =
                RoadEditingModuleInput::try_new("road-editing", &bytes, None).expect("input");
            let verified = verify_source(input, &limits, 0, 0).expect("valid source");
            let phase = verified.root().signal_phases().get(0);
            let reference = phase.states().get(1).signal_group();
            (
                (reference.as_ptr() as usize)
                    .checked_sub(bytes.as_ptr() as usize)
                    .expect("reference belongs to source buffer"),
                reference.len(),
            )
        };
        bytes[reference_offset + reference_len - 1] = b'>';
        let input = RoadEditingModuleInput::try_new("road-editing", &bytes, None).expect("input");

        let error = verify_source(input, &limits, 0, 0).expect_err("malformed phase-state ref");
        let location = first_diagnostic(&error)
            .primary_location()
            .and_then(SourceLocation::road_editing)
            .expect("wire fallback location");
        assert!(matches!(
            location.subject(),
            RoadEditingSubject::Wire {
                root_vector: RoadEditingRootVectorKind::SignalPhase,
                physical_index: 0,
                table: RoadEditingTableKind::SignalPhase,
            }
        ));
        assert!(location.property_path().is_none());
    }

    #[test]
    fn malformed_import_does_not_claim_a_canonical_occurrence() {
        let limits = CompileLimits::p100_initial_v1();
        let buffer = signal_source_buffer(&limits);
        let mut bytes = buffer.as_bytes().to_vec();
        let (import_offset, import_len) = {
            let input =
                RoadEditingModuleInput::try_new("road-editing", &bytes, None).expect("input");
            let verified = verify_source(input, &limits, 0, 0).expect("valid source");
            let import = verified.root().module_header().imports().get(0);
            (
                (import.as_ptr() as usize)
                    .checked_sub(bytes.as_ptr() as usize)
                    .expect("import belongs to source buffer"),
                import.len(),
            )
        };
        bytes[import_offset + import_len - 1] = b'>';
        let input = RoadEditingModuleInput::try_new("road-editing", &bytes, None).expect("input");

        let error = verify_source(input, &limits, 0, 0).expect_err("malformed import");
        let location = first_diagnostic(&error)
            .primary_location()
            .and_then(SourceLocation::road_editing)
            .expect("module-header fallback location");
        assert!(matches!(
            location.subject(),
            RoadEditingSubject::ModuleHeader
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
        bytes[4..8].copy_from_slice(&5_u32.to_le_bytes());
        let input = RoadEditingModuleInput::try_new("roads/main", &bytes, None).expect("input");

        let error = verify_source(input, &limits, 0, 0).expect_err("malformed root offset");

        assert!(matches!(
            first_diagnostic(&error).payload(),
            DiagnosticPayload::InvalidRoadEditingSource {
                violation: RoadEditingSourceViolation::MalformedWire,
                ..
            }
        ));
        let Some(crate::SourceLocation::RoadEditing(location)) =
            first_diagnostic(&error).primary_location()
        else {
            panic!("unscoped verifier error must retain an input location");
        };
        assert!(matches!(
            location.subject(),
            RoadEditingSubject::ModuleHeader
        ));
        assert!(location.property_path().is_none());
        let range = location.byte_range().expect("checked direct byte range");
        assert!(range.length() > 0);
        assert!(
            u64::from(range.start().saturating_add(range.length()))
                <= u64::try_from(bytes.len()).unwrap()
        );
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
        assert!(location.property_path().is_none());
        let range = location.byte_range().expect("verified byte range");
        assert!(range.start() <= u32::try_from(key_position).unwrap());
        assert!(
            range.start().saturating_add(range.length()) > u32::try_from(key_position).unwrap()
        );
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
