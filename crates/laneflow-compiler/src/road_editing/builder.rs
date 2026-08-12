use std::collections::{BTreeMap, BTreeSet};

use laneflow_static_contract::{EntityKind, EntityKindMarker};

use super::model::*;
use super::rules::input_error;
use crate::{
    CompileLimitDimension, CompileLimits, Diagnostic, DiagnosticBundle, GeometryAccuracyProfile,
    GeometryDirectionProfile, RoadEditingInputViolation,
};

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct DeclarationAddress {
    kind: EntityKind,
    owner_keys: Box<[Box<str>]>,
    local_key: Box<str>,
}

impl DeclarationAddress {
    fn from_declaration(value: &RoadEditingDeclaration) -> Self {
        Self {
            kind: value.entity_kind(),
            owner_keys: value
                .owner_key_components()
                .iter()
                .map(|component| Box::<str>::from(*component))
                .collect(),
            local_key: value.local_key().into(),
        }
    }

    fn from_reference<K: EntityKindMarker>(value: &RoadEditingReference<K>) -> Self {
        Self {
            kind: K::KIND,
            owner_keys: value.owner_keys().map(Box::<str>::from).collect(),
            local_key: value.local_key().into(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct ModuleUsage {
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
    wire_upper_bound: u64,
}

impl ModuleUsage {
    fn charge_table(&mut self, field_count: u64, slot_bytes: u64) {
        self.wire_upper_bound = self
            .wire_upper_bound
            .saturating_add(33)
            .saturating_add(slot_bytes)
            .saturating_add(field_count.saturating_mul(9));
    }

    fn charge_vector(&mut self, count: usize, element_bytes: u64) {
        self.wire_upper_bound = self.wire_upper_bound.saturating_add(12).saturating_add(
            u64::try_from(count)
                .unwrap_or(u64::MAX)
                .saturating_mul(element_bytes),
        );
    }

    fn charge_root_vector_element(&mut self) {
        self.wire_upper_bound = self.wire_upper_bound.saturating_add(4);
    }

    fn charge_token(
        &mut self,
        value: &str,
        limits: &CompileLimits,
    ) -> Result<(), DiagnosticBundle> {
        let bytes = u64::try_from(value.len()).unwrap_or(u64::MAX);
        let limit = limits.value(CompileLimitDimension::SingleStringBytes);
        if bytes > limit {
            return Err(DiagnosticBundle::single(
                Diagnostic::compile_limit_exceeded(
                    CompileLimitDimension::SingleStringBytes,
                    limit,
                    bytes,
                ),
            ));
        }
        self.string_item_count = self.string_item_count.saturating_add(1);
        self.total_string_bytes = self.total_string_bytes.saturating_add(bytes);
        self.wire_upper_bound = self
            .wire_upper_bound
            .saturating_add(bytes)
            .saturating_add(13);
        Ok(())
    }

    fn charge_reference<K: EntityKindMarker>(
        &mut self,
        value: &RoadEditingReference<K>,
        current_namespace: &str,
        import_namespaces: &BTreeSet<Box<str>>,
        limits: &CompileLimits,
    ) -> Result<(), DiagnosticBundle> {
        if let Some(namespace) = value.module_namespace()
            && (namespace == current_namespace || !import_namespaces.contains(namespace))
        {
            return Err(input_error(
                "roadEditingSource.reference.moduleNamespace",
                RoadEditingInputViolation::InvalidCombination,
            ));
        }
        let single_limit = limits.value(CompileLimitDimension::SingleStringBytes);
        let mut component_count = 0_u64;
        let mut component_bytes = 0_u64;
        for component in value
            .module_namespace()
            .into_iter()
            .chain(value.owner_keys())
            .chain(std::iter::once(value.local_key()))
        {
            let observed = u64::try_from(component.len()).unwrap_or(u64::MAX);
            if observed > single_limit {
                return Err(DiagnosticBundle::single(
                    Diagnostic::compile_limit_exceeded(
                        CompileLimitDimension::SingleStringBytes,
                        single_limit,
                        observed,
                    ),
                ));
            }
            component_count = component_count.saturating_add(1);
            component_bytes = component_bytes.saturating_add(observed);
        }
        self.reference_count = self.reference_count.saturating_add(1);
        self.string_item_count = self.string_item_count.saturating_add(component_count);
        let wire_bytes = u64::try_from(value.wire_len()).unwrap_or(u64::MAX);
        self.total_string_bytes = self.total_string_bytes.saturating_add(component_bytes);
        self.wire_upper_bound = self
            .wire_upper_bound
            .saturating_add(wire_bytes)
            .saturating_add(13);
        Ok(())
    }

    fn charge_canvas(
        &mut self,
        value: Option<&str>,
        limits: &CompileLimits,
    ) -> Result<(), DiagnosticBundle> {
        if let Some(value) = value {
            self.charge_token(value, limits)?;
        }
        Ok(())
    }

    fn charge_curve(
        &mut self,
        value: &RoadEditingCurveProgram,
        limits: &CompileLimits,
    ) -> Result<(), DiagnosticBundle> {
        self.typed_ast_record_count = self.typed_ast_record_count.saturating_add(1);
        self.geometry_point_count = self.geometry_point_count.saturating_add(1);
        self.charge_table(2, 28);
        self.charge_vector(value.segments().len(), 4);
        for segment in value.segments() {
            self.typed_ast_record_count = self.typed_ast_record_count.saturating_add(2);
            self.charge_table(3, 9);
            self.geometry_point_count =
                self.geometry_point_count
                    .saturating_add(match segment.geometry() {
                        RoadEditingCurveSegmentGeometry::Line { .. } => {
                            self.charge_table(1, 24);
                            1
                        }
                        RoadEditingCurveSegmentGeometry::CubicBezier { .. } => {
                            self.charge_table(3, 72);
                            3
                        }
                    });
            self.charge_canvas(segment.canvas_selection(), limits)?;
        }
        Ok(())
    }

    fn validate(self, limits: &CompileLimits) -> Result<(), DiagnosticBundle> {
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
            (
                CompileLimitDimension::SourceBytesPerModule,
                self.wire_upper_bound,
            ),
        ] {
            let limit = limits.value(dimension);
            if observed > limit {
                return Err(DiagnosticBundle::single(
                    Diagnostic::compile_limit_exceeded(dimension, limit, observed),
                ));
            }
        }
        Ok(())
    }
}

/// 已通过第一方构造约束、尚未编码为 FlatBuffers 的道路编辑模块。
#[derive(Debug)]
pub struct RoadEditingSourceModule {
    header: RoadEditingModuleHeader,
    geometry_accuracy_profile: GeometryAccuracyProfile,
    geometry_direction_profile: GeometryDirectionProfile,
    road_alignments: Box<[RoadAlignmentInput]>,
    declarations: Box<[RoadEditingDeclaration]>,
    wire_upper_bound: u64,
}

pub(super) struct RoadEditingSourceModuleParts {
    pub(super) header: RoadEditingModuleHeader,
    pub(super) geometry_accuracy_profile: GeometryAccuracyProfile,
    pub(super) geometry_direction_profile: GeometryDirectionProfile,
    pub(super) road_alignments: Box<[RoadAlignmentInput]>,
    pub(super) declarations: Box<[RoadEditingDeclaration]>,
    pub(super) wire_upper_bound: u64,
}

impl RoadEditingSourceModule {
    #[must_use]
    pub const fn header(&self) -> &RoadEditingModuleHeader {
        &self.header
    }

    #[must_use]
    pub const fn geometry_accuracy_profile(&self) -> GeometryAccuracyProfile {
        self.geometry_accuracy_profile
    }

    #[must_use]
    pub const fn geometry_direction_profile(&self) -> GeometryDirectionProfile {
        self.geometry_direction_profile
    }

    #[must_use]
    pub fn road_alignments(&self) -> &[RoadAlignmentInput] {
        &self.road_alignments
    }

    #[must_use]
    pub fn declarations(&self) -> &[RoadEditingDeclaration] {
        &self.declarations
    }

    pub(super) fn into_parts(self) -> RoadEditingSourceModuleParts {
        RoadEditingSourceModuleParts {
            header: self.header,
            geometry_accuracy_profile: self.geometry_accuracy_profile,
            geometry_direction_profile: self.geometry_direction_profile,
            road_alignments: self.road_alignments,
            declarations: self.declarations,
            wire_upper_bound: self.wire_upper_bound,
        }
    }
}

/// 第一方道路编辑模块的失败关闭构建器。
pub struct RoadEditingSourceModuleBuilder<'limits> {
    header: RoadEditingModuleHeader,
    geometry_accuracy_profile: GeometryAccuracyProfile,
    geometry_direction_profile: GeometryDirectionProfile,
    limits: &'limits CompileLimits,
    road_alignments: Vec<RoadAlignmentInput>,
    declarations: Vec<RoadEditingDeclaration>,
    alignment_keys: BTreeSet<Box<str>>,
    declaration_addresses: BTreeSet<DeclarationAddress>,
    import_namespaces: BTreeSet<Box<str>>,
    usage: ModuleUsage,
}

impl<'limits> RoadEditingSourceModuleBuilder<'limits> {
    pub fn new(
        header: RoadEditingModuleHeader,
        geometry_accuracy_profile: GeometryAccuracyProfile,
        geometry_direction_profile: GeometryDirectionProfile,
        limits: &'limits CompileLimits,
    ) -> Result<Self, DiagnosticBundle> {
        let mut usage = ModuleUsage {
            // root 与 Provenance 不进入 Typed AST；ModuleHeader 恰好消费一条记录。
            typed_ast_record_count: 1,
            wire_upper_bound: 32,
            ..ModuleUsage::default()
        };
        usage.charge_table(27, 102);
        for _ in 0..23 {
            usage.charge_vector(0, 4);
        }
        usage.charge_table(6, 81);
        usage.charge_table(4, 16);
        usage.charge_token(header.authoring_namespace_id(), limits)?;
        usage.charge_token(header.source_document_key(), limits)?;
        for import in header.imports() {
            usage.charge_token(import, limits)?;
        }
        usage.charge_vector(header.imports().len(), 4);
        usage.charge_token(header.provenance().generator_build_id(), limits)?;
        usage.charge_token(header.provenance().description(), limits)?;
        let import_count = u64::try_from(header.imports().len()).unwrap_or(u64::MAX);
        let import_limit = limits.value(CompileLimitDimension::ImportEdgeCount);
        if import_count > import_limit {
            return Err(DiagnosticBundle::single(
                Diagnostic::compile_limit_exceeded(
                    CompileLimitDimension::ImportEdgeCount,
                    import_limit,
                    import_count,
                ),
            ));
        }
        usage.validate(limits)?;
        let import_namespaces = header.imports().map(Box::<str>::from).collect();
        Ok(Self {
            header,
            geometry_accuracy_profile,
            geometry_direction_profile,
            limits,
            road_alignments: Vec::new(),
            declarations: Vec::new(),
            alignment_keys: BTreeSet::new(),
            declaration_addresses: BTreeSet::new(),
            import_namespaces,
            usage,
        })
    }

    pub fn add_alignment(
        &mut self,
        value: RoadAlignmentInput,
    ) -> Result<&mut Self, DiagnosticBundle> {
        if self.alignment_keys.contains(value.road_alignment_key()) {
            return Err(input_error(
                "roadAlignments.roadAlignmentKey",
                RoadEditingInputViolation::DuplicateValue,
            ));
        }
        let mut usage = self.usage;
        usage.typed_ast_record_count = usage.typed_ast_record_count.saturating_add(1);
        usage.charge_root_vector_element();
        usage.charge_table(4, 16);
        usage.charge_token(value.road_alignment_key(), self.limits)?;
        usage.charge_reference(
            value.canonical_frame(),
            self.header.authoring_namespace_id(),
            &self.import_namespaces,
            self.limits,
        )?;
        usage.charge_curve(value.reference_line(), self.limits)?;
        usage.charge_canvas(value.canvas_selection(), self.limits)?;
        usage.validate(self.limits)?;

        self.alignment_keys
            .insert(value.road_alignment_key().into());
        self.road_alignments.push(value);
        self.usage = usage;
        Ok(self)
    }

    pub fn add_declaration(
        &mut self,
        value: RoadEditingDeclaration,
    ) -> Result<&mut Self, DiagnosticBundle> {
        let address = DeclarationAddress::from_declaration(&value);
        if self.declaration_addresses.contains(&address) {
            return Err(input_error(
                "roadEditingSource.declarations",
                RoadEditingInputViolation::DuplicateValue,
            ));
        }
        ensure_local_owner(&value)?;
        let mut usage = self.usage;
        usage.charge_root_vector_element();
        charge_declaration(
            &mut usage,
            &value,
            self.header.authoring_namespace_id(),
            &self.import_namespaces,
            self.limits,
        )?;
        usage.validate(self.limits)?;

        self.declaration_addresses.insert(address);
        self.declarations.push(value);
        self.usage = usage;
        Ok(self)
    }

    pub fn finish(mut self) -> Result<RoadEditingSourceModule, DiagnosticBundle> {
        validate_owner_tree(&self.declarations, &self.declaration_addresses)?;
        self.road_alignments.sort_unstable_by(|left, right| {
            left.road_alignment_key().cmp(right.road_alignment_key())
        });
        self.declarations
            .sort_unstable_by(RoadEditingDeclaration::canonical_address_cmp);
        Ok(RoadEditingSourceModule {
            header: self.header,
            geometry_accuracy_profile: self.geometry_accuracy_profile,
            geometry_direction_profile: self.geometry_direction_profile,
            road_alignments: self.road_alignments.into_boxed_slice(),
            declarations: self.declarations.into_boxed_slice(),
            wire_upper_bound: self.usage.wire_upper_bound,
        })
    }
}

fn ensure_local_owner(value: &RoadEditingDeclaration) -> Result<(), DiagnosticBundle> {
    let owner_is_imported = match value {
        RoadEditingDeclaration::RoadSection(value) => {
            value.road_corridor().module_namespace().is_some()
        }
        RoadEditingDeclaration::AuthoringLane(value) => {
            value.road_section().module_namespace().is_some()
        }
        RoadEditingDeclaration::Movement(value) => value.junction().module_namespace().is_some(),
        RoadEditingDeclaration::ManeuverPath(value) => {
            value.movement().module_namespace().is_some()
        }
        RoadEditingDeclaration::ManeuverGate(value) => {
            value.maneuver_path().module_namespace().is_some()
        }
        RoadEditingDeclaration::WaitingZone(value) => {
            value.maneuver_path().module_namespace().is_some()
        }
        RoadEditingDeclaration::SignalPhase(value) => {
            value.signal_controller().module_namespace().is_some()
        }
        RoadEditingDeclaration::LaneGroup(value) => {
            value.road_section().module_namespace().is_some()
        }
        RoadEditingDeclaration::FacilityBand(value) => {
            value.road_corridor().module_namespace().is_some()
        }
        _ => false,
    };
    if owner_is_imported {
        return Err(input_error(
            "roadEditingSource.owner",
            RoadEditingInputViolation::InvalidCombination,
        ));
    }
    Ok(())
}

fn charge_relation(usage: &mut ModuleUsage, count: usize) {
    usage.relation_occurrence_count = usage
        .relation_occurrence_count
        .saturating_add(u64::try_from(count).unwrap_or(u64::MAX));
}

fn charge_references<K: EntityKindMarker>(
    usage: &mut ModuleUsage,
    values: &[RoadEditingReference<K>],
    current_namespace: &str,
    import_namespaces: &BTreeSet<Box<str>>,
    limits: &CompileLimits,
) -> Result<(), DiagnosticBundle> {
    charge_relation(usage, values.len());
    for value in values {
        usage.charge_reference(value, current_namespace, import_namespaces, limits)?;
    }
    Ok(())
}

fn charge_declaration(
    usage: &mut ModuleUsage,
    value: &RoadEditingDeclaration,
    current_namespace: &str,
    import_namespaces: &BTreeSet<Box<str>>,
    limits: &CompileLimits,
) -> Result<(), DiagnosticBundle> {
    usage.declaration_count = usage.declaration_count.saturating_add(1);
    usage.symbol_count = usage.symbol_count.saturating_add(1);
    usage.typed_ast_record_count = usage.typed_ast_record_count.saturating_add(1);
    usage.identity_field_occurrence_count = usage.identity_field_occurrence_count.saturating_add(
        u64::try_from(value.entity_kind().required_tags().len()).unwrap_or(u64::MAX),
    );
    usage.charge_token(value.local_key(), limits)?;

    match value {
        RoadEditingDeclaration::RoadCorridor(value) => {
            usage.charge_table(9, 41);
            usage.charge_vector(value.elements().len(), 4);
            usage.charge_token(value.road_alignment().key(), limits)?;
            usage.charge_reference(
                value.reference_section(),
                current_namespace,
                import_namespaces,
                limits,
            )?;
            usage.charge_reference(
                value.reference_lane(),
                current_namespace,
                import_namespaces,
                limits,
            )?;
            charge_relation(usage, value.elements().len());
            for element in value.elements() {
                usage.typed_ast_record_count = usage.typed_ast_record_count.saturating_add(1);
                usage.charge_table(2, 5);
                match element {
                    RoadEditingCorridorElement::RoadSection(reference) => usage.charge_reference(
                        reference,
                        current_namespace,
                        import_namespaces,
                        limits,
                    )?,
                    RoadEditingCorridorElement::FacilityBand(reference) => usage.charge_reference(
                        reference,
                        current_namespace,
                        import_namespaces,
                        limits,
                    )?,
                }
            }
            usage.charge_canvas(value.canvas_selection(), limits)?;
        }
        RoadEditingDeclaration::RoadSection(value) => {
            usage.charge_table(5, 20);
            usage.charge_vector(value.authoring_lanes().len(), 4);
            usage.charge_token(value.kind_id(), limits)?;
            charge_references(
                usage,
                value.authoring_lanes(),
                current_namespace,
                import_namespaces,
                limits,
            )?;
            usage.charge_reference(
                value.road_corridor(),
                current_namespace,
                import_namespaces,
                limits,
            )?;
            usage.charge_canvas(value.canvas_selection(), limits)?;
        }
        RoadEditingDeclaration::AuthoringLane(value) => {
            usage.charge_table(7, 37);
            usage.charge_reference(
                value.lane_edge(),
                current_namespace,
                import_namespaces,
                limits,
            )?;
            if let Some(reference) = value.lane_group() {
                usage.charge_reference(reference, current_namespace, import_namespaces, limits)?;
            }
            usage.charge_reference(
                value.road_section(),
                current_namespace,
                import_namespaces,
                limits,
            )?;
            usage.charge_canvas(value.canvas_selection(), limits)?;
        }
        RoadEditingDeclaration::LaneEdge(value) => {
            usage.charge_table(5, 24);
            usage.charge_vector(value.successors().len(), 4);
            charge_references(
                usage,
                value.successors(),
                current_namespace,
                import_namespaces,
                limits,
            )?;
            if let Some(curve) = value.explicit_geometry() {
                usage.charge_curve(curve, limits)?;
            }
            usage.charge_canvas(value.canvas_selection(), limits)?;
        }
        RoadEditingDeclaration::Junction(value) => {
            usage.charge_table(4, 16);
            usage.charge_vector(value.approach_edges().len(), 4);
            usage.charge_vector(value.internal_edges().len(), 4);
            charge_references(
                usage,
                value.approach_edges(),
                current_namespace,
                import_namespaces,
                limits,
            )?;
            charge_references(
                usage,
                value.internal_edges(),
                current_namespace,
                import_namespaces,
                limits,
            )?;
            usage.charge_canvas(value.canvas_selection(), limits)?;
        }
        RoadEditingDeclaration::Movement(value) => {
            usage.charge_table(5, 20);
            usage.charge_reference(
                value.junction(),
                current_namespace,
                import_namespaces,
                limits,
            )?;
            usage.charge_token(value.directed_entry_approach_key(), limits)?;
            usage.charge_token(value.directed_exit_approach_key(), limits)?;
            usage.charge_canvas(value.canvas_selection(), limits)?;
        }
        RoadEditingDeclaration::ManeuverPath(value) => {
            usage.charge_table(6, 24);
            usage.charge_vector(value.internal_edges().len(), 4);
            usage.charge_reference(
                value.movement(),
                current_namespace,
                import_namespaces,
                limits,
            )?;
            usage.charge_reference(
                value.entry_edge(),
                current_namespace,
                import_namespaces,
                limits,
            )?;
            charge_references(
                usage,
                value.internal_edges(),
                current_namespace,
                import_namespaces,
                limits,
            )?;
            usage.charge_reference(
                value.exit_edge(),
                current_namespace,
                import_namespaces,
                limits,
            )?;
            usage.charge_canvas(value.canvas_selection(), limits)?;
        }
        RoadEditingDeclaration::ManeuverGate(value) => {
            usage.charge_table(7, 25);
            usage.maneuver_gate_count = usage.maneuver_gate_count.saturating_add(1);
            usage.charge_reference(
                value.maneuver_path(),
                current_namespace,
                import_namespaces,
                limits,
            )?;
            usage.charge_reference(
                value.stop_line(),
                current_namespace,
                import_namespaces,
                limits,
            )?;
            if let RoadEditingSignalControl::SignalGroup(reference) = value.signal_control() {
                usage.charge_reference(reference, current_namespace, import_namespaces, limits)?;
            }
            usage.charge_canvas(value.canvas_selection(), limits)?;
        }
        RoadEditingDeclaration::WaitingZone(value) => {
            usage.charge_table(6, 24);
            usage.waiting_zone_count = usage.waiting_zone_count.saturating_add(1);
            usage.charge_reference(
                value.maneuver_path(),
                current_namespace,
                import_namespaces,
                limits,
            )?;
            usage.charge_reference(
                value.entry_gate(),
                current_namespace,
                import_namespaces,
                limits,
            )?;
            usage.charge_reference(
                value.release_gate(),
                current_namespace,
                import_namespaces,
                limits,
            )?;
            usage.charge_canvas(value.canvas_selection(), limits)?;
        }
        RoadEditingDeclaration::StopLine(value) => {
            usage.charge_table(3, 12);
            usage.charge_reference(
                value.lane_edge(),
                current_namespace,
                import_namespaces,
                limits,
            )?;
            usage.charge_canvas(value.canvas_selection(), limits)?;
        }
        RoadEditingDeclaration::SignalGroup(value) => {
            usage.charge_table(2, 8);
            usage.charge_canvas(value.canvas_selection(), limits)?
        }
        RoadEditingDeclaration::SignalController(value) => {
            usage.charge_table(5, 24);
            usage.charge_vector(value.signal_groups().len(), 4);
            usage.charge_vector(value.signal_phases().len(), 4);
            charge_references(
                usage,
                value.signal_groups(),
                current_namespace,
                import_namespaces,
                limits,
            )?;
            charge_references(
                usage,
                value.signal_phases(),
                current_namespace,
                import_namespaces,
                limits,
            )?;
            usage.charge_canvas(value.canvas_selection(), limits)?;
        }
        RoadEditingDeclaration::SignalPhase(value) => {
            usage.charge_table(5, 24);
            usage.charge_vector(value.states().len(), 4);
            usage.charge_reference(
                value.signal_controller(),
                current_namespace,
                import_namespaces,
                limits,
            )?;
            charge_relation(usage, value.states().len());
            for state in value.states() {
                usage.typed_ast_record_count = usage.typed_ast_record_count.saturating_add(1);
                usage.charge_table(2, 5);
                usage.charge_reference(
                    state.signal_group(),
                    current_namespace,
                    import_namespaces,
                    limits,
                )?;
            }
            usage.charge_canvas(value.canvas_selection(), limits)?;
        }
        RoadEditingDeclaration::ParkingArea(value) => {
            usage.charge_table(2, 8);
            usage.charge_canvas(value.canvas_selection(), limits)?
        }
        RoadEditingDeclaration::ParkingSpace(value) => {
            usage.charge_table(6, 24);
            usage.charge_table(2, 12);
            usage.charge_table(2, 12);
            usage.charge_table(4, 32);
            if let Some(reference) = value.parking_area() {
                usage.charge_reference(reference, current_namespace, import_namespaces, limits)?;
            }
            usage.typed_ast_record_count = usage.typed_ast_record_count.saturating_add(3);
            usage.charge_reference(
                value.entry().lane_edge(),
                current_namespace,
                import_namespaces,
                limits,
            )?;
            usage.charge_reference(
                value.exit().lane_edge(),
                current_namespace,
                import_namespaces,
                limits,
            )?;
            usage.charge_canvas(value.canvas_selection(), limits)?;
        }
        RoadEditingDeclaration::LaneGroup(value) => {
            usage.charge_table(3, 12);
            usage.charge_reference(
                value.road_section(),
                current_namespace,
                import_namespaces,
                limits,
            )?;
            usage.charge_canvas(value.canvas_selection(), limits)?;
        }
        RoadEditingDeclaration::FacilityBand(value) => {
            usage.charge_table(5, 32);
            usage.charge_token(value.kind_id(), limits)?;
            usage.charge_reference(
                value.road_corridor(),
                current_namespace,
                import_namespaces,
                limits,
            )?;
            usage.charge_canvas(value.canvas_selection(), limits)?;
        }
        RoadEditingDeclaration::ParticipantClass(value) => {
            usage.charge_table(3, 12);
            if let Some(reference) = value.extends() {
                usage.charge_reference(reference, current_namespace, import_namespaces, limits)?;
            }
            usage.charge_canvas(value.canvas_selection(), limits)?;
        }
        RoadEditingDeclaration::AccessRule(value) => {
            usage.charge_table(8, 26);
            usage.charge_vector(value.participant_classes().len(), 4);
            match value.target() {
                RoadEditingAccessTarget::LaneEdge(reference) => usage.charge_reference(
                    reference,
                    current_namespace,
                    import_namespaces,
                    limits,
                )?,
                RoadEditingAccessTarget::LaneGroup(reference) => usage.charge_reference(
                    reference,
                    current_namespace,
                    import_namespaces,
                    limits,
                )?,
                RoadEditingAccessTarget::RoadSection(reference) => usage.charge_reference(
                    reference,
                    current_namespace,
                    import_namespaces,
                    limits,
                )?,
                RoadEditingAccessTarget::ManeuverPath(reference) => usage.charge_reference(
                    reference,
                    current_namespace,
                    import_namespaces,
                    limits,
                )?,
            }
            charge_references(
                usage,
                value.participant_classes(),
                current_namespace,
                import_namespaces,
                limits,
            )?;
            if let Some(regulation) = value.regulation() {
                usage.typed_ast_record_count = usage.typed_ast_record_count.saturating_add(1);
                usage.charge_table(3, 12);
                usage.charge_token(regulation.jurisdiction(), limits)?;
                usage.charge_token(regulation.version(), limits)?;
                if let Some(source) = regulation.source() {
                    usage.charge_token(source, limits)?;
                }
            }
            usage.charge_canvas(value.canvas_selection(), limits)?;
        }
        RoadEditingDeclaration::VehicleProfile(value) => {
            usage.charge_table(4, 16);
            usage.charge_table(7, 56);
            usage.typed_ast_record_count = usage.typed_ast_record_count.saturating_add(1);
            usage.charge_reference(
                value.participant_class(),
                current_namespace,
                import_namespaces,
                limits,
            )?;
            usage.charge_canvas(value.canvas_selection(), limits)?;
        }
        RoadEditingDeclaration::StaticRoute(value) => {
            usage.charge_table(3, 12);
            usage.charge_vector(value.edge_sequence().len(), 4);
            usage.route_occurrence_count = usage
                .route_occurrence_count
                .saturating_add(u64::try_from(value.edge_sequence().len()).unwrap_or(u64::MAX));
            charge_references(
                usage,
                value.edge_sequence(),
                current_namespace,
                import_namespaces,
                limits,
            )?;
            usage.charge_canvas(value.canvas_selection(), limits)?;
        }
        RoadEditingDeclaration::CanonicalFrame(value) => {
            usage.charge_table(2, 8);
            usage.charge_canvas(value.canvas_selection(), limits)?
        }
    }
    Ok(())
}

fn parent_address(value: &RoadEditingDeclaration) -> Option<DeclarationAddress> {
    match value {
        RoadEditingDeclaration::RoadSection(value) => {
            Some(DeclarationAddress::from_reference(value.road_corridor()))
        }
        RoadEditingDeclaration::AuthoringLane(value) => {
            Some(DeclarationAddress::from_reference(value.road_section()))
        }
        RoadEditingDeclaration::Movement(value) => {
            Some(DeclarationAddress::from_reference(value.junction()))
        }
        RoadEditingDeclaration::ManeuverPath(value) => {
            Some(DeclarationAddress::from_reference(value.movement()))
        }
        RoadEditingDeclaration::ManeuverGate(value) => {
            Some(DeclarationAddress::from_reference(value.maneuver_path()))
        }
        RoadEditingDeclaration::WaitingZone(value) => {
            Some(DeclarationAddress::from_reference(value.maneuver_path()))
        }
        RoadEditingDeclaration::SignalPhase(value) => Some(DeclarationAddress::from_reference(
            value.signal_controller(),
        )),
        RoadEditingDeclaration::LaneGroup(value) => {
            Some(DeclarationAddress::from_reference(value.road_section()))
        }
        RoadEditingDeclaration::FacilityBand(value) => {
            Some(DeclarationAddress::from_reference(value.road_corridor()))
        }
        _ => None,
    }
}

fn validate_owner_tree(
    declarations: &[RoadEditingDeclaration],
    addresses: &BTreeSet<DeclarationAddress>,
) -> Result<(), DiagnosticBundle> {
    for declaration in declarations {
        if let Some(parent) = parent_address(declaration)
            && !addresses.contains(&parent)
        {
            return Err(input_error(
                "roadEditingSource.owner",
                RoadEditingInputViolation::InvalidCombination,
            ));
        }
    }

    let by_address = declarations
        .iter()
        .map(|declaration| {
            (
                DeclarationAddress::from_declaration(declaration),
                declaration,
            )
        })
        .collect::<BTreeMap<_, _>>();
    let mut owned_sections = BTreeSet::new();
    let mut owned_lanes = BTreeSet::new();
    let mut owned_facility_bands = BTreeSet::new();
    let mut owned_signal_groups = BTreeSet::new();
    let mut owned_signal_phases = BTreeSet::new();
    for declaration in declarations {
        let owner = DeclarationAddress::from_declaration(declaration);
        match declaration {
            RoadEditingDeclaration::RoadCorridor(corridor) => {
                for element in corridor.elements() {
                    match element {
                        RoadEditingCorridorElement::RoadSection(reference) => {
                            register_owned_child(
                                reference,
                                &owner,
                                &by_address,
                                &mut owned_sections,
                            )?;
                        }
                        RoadEditingCorridorElement::FacilityBand(reference) => {
                            register_owned_child(
                                reference,
                                &owner,
                                &by_address,
                                &mut owned_facility_bands,
                            )?;
                        }
                    }
                }
            }
            RoadEditingDeclaration::RoadSection(section) => {
                for reference in section.authoring_lanes() {
                    register_owned_child(reference, &owner, &by_address, &mut owned_lanes)?;
                }
            }
            RoadEditingDeclaration::SignalController(controller) => {
                let expected_phase_groups = controller
                    .signal_groups()
                    .iter()
                    .map(DeclarationAddress::from_reference)
                    .collect::<BTreeSet<_>>();
                for reference in controller.signal_phases() {
                    register_owned_child(reference, &owner, &by_address, &mut owned_signal_phases)?;
                    let phase_address = DeclarationAddress::from_reference(reference);
                    let Some(RoadEditingDeclaration::SignalPhase(phase)) =
                        by_address.get(&phase_address).copied()
                    else {
                        return owner_tree_error();
                    };
                    let mut actual_phase_groups = BTreeSet::new();
                    for state in phase.states() {
                        require_local_reference_namespace(state.signal_group().module_namespace())?;
                        actual_phase_groups
                            .insert(DeclarationAddress::from_reference(state.signal_group()));
                    }
                    if actual_phase_groups != expected_phase_groups {
                        return owner_tree_error();
                    }
                }
                for reference in controller.signal_groups() {
                    require_local_reference_namespace(reference.module_namespace())?;
                    if !by_address.contains_key(&DeclarationAddress::from_reference(reference))
                        || !owned_signal_groups.insert(reference.clone())
                    {
                        return owner_tree_error();
                    }
                }
            }
            _ => {}
        }
    }

    for (kind, owned_count) in [
        (EntityKind::RoadSection, owned_sections.len()),
        (EntityKind::AuthoringLane, owned_lanes.len()),
        (EntityKind::FacilityBand, owned_facility_bands.len()),
        (EntityKind::SignalGroup, owned_signal_groups.len()),
        (EntityKind::SignalPhase, owned_signal_phases.len()),
    ] {
        let declared_count = declarations
            .iter()
            .filter(|value| value.entity_kind() == kind)
            .count();
        if owned_count != declared_count {
            return owner_tree_error();
        }
    }
    Ok(())
}

fn register_owned_child<K: EntityKindMarker>(
    reference: &RoadEditingReference<K>,
    expected_parent: &DeclarationAddress,
    by_address: &BTreeMap<DeclarationAddress, &RoadEditingDeclaration>,
    owned: &mut BTreeSet<DeclarationAddress>,
) -> Result<(), DiagnosticBundle> {
    require_local_reference_namespace(reference.module_namespace())?;
    let address = DeclarationAddress::from_reference(reference);
    let Some(child) = by_address.get(&address) else {
        return owner_tree_error();
    };
    if parent_address(child).as_ref() != Some(expected_parent) || !owned.insert(address) {
        return owner_tree_error();
    }
    Ok(())
}

fn require_local_reference_namespace(value: Option<&str>) -> Result<(), DiagnosticBundle> {
    if value.is_some() {
        return owner_tree_error();
    }
    Ok(())
}

fn owner_tree_error<T>() -> Result<T, DiagnosticBundle> {
    Err(input_error(
        "roadEditingSource.ownerTree",
        RoadEditingInputViolation::InvalidCombination,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header() -> RoadEditingModuleHeader {
        RoadEditingModuleHeader::try_new(
            "city",
            "road-editing",
            Vec::new(),
            RoadEditingProvenance::direct("editor save").expect("provenance"),
        )
        .expect("header")
    }

    fn builder(limits: &CompileLimits) -> RoadEditingSourceModuleBuilder<'_> {
        RoadEditingSourceModuleBuilder::new(
            header(),
            GeometryAccuracyProfile::Balanced5Cm,
            GeometryDirectionProfile::Balanced2Deg,
            limits,
        )
        .expect("builder")
    }

    #[test]
    fn duplicate_add_does_not_pollute_builder() {
        let limits = CompileLimits::p100_initial_v1();
        let mut builder = builder(&limits);
        builder
            .add_declaration(RoadEditingDeclaration::CanonicalFrame(
                CanonicalFrameInput::try_new("frame-a").expect("frame"),
            ))
            .expect("first declaration");
        assert!(
            builder
                .add_declaration(RoadEditingDeclaration::CanonicalFrame(
                    CanonicalFrameInput::try_new("frame-a").expect("duplicate frame"),
                ))
                .is_err()
        );
        builder
            .add_declaration(RoadEditingDeclaration::CanonicalFrame(
                CanonicalFrameInput::try_new("frame-b").expect("second frame"),
            ))
            .expect("builder remains reusable");

        let module = builder.finish().expect("finished module");
        assert_eq!(module.declarations().len(), 2);
    }

    #[test]
    fn finish_rejects_missing_identity_owner() {
        let limits = CompileLimits::p100_initial_v1();
        let mut builder = builder(&limits);
        let missing_section =
            RoadSectionReference::owner_scoped(vec!["corridor-a".into()], "section-a")
                .expect("section reference");
        builder
            .add_declaration(RoadEditingDeclaration::LaneGroup(
                LaneGroupInput::try_new("group-a", missing_section).expect("lane group"),
            ))
            .expect("add defers owner closure");

        assert!(builder.finish().is_err());
    }

    #[test]
    fn add_rejects_imported_identity_owner() {
        let limits = CompileLimits::p100_initial_v1();
        let mut builder = builder(&limits);
        let imported_section =
            RoadSectionReference::imported("other-city", vec!["corridor-a".into()], "section-a")
                .expect("imported section reference");

        assert!(
            builder
                .add_declaration(RoadEditingDeclaration::LaneGroup(
                    LaneGroupInput::try_new("group-a", imported_section).expect("lane group"),
                ))
                .is_err()
        );
        assert!(builder.finish().is_ok());
    }

    #[test]
    fn add_checks_string_limits_before_mutation() {
        let limits = CompileLimits::p100_initial_v1();
        let mut builder = builder(&limits);
        let oversized_key = "x".repeat(54);
        assert!(
            builder
                .add_declaration(RoadEditingDeclaration::CanonicalFrame(
                    CanonicalFrameInput::try_new(oversized_key).expect("lexically valid frame"),
                ))
                .is_err()
        );
        builder
            .add_declaration(RoadEditingDeclaration::CanonicalFrame(
                CanonicalFrameInput::try_new("frame-a").expect("valid frame"),
            ))
            .expect("builder remains reusable");
        assert_eq!(builder.finish().expect("module").declarations().len(), 1);
    }

    #[test]
    fn module_header_is_the_only_initial_typed_ast_record() {
        let limits = CompileLimits::p100_initial_v1()
            .with_test_admission_limit(CompileLimitDimension::TypedAstRecordCount, 1);

        assert!(builder(&limits).finish().is_ok());
    }

    #[test]
    fn add_rejects_a_self_qualified_reference() {
        let limits = CompileLimits::p100_initial_v1();
        let mut builder = builder(&limits);
        let self_qualified_frame = CanonicalFrameReference::imported("city", Vec::new(), "frame-a")
            .expect("self-qualified frame reference");
        let alignment = RoadAlignmentInput::try_new(
            "alignment-a",
            self_qualified_frame,
            RoadEditingCurveProgram::try_new(
                RoadEditingPoint3::try_new(0.0, 0.0, 0.0).expect("start"),
                vec![RoadEditingCurveSegment::line(
                    RoadEditingPoint3::try_new(1.0, 0.0, 0.0).expect("end"),
                )],
            )
            .expect("curve"),
        )
        .expect("alignment");

        assert!(builder.add_alignment(alignment).is_err());
        assert!(builder.finish().is_ok());
    }

    #[test]
    fn add_rejects_a_reference_outside_declared_imports() {
        let limits = CompileLimits::p100_initial_v1();
        let header = RoadEditingModuleHeader::try_new(
            "city",
            "road-editing",
            vec!["declared".into()],
            RoadEditingProvenance::direct("editor save").expect("provenance"),
        )
        .expect("header");
        let mut builder = RoadEditingSourceModuleBuilder::new(
            header,
            GeometryAccuracyProfile::Balanced5Cm,
            GeometryDirectionProfile::Balanced2Deg,
            &limits,
        )
        .expect("builder");
        let alignment = RoadAlignmentInput::try_new(
            "alignment-a",
            CanonicalFrameReference::imported("undeclared", Vec::new(), "frame-a")
                .expect("imported frame reference"),
            RoadEditingCurveProgram::try_new(
                RoadEditingPoint3::try_new(0.0, 0.0, 0.0).expect("start"),
                vec![RoadEditingCurveSegment::line(
                    RoadEditingPoint3::try_new(1.0, 0.0, 0.0).expect("end"),
                )],
            )
            .expect("curve"),
        )
        .expect("alignment");

        assert!(builder.add_alignment(alignment).is_err());
        assert!(builder.finish().is_ok());
    }

    #[test]
    fn finish_requires_every_phase_to_cover_its_controllers_groups() {
        let limits = CompileLimits::p100_initial_v1();
        let mut builder = builder(&limits);
        let group_a = SignalGroupReference::local("group-a").expect("group a");
        let group_b = SignalGroupReference::local("group-b").expect("group b");
        let phase = SignalPhaseReference::owner_scoped(vec!["controller-a".into()], "phase-a")
            .expect("phase");
        for declaration in [
            RoadEditingDeclaration::SignalGroup(
                SignalGroupInput::try_new("group-a").expect("group a declaration"),
            ),
            RoadEditingDeclaration::SignalGroup(
                SignalGroupInput::try_new("group-b").expect("group b declaration"),
            ),
            RoadEditingDeclaration::SignalController(
                SignalControllerInput::try_new(
                    "controller-a",
                    0,
                    vec![group_a.clone(), group_b],
                    vec![phase],
                )
                .expect("controller"),
            ),
            RoadEditingDeclaration::SignalPhase(
                SignalPhaseInput::try_new(
                    "phase-a",
                    1_000,
                    vec![
                        RoadEditingSignalPhaseState::try_new(
                            group_a,
                            laneflow_static_contract::SignalAspect::Green,
                        )
                        .expect("phase state"),
                    ],
                    SignalControllerReference::local("controller-a").expect("controller reference"),
                )
                .expect("phase declaration"),
            ),
        ] {
            builder.add_declaration(declaration).expect("declaration");
        }

        assert!(builder.finish().is_err());
    }

    #[test]
    fn finish_does_not_treat_an_imported_phase_group_as_the_local_owned_group() {
        let limits = CompileLimits::p100_initial_v1();
        let header = RoadEditingModuleHeader::try_new(
            "city",
            "road-editing",
            vec!["other".into()],
            RoadEditingProvenance::direct("editor save").expect("provenance"),
        )
        .expect("header");
        let mut builder = RoadEditingSourceModuleBuilder::new(
            header,
            GeometryAccuracyProfile::Balanced5Cm,
            GeometryDirectionProfile::Balanced2Deg,
            &limits,
        )
        .expect("builder");
        let local_group = SignalGroupReference::local("group-a").expect("local group");
        let imported_group =
            SignalGroupReference::imported("other", Vec::new(), "group-a").expect("imported group");
        let phase = SignalPhaseReference::owner_scoped(vec!["controller-a".into()], "phase-a")
            .expect("phase");

        for declaration in [
            RoadEditingDeclaration::SignalGroup(
                SignalGroupInput::try_new("group-a").expect("group declaration"),
            ),
            RoadEditingDeclaration::SignalController(
                SignalControllerInput::try_new("controller-a", 0, vec![local_group], vec![phase])
                    .expect("controller"),
            ),
            RoadEditingDeclaration::SignalPhase(
                SignalPhaseInput::try_new(
                    "phase-a",
                    1_000,
                    vec![
                        RoadEditingSignalPhaseState::try_new(
                            imported_group,
                            laneflow_static_contract::SignalAspect::Green,
                        )
                        .expect("phase state"),
                    ],
                    SignalControllerReference::local("controller-a").expect("controller reference"),
                )
                .expect("phase declaration"),
            ),
        ] {
            builder.add_declaration(declaration).expect("declaration");
        }

        assert!(builder.finish().is_err());
    }

    #[test]
    fn finish_rejects_an_owner_vector_that_disagrees_with_the_child_parent() {
        let limits = CompileLimits::p100_initial_v1();
        let mut builder = builder(&limits);
        let group_a = SignalGroupReference::local("group-a").expect("group a");
        let group_b = SignalGroupReference::local("group-b").expect("group b");
        let phase_a = SignalPhaseReference::owner_scoped(vec!["controller-a".into()], "phase-a")
            .expect("phase a");
        let phase_b = SignalPhaseReference::owner_scoped(vec!["controller-b".into()], "phase-b")
            .expect("phase b");

        for declaration in [
            RoadEditingDeclaration::SignalGroup(
                SignalGroupInput::try_new("group-a").expect("group a declaration"),
            ),
            RoadEditingDeclaration::SignalGroup(
                SignalGroupInput::try_new("group-b").expect("group b declaration"),
            ),
            RoadEditingDeclaration::SignalController(
                SignalControllerInput::try_new(
                    "controller-a",
                    0,
                    vec![group_a.clone()],
                    vec![phase_b],
                )
                .expect("controller a"),
            ),
            RoadEditingDeclaration::SignalController(
                SignalControllerInput::try_new(
                    "controller-b",
                    0,
                    vec![group_b.clone()],
                    vec![phase_a],
                )
                .expect("controller b"),
            ),
            RoadEditingDeclaration::SignalPhase(
                SignalPhaseInput::try_new(
                    "phase-a",
                    1_000,
                    vec![
                        RoadEditingSignalPhaseState::try_new(
                            group_a,
                            laneflow_static_contract::SignalAspect::Green,
                        )
                        .expect("phase a state"),
                    ],
                    SignalControllerReference::local("controller-a")
                        .expect("controller a reference"),
                )
                .expect("phase a declaration"),
            ),
            RoadEditingDeclaration::SignalPhase(
                SignalPhaseInput::try_new(
                    "phase-b",
                    1_000,
                    vec![
                        RoadEditingSignalPhaseState::try_new(
                            group_b,
                            laneflow_static_contract::SignalAspect::Green,
                        )
                        .expect("phase b state"),
                    ],
                    SignalControllerReference::local("controller-b")
                        .expect("controller b reference"),
                )
                .expect("phase b declaration"),
            ),
        ] {
            builder
                .add_declaration(declaration)
                .expect("declaration is individually valid");
        }

        assert!(builder.finish().is_err());
    }

    #[test]
    fn owner_qualified_reference_charges_each_semantic_component() {
        let limits = CompileLimits::p100_initial_v1()
            .with_test_admission_limit(CompileLimitDimension::StringItemCount, 12);
        let mut builder = builder(&limits);
        builder
            .add_declaration(RoadEditingDeclaration::Movement(
                MovementInput::try_new(
                    "movement",
                    JunctionReference::local("junction").expect("junction reference"),
                    "entry",
                    "exit",
                )
                .expect("movement"),
            ))
            .expect("movement declaration");

        let path = ManeuverPathInput::try_new(
            "path",
            MovementReference::owner_scoped(vec!["junction".into()], "movement")
                .expect("movement reference"),
            LaneEdgeReference::local("entry-edge").expect("entry edge"),
            Vec::new(),
            LaneEdgeReference::local("exit-edge").expect("exit edge"),
        )
        .expect("path");
        let error = match builder.add_declaration(RoadEditingDeclaration::ManeuverPath(path)) {
            Ok(_) => panic!("four path reference components exceed the remaining item budget"),
            Err(error) => error,
        };

        assert!(matches!(
            error.diagnostics()[0].payload(),
            crate::DiagnosticPayload::CompileLimitExceeded {
                dimension: CompileLimitDimension::StringItemCount,
                limit: 12,
                observed: 13,
            }
        ));
    }

    #[test]
    fn same_phase_key_under_different_controllers_is_legal() {
        let limits = CompileLimits::p100_initial_v1();
        let mut builder = builder(&limits);
        for controller_key in ["controller-a", "controller-b"] {
            let group_key = format!("group-{}", &controller_key[11..]);
            let group = SignalGroupReference::local(group_key.clone()).expect("group reference");
            let phase =
                SignalPhaseReference::owner_scoped(vec![controller_key.into()], "phase-main")
                    .expect("phase reference");
            builder
                .add_declaration(RoadEditingDeclaration::SignalGroup(
                    SignalGroupInput::try_new(group_key).expect("signal group"),
                ))
                .expect("signal group declaration");
            builder
                .add_declaration(RoadEditingDeclaration::SignalController(
                    SignalControllerInput::try_new(
                        controller_key,
                        0,
                        vec![group.clone()],
                        vec![phase],
                    )
                    .expect("controller"),
                ))
                .expect("controller declaration");
            builder
                .add_declaration(RoadEditingDeclaration::SignalPhase(
                    SignalPhaseInput::try_new(
                        "phase-main",
                        1_000,
                        vec![
                            RoadEditingSignalPhaseState::try_new(
                                group,
                                laneflow_static_contract::SignalAspect::Green,
                            )
                            .expect("phase state"),
                        ],
                        SignalControllerReference::local(controller_key)
                            .expect("controller reference"),
                    )
                    .expect("phase"),
                ))
                .expect("phase declaration");
        }

        assert!(builder.finish().is_ok());
    }
}
