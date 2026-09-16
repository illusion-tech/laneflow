//! verifier 后、任何领域分配前的道路编辑来源语义预检。

use laneflow_road_editing_wire::generated::lane_flow::road_editing::v1 as wire;
use laneflow_road_editing_wire::runtime::{ForwardsUOffset, Vector};
use laneflow_static_contract::{
    CANONICAL_POINT_COMPONENT_MAX_METERS, CANONICAL_POINT_COMPONENT_MIN_METERS, EntityKind,
    MAX_CONFLICT_ZONE_REGION_RING_POINTS, MAX_LANE_EDGE_LENGTH_MM, MAX_MIN_GAP_MM,
    MAX_PARKING_LATERAL_OFFSET_ABS_MM, MAX_SPEED_MM_S, MAX_VEHICLE_LENGTH_MM,
    MIN_PARKING_LATERAL_OFFSET_ABS_MM, MIN_SPEED_MM_S, MIN_VEHICLE_LENGTH_MM,
    PARKING_ANCHOR_ENDPOINT_CLEARANCE_MM,
};

use super::model::{
    DIRECT_FRONTEND_OPTIONS_DIGEST, DIRECT_GENERATOR_BUILD_ID, DIRECT_INPUTS_DIGEST,
};
use super::rules::{
    accel_violation, heading_violation, inclusive_range_violation,
    millimetre_i32_abs_range_violation, millimetre_range_violation, non_negative_violation,
    positive_violation, time_headway_violation, token_violation, validate_wire_reference,
    visible_ascii_violation,
};
use crate::declaration::{MAX_PORTABLE_SIGNAL_TIME_MS, facility_kind_category};
use crate::{
    CompileLimitDimension, CompileLimits, Diagnostic, DiagnosticBundle, FacilityKindCategory,
    RoadEditingInputViolation, RoadEditingSourceViolation, SourceTextViolation,
};

type StringVector<'a> = Vector<'a, ForwardsUOffset<&'a str>>;
mod owner;
mod policy;
mod scratch;
use scratch::PreflightScratch;
#[cfg(test)]
mod benchmark;
#[cfg(test)]
mod index_tests;

/// 道路编辑来源语义预检的用量计数；在任何领域分配前累计记录、引用与字符串负载。
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct RoadEditingPreflightCounts {
    declaration_count: u64,
    typed_ast_record_count: u64,
    reference_count: u64,
    external_namespace_reference_count: u64,
    relation_occurrence_count: u64,
    identity_field_occurrence_count: u64,
    maneuver_gate_count: u64,
    waiting_zone_count: u64,
    authoring_point_count: u64,
    conflict_region_point_count: u64,
    symbol_count: u64,
    string_item_count: u64,
    total_string_bytes: u64,
    preflight_peak_scratch_bytes: u64,
}

impl RoadEditingPreflightCounts {
    /// 返回预检借用索引的最大共存请求字节数。
    pub(crate) const fn preflight_peak_scratch_bytes(self) -> u64 {
        self.preflight_peak_scratch_bytes
    }

    /// 返回声明计数。
    pub(crate) const fn declaration_count(self) -> u64 {
        self.declaration_count
    }

    /// 返回 Typed AST 来源记录计数。
    pub(crate) const fn typed_ast_record_count(self) -> u64 {
        self.typed_ast_record_count
    }

    /// 返回引用计数。
    pub(crate) const fn reference_count(self) -> u64 {
        self.reference_count
    }

    /// 返回指向外部编制命名空间的引用计数。
    pub(crate) const fn external_namespace_reference_count(self) -> u64 {
        self.external_namespace_reference_count
    }

    /// 返回关系出现项计数。
    pub(crate) const fn relation_occurrence_count(self) -> u64 {
        self.relation_occurrence_count
    }

    /// 返回身份字段出现项计数。
    pub(crate) const fn identity_field_occurrence_count(self) -> u64 {
        self.identity_field_occurrence_count
    }

    /// 返回机动门计数。
    pub(crate) const fn maneuver_gate_count(self) -> u64 {
        self.maneuver_gate_count
    }

    /// 返回等待区计数。
    pub(crate) const fn waiting_zone_count(self) -> u64 {
        self.waiting_zone_count
    }

    /// 返回冲突区空间区域的点计数。
    pub(crate) const fn conflict_region_point_count(self) -> u64 {
        self.conflict_region_point_count
    }

    /// 返回符号计数。
    pub(crate) const fn symbol_count(self) -> u64 {
        self.symbol_count
    }

    /// 返回字符串条目计数。
    pub(crate) const fn string_item_count(self) -> u64 {
        self.string_item_count
    }

    /// 返回字符串总字节数。
    pub(crate) const fn total_string_bytes(self) -> u64 {
        self.total_string_bytes
    }

    fn charge_token(
        &mut self,
        value: &str,
        field: &'static str,
        limits: &CompileLimits,
        expected_key: &str,
    ) -> Result<(), DiagnosticBundle> {
        let limit = limits.value(CompileLimitDimension::SingleStringBytes);
        if let Some(violation) = token_violation(value, limit, true) {
            return Err(semantic_error(field, violation, expected_key));
        }
        self.string_item_count = self.string_item_count.saturating_add(1);
        self.total_string_bytes = self
            .total_string_bytes
            .saturating_add(u64::try_from(value.len()).unwrap_or(u64::MAX));
        Ok(())
    }

    fn charge_visible_ascii(
        &mut self,
        value: &str,
        field: &'static str,
        limits: &CompileLimits,
        expected_key: &str,
    ) -> Result<(), DiagnosticBundle> {
        let limit = limits.value(CompileLimitDimension::SingleStringBytes);
        if let Some(violation) = visible_ascii_violation(value, limit) {
            return Err(semantic_error(field, violation, expected_key));
        }
        self.string_item_count = self.string_item_count.saturating_add(1);
        self.total_string_bytes = self
            .total_string_bytes
            .saturating_add(u64::try_from(value.len()).unwrap_or(u64::MAX));
        Ok(())
    }

