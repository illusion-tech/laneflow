//! size-prefixed RoadEditingSource 到共同官方模块接入的唯一原子事务。

use std::sync::Arc;

use laneflow_road_editing_wire::generated::lane_flow::road_editing::v1 as wire;

use super::RoadEditingModuleInput;
use super::compile_geometry::{GeometryCompilationError, compile_authoring_geometry};
use super::geometry::NumericFreezeError;
use super::location::RoadEditingLocationFactory;
use super::lowering::{
    lower_aggregate_declarations, lower_independent_declarations, lower_owner_scoped_declarations,
    lower_road_alignments, lower_topology_authoring_declarations,
};
use super::preflight::RoadEditingPreflightCounts;
use super::reader::{VerifiedRoadEditingSource, verify_source};
use crate::declaration::{CanonicalPoint3F32Input, TypedAstDeclaration};
use crate::geometry_profile::GeometryCompilationProfiles;
use crate::module::{
    AdmittedOfficialModule, ImportRecord, ModuleResourceCounts, SOURCE_DOCUMENT_SET_DIGEST_VERSION,
    SourceDocumentDescriptor, SourceDocumentOrigin, SourceLanguage, SourceModuleDescriptor,
    TypedAstModule, freeze_source_documents, size_bytes, source_document_digest,
};
use crate::{
    CompilationUnitBuilder, CompileLimitDimension, CompileLimits, Diagnostic, DiagnosticBundle,
    GeometryAccuracyProfile, GeometryDirectionProfile, RoadEditingNumericViolation,
    RoadEditingPropertyStep, RoadEditingRelationKind, RoadEditingRelationOccurrence,
    RoadEditingSourceViolation, RoadEditingTableKind,
};

const ROAD_EDITING_FRONTEND_VERSION: u32 = 1;
// 每个 verifier table 对应的 retained Typed AST 请求上界。它覆盖外层 enum、嵌套 record、
// Box/Arc handle 与来源位置；字符串 payload、共享 context 和最终 f32 点表另行精确计量。
// G3 allocator evidence 会验证实际请求不超过该保守上界。
const RETAINED_TYPED_AST_RECORD_UPPER_BOUND_BYTES: u64 =
    core::mem::size_of::<TypedAstDeclaration>() as u64;
const ARC_ALLOCATION_HEADER_BYTES: u64 = (core::mem::size_of::<usize>() as u64).saturating_mul(2);
// v1 闭合属性路径表小于 512 项、每项最多四步。固定部分和输入相关的唯一
// string/canvas slots 分开，为 context 分配前的失败关闭预收费提供上界。
const LOCATION_CONTEXT_FIXED_UPPER_BOUND_BYTES: u64 = 64 * 1024;

#[derive(Clone, Copy)]
struct GeometryScratchAllowance {
    limit: u64,
    limiting_dimension: CompileLimitDimension,
    live_bytes_before_scratch: u64,
}

impl CompilationUnitBuilder {
    /// 原子验证并加入一份 size-prefixed `LFRE` 道路编辑来源。
    ///
    /// 成功后 builder 只保留拥有的描述符、Typed AST、已编译规范点表与共享来源位置
    /// context；不会保留 `input` 的 wire bytes 或 FlatBuffers view。任一失败都发生在共同
    /// admission 提交之前，因此同一 builder 可以立即重试。
    ///
    /// # Errors
    ///
    /// 当 framing/verifier/版本/语义/数值冻结失败，或候选累计资源、namespace、文档键
    /// 违反共同准入约束时返回结构化诊断。
    pub fn add_road_editing_module(
        &mut self,
        input: RoadEditingModuleInput<'_>,
    ) -> Result<&mut Self, DiagnosticBundle> {
        let limits = self.road_editing_limits().clone();
        let verified = verify_source(
            input,
            &limits,
            self.road_editing_source_bytes_already_admitted(),
            self.road_editing_typed_ast_records_already_admitted(),
        )?;
        precheck_accumulated_counts(self, &limits, &verified)?;
        let remaining_geometry_points = self.road_editing_remaining_geometry_points();
        let scratch_allowance = geometry_scratch_allowance(self, &limits, &verified);
        let admitted = lower_verified_source(
            verified,
            &limits,
            remaining_geometry_points,
            scratch_allowance,
        )?;
        self.admit_official_module(admitted)
    }
}

