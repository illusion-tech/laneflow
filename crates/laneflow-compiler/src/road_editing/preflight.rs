//! verifier 后、任何领域分配前的道路编辑来源语义预检。

use laneflow_road_editing_wire::generated::lane_flow::road_editing::v1 as wire;
use laneflow_road_editing_wire::runtime::{ForwardsUOffset, Vector};
use laneflow_static_contract::{
    EntityKind, MIN_PARKING_EXTENT_EXCLUSIVE_METERS,
    MIN_PARKING_LATERAL_OFFSET_ABS_EXCLUSIVE_METERS, MIN_VEHICLE_LENGTH_EXCLUSIVE_METERS,
    PARKING_ANCHOR_ENDPOINT_CLEARANCE_METERS, PARKING_HEADING_OFFSET_MAXIMUM_RADIANS,
    PARKING_HEADING_OFFSET_MINIMUM_RADIANS,
};

use super::model::{
    DIRECT_FRONTEND_OPTIONS_DIGEST, DIRECT_GENERATOR_BUILD_ID, DIRECT_INPUTS_DIGEST,
};
use super::rules::{
    finite_violation, non_negative_violation, positive_violation, token_violation,
    validate_wire_reference, visible_ascii_violation,
};
use crate::{
    CompileLimitDimension, CompileLimits, Diagnostic, DiagnosticBundle, RoadEditingInputViolation,
    RoadEditingSourceViolation,
};

type StringVector<'a> = Vector<'a, ForwardsUOffset<&'a str>>;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct RoadEditingPreflightCounts {
    declaration_count: u64,
    typed_ast_record_count: u64,
    reference_count: u64,
    relation_occurrence_count: u64,
    identity_field_occurrence_count: u64,
    route_occurrence_count: u64,
    maneuver_gate_count: u64,
    waiting_zone_count: u64,
    geometry_point_count: u64,
    symbol_count: u64,
    string_item_count: u64,
    total_string_bytes: u64,
}

impl RoadEditingPreflightCounts {
    pub(crate) const fn declaration_count(self) -> u64 {
        self.declaration_count
    }

    pub(crate) const fn typed_ast_record_count(self) -> u64 {
        self.typed_ast_record_count
    }

    pub(crate) const fn reference_count(self) -> u64 {
        self.reference_count
    }

    pub(crate) const fn relation_occurrence_count(self) -> u64 {
        self.relation_occurrence_count
    }

    pub(crate) const fn identity_field_occurrence_count(self) -> u64 {
        self.identity_field_occurrence_count
    }

    pub(crate) const fn route_occurrence_count(self) -> u64 {
        self.route_occurrence_count
    }

    pub(crate) const fn maneuver_gate_count(self) -> u64 {
        self.maneuver_gate_count
    }

    pub(crate) const fn waiting_zone_count(self) -> u64 {
        self.waiting_zone_count
    }

    pub(crate) const fn geometry_point_count(self) -> u64 {
        self.geometry_point_count
    }

    pub(crate) const fn symbol_count(self) -> u64 {
        self.symbol_count
    }

