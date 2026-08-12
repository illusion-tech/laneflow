//! verifier 后、任何领域分配前的道路编辑来源语义预检。

use std::cmp::Ordering;
use std::ops::{Deref, DerefMut};

use laneflow_road_editing_wire::generated::lane_flow::road_editing::v1 as wire;
use laneflow_road_editing_wire::runtime::{ForwardsUOffset, Vector};
use laneflow_static_contract::{
    CANONICAL_POINT_COMPONENT_MAX_METERS, CANONICAL_POINT_COMPONENT_MIN_METERS, EntityKind,
    MIN_PARKING_EXTENT_EXCLUSIVE_METERS, MIN_PARKING_LATERAL_OFFSET_ABS_EXCLUSIVE_METERS,
    MIN_VEHICLE_LENGTH_EXCLUSIVE_METERS, PARKING_ANCHOR_ENDPOINT_CLEARANCE_METERS,
    PARKING_HEADING_OFFSET_MAXIMUM_RADIANS, PARKING_HEADING_OFFSET_MINIMUM_RADIANS,
};

use super::location::{
    RoadEditingLocationFactory, SemanticPreflightSite, SemanticPreflightSubjectSite,
};
use super::model::{
    DIRECT_FRONTEND_OPTIONS_DIGEST, DIRECT_GENERATOR_BUILD_ID, DIRECT_INPUTS_DIGEST,
};
use super::rules::{
    finite_violation, inclusive_range_violation, non_negative_violation, positive_violation,
    token_violation, validate_wire_reference, visible_ascii_violation,
};
use crate::{
    CompileLimitDimension, CompileLimits, Diagnostic, DiagnosticBundle, DiagnosticPayload,
    RoadEditingInputViolation, RoadEditingRelationKind, RoadEditingRelationOccurrence,
    RoadEditingRootVectorKind, RoadEditingSourceViolation,
};

type StringVector<'a> = Vector<'a, ForwardsUOffset<&'a str>>;
type SignalPhaseStateVector<'a> = Vector<'a, ForwardsUOffset<wire::SignalPhaseState<'a>>>;

#[derive(Clone, Copy)]
enum ReferenceOccurrenceKind {
    Ordered,
    Canonical,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct RoadEditingPreflightCounts {
    declaration_count: u64,
    typed_ast_record_count: u64,
    reference_count: u64,
    external_namespace_reference_count: u64,
    relation_occurrence_count: u64,
    identity_field_occurrence_count: u64,
    route_occurrence_count: u64,
    maneuver_gate_count: u64,
    waiting_zone_count: u64,
    authoring_point_count: u64,
    symbol_count: u64,
    string_item_count: u64,
    total_string_bytes: u64,
}

struct RoadEditingPreflightState {
    counts: RoadEditingPreflightCounts,
    failure_subject: Option<SemanticPreflightSubjectSite>,
}

impl RoadEditingPreflightState {
    fn new(counts: RoadEditingPreflightCounts) -> Self {
        Self {
            counts,
            failure_subject: Some(SemanticPreflightSubjectSite::ModuleHeader),
        }
    }

    fn clear_site(&mut self) {
        self.failure_subject = None;
    }

    fn set_root_site(&mut self, vector: RoadEditingRootVectorKind, physical_index: usize) {
        self.failure_subject = Some(SemanticPreflightSubjectSite::Root {
            vector,
            physical_index: u32::try_from(physical_index).unwrap_or(u32::MAX),
        });
    }

    fn set_wire_site(&mut self, vector: RoadEditingRootVectorKind, physical_index: usize) {
        self.failure_subject = Some(SemanticPreflightSubjectSite::Wire {
            vector,
            physical_index: u32::try_from(physical_index).unwrap_or(u32::MAX),
        });
    }

    fn set_owner_local_site(
        &mut self,
        owner_vector: RoadEditingRootVectorKind,
        owner_physical_index: usize,
        relation: RoadEditingRelationKind,
        occurrence: RoadEditingRelationOccurrence,
    ) {
        self.failure_subject = Some(SemanticPreflightSubjectSite::OwnerLocal {
            owner_vector,
            owner_physical_index: u32::try_from(owner_physical_index).unwrap_or(u32::MAX),
            relation,
            occurrence,
        });
    }
}

impl Deref for RoadEditingPreflightState {
    type Target = RoadEditingPreflightCounts;

    fn deref(&self) -> &Self::Target {
        &self.counts
    }
}

impl DerefMut for RoadEditingPreflightState {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.counts
    }
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