fn precheck_accumulated_counts(
    builder: &CompilationUnitBuilder,
    limits: &CompileLimits,
    verified: &VerifiedRoadEditingSource<'_>,
) -> Result<(), DiagnosticBundle> {
    let counts = verified.preflight_counts();
    let display_source = verified.input().display_source();
    let display_items = u64::from(display_source.is_some());
    let display_bytes =
        display_source.map_or(0, |value| u64::try_from(value.len()).unwrap_or(u64::MAX));
    let import_count =
        u64::try_from(verified.root().module_header().imports().len()).unwrap_or(u64::MAX);

    for (dimension, delta) in [
        (CompileLimitDimension::ModuleCount, 1),
        (CompileLimitDimension::ImportEdgeCount, import_count),
        (
            CompileLimitDimension::DeclarationCount,
            counts.declaration_count(),
        ),
        (
            CompileLimitDimension::TypedAstRecordCount,
            counts.typed_ast_record_count(),
        ),
        (
            CompileLimitDimension::ReferenceCount,
            counts.reference_count(),
        ),
        (
            CompileLimitDimension::RelationOccurrenceCount,
            counts.relation_occurrence_count(),
        ),
        (
            CompileLimitDimension::IdentityFieldOccurrenceCount,
            counts.identity_field_occurrence_count(),
        ),
        (CompileLimitDimension::SymbolCount, counts.symbol_count()),
        (
            CompileLimitDimension::StringItemCount,
            counts.string_item_count().saturating_add(display_items),
        ),
        (
            CompileLimitDimension::TotalStringBytes,
            counts.total_string_bytes().saturating_add(display_bytes),
        ),
        (
            CompileLimitDimension::ManeuverGateCount,
            counts.maneuver_gate_count(),
        ),
        (
            CompileLimitDimension::WaitingZoneCount,
            counts.waiting_zone_count(),
        ),
        (
            CompileLimitDimension::RouteOccurrenceCount,
            counts.route_occurrence_count(),
        ),
        (
            CompileLimitDimension::GeometryPointCount,
            counts.geometry_point_count(),
        ),
    ] {
        let observed = builder.already_admitted(dimension).saturating_add(delta);
        let limit = limits.value(dimension);
        if observed > limit {
            return Err(accumulated_limit_error(
                dimension, limit, observed, verified,
            ));
        }
    }
    if let Some(limit) = limits.source_document_count_limit() {
        let observed = builder
            .already_admitted(CompileLimitDimension::SourceDocumentCount)
            .saturating_add(1);
        if observed > limit {
            return Err(accumulated_limit_error(
                CompileLimitDimension::SourceDocumentCount,
                limit,
                observed,
                verified,
            ));
        }
    }

    let candidate_live_upper = preallocation_live_upper_bound(
        counts,
        builder.road_editing_remaining_geometry_points(),
        display_items,
        display_bytes,
        import_count,
    );
    let observed_live = builder
        .already_admitted(CompileLimitDimension::CompilerControlledLiveBytes)
        .saturating_add(candidate_live_upper);
    let live_limit = limits.value(CompileLimitDimension::CompilerControlledLiveBytes);
    if observed_live > live_limit {
        return Err(accumulated_limit_error(
            CompileLimitDimension::CompilerControlledLiveBytes,
            live_limit,
            observed_live,
            verified,
        ));
    }
    Ok(())
}

fn geometry_scratch_allowance(
    builder: &CompilationUnitBuilder,
    limits: &CompileLimits,
    verified: &VerifiedRoadEditingSource<'_>,
) -> GeometryScratchAllowance {
    let display_source = verified.input().display_source();
    let display_items = u64::from(display_source.is_some());
    let display_bytes =
        display_source.map_or(0, |value| u64::try_from(value.len()).unwrap_or(u64::MAX));
    let import_count =
        u64::try_from(verified.root().module_header().imports().len()).unwrap_or(u64::MAX);
    let candidate_live_upper = preallocation_live_upper_bound(
        verified.preflight_counts(),
        builder.road_editing_remaining_geometry_points(),
        display_items,
        display_bytes,
        import_count,
    );
    let live_bytes_before_scratch = builder
        .already_admitted(CompileLimitDimension::CompilerControlledLiveBytes)
        .saturating_add(candidate_live_upper);
    let live_headroom = limits
        .value(CompileLimitDimension::CompilerControlledLiveBytes)
        .saturating_sub(live_bytes_before_scratch);
    let stage_limit = limits.value(CompileLimitDimension::StageScratchBytes);
    if stage_limit <= live_headroom {
        GeometryScratchAllowance {
            limit: stage_limit,
            limiting_dimension: CompileLimitDimension::StageScratchBytes,
            live_bytes_before_scratch,
        }
    } else {
        GeometryScratchAllowance {
            limit: live_headroom,
            limiting_dimension: CompileLimitDimension::CompilerControlledLiveBytes,
            live_bytes_before_scratch,
        }
    }
}

fn accumulated_limit_error(
    dimension: CompileLimitDimension,
    limit: u64,
    observed: u64,
    verified: &VerifiedRoadEditingSource<'_>,
) -> DiagnosticBundle {
    let header = verified.root().module_header();
    DiagnosticBundle::single(Diagnostic::compile_limit_exceeded_at(
        dimension,
        limit,
        observed,
        Some(RoadEditingLocationFactory::verified_module_header(
            header.authoring_namespace_id(),
            header.source_document_key(),
        )),
        Some(header.authoring_namespace_id().into()),
    ))
}