    pub(crate) const fn string_item_count(self) -> u64 {
        self.string_item_count
    }

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
        if let Some(namespace) = parsed.namespace()
            && namespace != current_namespace
            && !imports.iter().any(|import| import == namespace)
        {
            return Err(semantic_error(
                field,
                RoadEditingInputViolation::InvalidCombination,
                expected_key,
            ));
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
                CompileLimitDimension::RouteOccurrenceCount,
                self.route_occurrence_count,
            ),
            (
                CompileLimitDimension::ManeuverGateCount,
                self.maneuver_gate_count,
            ),
            (
                CompileLimitDimension::WaitingZoneCount,
                self.waiting_zone_count,
            ),
            (
                CompileLimitDimension::GeometryPointCount,
                self.geometry_point_count,
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

pub(crate) fn preflight_source(
    root: wire::RoadEditingSource<'_>,
    limits: &CompileLimits,
    expected_key: &str,
) -> Result<RoadEditingPreflightCounts, DiagnosticBundle> {
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
    ensure_unique_strings(imports, "moduleHeader.imports", expected_key)?;
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
    let import_count = u64::try_from(imports.len()).unwrap_or(u64::MAX);
    let import_limit = limits.value(CompileLimitDimension::ImportEdgeCount);
    if import_count > import_limit {
        return Err(limit_error(
            CompileLimitDimension::ImportEdgeCount,
            import_limit,
            import_count,
        ));
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

    usage.validate(limits)
}

fn validate_provenance(
    usage: &mut RoadEditingPreflightCounts,
    provenance: wire::Provenance<'_>,
    limits: &CompileLimits,
    expected_key: &str,
) -> Result<(), DiagnosticBundle> {
    usage.charge_visible_ascii(
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
    limits: &CompileLimits,
    expected_key: &str,
) -> Result<(), DiagnosticBundle> {
    ensure_unique_by(
        root.road_alignments().iter(),
        |value| value.road_alignment_key(),
        "roadAlignments.roadAlignmentKey",
        expected_key,
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
    limits: &CompileLimits,
    expected_key: &str,
) -> Result<(), DiagnosticBundle> {
    usage.typed_ast_record_count = usage.typed_ast_record_count.saturating_add(1);
    usage.geometry_point_count = usage.geometry_point_count.saturating_add(1);
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
                usage.geometry_point_count = usage.geometry_point_count.saturating_add(1);
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
                usage.geometry_point_count = usage.geometry_point_count.saturating_add(3);
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
        if let Some(violation) = finite_violation(component) {
            return Err(semantic_error(field, violation, expected_key));
        }
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
    limits: &CompileLimits,
    expected_key: &str,
) -> Result<(), DiagnosticBundle> {
    if non_empty && values.is_empty() {
        return Err(semantic_error(
            field,
            RoadEditingInputViolation::EmptyCollection,
            expected_key,
        ));
    }
    if unique {
        ensure_unique_references(values, namespace, field, expected_key)?;
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
    limits: &CompileLimits,
    expected_key: &str,
) -> Result<(), DiagnosticBundle> {
    ensure_unique_by(
        root.road_corridors().iter(),
        |value| value.road_corridor_key(),
        "roadCorridors.roadCorridorKey",
        expected_key,
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
        let elements = value.elements();
        if elements.is_empty() {
            return Err(semantic_error(
                "roadCorridor.elements",
                RoadEditingInputViolation::EmptyCollection,
                expected_key,
            ));
        }
        usage.charge_relation(elements.len());
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
            for other in elements.iter().skip(index + 1) {
                if element.kind() == other.kind()
                    && references_equal(
                        element.entity_reference(),
                        other.entity_reference(),
                        namespace,
                    )
                {
                    return Err(semantic_error(
                        "roadCorridor.elements",
                        RoadEditingInputViolation::DuplicateValue,
                        expected_key,
                    ));
                }
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
    limits: &CompileLimits,
    expected_key: &str,
) -> Result<(), DiagnosticBundle> {
    ensure_unique_by(
        root.road_sections().iter(),
        |value| (value.road_corridor(), value.road_section_key()),
        "roadSections.address",
        expected_key,
    )?;
    for value in root.road_sections() {
        usage.charge_declaration(EntityKind::RoadSection);
        usage.charge_token(
            value.road_section_key(),
            "roadSection.roadSectionKey",
            limits,
            expected_key,
        )?;
        usage.charge_token(value.kind_id(), "roadSection.kindId", limits, expected_key)?;
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
        usage.charge_canvas(value.canvas_selection(), limits, expected_key)?;
    }
    Ok(())
}

fn validate_authoring_lanes(
    usage: &mut RoadEditingPreflightCounts,
    root: wire::RoadEditingSource<'_>,
    namespace: &str,
    imports: StringVector<'_>,
    limits: &CompileLimits,
    expected_key: &str,
) -> Result<(), DiagnosticBundle> {
    ensure_unique_by(
        root.authoring_lanes().iter(),
        |value| (value.road_section(), value.authoring_lane_key()),
        "authoringLanes.address",
        expected_key,
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
    limits: &CompileLimits,
    expected_key: &str,
) -> Result<(), DiagnosticBundle> {
    ensure_unique_by(
        root.lane_edges().iter(),
        |value| value.lane_edge_key(),
        "laneEdges.laneEdgeKey",
        expected_key,
    )?;
    for value in root.lane_edges() {
        usage.charge_declaration(EntityKind::LaneEdge);
        usage.charge_token(
            value.lane_edge_key(),
            "laneEdge.laneEdgeKey",
            limits,
            expected_key,
        )?;
        if let Some(violation) = positive_violation(value.speed_limit_meters_per_second()) {
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
    limits: &CompileLimits,
    expected_key: &str,
) -> Result<(), DiagnosticBundle> {
    ensure_unique_by(
        root.junctions().iter(),
        |value| value.junction_key(),
        "junctions.junctionKey",
        expected_key,
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
            true,
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
    limits: &CompileLimits,
    expected_key: &str,
) -> Result<(), DiagnosticBundle> {
    ensure_unique_by(
        root.movements().iter(),
        |value| (value.junction(), value.movement_key()),
        "movements.address",
        expected_key,
    )?;
    for value in root.movements() {
        usage.charge_declaration(EntityKind::Movement);
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
    limits: &CompileLimits,
    expected_key: &str,
) -> Result<(), DiagnosticBundle> {
    ensure_unique_by(
        root.maneuver_paths().iter(),
        |value| (value.movement(), value.maneuver_path_key()),
        "maneuverPaths.address",
        expected_key,
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
    limits: &CompileLimits,
    expected_key: &str,
) -> Result<(), DiagnosticBundle> {
    ensure_unique_by(
        root.maneuver_gates().iter(),
        |value| (value.maneuver_path(), value.maneuver_gate_key()),
        "maneuverGates.address",
        expected_key,
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
    limits: &CompileLimits,
    expected_key: &str,
) -> Result<(), DiagnosticBundle> {
    ensure_unique_by(
        root.waiting_zones().iter(),
        |value| (value.maneuver_path(), value.waiting_zone_key()),
        "waitingZones.address",
        expected_key,
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
    limits: &CompileLimits,
    expected_key: &str,
) -> Result<(), DiagnosticBundle> {
    ensure_unique_by(
        root.stop_lines().iter(),
        |value| value.stop_line_key(),
        "stopLines.stopLineKey",
        expected_key,
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
    limits: &CompileLimits,
    expected_key: &str,
) -> Result<(), DiagnosticBundle> {
    ensure_unique_by(
        root.signal_controllers().iter(),
        |value| value.signal_controller_key(),
        "signalControllers.signalControllerKey",
        expected_key,
    )?;
    for value in root.signal_controllers() {
        usage.charge_declaration(EntityKind::SignalController);
        usage.charge_token(
            value.signal_controller_key(),
            "signalController.signalControllerKey",
            limits,
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
    )?;
    for value in root.signal_phases() {
        usage.charge_declaration(EntityKind::SignalPhase);
        usage.charge_token(
            value.signal_phase_key(),
            "signalPhase.signalPhaseKey",
            limits,
            expected_key,
        )?;
        if value.duration_milliseconds() == 0 {
            return Err(invalid_combination(
                "signalPhase.durationMilliseconds",
                expected_key,
            ));
        }
        let states = value.states();
        if states.is_empty() {
            return Err(semantic_error(
                "signalPhase.states",
                RoadEditingInputViolation::EmptyCollection,
                expected_key,
            ));
        }
        usage.charge_relation(states.len());
        for (index, state) in states.iter().enumerate() {
            usage.typed_ast_record_count = usage.typed_ast_record_count.saturating_add(1);
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
            for other in states.iter().skip(index + 1) {
                if references_equal(state.signal_group(), other.signal_group(), namespace) {
                    return Err(semantic_error(
                        "signalPhase.states.signalGroup",
                        RoadEditingInputViolation::DuplicateValue,
                        expected_key,
                    ));
                }
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
    limits: &CompileLimits,
    expected_key: &str,
) -> Result<(), DiagnosticBundle> {
    ensure_unique_by(
        root.parking_areas().iter(),
        |value| value.parking_area_key(),
        "parkingAreas.parkingAreaKey",
        expected_key,
    )?;
    for value in root.parking_areas() {
        usage.charge_declaration(EntityKind::ParkingArea);
        usage.charge_token(
            value.parking_area_key(),
            "parkingArea.parkingAreaKey",
            limits,
            expected_key,
        )?;
        usage.charge_canvas(value.canvas_selection(), limits, expected_key)?;
    }

    ensure_unique_by(
        root.parking_spaces().iter(),
        |value| value.parking_space_key(),
        "parkingSpaces.parkingSpaceKey",
        expected_key,
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
        if let Some(area) = value.parking_area() {
            usage.charge_reference(
                area,
                1,
                true,
                "parkingSpace.parkingArea",
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
        if let Some(violation) = finite_violation(geometry.lateral_offset_meters()) {
            return Err(semantic_error(
                "parkingSpace.geometry.lateralOffsetMeters",
                violation,
                expected_key,
            ));
        }
        if geometry.lateral_offset_meters().abs() <= MIN_PARKING_LATERAL_OFFSET_ABS_EXCLUSIVE_METERS
        {
            return Err(invalid_combination(
                "parkingSpace.geometry.lateralOffsetMeters",
                expected_key,
            ));
        }
        if let Some(violation) = finite_violation(geometry.heading_offset_radians()) {
            return Err(semantic_error(
                "parkingSpace.geometry.headingOffsetRadians",
                violation,
                expected_key,
            ));
        }
        if !(PARKING_HEADING_OFFSET_MINIMUM_RADIANS..PARKING_HEADING_OFFSET_MAXIMUM_RADIANS)
            .contains(&geometry.heading_offset_radians())
        {
            return Err(invalid_combination(
                "parkingSpace.geometry.headingOffsetRadians",
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
            if let Some(violation) = positive_violation(extent) {
                return Err(semantic_error(field, violation, expected_key));
            }
            if extent <= MIN_PARKING_EXTENT_EXCLUSIVE_METERS {
                return Err(invalid_combination(field, expected_key));
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
    limits: &CompileLimits,
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
    if let Some(violation) = positive_violation(value.progress_meters()) {
        return Err(semantic_error(field, violation, expected_key));
    }
    if value.progress_meters() <= PARKING_ANCHOR_ENDPOINT_CLEARANCE_METERS {
        return Err(invalid_combination(field, expected_key));
    }
    Ok(())
}

fn validate_lane_groups_and_facility_bands(
    usage: &mut RoadEditingPreflightCounts,
    root: wire::RoadEditingSource<'_>,
    namespace: &str,
    imports: StringVector<'_>,
    limits: &CompileLimits,
    expected_key: &str,
) -> Result<(), DiagnosticBundle> {
    ensure_unique_by(
        root.lane_groups().iter(),
        |value| (value.road_section(), value.lane_group_key()),
        "laneGroups.address",
        expected_key,
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
    limits: &CompileLimits,
    expected_key: &str,
) -> Result<(), DiagnosticBundle> {
    ensure_unique_by(
        root.participant_classes().iter(),
        |value| value.participant_class_key(),
        "participantClasses.participantClassKey",
        expected_key,
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
            usage.charge_visible_ascii(
                regulation.jurisdiction(),
                "accessRegulation.jurisdiction",
                limits,
                expected_key,
            )?;
            usage.charge_visible_ascii(
                regulation.version(),
                "accessRegulation.version",
                limits,
                expected_key,
            )?;
            if let Some(source) = regulation.source() {
                usage.charge_visible_ascii(
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
    let positive = [
        ("vehicleProfile.iidm.lengthMeters", value.length_meters()),
        (
            "vehicleProfile.iidm.desiredSpeedMetersPerSecond",
            value.desired_speed_meters_per_second(),
        ),
        (
            "vehicleProfile.iidm.timeHeadwaySeconds",
            value.time_headway_seconds(),
        ),
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
    ];
    for (field, number) in positive {
        if let Some(violation) = positive_violation(number) {
            return Err(semantic_error(field, violation, expected_key));
        }
    }
    if value.length_meters() <= MIN_VEHICLE_LENGTH_EXCLUSIVE_METERS {
        return Err(invalid_combination(
            "vehicleProfile.iidm.lengthMeters",
            expected_key,
        ));
    }
    if let Some(violation) = non_negative_violation(value.min_gap_meters()) {
        return Err(semantic_error(
            "vehicleProfile.iidm.minGapMeters",
            violation,
            expected_key,
        ));
    }
    if value.emergency_deceleration_meters_per_second_squared()
        < value.comfortable_deceleration_meters_per_second_squared()
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
    limits: &CompileLimits,
    expected_key: &str,
) -> Result<(), DiagnosticBundle> {
    ensure_unique_by(
        root.static_routes().iter(),
        |value| value.static_route_key(),
        "staticRoutes.staticRouteKey",
        expected_key,
    )?;
    for value in root.static_routes() {
        usage.charge_declaration(EntityKind::StaticRoute);
        usage.charge_token(
            value.static_route_key(),
            "staticRoute.staticRouteKey",
            limits,
            expected_key,
        )?;
        let edges = value.edge_sequence();
        if edges.is_empty() {
            return Err(semantic_error(
                "staticRoute.edgeSequence",
                RoadEditingInputViolation::EmptyCollection,
                expected_key,
            ));
        }
        usage.route_occurrence_count = usage
            .route_occurrence_count
            .saturating_add(u64::try_from(edges.len()).unwrap_or(u64::MAX));
        validate_reference_vector(
            usage,
            edges,
            1,
            "staticRoute.edgeSequence",
            false,
            false,
            true,
            namespace,
            imports,
            limits,
            expected_key,
        )?;
        usage.charge_canvas(value.canvas_selection(), limits, expected_key)?;
    }

    ensure_unique_by(
        root.canonical_frames().iter(),
        |value| value.canonical_frame_key(),
        "canonicalFrames.canonicalFrameKey",
        expected_key,
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

fn ensure_unique_strings(
    values: StringVector<'_>,
    field: &'static str,
    expected_key: &str,
) -> Result<(), DiagnosticBundle> {
    for left_index in 0..values.len() {
        let left = values.get(left_index);
        for right_index in left_index + 1..values.len() {
            if left == values.get(right_index) {
                return Err(semantic_error(
                    field,
                    RoadEditingInputViolation::DuplicateValue,
                    expected_key,
                ));
            }
        }
    }
    Ok(())
}

fn ensure_unique_references(
    values: StringVector<'_>,
    namespace: &str,
    field: &'static str,
    expected_key: &str,
) -> Result<(), DiagnosticBundle> {
    for left_index in 0..values.len() {
        let left = values.get(left_index);
        for right_index in left_index + 1..values.len() {
            if references_equal(left, values.get(right_index), namespace) {
                return Err(semantic_error(
                    field,
                    RoadEditingInputViolation::DuplicateValue,
                    expected_key,
                ));
            }
        }
    }
    Ok(())
}

fn references_equal(left: &str, right: &str, namespace: &str) -> bool {
    fn parts<'a>(value: &'a str, namespace: &'a str) -> (&'a str, &'a str) {
        match value.split_once("::") {
            Some((module, key)) => (module, key),
            None => (namespace, value),
        }
    }
    parts(left, namespace) == parts(right, namespace)
}

fn ensure_unique_by<I, T, K, F>(
    values: I,
    key: F,
    field: &'static str,
    expected_key: &str,
) -> Result<(), DiagnosticBundle>
where
    I: Clone + Iterator<Item = T>,
    T: Copy,
    K: PartialEq,
    F: Copy + Fn(T) -> K,
{
    for (index, left) in values.clone().enumerate() {
        if values
            .clone()
            .skip(index + 1)
            .any(|right| key(left) == key(right))
        {
            return Err(semantic_error(
                field,
                RoadEditingInputViolation::DuplicateValue,
                expected_key,
            ));
        }
    }
    Ok(())
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

fn invalid_combination(field: &'static str, expected_key: &str) -> DiagnosticBundle {
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