    fn charge_non_empty_text(
        &mut self,
        value: &str,
        field: &'static str,
        limits: &CompileLimits,
        expected_key: &str,
    ) -> Result<(), DiagnosticBundle> {
        let limit = limits.value(CompileLimitDimension::SingleStringBytes);
        let observed = u64::try_from(value.len()).unwrap_or(u64::MAX);
        let violation = if value.is_empty() {
            Some(RoadEditingInputViolation::InvalidText(
                SourceTextViolation::Empty,
            ))
        } else if observed > limit {
            Some(RoadEditingInputViolation::InvalidText(
                SourceTextViolation::TooLong { limit, observed },
            ))
        } else {
            None
        };
        if let Some(violation) = violation {
            return Err(semantic_error(field, violation, expected_key));
        }
        self.string_item_count = self.string_item_count.saturating_add(1);
        self.total_string_bytes = self.total_string_bytes.saturating_add(observed);
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn charge_reference(
        &mut self,
        value: &str,
        component_count: u8,
        allow_qualified: bool,
        field: &'static str,
        current_namespace: &str,
        imports: StringVector<'_>,
        limits: &CompileLimits,
        expected_key: &str,
    ) -> Result<(), DiagnosticBundle> {
        let parsed = validate_wire_reference(value, component_count, allow_qualified)
            .map_err(|violation| semantic_error(field, violation, expected_key))?;
        if let Some(namespace) = parsed.namespace() {
            if namespace == current_namespace || !imports.iter().any(|import| import == namespace) {
                return Err(semantic_error(
                    field,
                    RoadEditingInputViolation::InvalidCombination,
                    expected_key,
                ));
            }
            self.external_namespace_reference_count =
                self.external_namespace_reference_count.saturating_add(1);
        }
        if let Some(namespace) = parsed.namespace() {
            self.charge_token(namespace, field, limits, expected_key)?;
        }
        for component in parsed.key_components() {
            self.charge_token(component, field, limits, expected_key)?;
        }
        self.reference_count = self.reference_count.saturating_add(1);
        Ok(())
    }

    fn charge_canvas(
        &mut self,
        value: Option<&str>,
        limits: &CompileLimits,
        expected_key: &str,
    ) -> Result<(), DiagnosticBundle> {
        if let Some(value) = value {
            self.charge_token(value, "canvasSelection", limits, expected_key)?;
        }
        Ok(())
    }

    fn charge_declaration(&mut self, kind: EntityKind) {
        self.declaration_count = self.declaration_count.saturating_add(1);
        self.symbol_count = self.symbol_count.saturating_add(1);
        self.typed_ast_record_count = self.typed_ast_record_count.saturating_add(1);
        self.identity_field_occurrence_count = self
            .identity_field_occurrence_count
            .saturating_add(u64::try_from(kind.required_tags().len()).unwrap_or(u64::MAX));
    }

    fn charge_relation(&mut self, count: usize) {
        self.relation_occurrence_count = self
            .relation_occurrence_count
            .saturating_add(u64::try_from(count).unwrap_or(u64::MAX));
    }

    fn require_relation_capacity(
        &self,
        count: usize,
        limits: &CompileLimits,
    ) -> Result<(), DiagnosticBundle> {
        let observed = self
            .relation_occurrence_count
            .saturating_add(u64::try_from(count).unwrap_or(u64::MAX));
        let limit = limits.value(CompileLimitDimension::RelationOccurrenceCount);
        if observed > limit {
            return Err(limit_error(
                CompileLimitDimension::RelationOccurrenceCount,
                limit,
                observed,
            ));
        }
        Ok(())
    }

    fn validate(self, limits: &CompileLimits) -> Result<Self, DiagnosticBundle> {
        for (dimension, observed) in [
            (
                CompileLimitDimension::DeclarationCount,
                self.declaration_count,
            ),
            (
                CompileLimitDimension::TypedAstRecordCount,
                self.typed_ast_record_count,
            ),
            (CompileLimitDimension::ReferenceCount, self.reference_count),
            (
                CompileLimitDimension::RelationOccurrenceCount,
                self.relation_occurrence_count,
            ),
            (
                CompileLimitDimension::IdentityFieldOccurrenceCount,
                self.identity_field_occurrence_count,
            ),
            (
                CompileLimitDimension::ManeuverGateCount,
                self.maneuver_gate_count,
            ),
            (
                CompileLimitDimension::WaitingZoneCount,
                self.waiting_zone_count,
            ),
            (CompileLimitDimension::SymbolCount, self.symbol_count),
            (
                CompileLimitDimension::StringItemCount,
                self.string_item_count,
            ),
            (
                CompileLimitDimension::TotalStringBytes,
                self.total_string_bytes,
            ),
        ] {
            let limit = limits.value(dimension);
            if observed > limit {
                return Err(limit_error(dimension, limit, observed));
            }
        }
        Ok(self)
    }
}

/// 对已验证根表执行完整语义预检：校验各字段取值并累计用量计数，在任何领域分配前失败关闭。
pub(crate) fn preflight_source(
    root: wire::RoadEditingSource<'_>,
    limits: &CompileLimits,
    expected_key: &str,
    admitted_live_bytes: u64,
) -> Result<RoadEditingPreflightCounts, DiagnosticBundle> {
    let scratch = PreflightScratch::new(limits, admitted_live_bytes);
    let limits = &scratch;
    let header = root.module_header();
    let namespace = header.authoring_namespace_id();
    let imports = header.imports();
    let mut usage = RoadEditingPreflightCounts {
        typed_ast_record_count: 1, // root 与 Provenance 不计；ModuleHeader 计一条
        ..RoadEditingPreflightCounts::default()
    };

    usage.charge_token(
        namespace,
        "moduleHeader.authoringNamespaceId",
        limits,
        expected_key,
    )?;
    usage.charge_token(
        header.source_document_key(),
        "moduleHeader.sourceDocumentKey",
        limits,
        expected_key,
    )?;
    let import_count = u64::try_from(imports.len()).unwrap_or(u64::MAX);
    let import_limit = limits.value(CompileLimitDimension::ImportEdgeCount);
    if import_count > import_limit {
        return Err(limit_error(
            CompileLimitDimension::ImportEdgeCount,
            import_limit,
            import_count,
        ));
    }
    ensure_unique_strings(imports, "moduleHeader.imports", expected_key, limits)?;
    for import in imports {
        usage.charge_token(import, "moduleHeader.imports", limits, expected_key)?;
        if import == namespace {
            return Err(semantic_error(
                "moduleHeader.imports",
                RoadEditingInputViolation::InvalidCombination,
                expected_key,
            ));
        }
    }
    validate_provenance(&mut usage, header.provenance(), limits, expected_key)?;

    if !matches!(
        root.geometry_accuracy_profile(),
        wire::GeometryAccuracyProfile::Fine2Cm
            | wire::GeometryAccuracyProfile::Balanced5Cm
            | wire::GeometryAccuracyProfile::Compact10Cm
    ) {
        return Err(invalid_combination(
            "roadEditingSource.geometryAccuracyProfile",
            expected_key,
        ));
    }
    if !matches!(
        root.geometry_direction_profile(),
        wire::GeometryDirectionProfile::Smooth1Deg
            | wire::GeometryDirectionProfile::Balanced2Deg
            | wire::GeometryDirectionProfile::Compact5Deg
    ) {
        return Err(invalid_combination(
            "roadEditingSource.geometryDirectionProfile",
            expected_key,
        ));
    }

    validate_alignments(&mut usage, root, namespace, imports, limits, expected_key)?;
    validate_corridors(&mut usage, root, namespace, imports, limits, expected_key)?;
    validate_sections(&mut usage, root, namespace, imports, limits, expected_key)?;
    validate_authoring_lanes(&mut usage, root, namespace, imports, limits, expected_key)?;
    validate_lane_edges(&mut usage, root, namespace, imports, limits, expected_key)?;
    validate_junctions(&mut usage, root, namespace, imports, limits, expected_key)?;
    validate_movements(&mut usage, root, namespace, imports, limits, expected_key)?;
    validate_maneuver_paths(&mut usage, root, namespace, imports, limits, expected_key)?;
    validate_maneuver_gates(&mut usage, root, namespace, imports, limits, expected_key)?;
    validate_waiting_zones(&mut usage, root, namespace, imports, limits, expected_key)?;
    validate_stop_lines_and_signal_groups(
        &mut usage,
        root,
        namespace,
        imports,
        limits,
        expected_key,
    )?;
    validate_signal_controllers_and_phases(
        &mut usage,
        root,
        namespace,
        imports,
        limits,
        expected_key,
    )?;
    validate_parking(&mut usage, root, namespace, imports, limits, expected_key)?;
    validate_lane_groups_and_facility_bands(
        &mut usage,
        root,
        namespace,
        imports,
        limits,
        expected_key,
    )?;
    validate_access_and_profiles(&mut usage, root, namespace, imports, limits, expected_key)?;
    validate_routes_and_frames(&mut usage, root, namespace, imports, limits, expected_key)?;
    validate_conflicts_and_regions(&mut usage, root, namespace, imports, limits, expected_key)?;
    policy::validate(&mut usage, root, namespace, imports, limits, expected_key)?;

    let mut usage = usage.validate(limits)?;
    owner::validate(root, limits, expected_key)?;
    usage.preflight_peak_scratch_bytes = scratch.peak_bytes();
    Ok(usage)
}

fn validate_provenance(
    usage: &mut RoadEditingPreflightCounts,
    provenance: wire::Provenance<'_>,
    limits: &PreflightScratch<'_>,
    expected_key: &str,
) -> Result<(), DiagnosticBundle> {
    usage.charge_token(
        provenance.generator_build_id(),
        "moduleHeader.provenance.generatorBuildId",
        limits,
        expected_key,
    )?;
    usage.charge_visible_ascii(
        provenance.description(),
        "moduleHeader.provenance.description",
        limits,
        expected_key,
    )?;
    match provenance.kind() {
        wire::ProvenanceKind::Direct => {
            let input_digest_matches = provenance
                .parameters_and_inputs_digest()
                .bytes()
                .iter()
                .eq(DIRECT_INPUTS_DIGEST);
            let options_digest_matches = provenance
                .frontend_options_digest()
                .bytes()
                .iter()
                .eq(DIRECT_FRONTEND_OPTIONS_DIGEST);
            if provenance.generator_build_id() != DIRECT_GENERATOR_BUILD_ID
                || !input_digest_matches
                || !options_digest_matches
                || provenance.random_seed().is_some()
            {
                return Err(invalid_combination("moduleHeader.provenance", expected_key));
            }
        }
        wire::ProvenanceKind::Generated => {}
        _ => {
            return Err(invalid_combination(
                "moduleHeader.provenance.kind",
                expected_key,
            ));
        }
    }
    Ok(())
}

fn validate_alignments(
    usage: &mut RoadEditingPreflightCounts,
    root: wire::RoadEditingSource<'_>,
    namespace: &str,
    imports: StringVector<'_>,
    limits: &PreflightScratch<'_>,
    expected_key: &str,
) -> Result<(), DiagnosticBundle> {
    ensure_unique_by(
        root.road_alignments().iter(),
        |value| value.road_alignment_key(),
        "roadAlignments.roadAlignmentKey",
        expected_key,
        limits,
    )?;
    for value in root.road_alignments() {
        usage.typed_ast_record_count = usage.typed_ast_record_count.saturating_add(1);
        usage.charge_token(
            value.road_alignment_key(),
            "roadAlignment.roadAlignmentKey",
            limits,
            expected_key,
        )?;
        usage.charge_reference(
            value.canonical_frame(),
            1,
            true,
            "roadAlignment.canonicalFrame",
            namespace,
            imports,
            limits,
            expected_key,
        )?;
        validate_curve(usage, value.reference_line(), limits, expected_key)?;
        usage.charge_canvas(value.canvas_selection(), limits, expected_key)?;
    }
    Ok(())
}

fn validate_curve(
    usage: &mut RoadEditingPreflightCounts,
    value: wire::CurveProgram<'_>,
    limits: &PreflightScratch<'_>,
    expected_key: &str,
) -> Result<(), DiagnosticBundle> {
    usage.typed_ast_record_count = usage.typed_ast_record_count.saturating_add(1);
    usage.authoring_point_count = usage.authoring_point_count.saturating_add(1);
    validate_point(value.start(), "curveProgram.start", expected_key)?;
    if value.segments().is_empty() {
        return Err(semantic_error(
            "curveProgram.segments",
            RoadEditingInputViolation::EmptyCollection,
            expected_key,
        ));
    }
    for segment in value.segments() {
        usage.typed_ast_record_count = usage.typed_ast_record_count.saturating_add(2);
        match segment.geometry_type() {
            wire::CurveSegmentGeometry::LineSegment => {
                let geometry = segment
                    .geometry_as_line_segment()
                    .ok_or_else(|| invalid_combination("curveSegment.geometry", expected_key))?;
                validate_point(
                    geometry.end(),
                    "curveSegment.geometry.line.end",
                    expected_key,
                )?;
                usage.authoring_point_count = usage.authoring_point_count.saturating_add(1);
            }
            wire::CurveSegmentGeometry::CubicBezierSegment => {
                let geometry = segment
                    .geometry_as_cubic_bezier_segment()
                    .ok_or_else(|| invalid_combination("curveSegment.geometry", expected_key))?;
                validate_point(
                    geometry.control_1(),
                    "curveSegment.geometry.cubic.control1",
                    expected_key,
                )?;
                validate_point(
                    geometry.control_2(),
                    "curveSegment.geometry.cubic.control2",
                    expected_key,
                )?;
                validate_point(
                    geometry.end(),
                    "curveSegment.geometry.cubic.end",
                    expected_key,
                )?;
                usage.authoring_point_count = usage.authoring_point_count.saturating_add(3);
            }
            _ => return Err(invalid_combination("curveSegment.geometry", expected_key)),
        }
        usage.charge_canvas(segment.canvas_selection(), limits, expected_key)?;
    }
    Ok(())
}

fn validate_point(
    value: &wire::Vec3F64,
    field: &'static str,
    expected_key: &str,
) -> Result<(), DiagnosticBundle> {
    for component in [value.x(), value.y(), value.z()] {
        let minimum = f64::from(CANONICAL_POINT_COMPONENT_MIN_METERS);
        let maximum = f64::from(CANONICAL_POINT_COMPONENT_MAX_METERS);
        if let Some(violation) = inclusive_range_violation(component, minimum, maximum) {
            return Err(semantic_error(field, violation, expected_key));
        }
    }
    Ok(())
}

fn validate_corridor_owned_reference(
    value: &str,
    component_count: u8,
    corridor_key: &str,
    field: &'static str,
    expected_key: &str,
) -> Result<(), DiagnosticBundle> {
    let reference = validate_wire_reference(value, component_count, true)
        .map_err(|violation| semantic_error(field, violation, expected_key))?;
    if reference.namespace().is_some()
        || reference
            .key_components()
            .next()
            .is_none_or(|owner| owner != corridor_key)
    {
        return Err(invalid_combination(field, expected_key));
    }
    Ok(())
}

fn validate_width(
    value: &wire::LinearWidthProfile,
    field: &'static str,
    expected_key: &str,
) -> Result<(), DiagnosticBundle> {
    if let Some(violation) = non_negative_violation(value.start_width_meters()) {
        return Err(semantic_error(field, violation, expected_key));
    }
    if let Some(violation) = non_negative_violation(value.end_width_meters()) {
        return Err(semantic_error(field, violation, expected_key));
    }
    if value.start_width_meters() == 0.0 && value.end_width_meters() == 0.0 {
        return Err(invalid_combination(field, expected_key));
    }
    Ok(())
}

fn validate_facility_kind_category(
    value: &str,
    expected: FacilityKindCategory,
    field: &'static str,
    expected_key: &str,
) -> Result<(), DiagnosticBundle> {
    if facility_kind_category(value) != Some(expected) {
        return Err(invalid_combination(field, expected_key));
    }
    Ok(())
}

fn validate_portable_signal_time(
    value: u64,
    allow_zero: bool,
    field: &'static str,
    expected_key: &str,
) -> Result<(), DiagnosticBundle> {
    if (!allow_zero && value == 0) || value > MAX_PORTABLE_SIGNAL_TIME_MS {
        return Err(invalid_combination(field, expected_key));
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn validate_reference_vector(
    usage: &mut RoadEditingPreflightCounts,
    values: StringVector<'_>,
    component_count: u8,
    field: &'static str,
    non_empty: bool,
    unique: bool,
    relation: bool,
    namespace: &str,
    imports: StringVector<'_>,
    limits: &PreflightScratch<'_>,
    expected_key: &str,
) -> Result<(), DiagnosticBundle> {
    if non_empty && values.is_empty() {
        return Err(semantic_error(
            field,
            RoadEditingInputViolation::EmptyCollection,
            expected_key,
        ));
    }
    if relation {
        usage.require_relation_capacity(values.len(), limits)?;
    }
    if unique {
        ensure_unique_references(values, namespace, field, expected_key, limits)?;
    }
    if relation {
        usage.charge_relation(values.len());
    }
    for value in values {
        usage.charge_reference(
            value,
            component_count,
            true,
            field,
            namespace,
            imports,
            limits,
            expected_key,
        )?;
    }
    Ok(())
}

fn validate_corridors(
    usage: &mut RoadEditingPreflightCounts,
    root: wire::RoadEditingSource<'_>,
    namespace: &str,
    imports: StringVector<'_>,
    limits: &PreflightScratch<'_>,
    expected_key: &str,
) -> Result<(), DiagnosticBundle> {
    ensure_unique_by(
        root.road_corridors().iter(),
        |value| value.road_corridor_key(),
        "roadCorridors.roadCorridorKey",
        expected_key,
        limits,
    )?;
    for value in root.road_corridors() {
        usage.charge_declaration(EntityKind::RoadCorridor);
        usage.charge_token(
            value.road_corridor_key(),
            "roadCorridor.roadCorridorKey",
            limits,
            expected_key,
        )?;
        usage.charge_token(
            value.road_alignment_key(),
            "roadCorridor.roadAlignmentKey",
            limits,
            expected_key,
        )?;
        if let Some(violation) = non_negative_violation(value.start_station_meters()) {
            return Err(semantic_error(
                "roadCorridor.startStationMeters",
                violation,
                expected_key,
            ));
        }
        match value.end_station_kind() {
            wire::StationEndKind::Finite => {
                if let Some(violation) = positive_violation(value.end_station_meters()) {
                    return Err(semantic_error(
                        "roadCorridor.endStationMeters",
                        violation,
                        expected_key,
                    ));
                }
                if value.end_station_meters() <= value.start_station_meters() {
                    return Err(invalid_combination(
                        "roadCorridor.endStationMeters",
                        expected_key,
                    ));
                }
            }
            wire::StationEndKind::AlignmentEnd => {
                if value.end_station_meters().to_bits() != 0.0_f64.to_bits() {
                    return Err(invalid_combination(
                        "roadCorridor.endStationMeters",
                        expected_key,
                    ));
                }
            }
            _ => {
                return Err(invalid_combination(
                    "roadCorridor.endStationKind",
                    expected_key,
                ));
            }
        }
        usage.charge_reference(
            value.reference_section(),
            2,
            true,
            "roadCorridor.referenceSection",
            namespace,
            imports,
            limits,
            expected_key,
        )?;
        validate_corridor_owned_reference(
            value.reference_section(),
            2,
            value.road_corridor_key(),
            "roadCorridor.referenceSection",
            expected_key,
        )?;
        usage.charge_reference(
            value.reference_lane(),
            3,
            true,
            "roadCorridor.referenceLane",
            namespace,
            imports,
            limits,
            expected_key,
        )?;
        validate_corridor_owned_reference(
            value.reference_lane(),
            3,
            value.road_corridor_key(),
            "roadCorridor.referenceLane",
            expected_key,
        )?;
        let elements = value.elements();
        if elements.is_empty() {
            return Err(semantic_error(
                "roadCorridor.elements",
                RoadEditingInputViolation::EmptyCollection,
                expected_key,
            ));
        }
        usage.charge_relation(elements.len());
        let duplicate_index = first_duplicate_index(
            elements.iter(),
            |element| {
                (
                    element.kind().0,
                    reference_parts(element.entity_reference(), namespace),
                )
            },
            limits,
        )?;
        for (index, element) in elements.iter().enumerate() {
            usage.typed_ast_record_count = usage.typed_ast_record_count.saturating_add(1);
            let depth = match element.kind() {
                wire::CorridorElementKind::RoadSection
                | wire::CorridorElementKind::FacilityBand => 2,
                _ => {
                    return Err(invalid_combination(
                        "roadCorridor.elements.kind",
                        expected_key,
                    ));
                }
            };
            usage.charge_reference(
                element.entity_reference(),
                depth,
                false,
                "roadCorridor.elements.entityReference",
                namespace,
                imports,
                limits,
                expected_key,
            )?;
            validate_corridor_owned_reference(
                element.entity_reference(),
                depth,
                value.road_corridor_key(),
                "roadCorridor.elements.entityReference",
                expected_key,
            )?;
            if duplicate_index == Some(index) {
                return Err(semantic_error(
                    "roadCorridor.elements",
                    RoadEditingInputViolation::DuplicateValue,
                    expected_key,
                ));
            }
        }
        usage.charge_canvas(value.canvas_selection(), limits, expected_key)?;
    }
    Ok(())
}

fn validate_sections(
    usage: &mut RoadEditingPreflightCounts,
    root: wire::RoadEditingSource<'_>,
    namespace: &str,
    imports: StringVector<'_>,
    limits: &PreflightScratch<'_>,
    expected_key: &str,
) -> Result<(), DiagnosticBundle> {
    ensure_unique_by(
        root.road_sections().iter(),
        |value| (value.road_corridor(), value.road_section_key()),
        "roadSections.address",
        expected_key,
        limits,
    )?;
    let mut corridor_keys = limits.collect(
        root.road_corridors()
            .iter()
            .map(|value| value.road_corridor_key()),
    )?;
    corridor_keys.sort_unstable();
    for value in root.road_sections() {
        usage.charge_declaration(EntityKind::RoadSection);
        usage.charge_token(
            value.road_section_key(),
            "roadSection.roadSectionKey",
            limits,
            expected_key,
        )?;
        usage.charge_token(value.kind_id(), "roadSection.kindId", limits, expected_key)?;
        validate_facility_kind_category(
            value.kind_id(),
            FacilityKindCategory::LaneBearing,
            "roadSection.kindId",
            expected_key,
        )?;
        validate_reference_vector(
            usage,
            value.authoring_lanes(),
            3,
            "roadSection.authoringLanes",
            true,
            true,
            true,
            namespace,
            imports,
            limits,
            expected_key,
        )?;
        usage.charge_reference(
            value.road_corridor(),
            1,
            false,
            "roadSection.roadCorridor",
            namespace,
            imports,
            limits,
            expected_key,
        )?;
        if corridor_keys.binary_search(&value.road_corridor()).is_err() {
            return Err(invalid_combination(
                "roadSection.roadCorridor",
                expected_key,
            ));
        }
        usage.charge_canvas(value.canvas_selection(), limits, expected_key)?;
    }
    Ok(())
}

fn validate_authoring_lanes(
    usage: &mut RoadEditingPreflightCounts,
    root: wire::RoadEditingSource<'_>,
    namespace: &str,
    imports: StringVector<'_>,
    limits: &PreflightScratch<'_>,
    expected_key: &str,
) -> Result<(), DiagnosticBundle> {
    ensure_unique_by(
        root.authoring_lanes().iter(),
        |value| (value.road_section(), value.authoring_lane_key()),
        "authoringLanes.address",
        expected_key,
        limits,
    )?;
    for value in root.authoring_lanes() {
        usage.charge_declaration(EntityKind::AuthoringLane);
        usage.charge_token(
            value.authoring_lane_key(),
            "authoringLane.authoringLaneKey",
            limits,
            expected_key,
        )?;
        usage.charge_reference(
            value.lane_edge(),
            1,
            true,
            "authoringLane.laneEdge",
            namespace,
            imports,
            limits,
            expected_key,
        )?;
        if !matches!(
            value.direction(),
            wire::LaneDirection::Forward | wire::LaneDirection::Backward
        ) {
            return Err(invalid_combination("authoringLane.direction", expected_key));
        }
        validate_width(
            value.width_profile(),
            "authoringLane.widthProfile",
            expected_key,
        )?;
        if let Some(group) = value.lane_group() {
            usage.charge_reference(
                group,
                3,
                true,
                "authoringLane.laneGroup",
                namespace,
                imports,
                limits,
                expected_key,
            )?;
        }
        usage.charge_reference(
            value.road_section(),
            2,
            false,
            "authoringLane.roadSection",
            namespace,
            imports,
            limits,
            expected_key,
        )?;
        usage.charge_canvas(value.canvas_selection(), limits, expected_key)?;
    }
    Ok(())
}

fn validate_lane_edges(
    usage: &mut RoadEditingPreflightCounts,
    root: wire::RoadEditingSource<'_>,
    namespace: &str,
    imports: StringVector<'_>,
    limits: &PreflightScratch<'_>,
    expected_key: &str,
) -> Result<(), DiagnosticBundle> {
    ensure_unique_by(
        root.lane_edges().iter(),
        |value| value.lane_edge_key(),
        "laneEdges.laneEdgeKey",
        expected_key,
        limits,
    )?;
    for value in root.lane_edges() {
        usage.charge_declaration(EntityKind::LaneEdge);
        usage.charge_token(
            value.lane_edge_key(),
            "laneEdge.laneEdgeKey",
            limits,
            expected_key,
        )?;
        if let Some(violation) = millimetre_range_violation(
            value.speed_limit_meters_per_second(),
            MIN_SPEED_MM_S,
            MAX_SPEED_MM_S,
        ) {
            return Err(semantic_error(
                "laneEdge.speedLimitMetersPerSecond",
                violation,
                expected_key,
            ));
        }
        validate_reference_vector(
            usage,
            value.successors(),
            1,
            "laneEdge.successors",
            false,
            true,
            true,
            namespace,
            imports,
            limits,
            expected_key,
        )?;
        if let Some(curve) = value.explicit_geometry() {
            validate_curve(usage, curve, limits, expected_key)?;
        }
        usage.charge_canvas(value.canvas_selection(), limits, expected_key)?;
    }
    Ok(())
}

fn validate_junctions(
    usage: &mut RoadEditingPreflightCounts,
    root: wire::RoadEditingSource<'_>,
    namespace: &str,
    imports: StringVector<'_>,
    limits: &PreflightScratch<'_>,
    expected_key: &str,
) -> Result<(), DiagnosticBundle> {
    ensure_unique_by(
        root.junctions().iter(),
        |value| value.junction_key(),
        "junctions.junctionKey",
        expected_key,
        limits,
    )?;
    for value in root.junctions() {
        usage.charge_declaration(EntityKind::Junction);
        usage.charge_token(
            value.junction_key(),
            "junction.junctionKey",
            limits,
            expected_key,
        )?;
        validate_reference_vector(
            usage,
            value.approach_edges(),
            1,
            "junction.approachEdges",
            true,
            true,
            true,
            namespace,
            imports,
            limits,
            expected_key,
        )?;
        validate_reference_vector(
            usage,
            value.internal_edges(),
            1,
            "junction.internalEdges",
            false,
            true,
            true,
            namespace,
            imports,
            limits,
            expected_key,
        )?;
        for approach in value.approach_edges() {
            if value
                .internal_edges()
                .iter()
                .any(|internal| references_equal(approach, internal, namespace))
            {
                return Err(invalid_combination("junction.edgeRoles", expected_key));
            }
        }
        usage.charge_canvas(value.canvas_selection(), limits, expected_key)?;
    }
    Ok(())
}

fn validate_movements(
    usage: &mut RoadEditingPreflightCounts,
    root: wire::RoadEditingSource<'_>,
    namespace: &str,
    imports: StringVector<'_>,
    limits: &PreflightScratch<'_>,
    expected_key: &str,
) -> Result<(), DiagnosticBundle> {
    ensure_unique_by(
        root.movements().iter(),
        |value| (value.junction(), value.movement_key()),
        "movements.address",
        expected_key,
        limits,
    )?;
    for value in root.movements() {
        if value
            .turn_direction()
            .is_some_and(|direction| crate::ManeuverDirection::from_code(direction.0).is_none())
        {
            return Err(invalid_combination("movement.turnDirection", expected_key));
        }
        usage.charge_declaration(EntityKind::Movement);
        if value.turn_direction().is_some() {
            usage.typed_ast_record_count = usage.typed_ast_record_count.saturating_add(1);
            usage.charge_relation(1);
        }
        usage.charge_token(
            value.movement_key(),
            "movement.movementKey",
            limits,
            expected_key,
        )?;
        usage.charge_reference(
            value.junction(),
            1,
            false,
            "movement.junction",
            namespace,
            imports,
            limits,
            expected_key,
        )?;
        usage.charge_token(
            value.directed_entry_approach_key(),
            "movement.directedEntryApproachKey",
            limits,
            expected_key,
        )?;
        usage.charge_token(
            value.directed_exit_approach_key(),
            "movement.directedExitApproachKey",
            limits,
            expected_key,
        )?;
        usage.charge_canvas(value.canvas_selection(), limits, expected_key)?;
    }
    Ok(())
}

fn validate_maneuver_paths(
    usage: &mut RoadEditingPreflightCounts,
    root: wire::RoadEditingSource<'_>,
    namespace: &str,
    imports: StringVector<'_>,
    limits: &PreflightScratch<'_>,
    expected_key: &str,
) -> Result<(), DiagnosticBundle> {
    ensure_unique_by(
        root.maneuver_paths().iter(),
        |value| (value.movement(), value.maneuver_path_key()),
        "maneuverPaths.address",
        expected_key,
        limits,
    )?;
    for value in root.maneuver_paths() {
        usage.charge_declaration(EntityKind::ManeuverPath);
        usage.charge_token(
            value.maneuver_path_key(),
            "maneuverPath.maneuverPathKey",
            limits,
            expected_key,
        )?;
        usage.charge_reference(
            value.movement(),
            2,
            false,
            "maneuverPath.movement",
            namespace,
            imports,
            limits,
            expected_key,
        )?;
        usage.charge_reference(
            value.entry_edge(),
            1,
            true,
            "maneuverPath.entryEdge",
            namespace,
            imports,
            limits,
            expected_key,
        )?;
        validate_reference_vector(
            usage,
            value.internal_edges(),
            1,
            "maneuverPath.internalEdges",
            false,
            true,
            true,
            namespace,
            imports,
            limits,
            expected_key,
        )?;
        usage.charge_reference(
            value.exit_edge(),
            1,
            true,
            "maneuverPath.exitEdge",
            namespace,
            imports,
            limits,
            expected_key,
        )?;
        usage.charge_canvas(value.canvas_selection(), limits, expected_key)?;
    }
    Ok(())
}

fn validate_maneuver_gates(
    usage: &mut RoadEditingPreflightCounts,
    root: wire::RoadEditingSource<'_>,
    namespace: &str,
    imports: StringVector<'_>,
    limits: &PreflightScratch<'_>,
    expected_key: &str,
) -> Result<(), DiagnosticBundle> {
    ensure_unique_by(
        root.maneuver_gates().iter(),
        |value| (value.maneuver_path(), value.maneuver_gate_key()),
        "maneuverGates.address",
        expected_key,
        limits,
    )?;
    for value in root.maneuver_gates() {
        usage.charge_declaration(EntityKind::ManeuverGate);
        usage.maneuver_gate_count = usage.maneuver_gate_count.saturating_add(1);
        usage.charge_token(
            value.maneuver_gate_key(),
            "maneuverGate.maneuverGateKey",
            limits,
            expected_key,
        )?;
        usage.charge_reference(
            value.maneuver_path(),
            3,
            false,
            "maneuverGate.maneuverPath",
            namespace,
            imports,
            limits,
            expected_key,
        )?;
        usage.charge_reference(
            value.stop_line(),
            1,
            true,
            "maneuverGate.stopLine",
            namespace,
            imports,
            limits,
            expected_key,
        )?;
        match (value.signal_control(), value.signal_group()) {
            (wire::SignalControlKind::None, None) => {}
            (wire::SignalControlKind::SignalGroup, Some(group)) => usage.charge_reference(
                group,
                1,
                true,
                "maneuverGate.signalGroup",
                namespace,
                imports,
                limits,
                expected_key,
            )?,
            _ => {
                return Err(invalid_combination(
                    "maneuverGate.signalControl",
                    expected_key,
                ));
            }
        }
        usage.charge_canvas(value.canvas_selection(), limits, expected_key)?;
    }
    Ok(())
}

fn validate_waiting_zones(
    usage: &mut RoadEditingPreflightCounts,
    root: wire::RoadEditingSource<'_>,
    namespace: &str,
    imports: StringVector<'_>,
    limits: &PreflightScratch<'_>,
    expected_key: &str,
) -> Result<(), DiagnosticBundle> {
    ensure_unique_by(
        root.waiting_zones().iter(),
        |value| (value.maneuver_path(), value.waiting_zone_key()),
        "waitingZones.address",
        expected_key,
        limits,
    )?;
    for value in root.waiting_zones() {
        usage.charge_declaration(EntityKind::WaitingZone);
        usage.waiting_zone_count = usage.waiting_zone_count.saturating_add(1);
        usage.charge_token(
            value.waiting_zone_key(),
            "waitingZone.waitingZoneKey",
            limits,
            expected_key,
        )?;
        usage.charge_reference(
            value.maneuver_path(),
            3,
            false,
            "waitingZone.maneuverPath",
            namespace,
            imports,
            limits,
            expected_key,
        )?;
        usage.charge_reference(
            value.entry_gate(),
            4,
            true,
            "waitingZone.entryGate",
            namespace,
            imports,
            limits,
            expected_key,
        )?;
        usage.charge_reference(
            value.release_gate(),
            4,
            true,
            "waitingZone.releaseGate",
            namespace,
            imports,
            limits,
            expected_key,
        )?;
        if value.max_occupancy() == 0 {
            return Err(invalid_combination(
                "waitingZone.maxOccupancy",
                expected_key,
            ));
        }
        usage.charge_canvas(value.canvas_selection(), limits, expected_key)?;
    }
    Ok(())
}

fn validate_stop_lines_and_signal_groups(
    usage: &mut RoadEditingPreflightCounts,
    root: wire::RoadEditingSource<'_>,
    namespace: &str,
    imports: StringVector<'_>,
    limits: &PreflightScratch<'_>,
    expected_key: &str,
) -> Result<(), DiagnosticBundle> {
    ensure_unique_by(
        root.stop_lines().iter(),
        |value| value.stop_line_key(),
        "stopLines.stopLineKey",
        expected_key,
        limits,
    )?;
    for value in root.stop_lines() {
        usage.charge_declaration(EntityKind::StopLine);
        usage.charge_token(
            value.stop_line_key(),
            "stopLine.stopLineKey",
            limits,
            expected_key,
        )?;
        usage.charge_reference(
            value.lane_edge(),
            1,
            true,
            "stopLine.laneEdge",
            namespace,
            imports,
            limits,
            expected_key,
        )?;
        usage.charge_canvas(value.canvas_selection(), limits, expected_key)?;
    }
    ensure_unique_by(
        root.signal_groups().iter(),
        |value| value.signal_group_key(),
        "signalGroups.signalGroupKey",
        expected_key,
        limits,
    )?;
    for value in root.signal_groups() {
        usage.charge_declaration(EntityKind::SignalGroup);
        usage.charge_token(
            value.signal_group_key(),
            "signalGroup.signalGroupKey",
            limits,
            expected_key,
        )?;
        usage.charge_canvas(value.canvas_selection(), limits, expected_key)?;
    }
    Ok(())
}

fn validate_signal_controllers_and_phases(
    usage: &mut RoadEditingPreflightCounts,
    root: wire::RoadEditingSource<'_>,
    namespace: &str,
    imports: StringVector<'_>,
    limits: &PreflightScratch<'_>,
    expected_key: &str,
) -> Result<(), DiagnosticBundle> {
    ensure_unique_by(
        root.signal_controllers().iter(),
        |value| value.signal_controller_key(),
        "signalControllers.signalControllerKey",
        expected_key,
        limits,
    )?;
    for value in root.signal_controllers() {
        usage.charge_declaration(EntityKind::SignalController);
        usage.charge_token(
            value.signal_controller_key(),
            "signalController.signalControllerKey",
            limits,
            expected_key,
        )?;
        validate_portable_signal_time(
            value.offset_milliseconds(),
            true,
            "signalController.offsetMilliseconds",
            expected_key,
        )?;
        validate_reference_vector(
            usage,
            value.signal_groups(),
            1,
            "signalController.signalGroups",
            true,
            true,
            true,
            namespace,
            imports,
            limits,
            expected_key,
        )?;
        validate_reference_vector(
            usage,
            value.signal_phases(),
            2,
            "signalController.signalPhases",
            true,
            true,
            true,
            namespace,
            imports,
            limits,
            expected_key,
        )?;
        usage.charge_canvas(value.canvas_selection(), limits, expected_key)?;
    }

    ensure_unique_by(
        root.signal_phases().iter(),
        |value| (value.signal_controller(), value.signal_phase_key()),
        "signalPhases.address",
        expected_key,
        limits,
    )?;
    for value in root.signal_phases() {
        usage.charge_declaration(EntityKind::SignalPhase);
        usage.charge_token(
            value.signal_phase_key(),
            "signalPhase.signalPhaseKey",
            limits,
            expected_key,
        )?;
        validate_portable_signal_time(
            value.duration_milliseconds(),
            false,
            "signalPhase.durationMilliseconds",
            expected_key,
        )?;
        let states = value.states();
        if states.is_empty() {
            return Err(semantic_error(
                "signalPhase.states",
                RoadEditingInputViolation::EmptyCollection,
                expected_key,
            ));
        }
        usage.charge_relation(states.len());
        let duplicate_index = first_duplicate_index(
            states.iter(),
            |state| reference_parts(state.signal_group(), namespace),
            limits,
        )?;
        for (index, state) in states.iter().enumerate() {
            usage.typed_ast_record_count = usage.typed_ast_record_count.saturating_add(1);
            if state.signal_group().contains("::") {
                return Err(invalid_combination(
                    "signalPhase.states.signalGroup",
                    expected_key,
                ));
            }
            usage.charge_reference(
                state.signal_group(),
                1,
                true,
                "signalPhase.states.signalGroup",
                namespace,
                imports,
                limits,
                expected_key,
            )?;
            if !matches!(
                state.aspect(),
                wire::SignalAspect::Red | wire::SignalAspect::Yellow | wire::SignalAspect::Green
            ) {
                return Err(invalid_combination(
                    "signalPhase.states.aspect",
                    expected_key,
                ));
            }
            if duplicate_index == Some(index) {
                return Err(semantic_error(
                    "signalPhase.states.signalGroup",
                    RoadEditingInputViolation::DuplicateValue,
                    expected_key,
                ));
            }
        }
        usage.charge_reference(
            value.signal_controller(),
            1,
            false,
            "signalPhase.signalController",
            namespace,
            imports,
            limits,
            expected_key,
        )?;
        usage.charge_canvas(value.canvas_selection(), limits, expected_key)?;
    }
    Ok(())
}

fn validate_parking(
    usage: &mut RoadEditingPreflightCounts,
    root: wire::RoadEditingSource<'_>,
    namespace: &str,
    imports: StringVector<'_>,
    limits: &PreflightScratch<'_>,
    expected_key: &str,
) -> Result<(), DiagnosticBundle> {
    ensure_unique_by(
        root.parking_facilities().iter(),
        |value| value.parking_facility_key(),
        "parkingFacilities.parkingFacilityKey",
        expected_key,
        limits,
    )?;
    for value in root.parking_facilities() {
        usage.charge_declaration(EntityKind::ParkingFacility);
        usage.charge_token(
            value.parking_facility_key(),
            "parkingFacility.parkingFacilityKey",
            limits,
            expected_key,
        )?;
        let virtual_entries = value.virtual_entries();
        let virtual_exits = value.virtual_exits();
        if (value.virtual_capacity() == 0
            && (!virtual_entries.is_empty() || !virtual_exits.is_empty()))
            || (value.virtual_capacity() != 0
                && (virtual_entries.is_empty() || virtual_exits.is_empty()))
        {
            return Err(invalid_combination(
                "parkingFacility.virtualCapacity",
                expected_key,
            ));
        }
        usage.typed_ast_record_count = usage
            .typed_ast_record_count
            .saturating_add(u64::try_from(virtual_entries.len()).unwrap_or(u64::MAX))
            .saturating_add(u64::try_from(virtual_exits.len()).unwrap_or(u64::MAX));
        usage.charge_relation(virtual_entries.len().saturating_add(virtual_exits.len()));
        for anchor in virtual_entries {
            validate_parking_anchor(
                usage,
                anchor,
                "parkingFacility.virtualEntries",
                namespace,
                imports,
                limits,
                expected_key,
            )?;
        }
        for anchor in virtual_exits {
            validate_parking_anchor(
                usage,
                anchor,
                "parkingFacility.virtualExits",
                namespace,
                imports,
                limits,
                expected_key,
            )?;
        }
        usage.charge_canvas(value.canvas_selection(), limits, expected_key)?;
    }

    ensure_unique_by(
        root.parking_spaces().iter(),
        |value| value.parking_space_key(),
        "parkingSpaces.parkingSpaceKey",
        expected_key,
        limits,
    )?;
    for value in root.parking_spaces() {
        usage.charge_declaration(EntityKind::ParkingSpace);
        usage.typed_ast_record_count = usage.typed_ast_record_count.saturating_add(3);
        usage.charge_token(
            value.parking_space_key(),
            "parkingSpace.parkingSpaceKey",
            limits,
            expected_key,
        )?;
        if let Some(area) = value.parking_facility() {
            usage.charge_reference(
                area,
                1,
                true,
                "parkingSpace.parkingFacility",
                namespace,
                imports,
                limits,
                expected_key,
            )?;
        }
        validate_parking_anchor(
            usage,
            value.entry(),
            "parkingSpace.entry",
            namespace,
            imports,
            limits,
            expected_key,
        )?;
        validate_parking_anchor(
            usage,
            value.exit(),
            "parkingSpace.exit",
            namespace,
            imports,
            limits,
            expected_key,
        )?;
        let geometry = value.geometry();
        if let Some(violation) = millimetre_i32_abs_range_violation(
            geometry.lateral_offset_meters(),
            MIN_PARKING_LATERAL_OFFSET_ABS_MM,
            MAX_PARKING_LATERAL_OFFSET_ABS_MM,
        ) {
            return Err(semantic_error(
                "parkingSpace.geometry.lateralOffsetMeters",
                violation,
                expected_key,
            ));
        }
        if let Some(violation) = heading_violation(geometry.heading_offset_radians()) {
            return Err(semantic_error(
                "parkingSpace.geometry.headingOffsetRadians",
                violation,
                expected_key,
            ));
        }
        for (field, extent) in [
            (
                "parkingSpace.geometry.lengthMeters",
                geometry.length_meters(),
            ),
            ("parkingSpace.geometry.widthMeters", geometry.width_meters()),
        ] {
            if let Some(violation) =
                millimetre_range_violation(extent, MIN_VEHICLE_LENGTH_MM, MAX_VEHICLE_LENGTH_MM)
            {
                return Err(semantic_error(field, violation, expected_key));
            }
        }
        usage.charge_canvas(value.canvas_selection(), limits, expected_key)?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn validate_parking_anchor(
    usage: &mut RoadEditingPreflightCounts,
    value: wire::ParkingLaneAnchor<'_>,
    field: &'static str,
    namespace: &str,
    imports: StringVector<'_>,
    limits: &PreflightScratch<'_>,
    expected_key: &str,
) -> Result<(), DiagnosticBundle> {
    usage.charge_reference(
        value.lane_edge(),
        1,
        true,
        field,
        namespace,
        imports,
        limits,
        expected_key,
    )?;
    if let Some(violation) = millimetre_range_violation(
        value.progress_meters(),
        PARKING_ANCHOR_ENDPOINT_CLEARANCE_MM,
        MAX_LANE_EDGE_LENGTH_MM.saturating_sub(PARKING_ANCHOR_ENDPOINT_CLEARANCE_MM),
    ) {
        return Err(semantic_error(field, violation, expected_key));
    }
    Ok(())
}

fn validate_lane_groups_and_facility_bands(
    usage: &mut RoadEditingPreflightCounts,
    root: wire::RoadEditingSource<'_>,
    namespace: &str,
    imports: StringVector<'_>,
    limits: &PreflightScratch<'_>,
    expected_key: &str,
) -> Result<(), DiagnosticBundle> {
    ensure_unique_by(
        root.lane_groups().iter(),
        |value| (value.road_section(), value.lane_group_key()),
        "laneGroups.address",
        expected_key,
        limits,
    )?;
    for value in root.lane_groups() {
        usage.charge_declaration(EntityKind::LaneGroup);
        usage.charge_token(
            value.lane_group_key(),
            "laneGroup.laneGroupKey",
            limits,
            expected_key,
        )?;
        usage.charge_reference(
            value.road_section(),
            2,
            false,
            "laneGroup.roadSection",
            namespace,
            imports,
            limits,
            expected_key,
        )?;
        usage.charge_canvas(value.canvas_selection(), limits, expected_key)?;
    }

    ensure_unique_by(
        root.facility_bands().iter(),
        |value| (value.road_corridor(), value.facility_band_key()),
        "facilityBands.address",
        expected_key,
        limits,
    )?;
    for value in root.facility_bands() {
        usage.charge_declaration(EntityKind::FacilityBand);
        usage.charge_token(
            value.facility_band_key(),
            "facilityBand.facilityBandKey",
            limits,
            expected_key,
        )?;
        usage.charge_token(value.kind_id(), "facilityBand.kindId", limits, expected_key)?;
        validate_facility_kind_category(
            value.kind_id(),
            FacilityKindCategory::NonTraversable,
            "facilityBand.kindId",
            expected_key,
        )?;
        validate_width(
            value.width_profile(),
            "facilityBand.widthProfile",
            expected_key,
        )?;
        usage.charge_reference(
            value.road_corridor(),
            1,
            false,
            "facilityBand.roadCorridor",
            namespace,
            imports,
            limits,
            expected_key,
        )?;
        usage.charge_canvas(value.canvas_selection(), limits, expected_key)?;
    }
    Ok(())
}

fn validate_access_and_profiles(
    usage: &mut RoadEditingPreflightCounts,
    root: wire::RoadEditingSource<'_>,
    namespace: &str,
    imports: StringVector<'_>,
    limits: &PreflightScratch<'_>,
    expected_key: &str,
) -> Result<(), DiagnosticBundle> {
    ensure_unique_by(
        root.participant_classes().iter(),
        |value| value.participant_class_key(),
        "participantClasses.participantClassKey",
        expected_key,
        limits,
    )?;
    for value in root.participant_classes() {
        usage.charge_declaration(EntityKind::ParticipantClass);
        usage.charge_token(
            value.participant_class_key(),
            "participantClass.participantClassKey",
            limits,
            expected_key,
        )?;
        if let Some(parent) = value.extends() {
            usage.charge_reference(
                parent,
                1,
                true,
                "participantClass.extends",
                namespace,
                imports,
                limits,
                expected_key,
            )?;
        }
        usage.charge_canvas(value.canvas_selection(), limits, expected_key)?;
    }

    ensure_unique_by(
        root.access_rules().iter(),
        |value| value.access_rule_key(),
        "accessRules.accessRuleKey",
        expected_key,
        limits,
    )?;
    for value in root.access_rules() {
        usage.charge_declaration(EntityKind::AccessRule);
        usage.charge_token(
            value.access_rule_key(),
            "accessRule.accessRuleKey",
            limits,
            expected_key,
        )?;
        let target_depth = match value.target_kind() {
            wire::AccessTargetKind::LaneEdge => 1,
            wire::AccessTargetKind::LaneGroup => 3,
            wire::AccessTargetKind::RoadSection => 2,
            wire::AccessTargetKind::ManeuverPath => 3,
            _ => return Err(invalid_combination("accessRule.targetKind", expected_key)),
        };
        usage.charge_reference(
            value.target_reference(),
            target_depth,
            true,
            "accessRule.targetReference",
            namespace,
            imports,
            limits,
            expected_key,
        )?;
        if !matches!(
            value.effect(),
            wire::AccessEffect::Allow | wire::AccessEffect::Deny
        ) {
            return Err(invalid_combination("accessRule.effect", expected_key));
        }
        validate_reference_vector(
            usage,
            value.participant_classes(),
            1,
            "accessRule.participantClasses",
            true,
            true,
            true,
            namespace,
            imports,
            limits,
            expected_key,
        )?;
        if let Some(regulation) = value.regulation() {
            usage.typed_ast_record_count = usage.typed_ast_record_count.saturating_add(1);
            usage.charge_non_empty_text(
                regulation.jurisdiction(),
                "accessRegulation.jurisdiction",
                limits,
                expected_key,
            )?;
            usage.charge_non_empty_text(
                regulation.version(),
                "accessRegulation.version",
                limits,
                expected_key,
            )?;
            if let Some(source) = regulation.source() {
                usage.charge_non_empty_text(
                    source,
                    "accessRegulation.source",
                    limits,
                    expected_key,
                )?;
            }
        }
        usage.charge_canvas(value.canvas_selection(), limits, expected_key)?;
    }

    ensure_unique_by(
        root.vehicle_profiles().iter(),
        |value| value.vehicle_profile_key(),
        "vehicleProfiles.vehicleProfileKey",
        expected_key,
        limits,
    )?;
    for value in root.vehicle_profiles() {
        usage.charge_declaration(EntityKind::VehicleProfile);
        usage.typed_ast_record_count = usage.typed_ast_record_count.saturating_add(1);
        usage.charge_token(
            value.vehicle_profile_key(),
            "vehicleProfile.vehicleProfileKey",
            limits,
            expected_key,
        )?;
        usage.charge_reference(
            value.participant_class(),
            1,
            true,
            "vehicleProfile.participantClass",
            namespace,
            imports,
            limits,
            expected_key,
        )?;
        validate_iidm(value.iidm(), expected_key)?;
        usage.charge_canvas(value.canvas_selection(), limits, expected_key)?;
    }
    Ok(())
}

fn validate_iidm(
    value: wire::IidmVehicleProfile<'_>,
    expected_key: &str,
) -> Result<(), DiagnosticBundle> {
    if let Some(violation) = millimetre_range_violation(
        value.length_meters(),
        MIN_VEHICLE_LENGTH_MM,
        MAX_VEHICLE_LENGTH_MM,
    ) {
        return Err(semantic_error(
            "vehicleProfile.iidm.lengthMeters",
            violation,
            expected_key,
        ));
    }
    if let Some(violation) = millimetre_range_violation(
        value.desired_speed_meters_per_second(),
        MIN_SPEED_MM_S,
        MAX_SPEED_MM_S,
    ) {
        return Err(semantic_error(
            "vehicleProfile.iidm.desiredSpeedMetersPerSecond",
            violation,
            expected_key,
        ));
    }
    if let Some(violation) = millimetre_range_violation(value.min_gap_meters(), 0, MAX_MIN_GAP_MM) {
        return Err(semantic_error(
            "vehicleProfile.iidm.minGapMeters",
            violation,
            expected_key,
        ));
    }
    if let Some(violation) = time_headway_violation(value.time_headway_seconds()) {
        return Err(semantic_error(
            "vehicleProfile.iidm.timeHeadwaySeconds",
            violation,
            expected_key,
        ));
    }
    for (field, number) in [
        (
            "vehicleProfile.iidm.maxAccelerationMetersPerSecondSquared",
            value.max_acceleration_meters_per_second_squared(),
        ),
        (
            "vehicleProfile.iidm.comfortableDecelerationMetersPerSecondSquared",
            value.comfortable_deceleration_meters_per_second_squared(),
        ),
        (
            "vehicleProfile.iidm.emergencyDecelerationMetersPerSecondSquared",
            value.emergency_deceleration_meters_per_second_squared(),
        ),
    ] {
        if let Some(violation) = accel_violation(number) {
            return Err(semantic_error(field, violation, expected_key));
        }
    }
    if (value.emergency_deceleration_meters_per_second_squared() as f32)
        < (value.comfortable_deceleration_meters_per_second_squared() as f32)
    {
        return Err(invalid_combination(
            "vehicleProfile.iidm.emergencyDecelerationMetersPerSecondSquared",
            expected_key,
        ));
    }
    Ok(())
}

fn validate_routes_and_frames(
    usage: &mut RoadEditingPreflightCounts,
    root: wire::RoadEditingSource<'_>,
    namespace: &str,
    imports: StringVector<'_>,
    limits: &PreflightScratch<'_>,
    expected_key: &str,
) -> Result<(), DiagnosticBundle> {
    let _ = namespace;
    let _ = imports;
    ensure_unique_by(
        root.canonical_frames().iter(),
        |value| value.canonical_frame_key(),
        "canonicalFrames.canonicalFrameKey",
        expected_key,
        limits,
    )?;
    for value in root.canonical_frames() {
        usage.charge_declaration(EntityKind::CanonicalFrame);
        usage.charge_token(
            value.canonical_frame_key(),
            "canonicalFrame.canonicalFrameKey",
            limits,
            expected_key,
        )?;
        usage.charge_canvas(value.canvas_selection(), limits, expected_key)?;
    }
    Ok(())
}

fn validate_conflicts_and_regions(
    usage: &mut RoadEditingPreflightCounts,
    root: wire::RoadEditingSource<'_>,
    namespace: &str,
    imports: StringVector<'_>,
    limits: &PreflightScratch<'_>,
    expected_key: &str,
) -> Result<(), DiagnosticBundle> {
    ensure_unique_by(
        root.conflict_zones().iter(),
        |value| (value.junction(), value.conflict_zone_key()),
        "conflictZones.junction+conflictZoneKey",
        expected_key,
        limits,
    )?;
    for value in root.conflict_zones() {
        usage.charge_declaration(EntityKind::ConflictZone);
        usage.charge_token(
            value.conflict_zone_key(),
            "conflictZone.conflictZoneKey",
            limits,
            expected_key,
        )?;
        usage.charge_reference(
            value.junction(),
            1,
            false,
            "conflictZone.junction",
            namespace,
            imports,
            limits,
            expected_key,
        )?;
        usage.charge_canvas(value.canvas_selection(), limits, expected_key)?;
    }

    ensure_unique_by(
        root.participant_streams().iter(),
        |value| (value.junction(), value.participant_stream_key()),
        "participantStreams.junction+participantStreamKey",
        expected_key,
        limits,
    )?;
    for value in root.participant_streams() {
        usage.charge_declaration(EntityKind::ParticipantStream);
        usage.charge_token(
            value.participant_stream_key(),
            "participantStream.participantStreamKey",
            limits,
            expected_key,
        )?;
        usage.charge_reference(
            value.junction(),
            1,
            false,
            "participantStream.junction",
            namespace,
            imports,
            limits,
            expected_key,
        )?;
        usage.charge_reference(
            value.maneuver_path(),
            3,
            true,
            "participantStream.maneuverPath",
            namespace,
            imports,
            limits,
            expected_key,
        )?;
        let passages = value.passages();
        if passages.is_empty() {
            return Err(semantic_error(
                "participantStream.passages",
                RoadEditingInputViolation::EmptyCollection,
                expected_key,
            ));
        }
        usage.require_relation_capacity(passages.len(), limits)?;
        usage.charge_relation(passages.len());
        usage.typed_ast_record_count = usage.typed_ast_record_count.saturating_add(
            u64::try_from(passages.len())
                .unwrap_or(u64::MAX)
                .saturating_mul(3),
        );
        for (index, passage) in passages.iter().enumerate() {
            usage.charge_reference(
                passage.conflict_zone(),
                2,
                true,
                "participantStream.passages.conflictZone",
                namespace,
                imports,
                limits,
                expected_key,
            )?;
            for previous in passages.iter().take(index) {
                if references_equal(previous.conflict_zone(), passage.conflict_zone(), namespace) {
                    return Err(semantic_error(
                        "participantStream.passages.conflictZone",
                        RoadEditingInputViolation::DuplicateValue,
                        expected_key,
                    ));
                }
            }
            validate_path_anchor(
                usage,
                passage.entry(),
                "participantStream.passages.entry",
                namespace,
                imports,
                limits,
                expected_key,
            )?;
            validate_path_anchor(
                usage,
                passage.exit(),
                "participantStream.passages.exit",
                namespace,
                imports,
                limits,
                expected_key,
            )?;
        }
        usage.charge_canvas(value.canvas_selection(), limits, expected_key)?;
    }

    let regions = root.conflict_zone_regions();
    usage.require_relation_capacity(regions.len(), limits)?;
    usage.charge_relation(regions.len());
    for (index, value) in regions.iter().enumerate() {
        usage.typed_ast_record_count = usage.typed_ast_record_count.saturating_add(1);
        for previous in regions.iter().take(index) {
            if references_equal(previous.conflict_zone(), value.conflict_zone(), namespace) {
                return Err(semantic_error(
                    "conflictZoneRegions.conflictZone",
                    RoadEditingInputViolation::DuplicateValue,
                    expected_key,
                ));
            }
        }
        usage.charge_reference(
            value.conflict_zone(),
            2,
            true,
            "conflictZoneRegion.conflictZone",
            namespace,
            imports,
            limits,
            expected_key,
        )?;
        usage.charge_reference(
            value.canonical_frame(),
            1,
            true,
            "conflictZoneRegion.canonicalFrame",
            namespace,
            imports,
            limits,
            expected_key,
        )?;
        let minimum = f64::from(CANONICAL_POINT_COMPONENT_MIN_METERS);
        let maximum = f64::from(CANONICAL_POINT_COMPONENT_MAX_METERS);
        for (field, component) in [
            ("conflictZoneRegion.minY", value.min_y()),
            ("conflictZoneRegion.maxY", value.max_y()),
        ] {
            if let Some(violation) = inclusive_range_violation(component, minimum, maximum) {
                return Err(semantic_error(field, violation, expected_key));
            }
        }
        if value.min_y() >= value.max_y() || value.ring_xz().len() < 3 {
            return Err(invalid_combination("conflictZoneRegion", expected_key));
        }
        let ring_point_count = u64::try_from(value.ring_xz().len()).unwrap_or(u64::MAX);
        if ring_point_count > u64::from(MAX_CONFLICT_ZONE_REGION_RING_POINTS) {
            return Err(semantic_error(
                "conflictZoneRegion.ringXZ",
                RoadEditingInputViolation::CollectionTooLarge {
                    maximum: u64::from(MAX_CONFLICT_ZONE_REGION_RING_POINTS),
                    actual: ring_point_count,
                },
                expected_key,
            ));
        }
        for point in value.ring_xz() {
            for component in [point.x(), point.z()] {
                if let Some(violation) = inclusive_range_violation(component, minimum, maximum) {
                    return Err(semantic_error(
                        "conflictZoneRegion.ringXZ",
                        violation,
                        expected_key,
                    ));
                }
            }
        }
        usage.conflict_region_point_count = usage
            .conflict_region_point_count
            .saturating_add(u64::try_from(value.ring_xz().len()).unwrap_or(u64::MAX));
        usage.charge_canvas(value.canvas_selection(), limits, expected_key)?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn validate_path_anchor(
    usage: &mut RoadEditingPreflightCounts,
    value: wire::PathAnchor<'_>,
    field: &'static str,
    namespace: &str,
    imports: StringVector<'_>,
    limits: &PreflightScratch<'_>,
    expected_key: &str,
) -> Result<(), DiagnosticBundle> {
    let canonical_zero = |value: f64| value.to_bits() == 0.0_f64.to_bits();
    match value.kind() {
        wire::PathAnchorKind::Gate
            if value.gate().is_some()
                && value.boundary_index() == 0
                && value.path_edge_index() == 0
                && canonical_zero(value.progress_meters()) =>
        {
            usage.charge_reference(
                value.gate().expect("guarded Gate reference"),
                4,
                true,
                field,
                namespace,
                imports,
                limits,
                expected_key,
            )?;
        }
        wire::PathAnchorKind::EdgeBoundary
            if value.gate().is_none()
                && value.path_edge_index() == 0
                && canonical_zero(value.progress_meters()) => {}
        wire::PathAnchorKind::Interior if value.gate().is_none() && value.boundary_index() == 0 => {
            if let Some(violation) = millimetre_range_violation(
                value.progress_meters(),
                1,
                MAX_LANE_EDGE_LENGTH_MM.saturating_sub(1),
            ) {
                return Err(semantic_error(field, violation, expected_key));
            }
        }
        _ => return Err(invalid_combination(field, expected_key)),
    }
    Ok(())
}

fn ensure_unique_strings(
    values: StringVector<'_>,
    field: &'static str,
    expected_key: &str,
    limits: &PreflightScratch<'_>,
) -> Result<(), DiagnosticBundle> {
    ensure_unique_by(values.iter(), |value| value, field, expected_key, limits)
}

fn ensure_unique_references(
    values: StringVector<'_>,
    namespace: &str,
    field: &'static str,
    expected_key: &str,
    limits: &PreflightScratch<'_>,
) -> Result<(), DiagnosticBundle> {
    ensure_unique_by(
        values.iter(),
        |value| reference_parts(value, namespace),
        field,
        expected_key,
        limits,
    )
}

fn reference_parts<'a>(value: &'a str, namespace: &'a str) -> (&'a str, &'a str) {
    value.split_once("::").unwrap_or((namespace, value))
}

fn references_equal(left: &str, right: &str, namespace: &str) -> bool {
    reference_parts(left, namespace) == reference_parts(right, namespace)
}

fn ensure_unique_by<I, T, K, F>(
    values: I,
    key: F,
    field: &'static str,
    expected_key: &str,
    limits: &PreflightScratch<'_>,
) -> Result<(), DiagnosticBundle>
where
    I: ExactSizeIterator<Item = T>,
    K: Ord,
    F: Fn(T) -> K,
{
    if values.len() < 2 {
        return Ok(());
    }
    let mut keys = limits.collect(values.map(key))?;
    keys.sort_unstable();
    if keys.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(semantic_error(
            field,
            RoadEditingInputViolation::DuplicateValue,
            expected_key,
        ));
    }
    Ok(())
}

fn first_duplicate_index<I, T, K, F>(
    values: I,
    key: F,
    limits: &PreflightScratch<'_>,
) -> Result<Option<usize>, DiagnosticBundle>
where
    I: ExactSizeIterator<Item = T>,
    K: Ord,
    F: Fn(T) -> K,
{
    if values.len() < 2 {
        return Ok(None);
    }
    let mut keys = limits.collect(values.enumerate().map(|(index, value)| (key(value), index)))?;
    keys.sort_unstable();
    // 旧循环在每项字段检查之后查找其后继重复项；必须保留最早原始左下标，
    // 不能把字典序最小的重复键提前成首错，也不能移动该项自身的字段检查。
    Ok(keys
        .windows(2)
        .filter(|pair| pair[0].0 == pair[1].0)
        .map(|pair| pair[0].1)
        .min())
}

fn semantic_error(
    field: &'static str,
    violation: RoadEditingInputViolation,
    expected_key: &str,
) -> DiagnosticBundle {
    DiagnosticBundle::single(Diagnostic::invalid_road_editing_source(
        RoadEditingSourceViolation::InvalidSemanticValue(violation),
        Some(field),
        expected_key,
        Some(expected_key),
    ))
}

/// 生成字段取值组合无效的来源语义诊断。
pub(super) fn invalid_combination(field: &'static str, expected_key: &str) -> DiagnosticBundle {
    semantic_error(
        field,
        RoadEditingInputViolation::InvalidCombination,
        expected_key,
    )
}

fn limit_error(dimension: CompileLimitDimension, limit: u64, observed: u64) -> DiagnosticBundle {
    DiagnosticBundle::single(Diagnostic::compile_limit_exceeded(
        dimension, limit, observed,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relation_capacity_is_checked_before_usage_is_mutated() {
        let limits = CompileLimits::p100_initial_v1()
            .with_test_admission_limit(CompileLimitDimension::RelationOccurrenceCount, 2);
        let mut usage = RoadEditingPreflightCounts {
            relation_occurrence_count: 1,
            ..RoadEditingPreflightCounts::default()
        };

        assert!(usage.require_relation_capacity(2, &limits).is_err());
        assert_eq!(usage.relation_occurrence_count, 1);
        usage
            .require_relation_capacity(1, &limits)
            .expect("remaining relation capacity");
        usage.charge_relation(1);
        assert_eq!(usage.relation_occurrence_count, 2);
    }

    #[test]
    fn generated_build_id_uses_the_token_rule() {
        let limits = CompileLimits::p100_initial_v1();
        let mut usage = RoadEditingPreflightCounts::default();

        assert!(
            usage
                .charge_token(
                    "generator build",
                    "moduleHeader.provenance.generatorBuildId",
                    &limits,
                    "roads/main",
                )
                .is_err()
        );
        assert_eq!(usage.string_item_count, 0);
        assert_eq!(usage.total_string_bytes, 0);
    }

    #[test]
    fn facility_kind_categories_match_the_first_party_model() {
        let expected_key = "roads/main";

        validate_facility_kind_category(
            "motorLane",
            FacilityKindCategory::LaneBearing,
            "roadSection.kindId",
            expected_key,
        )
        .expect("lane-bearing section kind");
        validate_facility_kind_category(
            "sidewalk",
            FacilityKindCategory::NonTraversable,
            "facilityBand.kindId",
            expected_key,
        )
        .expect("non-traversable band kind");

        assert!(
            validate_facility_kind_category(
                "sidewalk",
                FacilityKindCategory::LaneBearing,
                "roadSection.kindId",
                expected_key,
            )
            .is_err()
        );
        assert!(
            validate_facility_kind_category(
                "motorLane",
                FacilityKindCategory::NonTraversable,
                "facilityBand.kindId",
                expected_key,
            )
            .is_err()
        );
        assert!(
            validate_facility_kind_category(
                "unknown",
                FacilityKindCategory::NonTraversable,
                "facilityBand.kindId",
                expected_key,
            )
            .is_err()
        );
    }

    #[test]
    fn portable_signal_time_matches_the_first_party_model() {
        let expected_key = "roads/main";

        validate_portable_signal_time(
            MAX_PORTABLE_SIGNAL_TIME_MS,
            true,
            "signalController.offsetMilliseconds",
            expected_key,
        )
        .expect("maximum controller offset");
        validate_portable_signal_time(
            MAX_PORTABLE_SIGNAL_TIME_MS,
            false,
            "signalPhase.durationMilliseconds",
            expected_key,
        )
        .expect("maximum phase duration");
        assert!(
            validate_portable_signal_time(
                0,
                false,
                "signalPhase.durationMilliseconds",
                expected_key,
            )
            .is_err()
        );
        assert!(
            validate_portable_signal_time(
                MAX_PORTABLE_SIGNAL_TIME_MS + 1,
                true,
                "signalController.offsetMilliseconds",
                expected_key,
            )
            .is_err()
        );
    }

    #[test]
    fn regulation_text_accepts_bounded_unicode_and_rejects_empty_values() {
        let limits = CompileLimits::p100_initial_v1();
        let mut usage = RoadEditingPreflightCounts::default();

        usage
            .charge_non_empty_text(
                "中国",
                "accessRegulation.jurisdiction",
                &limits,
                "roads/main",
            )
            .expect("bounded unicode");
        assert_eq!(usage.string_item_count(), 1);
        assert!(
            usage
                .charge_non_empty_text("", "accessRegulation.jurisdiction", &limits, "roads/main",)
                .is_err()
        );
    }

    #[test]
    fn canonical_points_accept_exact_boundaries_and_reject_outside_controls() {
        let minimum = f64::from(CANONICAL_POINT_COMPONENT_MIN_METERS);
        let maximum = f64::from(CANONICAL_POINT_COMPONENT_MAX_METERS);
        let boundary = wire::Vec3F64::new(minimum, 0.0, maximum);
        validate_point(&boundary, "curve.control", "roads/main").expect("inclusive bounds");

        let outside = wire::Vec3F64::new(maximum + 0.25, 0.0, 0.0);
        let error = validate_point(&outside, "curve.control", "roads/main")
            .expect_err("outside canonical frame");
        assert!(matches!(
            error.diagnostics()[0].payload(),
            crate::DiagnosticPayload::InvalidRoadEditingSource {
                violation: RoadEditingSourceViolation::InvalidSemanticValue(
                    RoadEditingInputViolation::OutsideInclusiveRange { .. }
                ),
                field: Some(field),
                ..
            } if field.as_ref() == "curve.control"
        ));
    }

    #[test]
    fn corridor_owned_references_require_the_local_corridor_key() {
        validate_corridor_owned_reference(
            "corridor-a>section",
            2,
            "corridor-a",
            "roadCorridor.referenceSection",
            "roads/main",
        )
        .expect("matching local owner");
        assert!(
            validate_corridor_owned_reference(
                "corridor-b>section",
                2,
                "corridor-a",
                "roadCorridor.referenceSection",
                "roads/main",
            )
            .is_err()
        );
        assert!(
            validate_corridor_owned_reference(
                "city/base::corridor-a>section",
                2,
                "corridor-a",
                "roadCorridor.referenceSection",
                "roads/main",
            )
            .is_err()
        );
    }

    #[test]
    fn local_reference_matching_distinguishes_missing_section_owners() {
        assert!(references_equal("corridor-a", "corridor-a", "city"));
        assert!(!references_equal("corridor-missing", "corridor-a", "city"));
        assert!(!references_equal("other::corridor-a", "corridor-a", "city"));
    }
}