fn lower_verified_source(
    verified: VerifiedRoadEditingSource<'_>,
    limits: &CompileLimits,
    remaining_geometry_points: u64,
    geometry_scratch_allowance: GeometryScratchAllowance,
) -> Result<AdmittedOfficialModule, DiagnosticBundle> {
    let root = verified.root();
    let counts = verified.preflight_counts();
    let locations = RoadEditingLocationFactory::from_verified_root(root);
    let profiles = geometry_profiles(root);
    let shared_namespace: Arc<str> = Arc::from(root.module_header().authoring_namespace_id());

    let alignments = lower_road_alignments(root, &locations, &shared_namespace).into_boxed_slice();
    let top_level_declaration_count = usize::try_from(counts.declaration_count())
        .unwrap_or(usize::MAX)
        .saturating_sub(root.authoring_lanes().len())
        .saturating_sub(root.signal_phases().len());
    let mut declarations = Vec::<TypedAstDeclaration>::with_capacity(top_level_declaration_count);
    declarations.extend(lower_independent_declarations(
        root,
        &locations,
        &shared_namespace,
    ));
    declarations.extend(lower_owner_scoped_declarations(
        root,
        &locations,
        &shared_namespace,
    ));
    declarations.extend(lower_topology_authoring_declarations(
        root,
        &locations,
        &shared_namespace,
    )?);
    declarations.extend(lower_aggregate_declarations(
        root,
        &locations,
        &shared_namespace,
    ));
    debug_assert_eq!(declarations.len(), top_level_declaration_count);

    let namespace = root.module_header().authoring_namespace_id();
    let geometry_usage = compile_authoring_geometry(
        namespace,
        alignments,
        &mut declarations,
        profiles.accuracy,
        profiles.direction,
        remaining_geometry_points,
        geometry_scratch_allowance.limit,
        geometry_scratch_allowance.limit,
    )
    .map_err(|error| {
        geometry_compilation_diagnostic(
            error,
            &verified,
            &locations,
            limits,
            geometry_scratch_allowance,
        )
    })?;
    debug_assert!(geometry_usage.peak_scratch_bytes <= geometry_scratch_allowance.limit);
    let geometry_point_count = geometry_usage.output_point_count;

    let header = root.module_header();
    let namespace = shared_namespace;
    let source_document_key: Arc<str> = Arc::from(header.source_document_key());
    let mut canonical_import_names: Vec<_> = header.imports().iter().collect();
    canonical_import_names.sort_unstable_by(|left, right| left.as_bytes().cmp(right.as_bytes()));
    let imports = canonical_import_names
        .into_iter()
        .enumerate()
        .map(|(index, value)| ImportRecord {
            namespace: Arc::from(value),
            span: locations.module_owner_local(
                RoadEditingRelationKind::Import,
                RoadEditingRelationOccurrence::CanonicalSetOrdinal(
                    u32::try_from(index).expect("compile limits bound import ordinals"),
                ),
                &[RoadEditingPropertyStep::TableField {
                    table: RoadEditingTableKind::ModuleHeader,
                    field_id: 2,
                }],
            ),
        })
        .collect::<Vec<_>>();
    let descriptor_imports = imports
        .iter()
        .map(|record| Arc::clone(&record.namespace))
        .collect::<Vec<_>>()
        .into_boxed_slice();

    let input = verified.input();
    let source_record_byte_len = u32::try_from(input.source_bytes().len())
        .expect("SourceBytesPerModule is represented by u32");
    let source_document = SourceDocumentDescriptor {
        source_document_key,
        source_document_digest: source_document_digest(input.source_bytes()),
        source_record_byte_len,
        authoring_namespace_id: Arc::clone(&namespace),
        origin: SourceDocumentOrigin::road_editing(input.display_source().map(Arc::from)),
    };
    let (source_documents, source_document_set_digest) =
        freeze_source_documents(&namespace, source_document, Vec::new());
    let provenance = header.provenance();
    let descriptor = SourceModuleDescriptor {
        authoring_namespace_id: namespace,
        source_language: SourceLanguage::RoadEditingSource,
        source_document_set_digest,
        source_document_set_digest_version: SOURCE_DOCUMENT_SET_DIGEST_VERSION,
        frontend_version: ROAD_EDITING_FRONTEND_VERSION,
        frontend_options_digest: provenance.frontend_options_digest().0,
        generator_build_id: Arc::from(provenance.generator_build_id()),
        parameters_and_inputs_digest: provenance.parameters_and_inputs_digest().0,
        random_seed: provenance.random_seed().map(|value| value.value()),
        provenance: Arc::from(provenance.description()),
        imports: descriptor_imports,
    };

    let display_string_items = u64::from(input.display_source().is_some());
    let display_string_bytes = input
        .display_source()
        .map_or(0, |value| u64::try_from(value.len()).unwrap_or(u64::MAX));
    let controlled_live_bytes = controlled_live_bytes(
        counts,
        geometry_point_count,
        locations.controlled_live_bytes(),
        display_string_items,
        display_string_bytes,
        u64::try_from(imports.len()).unwrap_or(u64::MAX),
    );
    debug_assert!(
        controlled_live_bytes
            <= preallocation_live_upper_bound(
                counts,
                remaining_geometry_points,
                display_string_items,
                display_string_bytes,
                u64::try_from(imports.len()).unwrap_or(u64::MAX),
            ),
        "retained RoadEditingSource payload exceeded its preallocation upper bound"
    );
    let typed_ast = TypedAstModule {
        descriptor,
        declaration_span: locations.module_header(),
        source_documents,
        imports: imports.into_boxed_slice(),
        geometry_profiles: Some(profiles),
        road_alignments: Box::default(),
        declarations: declarations.into_boxed_slice(),
    };
    Ok(AdmittedOfficialModule::new(
        typed_ast,
        ModuleResourceCounts {
            source_bytes: u64::from(source_record_byte_len),
            declaration_count: counts.declaration_count(),
            typed_ast_record_count: verified.typed_ast_record_count(),
            reference_count: counts.reference_count(),
            relation_occurrence_count: counts.relation_occurrence_count(),
            identity_field_occurrence_count: counts.identity_field_occurrence_count(),
            symbol_count: counts.symbol_count(),
            string_item_count: counts
                .string_item_count()
                .saturating_add(display_string_items),
            string_bytes: counts
                .total_string_bytes()
                .saturating_add(display_string_bytes),
            maneuver_gate_count: counts.maneuver_gate_count(),
            waiting_zone_count: counts.waiting_zone_count(),
            route_occurrence_count: counts.route_occurrence_count(),
            geometry_point_count,
            controlled_live_bytes,
        },
    ))
}