    pub(crate) const fn external_namespace_reference_count(self) -> u64 {
        self.external_namespace_reference_count
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
        {
            if !imports.iter().any(|import| import == namespace) {
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
    let mut usage = RoadEditingPreflightState::new(RoadEditingPreflightCounts {
        typed_ast_record_count: 1, // root 与 Provenance 不计；ModuleHeader 计一条
        ..RoadEditingPreflightCounts::default()
    });

    let result = (|| {
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
        ensure_unique_strings(imports, "moduleHeader.imports", expected_key)?;
        for import in imports {
            usage.failure_subject = Some(SemanticPreflightSubjectSite::ModuleHeader);
            usage.charge_token(import, "moduleHeader.imports", limits, expected_key)?;
            let canonical_ordinal = imports
                .iter()
                .filter(|other| other.as_bytes() < import.as_bytes())
                .count();
            usage.failure_subject = Some(SemanticPreflightSubjectSite::ModuleOwnerLocal {
                relation: RoadEditingRelationKind::Import,
                occurrence: RoadEditingRelationOccurrence::CanonicalSetOrdinal(
                    u32::try_from(canonical_ordinal).unwrap_or(u32::MAX),
                ),
            });
            if import == namespace {
                return Err(semantic_error(
                    "moduleHeader.imports",
                    RoadEditingInputViolation::InvalidCombination,
                    expected_key,
                ));
            }
        }
        usage.failure_subject = Some(SemanticPreflightSubjectSite::ModuleHeader);
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

        usage.clear_site();
        validate_alignments(&mut usage, root, namespace, imports, limits, expected_key)?;
        usage.clear_site();
        validate_corridors(&mut usage, root, namespace, imports, limits, expected_key)?;
        usage.clear_site();
        validate_sections(&mut usage, root, namespace, imports, limits, expected_key)?;
        usage.clear_site();
        validate_authoring_lanes(&mut usage, root, namespace, imports, limits, expected_key)?;
        usage.clear_site();
        validate_lane_edges(&mut usage, root, namespace, imports, limits, expected_key)?;
        usage.clear_site();
        validate_junctions(&mut usage, root, namespace, imports, limits, expected_key)?;
        usage.clear_site();
        validate_movements(&mut usage, root, namespace, imports, limits, expected_key)?;
        usage.clear_site();
        validate_maneuver_paths(&mut usage, root, namespace, imports, limits, expected_key)?;
        usage.clear_site();
        validate_maneuver_gates(&mut usage, root, namespace, imports, limits, expected_key)?;
        usage.clear_site();
        validate_waiting_zones(&mut usage, root, namespace, imports, limits, expected_key)?;
        usage.clear_site();
        validate_stop_lines_and_signal_groups(
            &mut usage,
            root,
            namespace,
            imports,
            limits,
            expected_key,
        )?;
        usage.clear_site();
        validate_signal_controllers_and_phases(
            &mut usage,
            root,
            namespace,
            imports,
            limits,
            expected_key,
        )?;
        usage.clear_site();
        validate_parking(&mut usage, root, namespace, imports, limits, expected_key)?;
        usage.clear_site();
        validate_lane_groups_and_facility_bands(
            &mut usage,
            root,
            namespace,
            imports,
            limits,
            expected_key,
        )?;
        usage.clear_site();
        validate_access_and_profiles(&mut usage, root, namespace, imports, limits, expected_key)?;
        usage.clear_site();
        validate_routes_and_frames(&mut usage, root, namespace, imports, limits, expected_key)?;

        usage.clear_site();
        usage.counts.validate(limits)
    })();

    result.map_err(|bundle| {
        with_semantic_preflight_location(root, expected_key, usage.failure_subject, bundle)
    })
}

fn validate_provenance(
    usage: &mut RoadEditingPreflightState,
    provenance: wire::Provenance<'_>,
    limits: &CompileLimits,
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
    usage: &mut RoadEditingPreflightState,
    root: wire::RoadEditingSource<'_>,
    namespace: &str,
    imports: StringVector<'_>,
    limits: &CompileLimits,
    expected_key: &str,
) -> Result<(), DiagnosticBundle> {
    ensure_unique_by(
        usage,
        RoadEditingRootVectorKind::RoadAlignment,
        root.road_alignments().iter(),
        |value| value.road_alignment_key(),
        "roadAlignments.roadAlignmentKey",
        expected_key,
    )?;
    for (physical_index, value) in root.road_alignments().iter().enumerate() {
        usage.set_root_site(RoadEditingRootVectorKind::RoadAlignment, physical_index);
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
        validate_curve(
            usage,
            value.reference_line(),
            RoadEditingRootVectorKind::RoadAlignment,
            physical_index,
            limits,
            expected_key,
        )?;
        usage.set_root_site(RoadEditingRootVectorKind::RoadAlignment, physical_index);
        usage.charge_canvas(value.canvas_selection(), limits, expected_key)?;
    }
    Ok(())
}

fn validate_curve(
    usage: &mut RoadEditingPreflightState,
    value: wire::CurveProgram<'_>,
    owner_vector: RoadEditingRootVectorKind,
    owner_physical_index: usize,
    limits: &CompileLimits,
    expected_key: &str,
) -> Result<(), DiagnosticBundle> {
    usage.typed_ast_record_count = usage.typed_ast_record_count.saturating_add(1);
    usage.authoring_point_count = usage.authoring_point_count.saturating_add(1);
    let start_fields = match owner_vector {
        RoadEditingRootVectorKind::RoadAlignment => [
            "roadAlignment.referenceLine.start.x",
            "roadAlignment.referenceLine.start.y",
            "roadAlignment.referenceLine.start.z",
        ],
        RoadEditingRootVectorKind::LaneEdge => [
            "laneEdge.explicitGeometry.start.x",
            "laneEdge.explicitGeometry.start.y",
            "laneEdge.explicitGeometry.start.z",
        ],
        _ => unreachable!("only alignment and lane-edge declarations own curve programs"),
    };
    validate_point(value.start(), start_fields, expected_key)?;
    if value.segments().is_empty() {
        return Err(semantic_error(
            "curveProgram.segments",
            RoadEditingInputViolation::EmptyCollection,
            expected_key,
        ));
    }
    for (segment_index, segment) in value.segments().iter().enumerate() {
        usage.set_owner_local_site(
            owner_vector,
            owner_physical_index,
            RoadEditingRelationKind::CurveSegment,
            RoadEditingRelationOccurrence::OrderedProductOrdinal(
                u32::try_from(segment_index).unwrap_or(u32::MAX),
            ),
        );
        usage.typed_ast_record_count = usage.typed_ast_record_count.saturating_add(2);
        match segment.geometry_type() {
            wire::CurveSegmentGeometry::LineSegment => {
                let geometry = segment
                    .geometry_as_line_segment()
                    .ok_or_else(|| invalid_combination("curveSegment.geometry", expected_key))?;
                validate_point(
                    geometry.end(),
                    [
                        "curveSegment.geometry.line.end.x",
                        "curveSegment.geometry.line.end.y",
                        "curveSegment.geometry.line.end.z",
                    ],
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
                    [
                        "curveSegment.geometry.cubic.control1.x",
                        "curveSegment.geometry.cubic.control1.y",
                        "curveSegment.geometry.cubic.control1.z",
                    ],
                    expected_key,
                )?;
                validate_point(
                    geometry.control_2(),
                    [
                        "curveSegment.geometry.cubic.control2.x",
                        "curveSegment.geometry.cubic.control2.y",
                        "curveSegment.geometry.cubic.control2.z",
                    ],
                    expected_key,
                )?;
                validate_point(
                    geometry.end(),
                    [
                        "curveSegment.geometry.cubic.end.x",
                        "curveSegment.geometry.cubic.end.y",
                        "curveSegment.geometry.cubic.end.z",
                    ],
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
    fields: [&'static str; 3],
    expected_key: &str,
) -> Result<(), DiagnosticBundle> {
    for (component, field) in [value.x(), value.y(), value.z()].into_iter().zip(fields) {
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
    fields: [&'static str; 2],
    expected_key: &str,
) -> Result<(), DiagnosticBundle> {
    if let Some(violation) = non_negative_violation(value.start_width_meters()) {
        return Err(semantic_error(fields[0], violation, expected_key));
    }
    if let Some(violation) = non_negative_violation(value.end_width_meters()) {
        return Err(semantic_error(fields[1], violation, expected_key));
    }
    if value.start_width_meters() == 0.0 && value.end_width_meters() == 0.0 {
        return Err(invalid_combination(fields[1], expected_key));
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn validate_reference_vector(
    usage: &mut RoadEditingPreflightState,
    values: StringVector<'_>,
    component_count: u8,
    field: &'static str,
    non_empty: bool,
    unique: bool,
    relation: bool,
    owner_vector: RoadEditingRootVectorKind,
    owner_physical_index: usize,
    relation_kind: RoadEditingRelationKind,
    occurrence_kind: ReferenceOccurrenceKind,
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
    if relation {
        usage.require_relation_capacity(values.len(), limits)?;
    }
    if unique {
        ensure_unique_references(values, namespace, field, expected_key)?;
    }
    if relation {
        usage.charge_relation(values.len());
    }
    for (physical_index, value) in values.iter().enumerate() {
        usage.set_wire_site(owner_vector, owner_physical_index);
        let occurrence = match occurrence_kind {
            ReferenceOccurrenceKind::Ordered => {
                validate_wire_reference(value, component_count, true)
                    .ok()
                    .map(|_| {
                        RoadEditingRelationOccurrence::OrderedProductOrdinal(
                            u32::try_from(physical_index).unwrap_or(u32::MAX),
                        )
                    })
            }
            ReferenceOccurrenceKind::Canonical => {
                canonical_reference_ordinal(values, value, component_count, namespace)
                    .map(RoadEditingRelationOccurrence::CanonicalSetOrdinal)
            }
        };
        if let Some(occurrence) = occurrence {
            usage.set_owner_local_site(
                owner_vector,
                owner_physical_index,
                relation_kind,
                occurrence,
            );
        }
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
    usage.set_root_site(owner_vector, owner_physical_index);
    Ok(())
}

fn canonical_reference_ordinal(
    values: StringVector<'_>,
    target: &str,
    component_count: u8,
    namespace: &str,
) -> Option<u32> {
    validate_wire_reference(target, component_count, true).ok()?;
    let mut ordinal = 0_u32;
    for value in values {
        let ordering = compare_references(value, target, component_count, namespace)?;
        if ordering == Ordering::Less {
            ordinal = ordinal.saturating_add(1);
        }
    }
    Some(ordinal)
}

fn compare_references(
    left: &str,
    right: &str,
    component_count: u8,
    namespace: &str,
) -> Option<Ordering> {
    let left = validate_wire_reference(left, component_count, true).ok()?;
    let right = validate_wire_reference(right, component_count, true).ok()?;
    Some(
        left.namespace()
            .unwrap_or(namespace)
            .as_bytes()
            .cmp(right.namespace().unwrap_or(namespace).as_bytes())
            .then_with(|| {
                left.key_components()
                    .map(str::as_bytes)
                    .cmp(right.key_components().map(str::as_bytes))
            }),
    )
}

fn canonical_signal_phase_state_ordinal(
    states: SignalPhaseStateVector<'_>,
    target: &str,
    namespace: &str,
) -> Option<u32> {
    validate_wire_reference(target, 1, true).ok()?;
    let mut ordinal = 0_u32;
    for state in states {
        let ordering = compare_references(state.signal_group(), target, 1, namespace)?;
        if ordering == Ordering::Less {
            ordinal = ordinal.saturating_add(1);
        }
    }
    Some(ordinal)
}

fn validate_corridors(
    usage: &mut RoadEditingPreflightState,
    root: wire::RoadEditingSource<'_>,
    namespace: &str,
    imports: StringVector<'_>,
    limits: &CompileLimits,
    expected_key: &str,
) -> Result<(), DiagnosticBundle> {
    ensure_unique_by(
        usage,
        RoadEditingRootVectorKind::RoadCorridor,
        root.road_corridors().iter(),
        |value| value.road_corridor_key(),
        "roadCorridors.roadCorridorKey",
        expected_key,
    )?;
    for (physical_index, value) in root.road_corridors().iter().enumerate() {
        usage.set_root_site(RoadEditingRootVectorKind::RoadCorridor, physical_index);
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
        for (index, element) in elements.iter().enumerate() {
            usage.set_owner_local_site(
                RoadEditingRootVectorKind::RoadCorridor,
                physical_index,
                RoadEditingRelationKind::CorridorElement,
                RoadEditingRelationOccurrence::OrderedProductOrdinal(
                    u32::try_from(index).unwrap_or(u32::MAX),
                ),
            );
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
        usage.set_root_site(RoadEditingRootVectorKind::RoadCorridor, physical_index);
        usage.charge_canvas(value.canvas_selection(), limits, expected_key)?;
    }
    Ok(())
}

fn validate_sections(
    usage: &mut RoadEditingPreflightState,
    root: wire::RoadEditingSource<'_>,
    namespace: &str,
    imports: StringVector<'_>,
    limits: &CompileLimits,
    expected_key: &str,
) -> Result<(), DiagnosticBundle> {
    ensure_unique_by(
        usage,
        RoadEditingRootVectorKind::RoadSection,
        root.road_sections().iter(),
        |value| (value.road_corridor(), value.road_section_key()),
        "roadSections.address",
        expected_key,
    )?;
    for (physical_index, value) in root.road_sections().iter().enumerate() {
        usage.set_root_site(RoadEditingRootVectorKind::RoadSection, physical_index);
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
            RoadEditingRootVectorKind::RoadSection,
            physical_index,
            RoadEditingRelationKind::RoadSectionAuthoringLane,
            ReferenceOccurrenceKind::Ordered,
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
        if !root.road_corridors().iter().any(|corridor| {
            references_equal(
                value.road_corridor(),
                corridor.road_corridor_key(),
                namespace,
            )
        }) {
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
    usage: &mut RoadEditingPreflightState,
    root: wire::RoadEditingSource<'_>,
    namespace: &str,
    imports: StringVector<'_>,
    limits: &CompileLimits,
    expected_key: &str,
) -> Result<(), DiagnosticBundle> {
    ensure_unique_by(
        usage,
        RoadEditingRootVectorKind::AuthoringLane,
        root.authoring_lanes().iter(),
        |value| (value.road_section(), value.authoring_lane_key()),
        "authoringLanes.address",
        expected_key,
    )?;
    for (physical_index, value) in root.authoring_lanes().iter().enumerate() {
        usage.set_root_site(RoadEditingRootVectorKind::AuthoringLane, physical_index);
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
            [
                "authoringLane.widthProfile.startWidthMeters",
                "authoringLane.widthProfile.endWidthMeters",
            ],
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
    usage: &mut RoadEditingPreflightState,
    root: wire::RoadEditingSource<'_>,
    namespace: &str,
    imports: StringVector<'_>,
    limits: &CompileLimits,
    expected_key: &str,
) -> Result<(), DiagnosticBundle> {
    ensure_unique_by(
        usage,
        RoadEditingRootVectorKind::LaneEdge,
        root.lane_edges().iter(),
        |value| value.lane_edge_key(),
        "laneEdges.laneEdgeKey",
        expected_key,
    )?;
    for (physical_index, value) in root.lane_edges().iter().enumerate() {
        usage.set_root_site(RoadEditingRootVectorKind::LaneEdge, physical_index);
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
            RoadEditingRootVectorKind::LaneEdge,
            physical_index,
            RoadEditingRelationKind::LaneEdgeSuccessor,
            ReferenceOccurrenceKind::Canonical,
            namespace,
            imports,
            limits,
            expected_key,
        )?;
        if let Some(curve) = value.explicit_geometry() {
            validate_curve(
                usage,
                curve,
                RoadEditingRootVectorKind::LaneEdge,
                physical_index,
                limits,
                expected_key,
            )?;
        }
        usage.set_root_site(RoadEditingRootVectorKind::LaneEdge, physical_index);
        usage.charge_canvas(value.canvas_selection(), limits, expected_key)?;
    }
    Ok(())
}

fn validate_junctions(
    usage: &mut RoadEditingPreflightState,
    root: wire::RoadEditingSource<'_>,
    namespace: &str,
    imports: StringVector<'_>,
    limits: &CompileLimits,
    expected_key: &str,
) -> Result<(), DiagnosticBundle> {
    ensure_unique_by(
        usage,
        RoadEditingRootVectorKind::Junction,
        root.junctions().iter(),
        |value| value.junction_key(),
        "junctions.junctionKey",
        expected_key,
    )?;
    for (physical_index, value) in root.junctions().iter().enumerate() {
        usage.set_root_site(RoadEditingRootVectorKind::Junction, physical_index);
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
            RoadEditingRootVectorKind::Junction,
            physical_index,
            RoadEditingRelationKind::JunctionApproachEdge,
            ReferenceOccurrenceKind::Canonical,
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
            RoadEditingRootVectorKind::Junction,
            physical_index,
            RoadEditingRelationKind::JunctionInternalEdge,
            ReferenceOccurrenceKind::Canonical,
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
    usage: &mut RoadEditingPreflightState,
    root: wire::RoadEditingSource<'_>,
    namespace: &str,
    imports: StringVector<'_>,
    limits: &CompileLimits,
    expected_key: &str,
) -> Result<(), DiagnosticBundle> {
    ensure_unique_by(
        usage,
        RoadEditingRootVectorKind::Movement,
        root.movements().iter(),
        |value| (value.junction(), value.movement_key()),
        "movements.address",
        expected_key,
    )?;
    for (physical_index, value) in root.movements().iter().enumerate() {
        usage.set_root_site(RoadEditingRootVectorKind::Movement, physical_index);
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
    usage: &mut RoadEditingPreflightState,
    root: wire::RoadEditingSource<'_>,
    namespace: &str,
    imports: StringVector<'_>,
    limits: &CompileLimits,
    expected_key: &str,
) -> Result<(), DiagnosticBundle> {
    ensure_unique_by(
        usage,
        RoadEditingRootVectorKind::ManeuverPath,
        root.maneuver_paths().iter(),
        |value| (value.movement(), value.maneuver_path_key()),
        "maneuverPaths.address",
        expected_key,
    )?;
    for (physical_index, value) in root.maneuver_paths().iter().enumerate() {
        usage.set_root_site(RoadEditingRootVectorKind::ManeuverPath, physical_index);
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
            RoadEditingRootVectorKind::ManeuverPath,
            physical_index,
            RoadEditingRelationKind::ManeuverPathInternalEdge,
            ReferenceOccurrenceKind::Ordered,
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
    usage: &mut RoadEditingPreflightState,
    root: wire::RoadEditingSource<'_>,
    namespace: &str,
    imports: StringVector<'_>,
    limits: &CompileLimits,
    expected_key: &str,
) -> Result<(), DiagnosticBundle> {
    ensure_unique_by(
        usage,
        RoadEditingRootVectorKind::ManeuverGate,
        root.maneuver_gates().iter(),
        |value| (value.maneuver_path(), value.maneuver_gate_key()),
        "maneuverGates.address",
        expected_key,
    )?;
    for (physical_index, value) in root.maneuver_gates().iter().enumerate() {
        usage.set_root_site(RoadEditingRootVectorKind::ManeuverGate, physical_index);
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
    usage: &mut RoadEditingPreflightState,
    root: wire::RoadEditingSource<'_>,
    namespace: &str,
    imports: StringVector<'_>,
    limits: &CompileLimits,
    expected_key: &str,
) -> Result<(), DiagnosticBundle> {
    ensure_unique_by(
        usage,
        RoadEditingRootVectorKind::WaitingZone,
        root.waiting_zones().iter(),
        |value| (value.maneuver_path(), value.waiting_zone_key()),
        "waitingZones.address",
        expected_key,
    )?;
    for (physical_index, value) in root.waiting_zones().iter().enumerate() {
        usage.set_root_site(RoadEditingRootVectorKind::WaitingZone, physical_index);
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
    usage: &mut RoadEditingPreflightState,
    root: wire::RoadEditingSource<'_>,
    namespace: &str,
    imports: StringVector<'_>,
    limits: &CompileLimits,
    expected_key: &str,
) -> Result<(), DiagnosticBundle> {
    ensure_unique_by(
        usage,
        RoadEditingRootVectorKind::StopLine,
        root.stop_lines().iter(),
        |value| value.stop_line_key(),
        "stopLines.stopLineKey",
        expected_key,
    )?;
    for (physical_index, value) in root.stop_lines().iter().enumerate() {
        usage.set_root_site(RoadEditingRootVectorKind::StopLine, physical_index);
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
        usage,
        RoadEditingRootVectorKind::SignalGroup,
        root.signal_groups().iter(),
        |value| value.signal_group_key(),
        "signalGroups.signalGroupKey",
        expected_key,
    )?;
    for (physical_index, value) in root.signal_groups().iter().enumerate() {
        usage.set_root_site(RoadEditingRootVectorKind::SignalGroup, physical_index);
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
    usage: &mut RoadEditingPreflightState,
    root: wire::RoadEditingSource<'_>,
    namespace: &str,
    imports: StringVector<'_>,
    limits: &CompileLimits,
    expected_key: &str,
) -> Result<(), DiagnosticBundle> {
    ensure_unique_by(
        usage,
        RoadEditingRootVectorKind::SignalController,
        root.signal_controllers().iter(),
        |value| value.signal_controller_key(),
        "signalControllers.signalControllerKey",
        expected_key,
    )?;
    for (physical_index, value) in root.signal_controllers().iter().enumerate() {
        usage.set_root_site(RoadEditingRootVectorKind::SignalController, physical_index);
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
            RoadEditingRootVectorKind::SignalController,
            physical_index,
            RoadEditingRelationKind::SignalControllerGroup,
            ReferenceOccurrenceKind::Canonical,
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
            RoadEditingRootVectorKind::SignalController,
            physical_index,
            RoadEditingRelationKind::SignalControllerPhase,
            ReferenceOccurrenceKind::Ordered,
            namespace,
            imports,
            limits,
            expected_key,
        )?;
        usage.charge_canvas(value.canvas_selection(), limits, expected_key)?;
    }

    ensure_unique_by(
        usage,
        RoadEditingRootVectorKind::SignalPhase,
        root.signal_phases().iter(),
        |value| (value.signal_controller(), value.signal_phase_key()),
        "signalPhases.address",
        expected_key,
    )?;
    for (physical_index, value) in root.signal_phases().iter().enumerate() {
        usage.set_root_site(RoadEditingRootVectorKind::SignalPhase, physical_index);
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
            usage.set_wire_site(RoadEditingRootVectorKind::SignalPhase, physical_index);
            if let Some(occurrence) =
                canonical_signal_phase_state_ordinal(states, state.signal_group(), namespace)
            {
                usage.set_owner_local_site(
                    RoadEditingRootVectorKind::SignalPhase,
                    physical_index,
                    RoadEditingRelationKind::SignalPhaseState,
                    RoadEditingRelationOccurrence::CanonicalSetOrdinal(occurrence),
                );
            }
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
        usage.set_root_site(RoadEditingRootVectorKind::SignalPhase, physical_index);
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
    validate_signal_owner_closure(usage, root, namespace, expected_key)?;
    Ok(())
}

/// 验证 SignalController/SignalPhase 的同模块 owner tree 与状态完备性。
///
/// 第一方 builder 会提前拒绝这些错误，但 production reader 不能信任来源 writer。这里
/// 只遍历已通过语法与基数预检的借用 vector，不分配第二份索引或字符串。
fn validate_signal_owner_closure(
    usage: &mut RoadEditingPreflightState,
    root: wire::RoadEditingSource<'_>,
    namespace: &str,
    expected_key: &str,
) -> Result<(), DiagnosticBundle> {
    for (physical_index, controller) in root.signal_controllers().iter().enumerate() {
        usage.set_root_site(RoadEditingRootVectorKind::SignalController, physical_index);
        for group_reference in controller.signal_groups() {
            if group_reference.contains("::")
                || root
                    .signal_groups()
                    .iter()
                    .all(|group| group.signal_group_key() != group_reference)
            {
                return Err(invalid_combination(
                    "signalController.signalGroups",
                    expected_key,
                ));
            }
        }
        for phase_reference in controller.signal_phases() {
            if phase_reference.contains("::") {
                return Err(invalid_combination(
                    "signalController.signalPhases",
                    expected_key,
                ));
            }
            let (owner_key, phase_key) = phase_reference
                .split_once('>')
                .expect("reference syntax preflight proved two components");
            if owner_key != controller.signal_controller_key()
                || root
                    .signal_phases()
                    .iter()
                    .filter(|phase| {
                        phase.signal_controller() == owner_key
                            && phase.signal_phase_key() == phase_key
                    })
                    .count()
                    != 1
            {
                return Err(invalid_combination(
                    "signalController.signalPhases",
                    expected_key,
                ));
            }
        }
    }

    for (physical_index, phase) in root.signal_phases().iter().enumerate() {
        usage.set_root_site(RoadEditingRootVectorKind::SignalPhase, physical_index);
        let Some(controller) = root
            .signal_controllers()
            .iter()
            .find(|controller| controller.signal_controller_key() == phase.signal_controller())
        else {
            return Err(invalid_combination(
                "signalPhase.signalController",
                expected_key,
            ));
        };
        let reciprocal_count = controller
            .signal_phases()
            .iter()
            .filter(|reference| {
                reference
                    .split_once('>')
                    .is_some_and(|(owner_key, phase_key)| {
                        owner_key == controller.signal_controller_key()
                            && phase_key == phase.signal_phase_key()
                    })
            })
            .count();
        if reciprocal_count != 1 {
            return Err(invalid_combination(
                "signalPhase.signalController",
                expected_key,
            ));
        }

        if phase.states().len() != controller.signal_groups().len()
            || controller.signal_groups().iter().any(|group| {
                !phase
                    .states()
                    .iter()
                    .any(|state| references_equal(group, state.signal_group(), namespace))
            })
        {
            return Err(invalid_combination("signalPhase.states", expected_key));
        }
    }

    for (physical_index, group) in root.signal_groups().iter().enumerate() {
        usage.set_root_site(RoadEditingRootVectorKind::SignalGroup, physical_index);
        let owner_count = root
            .signal_controllers()
            .iter()
            .filter(|controller| {
                controller.signal_groups().iter().any(|reference| {
                    references_equal(reference, group.signal_group_key(), namespace)
                })
            })
            .count();
        if owner_count != 1 {
            return Err(invalid_combination(
                "signalController.signalGroups",
                expected_key,
            ));
        }
    }
    Ok(())
}

fn validate_parking(
    usage: &mut RoadEditingPreflightState,
    root: wire::RoadEditingSource<'_>,
    namespace: &str,
    imports: StringVector<'_>,
    limits: &CompileLimits,
    expected_key: &str,
) -> Result<(), DiagnosticBundle> {
    ensure_unique_by(
        usage,
        RoadEditingRootVectorKind::ParkingArea,
        root.parking_areas().iter(),
        |value| value.parking_area_key(),
        "parkingAreas.parkingAreaKey",
        expected_key,
    )?;
    for (physical_index, value) in root.parking_areas().iter().enumerate() {
        usage.set_root_site(RoadEditingRootVectorKind::ParkingArea, physical_index);
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
        usage,
        RoadEditingRootVectorKind::ParkingSpace,
        root.parking_spaces().iter(),
        |value| value.parking_space_key(),
        "parkingSpaces.parkingSpaceKey",
        expected_key,
    )?;
    for (physical_index, value) in root.parking_spaces().iter().enumerate() {
        usage.set_root_site(RoadEditingRootVectorKind::ParkingSpace, physical_index);
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
            [
                "parkingSpace.entry.laneEdge",
                "parkingSpace.entry.progressMeters",
            ],
            namespace,
            imports,
            limits,
            expected_key,
        )?;
        validate_parking_anchor(
            usage,
            value.exit(),
            [
                "parkingSpace.exit.laneEdge",
                "parkingSpace.exit.progressMeters",
            ],
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
    usage: &mut RoadEditingPreflightState,
    value: wire::ParkingLaneAnchor<'_>,
    fields: [&'static str; 2],
    namespace: &str,
    imports: StringVector<'_>,
    limits: &CompileLimits,
    expected_key: &str,
) -> Result<(), DiagnosticBundle> {
    usage.charge_reference(
        value.lane_edge(),
        1,
        true,
        fields[0],
        namespace,
        imports,
        limits,
        expected_key,
    )?;
    if let Some(violation) = positive_violation(value.progress_meters()) {
        return Err(semantic_error(fields[1], violation, expected_key));
    }
    if value.progress_meters() <= PARKING_ANCHOR_ENDPOINT_CLEARANCE_METERS {
        return Err(invalid_combination(fields[1], expected_key));
    }
    Ok(())
}

fn validate_lane_groups_and_facility_bands(
    usage: &mut RoadEditingPreflightState,
    root: wire::RoadEditingSource<'_>,
    namespace: &str,
    imports: StringVector<'_>,
    limits: &CompileLimits,
    expected_key: &str,
) -> Result<(), DiagnosticBundle> {
    ensure_unique_by(
        usage,
        RoadEditingRootVectorKind::LaneGroup,
        root.lane_groups().iter(),
        |value| (value.road_section(), value.lane_group_key()),
        "laneGroups.address",
        expected_key,
    )?;
    for (physical_index, value) in root.lane_groups().iter().enumerate() {
        usage.set_root_site(RoadEditingRootVectorKind::LaneGroup, physical_index);
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
        usage,
        RoadEditingRootVectorKind::FacilityBand,
        root.facility_bands().iter(),
        |value| (value.road_corridor(), value.facility_band_key()),
        "facilityBands.address",
        expected_key,
    )?;
    for (physical_index, value) in root.facility_bands().iter().enumerate() {
        usage.set_root_site(RoadEditingRootVectorKind::FacilityBand, physical_index);
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
            [
                "facilityBand.widthProfile.startWidthMeters",
                "facilityBand.widthProfile.endWidthMeters",
            ],
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
    usage: &mut RoadEditingPreflightState,
    root: wire::RoadEditingSource<'_>,
    namespace: &str,
    imports: StringVector<'_>,
    limits: &CompileLimits,
    expected_key: &str,
) -> Result<(), DiagnosticBundle> {
    ensure_unique_by(
        usage,
        RoadEditingRootVectorKind::ParticipantClass,
        root.participant_classes().iter(),
        |value| value.participant_class_key(),
        "participantClasses.participantClassKey",
        expected_key,
    )?;
    for (physical_index, value) in root.participant_classes().iter().enumerate() {
        usage.set_root_site(RoadEditingRootVectorKind::ParticipantClass, physical_index);
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
        usage,
        RoadEditingRootVectorKind::AccessRule,
        root.access_rules().iter(),
        |value| value.access_rule_key(),
        "accessRules.accessRuleKey",
        expected_key,
    )?;
    for (physical_index, value) in root.access_rules().iter().enumerate() {
        usage.set_root_site(RoadEditingRootVectorKind::AccessRule, physical_index);
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
            RoadEditingRootVectorKind::AccessRule,
            physical_index,
            RoadEditingRelationKind::AccessRuleParticipantClass,
            ReferenceOccurrenceKind::Canonical,
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
        usage,
        RoadEditingRootVectorKind::VehicleProfile,
        root.vehicle_profiles().iter(),
        |value| value.vehicle_profile_key(),
        "vehicleProfiles.vehicleProfileKey",
        expected_key,
    )?;
    for (physical_index, value) in root.vehicle_profiles().iter().enumerate() {
        usage.set_root_site(RoadEditingRootVectorKind::VehicleProfile, physical_index);
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
    usage: &mut RoadEditingPreflightState,
    root: wire::RoadEditingSource<'_>,
    namespace: &str,
    imports: StringVector<'_>,
    limits: &CompileLimits,
    expected_key: &str,
) -> Result<(), DiagnosticBundle> {
    ensure_unique_by(
        usage,
        RoadEditingRootVectorKind::StaticRoute,
        root.static_routes().iter(),
        |value| value.static_route_key(),
        "staticRoutes.staticRouteKey",
        expected_key,
    )?;
    for (physical_index, value) in root.static_routes().iter().enumerate() {
        usage.set_root_site(RoadEditingRootVectorKind::StaticRoute, physical_index);
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
            RoadEditingRootVectorKind::StaticRoute,
            physical_index,
            RoadEditingRelationKind::StaticRouteEdge,
            ReferenceOccurrenceKind::Ordered,
            namespace,
            imports,
            limits,
            expected_key,
        )?;
        usage.charge_canvas(value.canvas_selection(), limits, expected_key)?;
    }

    ensure_unique_by(
        usage,
        RoadEditingRootVectorKind::CanonicalFrame,
        root.canonical_frames().iter(),
        |value| value.canonical_frame_key(),
        "canonicalFrames.canonicalFrameKey",
        expected_key,
    )?;
    for (physical_index, value) in root.canonical_frames().iter().enumerate() {
        usage.set_root_site(RoadEditingRootVectorKind::CanonicalFrame, physical_index);
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
    usage: &mut RoadEditingPreflightState,
    vector: RoadEditingRootVectorKind,
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
    for (left_index, left) in values.clone().enumerate() {
        for (right_index, right) in values.clone().enumerate().skip(left_index + 1) {
            if key(left) == key(right) {
                usage.set_root_site(vector, right_index);
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

fn with_semantic_preflight_location(
    root: wire::RoadEditingSource<'_>,
    expected_key: &str,
    subject: Option<SemanticPreflightSubjectSite>,
    bundle: DiagnosticBundle,
) -> DiagnosticBundle {
    let Some(subject) = subject else {
        return bundle;
    };
    let field = bundle.diagnostics().iter().find_map(|diagnostic| {
        let DiagnosticPayload::InvalidRoadEditingSource { field, .. } = diagnostic.payload() else {
            return None;
        };
        field.as_deref()
    });
    let Some(field) = field else {
        return bundle;
    };
    if field == "moduleHeader.authoringNamespaceId" {
        return bundle.with_fallback_primary_location(
            RoadEditingLocationFactory::input_module_header(expected_key),
        );
    }
    let site = match subject {
        SemanticPreflightSubjectSite::ModuleHeader => {
            SemanticPreflightSite::module_header(Some(field))
        }
        SemanticPreflightSubjectSite::ModuleOwnerLocal {
            relation,
            occurrence,
        } => SemanticPreflightSite::module_owner_local(relation, occurrence, Some(field)),
        SemanticPreflightSubjectSite::Root {
            vector,
            physical_index,
        } => SemanticPreflightSite::root(
            vector,
            usize::try_from(physical_index).unwrap_or(usize::MAX),
            Some(field),
        ),
        SemanticPreflightSubjectSite::Wire {
            vector,
            physical_index,
        } => SemanticPreflightSite::wire(
            vector,
            usize::try_from(physical_index).unwrap_or(usize::MAX),
        ),
        SemanticPreflightSubjectSite::OwnerLocal {
            owner_vector,
            owner_physical_index,
            relation,
            occurrence,
        } => SemanticPreflightSite::owner_local(
            owner_vector,
            usize::try_from(owner_physical_index).unwrap_or(usize::MAX),
            relation,
            occurrence,
            Some(field),
        ),
    };
    match RoadEditingLocationFactory::semantic_preflight(root, expected_key, site) {
        Some(location) => bundle.with_fallback_primary_location(location),
        None => bundle,
    }
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
    fn canonical_points_accept_exact_boundaries_and_reject_outside_controls() {
        let minimum = f64::from(CANONICAL_POINT_COMPONENT_MIN_METERS);
        let maximum = f64::from(CANONICAL_POINT_COMPONENT_MAX_METERS);
        let boundary = wire::Vec3F64::new(minimum, 0.0, maximum);
        validate_point(
            &boundary,
            ["curve.control.x", "curve.control.y", "curve.control.z"],
            "roads/main",
        )
        .expect("inclusive bounds");

        let outside = wire::Vec3F64::new(maximum + 0.25, 0.0, 0.0);
        let error = validate_point(
            &outside,
            ["curve.control.x", "curve.control.y", "curve.control.z"],
            "roads/main",
        )
        .expect_err("outside canonical frame");
        assert!(matches!(
            error.diagnostics()[0].payload(),
            crate::DiagnosticPayload::InvalidRoadEditingSource {
                violation: RoadEditingSourceViolation::InvalidSemanticValue(
                    RoadEditingInputViolation::OutsideInclusiveRange { .. }
                ),
                field: Some(field),
                ..
            } if field.as_ref() == "curve.control.x"
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

    #[test]
    fn duplicate_root_site_and_canonical_reference_rank_are_stable() {
        let mut usage = RoadEditingPreflightState::new(RoadEditingPreflightCounts::default());
        let duplicate = ["a", "b", "a"];
        ensure_unique_by(
            &mut usage,
            RoadEditingRootVectorKind::CanonicalFrame,
            duplicate.into_iter(),
            |value| value,
            "canonicalFrames.canonicalFrameKey",
            "roads/main",
        )
        .expect_err("duplicate declaration");
        assert_eq!(
            usage.failure_subject,
            Some(SemanticPreflightSubjectSite::Root {
                vector: RoadEditingRootVectorKind::CanonicalFrame,
                physical_index: 2,
            })
        );

        let mut references = [
            "other::corridor>section>lane",
            "corridor>section>lane-z",
            "corridor>section>lane-a",
        ];
        references.sort_by(|left, right| {
            compare_references(left, right, 3, "city").expect("valid references")
        });
        assert_eq!(
            references,
            [
                "corridor>section>lane-a",
                "corridor>section>lane-z",
                "other::corridor>section>lane",
            ]
        );
    }
}