fn geometry_profiles(root: wire::RoadEditingSource<'_>) -> GeometryCompilationProfiles {
    let accuracy = match root.geometry_accuracy_profile() {
        wire::GeometryAccuracyProfile::Fine2Cm => GeometryAccuracyProfile::Fine2Cm,
        wire::GeometryAccuracyProfile::Balanced5Cm => GeometryAccuracyProfile::Balanced5Cm,
        wire::GeometryAccuracyProfile::Compact10Cm => GeometryAccuracyProfile::Compact10Cm,
        _ => unreachable!("semantic preflight accepts only closed accuracy profiles"),
    };
    let direction = match root.geometry_direction_profile() {
        wire::GeometryDirectionProfile::Smooth1Deg => GeometryDirectionProfile::Smooth1Deg,
        wire::GeometryDirectionProfile::Balanced2Deg => GeometryDirectionProfile::Balanced2Deg,
        wire::GeometryDirectionProfile::Compact5Deg => GeometryDirectionProfile::Compact5Deg,
        _ => unreachable!("semantic preflight accepts only closed direction profiles"),
    };
    GeometryCompilationProfiles {
        accuracy,
        direction,
    }
}

fn controlled_live_bytes(
    counts: RoadEditingPreflightCounts,
    geometry_point_count: u64,
    location_context_bytes: u64,
    display_string_items: u64,
    display_string_bytes: u64,
    import_count: u64,
) -> u64 {
    let string_items = counts
        .string_item_count()
        .saturating_add(display_string_items);
    counts
        .typed_ast_record_count()
        .saturating_mul(RETAINED_TYPED_AST_RECORD_UPPER_BOUND_BYTES)
        .saturating_add(counts.total_string_bytes())
        .saturating_add(display_string_bytes)
        .saturating_add(string_items.saturating_mul(ARC_ALLOCATION_HEADER_BYTES))
        .saturating_add(size_bytes::<CanonicalPoint3F32Input>(geometry_point_count))
        .saturating_add(location_context_bytes)
        .saturating_add(size_bytes::<SourceDocumentDescriptor>(1))
        .saturating_add(size_bytes::<ImportRecord>(import_count))
}

fn preallocation_live_upper_bound(
    counts: RoadEditingPreflightCounts,
    remaining_geometry_points: u64,
    display_string_items: u64,
    display_string_bytes: u64,
    import_count: u64,
) -> u64 {
    let string_items = counts
        .string_item_count()
        .saturating_add(display_string_items);
    let string_bytes = counts
        .total_string_bytes()
        .saturating_add(display_string_bytes);
    counts
        .typed_ast_record_count()
        .saturating_mul(RETAINED_TYPED_AST_RECORD_UPPER_BOUND_BYTES)
        // Typed AST/descriptor payload 与 context 唯一 token payload 各一份。
        .saturating_add(string_bytes.saturating_mul(2))
        // Typed AST Arc allocation，加 context Arc slot 与 allocation。
        .saturating_add(
            string_items.saturating_mul(
                ARC_ALLOCATION_HEADER_BYTES
                    .saturating_mul(2)
                    .saturating_add(
                        u64::try_from(core::mem::size_of::<Arc<str>>()).unwrap_or(u64::MAX),
                    ),
            ),
        )
        .saturating_add(size_bytes::<CanonicalPoint3F32Input>(
            remaining_geometry_points,
        ))
        .saturating_add(LOCATION_CONTEXT_FIXED_UPPER_BOUND_BYTES)
        .saturating_add(size_bytes::<SourceDocumentDescriptor>(1))
        .saturating_add(size_bytes::<ImportRecord>(import_count))
}

fn geometry_compilation_diagnostic(
    error: GeometryCompilationError,
    verified: &VerifiedRoadEditingSource<'_>,
    locations: &RoadEditingLocationFactory,
    limits: &CompileLimits,
    scratch_allowance: GeometryScratchAllowance,
) -> DiagnosticBundle {
    if let GeometryCompilationError::ScratchLimit {
        limit: error_limit,
        observed,
    } = error
    {
        debug_assert_eq!(error_limit, scratch_allowance.limit);
        let (limit, observed) = match scratch_allowance.limiting_dimension {
            CompileLimitDimension::StageScratchBytes => (
                limits.value(CompileLimitDimension::StageScratchBytes),
                observed,
            ),
            CompileLimitDimension::CompilerControlledLiveBytes => (
                limits.value(CompileLimitDimension::CompilerControlledLiveBytes),
                scratch_allowance
                    .live_bytes_before_scratch
                    .saturating_add(observed),
            ),
            _ => unreachable!("geometry scratch has exactly two limiting dimensions"),
        };
        return DiagnosticBundle::single(Diagnostic::compile_limit_exceeded_at(
            scratch_allowance.limiting_dimension,
            limit,
            observed,
            Some(locations.module_header()),
            Some(
                verified
                    .root()
                    .module_header()
                    .authoring_namespace_id()
                    .into(),
            ),
        ));
    }
    let GeometryCompilationError::Numeric(error) = error else {
        unreachable!("scratch limit was handled above")
    };
    if error == NumericFreezeError::GeometryPointLimit {
        let limit = limits.value(CompileLimitDimension::GeometryPointCount);
        return DiagnosticBundle::single(Diagnostic::compile_limit_exceeded_at(
            CompileLimitDimension::GeometryPointCount,
            limit,
            limit.saturating_add(1),
            Some(locations.module_header()),
            Some(
                verified
                    .root()
                    .module_header()
                    .authoring_namespace_id()
                    .into(),
            ),
        ));
    }
    if error == NumericFreezeError::StationRowLimit {
        let limit = limits.value(CompileLimitDimension::StageScratchBytes);
        return DiagnosticBundle::single(Diagnostic::compile_limit_exceeded_at(
            CompileLimitDimension::StageScratchBytes,
            limit,
            limit.saturating_add(1),
            Some(locations.module_header()),
            Some(
                verified
                    .root()
                    .module_header()
                    .authoring_namespace_id()
                    .into(),
            ),
        ));
    }
    let violation = match error {
        NumericFreezeError::NonFinite => RoadEditingNumericViolation::NonFinite,
        NumericFreezeError::DivisionByZero => RoadEditingNumericViolation::DivisionByZero,
        NumericFreezeError::SquareRootDomain => RoadEditingNumericViolation::SquareRootDomain,
        NumericFreezeError::HorizontalDerivativeZero => {
            RoadEditingNumericViolation::HorizontalDerivativeZero
        }
        NumericFreezeError::HorizontalDerivativeNotProvenNonZero => {
            RoadEditingNumericViolation::HorizontalDerivativeNotProvenNonZero
        }
        NumericFreezeError::CoordinateOutOfRange => {
            RoadEditingNumericViolation::CoordinateOutOfRange
        }
        NumericFreezeError::ApproximationNotConverged => {
            RoadEditingNumericViolation::ApproximationNotConverged
        }
        NumericFreezeError::StationOutOfRange => RoadEditingNumericViolation::StationOutOfRange,
        NumericFreezeError::GeometryTopologyMismatch => {
            RoadEditingNumericViolation::GeometryTopologyMismatch
        }
        NumericFreezeError::SourceJoinGapExceeded => {
            RoadEditingNumericViolation::SourceJoinGapExceeded
        }
        NumericFreezeError::DegenerateCanonicalSegment => {
            RoadEditingNumericViolation::DegenerateCanonicalSegment
        }
        NumericFreezeError::DirectionDiscontinuity => {
            RoadEditingNumericViolation::DirectionDiscontinuity
        }
        NumericFreezeError::GeometryPointLimit | NumericFreezeError::StationRowLimit => {
            unreachable!("handled above")
        }
    };
    let input = verified.input();
    DiagnosticBundle::single(Diagnostic::invalid_road_editing_source_at(
        RoadEditingSourceViolation::NumericFreeze(violation),
        Some("roadEditingSource.authoringGeometry"),
        input.expected_source_document_key(),
        Some(input.expected_source_document_key()),
        Some(locations.module_header()),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Compiler;
    use crate::road_editing::{
        AuthoringLaneInput, AuthoringLaneReference, CanonicalFrameInput, CanonicalFrameReference,
        FacilityBandInput, FacilityBandReference, LaneEdgeInput, LaneEdgeReference,
        LinearWidthProfile, RoadAlignmentInput, RoadAlignmentReference, RoadCorridorInput,
        RoadCorridorReference, RoadEditingCorridorElement, RoadEditingCurveProgram,
        RoadEditingCurveSegment, RoadEditingDeclaration, RoadEditingLaneDirection,
        RoadEditingModuleHeader, RoadEditingPoint3, RoadEditingProvenance,
        RoadEditingSourceModuleBuilder, RoadEditingSourceWriter, RoadEditingStationEnd,
        RoadSectionInput, RoadSectionReference,
    };
    use crate::{RoadEditingDocumentIdentity, SourceLocation};

    fn source_buffer(
        limits: &CompileLimits,
        namespace: &str,
        document_key: &str,
    ) -> super::super::OwnedRoadEditingSourceBuffer {
        let header = RoadEditingModuleHeader::try_new(
            namespace,
            document_key,
            Vec::new(),
            RoadEditingProvenance::direct("editor save").unwrap(),
        )
        .unwrap();
        let mut builder = RoadEditingSourceModuleBuilder::new(
            header,
            GeometryAccuracyProfile::Balanced5Cm,
            GeometryDirectionProfile::Balanced2Deg,
            limits,
        )
        .unwrap();
        builder
            .add_declaration(RoadEditingDeclaration::CanonicalFrame(
                CanonicalFrameInput::try_new("frame").unwrap(),
            ))
            .unwrap();
        RoadEditingSourceWriter::new(limits)
            .write(builder.finish().unwrap())
            .unwrap()
    }

    fn degenerate_geometry_buffer(
        limits: &CompileLimits,
    ) -> super::super::OwnedRoadEditingSourceBuffer {
        let header = RoadEditingModuleHeader::try_new(
            "city",
            "roads/main",
            Vec::new(),
            RoadEditingProvenance::direct("editor save").unwrap(),
        )
        .unwrap();
        let mut builder = RoadEditingSourceModuleBuilder::new(
            header,
            GeometryAccuracyProfile::Balanced5Cm,
            GeometryDirectionProfile::Balanced2Deg,
            limits,
        )
        .unwrap();
        builder
            .add_declaration(RoadEditingDeclaration::CanonicalFrame(
                CanonicalFrameInput::try_new("frame").unwrap(),
            ))
            .unwrap();
        let point = RoadEditingPoint3::try_new(0.0, 0.0, 0.0).unwrap();
        let curve =
            RoadEditingCurveProgram::try_new(point, vec![RoadEditingCurveSegment::line(point)])
                .unwrap();
        builder
            .add_alignment(
                RoadAlignmentInput::try_new(
                    "alignment",
                    CanonicalFrameReference::local("frame").unwrap(),
                    curve,
                )
                .unwrap(),
            )
            .unwrap();
        RoadEditingSourceWriter::new(limits)
            .write(builder.finish().unwrap())
            .unwrap()
    }

    fn complete_geometry_buffer(
        limits: &CompileLimits,
    ) -> super::super::OwnedRoadEditingSourceBuffer {
        let header = RoadEditingModuleHeader::try_new(
            "city",
            "road-editing",
            Vec::new(),
            RoadEditingProvenance::direct("editor save").unwrap(),
        )
        .unwrap();
        let mut builder = RoadEditingSourceModuleBuilder::new(
            header,
            GeometryAccuracyProfile::Balanced5Cm,
            GeometryDirectionProfile::Balanced2Deg,
            limits,
        )
        .unwrap();
        let corridor = RoadCorridorReference::local("corridor").unwrap();
        let section =
            RoadSectionReference::owner_scoped(vec!["corridor".into()], "section").unwrap();
        let lane =
            AuthoringLaneReference::owner_scoped(vec!["corridor".into(), "section".into()], "lane")
                .unwrap();
        let facility =
            FacilityBandReference::owner_scoped(vec!["corridor".into()], "facility").unwrap();
        let edge = LaneEdgeReference::local("edge").unwrap();
        let curve = RoadEditingCurveProgram::try_new(
            RoadEditingPoint3::try_new(0.0, 0.0, 0.0).unwrap(),
            vec![RoadEditingCurveSegment::line(
                RoadEditingPoint3::try_new(10.0, 0.0, 0.0).unwrap(),
            )],
        )
        .unwrap();
        builder
            .add_alignment(
                RoadAlignmentInput::try_new(
                    "alignment",
                    CanonicalFrameReference::local("frame").unwrap(),
                    curve,
                )
                .unwrap(),
            )
            .unwrap();
        for declaration in [
            RoadEditingDeclaration::CanonicalFrame(CanonicalFrameInput::try_new("frame").unwrap()),
            RoadEditingDeclaration::RoadCorridor(
                RoadCorridorInput::try_new(
                    "corridor",
                    RoadAlignmentReference::try_new("alignment").unwrap(),
                    0.0,
                    RoadEditingStationEnd::AlignmentEnd,
                    section.clone(),
                    lane.clone(),
                    vec![
                        RoadEditingCorridorElement::RoadSection(section.clone()),
                        RoadEditingCorridorElement::FacilityBand(facility),
                    ],
                )
                .unwrap(),
            ),
            RoadEditingDeclaration::RoadSection(
                RoadSectionInput::try_new("section", "road", vec![lane.clone()], corridor.clone())
                    .unwrap(),
            ),
            RoadEditingDeclaration::AuthoringLane(
                AuthoringLaneInput::try_new(
                    "lane",
                    edge.clone(),
                    RoadEditingLaneDirection::Forward,
                    LinearWidthProfile::try_new(3.5, 3.5).unwrap(),
                    None,
                    section,
                )
                .unwrap(),
            ),
            RoadEditingDeclaration::LaneEdge(
                LaneEdgeInput::try_new("edge", 10.0, Vec::new(), None).unwrap(),
            ),
            RoadEditingDeclaration::FacilityBand(
                FacilityBandInput::try_new(
                    "facility",
                    "median",
                    LinearWidthProfile::try_new(1.0, 1.0).unwrap(),
                    corridor,
                )
                .unwrap(),
            ),
        ] {
            builder.add_declaration(declaration).unwrap();
        }
        RoadEditingSourceWriter::new(limits)
            .write(builder.finish().unwrap())
            .unwrap()
    }

    #[test]
    fn admission_owns_results_without_retaining_wire_bytes() {
        let limits = CompileLimits::p100_initial_v1();
        let buffer = source_buffer(&limits, "city", "roads/main");
        let expected_digest = source_document_digest(buffer.as_bytes());
        let mut builder = CompilationUnitBuilder::new(limits);
        let input =
            RoadEditingModuleInput::try_new("roads/main", buffer.as_bytes(), Some("save slot 7"))
                .unwrap();
        builder.add_road_editing_module(input).unwrap();
        drop(buffer);

        let unit = builder.build().unwrap();
        assert_eq!(unit.module_count(), 1);
        let descriptor = unit.module_descriptors().next().unwrap();
        assert_eq!(
            descriptor.source_language(),
            SourceLanguage::RoadEditingSource
        );
        assert_eq!(descriptor.authoring_namespace_id(), "city");
        let document = unit.source_document_descriptors().next().unwrap();
        assert_eq!(document.source_document_digest(), &expected_digest);
        assert_eq!(document.origin().display_source(), Some("save slot 7"));
    }

    #[test]
    fn malformed_candidate_does_not_pollute_builder_and_retry_succeeds() {
        let limits = CompileLimits::p100_initial_v1();
        let mut builder = CompilationUnitBuilder::new(limits.clone());
        let malformed = [0_u8; 12];
        let input = RoadEditingModuleInput::try_new("bad", &malformed, None).unwrap();
        let Err(error) = builder.add_road_editing_module(input) else {
            panic!("malformed framing must fail");
        };
        let Some(SourceLocation::RoadEditing(location)) = error.diagnostics()[0].primary_location()
        else {
            panic!("wire failure must retain an input road-editing location");
        };
        assert!(matches!(
            location.document_identity(),
            RoadEditingDocumentIdentity::Input(_)
        ));

        let buffer = source_buffer(&limits, "city", "roads/main");
        builder
            .add_road_editing_module(
                RoadEditingModuleInput::try_new("roads/main", buffer.as_bytes(), None).unwrap(),
            )
            .unwrap();
        assert_eq!(builder.build().unwrap().module_count(), 1);
    }

    #[test]
    fn numeric_freeze_failure_is_structured_and_builder_remains_reusable() {
        let limits = CompileLimits::p100_initial_v1();
        let invalid = degenerate_geometry_buffer(&limits);
        let mut builder = CompilationUnitBuilder::new(limits.clone());
        let Err(error) = builder.add_road_editing_module(
            RoadEditingModuleInput::try_new("roads/main", invalid.as_bytes(), None).unwrap(),
        ) else {
            panic!("zero-length line must fail numeric freeze");
        };
        assert!(
            matches!(
                error.diagnostics()[0].payload(),
                crate::DiagnosticPayload::InvalidRoadEditingSource {
                    violation: RoadEditingSourceViolation::NumericFreeze(
                        RoadEditingNumericViolation::ApproximationNotConverged
                    ),
                    ..
                }
            ),
            "unexpected diagnostic: {:?}",
            error.diagnostics()[0].payload()
        );
        let Some(SourceLocation::RoadEditing(location)) = error.diagnostics()[0].primary_location()
        else {
            panic!("numeric failure must retain a verified road-editing location");
        };
        assert!(matches!(
            location.document_identity(),
            RoadEditingDocumentIdentity::Verified(_)
        ));

        let valid = source_buffer(&limits, "city", "roads/main");
        builder
            .add_road_editing_module(
                RoadEditingModuleInput::try_new("roads/main", valid.as_bytes(), None).unwrap(),
            )
            .unwrap();
        assert_eq!(builder.build().unwrap().module_count(), 1);
    }

    #[test]
    fn accumulated_source_limit_failure_keeps_the_first_module() {
        let broad = CompileLimits::p100_initial_v1();
        let first = source_buffer(&broad, "city/a", "roads/a");
        let second = source_buffer(&broad, "city/b", "roads/b");
        let total_limit = u32::try_from(first.as_bytes().len() + second.as_bytes().len() - 1)
            .expect("small fixture");
        let limits = broad.with_test_source_byte_limits(10_000, total_limit);
        let mut builder = CompilationUnitBuilder::new(limits);
        builder
            .add_road_editing_module(
                RoadEditingModuleInput::try_new("roads/a", first.as_bytes(), None).unwrap(),
            )
            .unwrap();
        assert!(
            builder
                .add_road_editing_module(
                    RoadEditingModuleInput::try_new("roads/b", second.as_bytes(), None).unwrap(),
                )
                .is_err(),
            "second source must exceed accumulated bytes"
        );

        let unit = builder.build().unwrap();
        assert_eq!(unit.module_count(), 1);
        assert_eq!(
            unit.module_descriptors()
                .next()
                .unwrap()
                .authoring_namespace_id(),
            "city/a"
        );
    }

    #[test]
    fn common_admission_failure_does_not_commit_candidate_indexes() {
        let limits = CompileLimits::p100_initial_v1();
        let first = source_buffer(&limits, "city", "roads/a");
        let duplicate = source_buffer(&limits, "city", "roads/b");
        let retry = source_buffer(&limits, "city/other", "roads/b");
        let mut builder = CompilationUnitBuilder::new(limits);
        builder
            .add_road_editing_module(
                RoadEditingModuleInput::try_new("roads/a", first.as_bytes(), None).unwrap(),
            )
            .unwrap();
        assert!(
            builder
                .add_road_editing_module(
                    RoadEditingModuleInput::try_new("roads/b", duplicate.as_bytes(), None).unwrap(),
                )
                .is_err(),
            "duplicate namespace must fail in common admission"
        );
        builder
            .add_road_editing_module(
                RoadEditingModuleInput::try_new("roads/b", retry.as_bytes(), None).unwrap(),
            )
            .unwrap();

        let unit = builder.build().unwrap();
        assert_eq!(unit.module_count(), 2);
        assert_eq!(unit.source_document_count(), 2);
    }

    #[test]
    fn road_editing_add_order_does_not_change_canonical_module_order() {
        fn order(reverse: bool) -> Vec<String> {
            let limits = CompileLimits::p100_initial_v1();
            let first = source_buffer(&limits, "city/a", "roads/a");
            let second = source_buffer(&limits, "city/b", "roads/b");
            let inputs = if reverse {
                [
                    ("roads/b", second.as_bytes()),
                    ("roads/a", first.as_bytes()),
                ]
            } else {
                [
                    ("roads/a", first.as_bytes()),
                    ("roads/b", second.as_bytes()),
                ]
            };
            let mut builder = CompilationUnitBuilder::new(limits);
            for (document_key, bytes) in inputs {
                builder
                    .add_road_editing_module(
                        RoadEditingModuleInput::try_new(document_key, bytes, None).unwrap(),
                    )
                    .unwrap();
            }
            builder
                .build()
                .unwrap()
                .module_descriptors()
                .map(|descriptor| descriptor.authoring_namespace_id().to_owned())
                .collect()
        }

        assert_eq!(order(false), order(true));
        assert_eq!(order(false), ["city/a", "city/b"]);
    }

    #[test]
    fn complete_authoring_fixture_reaches_the_common_compiler_pipeline() {
        let limits = CompileLimits::p100_initial_v1();
        let buffer = complete_geometry_buffer(&limits);
        let mut builder = CompilationUnitBuilder::new(limits);
        builder
            .add_road_editing_module(
                RoadEditingModuleInput::try_new("road-editing", buffer.as_bytes(), None).unwrap(),
            )
            .unwrap();
        let unit = builder.build().unwrap();

        let output = Compiler::new().compile(unit).unwrap();
        assert_eq!(output.lir().lane_edges().count(), 1);
        assert_eq!(output.lir().facility_bands().count(), 1);
        assert_eq!(
            output
                .lir()
                .facility_bands()
                .filter(|band| band.spatial_geometry().is_some())
                .count(),
            1
        );
    }

    #[test]
    fn geometry_scratch_limit_fails_before_candidate_commit() {
        let broad = CompileLimits::p100_initial_v1();
        let buffer = complete_geometry_buffer(&broad);
        let limits = broad.with_test_admission_limit(CompileLimitDimension::StageScratchBytes, 1);
        let mut builder = CompilationUnitBuilder::new(limits);
        let Err(error) = builder.add_road_editing_module(
            RoadEditingModuleInput::try_new("road-editing", buffer.as_bytes(), None).unwrap(),
        ) else {
            panic!("geometry scratch limit must reject the candidate");
        };
        assert!(matches!(
            error.diagnostics()[0].payload(),
            crate::DiagnosticPayload::CompileLimitExceeded {
                dimension: CompileLimitDimension::StageScratchBytes,
                limit: 1,
                observed,
            } if *observed > 1
        ));
        assert_eq!(
            builder.already_admitted(CompileLimitDimension::ModuleCount),
            0
        );
    }
}
