//! 私有 `LFSOURCE` 合成来源记录的确定性计量与编码。

use laneflow_static_contract::{
    AccessEffect, EntityKind, JunctionKind, LaneEdgeKind, ManeuverGateKind, ManeuverPathKind,
    MovementKind, ParticipantClassKind, RoadSectionKind, SignalAspect, StopLineKind,
};

use crate::declaration::{
    AccessRegulationInput, AccessRuleTargetInput, AuthoringLaneDeclaration,
    CanonicalFrameDeclaration, DeclarationHeader, LaneEdgeGeometryInput, OwnedAccessRegulation,
    OwnedAccessRuleTarget, OwnedCorridorElementReference, OwnedEntityReference, OwnedSignalControl,
    ParkingSpaceDeclaration, ParkingSpaceInput, ParticipantClassReference,
    SignalControllerDeclaration, SignalGroupReference, SignalPhaseInput, TypedAstDeclaration,
};
use crate::{
    CompileLimitDimension, Diagnostic, DiagnosticBundle, SourceLocation, SourceModuleHeader,
    SourceSpan,
};

use super::admission::ImportRecord;
use super::descriptor::SourceLanguage;
use super::synthetic::SYNTHETIC_FRONTEND_VERSION;

const SOURCE_RECORD_MAGIC: [u8; 8] = *b"LFSOURCE";

#[inline]
pub(super) fn encoded_source_record_len(
    header: &SourceModuleHeader,
    imports: &[ImportRecord],
    declarations: &[TypedAstDeclaration],
) -> Option<u64> {
    let mut length = u64::try_from(SOURCE_RECORD_MAGIC.len()).ok()?;
    length = length.checked_add(4 + 2)?;
    for value in [
        header.authoring_namespace_id.as_ref(),
        header.source_document_key.as_ref(),
        header.generator_build_id.as_ref(),
        header.provenance.as_ref(),
    ] {
        length = length.checked_add(4)?;
        length = length.checked_add(u64::try_from(value.len()).ok()?)?;
    }
    length = length.checked_add(32 + 32 + 1 + 8 + 16 + 4)?;
    for import in imports {
        length = length.checked_add(4)?;
        length = length.checked_add(u64::try_from(import.namespace.len()).ok()?)?;
        length = length.checked_add(16)?;
    }
    length = length.checked_add(4)?;
    for declaration in declarations {
        length = length.checked_add(encoded_declaration_len(declaration)?)?;
    }
    Some(length)
}

pub(super) fn encode_source_record(
    header: &SourceModuleHeader,
    imports: &[ImportRecord],
    declarations: &[TypedAstDeclaration],
    source_bytes_per_module_limit: u64,
) -> Result<Vec<u8>, DiagnosticBundle> {
    let expected_len = encoded_source_record_len(header, imports, declarations).unwrap_or(u64::MAX);
    let limit = source_bytes_per_module_limit.min(u64::from(u32::MAX));
    if expected_len > limit {
        return Err(DiagnosticBundle::single(
            Diagnostic::compile_limit_exceeded(
                CompileLimitDimension::SourceBytesPerModule,
                limit,
                expected_len,
            ),
        ));
    }
    let capacity = usize::try_from(expected_len).map_err(|_| {
        DiagnosticBundle::single(Diagnostic::compile_limit_exceeded(
            CompileLimitDimension::SourceBytesPerModule,
            limit,
            expected_len,
        ))
    })?;
    // 先精确计算并校验长度，再分配与写入；这样不可信规模不能通过 Vec 增长在上限检查
    // 之前制造线性分配。所有整数与 f64 都使用小端原始字节，字符串使用 u32 长度前缀。
    let mut bytes = Vec::with_capacity(capacity);
    bytes.extend_from_slice(&SOURCE_RECORD_MAGIC);
    bytes.extend_from_slice(&SYNTHETIC_FRONTEND_VERSION.to_le_bytes());
    bytes.extend_from_slice(&(SourceLanguage::SyntheticDsl as u16).to_le_bytes());
    put_bytes(&mut bytes, &header.authoring_namespace_id);
    put_bytes(&mut bytes, &header.source_document_key);
    put_bytes(&mut bytes, &header.generator_build_id);
    bytes.extend_from_slice(&header.parameters_and_inputs_digest);
    bytes.extend_from_slice(&header.frontend_options_digest);
    bytes.push(u8::from(header.random_seed.is_some()));
    bytes.extend_from_slice(&header.random_seed.unwrap_or(0).to_le_bytes());
    put_bytes(&mut bytes, &header.provenance);
    put_span(&mut bytes, &header.declaration_span);
    bytes.extend_from_slice(
        &u32::try_from(imports.len())
            .unwrap_or(u32::MAX)
            .to_le_bytes(),
    );
    for import in imports {
        put_bytes(&mut bytes, &import.namespace);
        put_source_location(&mut bytes, &import.span);
    }
    bytes.extend_from_slice(
        &u32::try_from(declarations.len())
            .unwrap_or(u32::MAX)
            .to_le_bytes(),
    );
    for declaration in declarations {
        put_declaration(&mut bytes, declaration);
    }
    debug_assert_eq!(bytes.len(), capacity);
    Ok(bytes)
}

pub(super) fn encoded_declaration_len(declaration: &TypedAstDeclaration) -> Option<u64> {
    match declaration {
        TypedAstDeclaration::LaneEdge(declaration) => {
            let mut length = lane_edge_declaration_base_len(&declaration.header.stable_key);
            for successor in &declaration.successors {
                length = length.checked_add(encoded_reference_len(
                    &successor.module_namespace,
                    successor.declaration_key(),
                ))?;
            }
            Some(length)
        }
        TypedAstDeclaration::RoadCorridor(declaration) => Some(road_corridor_declaration_len(
            &declaration.header.stable_key,
            &declaration.reference_section,
            &declaration.elements,
        )),
        TypedAstDeclaration::RoadSection(declaration) => Some(road_section_declaration_len(
            &declaration.header.stable_key,
            &declaration.kind_id,
            &declaration.lanes,
        )),
        TypedAstDeclaration::LaneGroup(declaration) => Some(lane_group_declaration_len(
            &declaration.header.stable_key,
            &declaration.road_section,
        )),
        TypedAstDeclaration::FacilityBand(declaration) => Some(facility_band_declaration_len(
            &declaration.header.stable_key,
            &declaration.kind_id,
        )),
        TypedAstDeclaration::Junction(declaration) => {
            Some(declaration_header_len(&declaration.header.stable_key))
        }
        TypedAstDeclaration::Movement(declaration) => Some(movement_declaration_len(
            &declaration.header.stable_key,
            &declaration.junction,
            &declaration.directed_entry_approach_key,
            &declaration.directed_exit_approach_key,
        )),
        TypedAstDeclaration::ManeuverPath(declaration) => Some(maneuver_path_declaration_len(
            &declaration.header.stable_key,
            &declaration.movement,
            &declaration.entry_edge,
            &declaration.internal_edges,
            &declaration.exit_edge,
        )),
        TypedAstDeclaration::StopLine(declaration) => Some(stop_line_declaration_len(
            &declaration.header.stable_key,
            &declaration.lane_edge,
        )),
        TypedAstDeclaration::ManeuverGate(declaration) => Some(maneuver_gate_declaration_len(
            &declaration.header.stable_key,
            &declaration.maneuver_path,
            declaration.transition_index,
            &declaration.stop_line,
            &declaration.signal_control,
        )),
        TypedAstDeclaration::WaitingZone(declaration) => Some(waiting_zone_declaration_len(
            &declaration.header.stable_key,
            &declaration.maneuver_path,
            &declaration.entry_gate,
            &declaration.release_gate,
        )),
        TypedAstDeclaration::SignalGroup(declaration) => {
            Some(declaration_header_len(&declaration.header.stable_key))
        }
        TypedAstDeclaration::SignalController(declaration) => {
            Some(signal_controller_declaration_len(declaration))
        }
        TypedAstDeclaration::ParkingFacility(declaration) => {
            Some(parking_facility_declaration_len(declaration))
        }
        TypedAstDeclaration::ParkingSpace(declaration) => {
            Some(parking_space_declaration_len(declaration))
        }
        TypedAstDeclaration::ParticipantClass(declaration) => {
            Some(participant_class_declaration_len(
                &declaration.header.stable_key,
                declaration.extends.as_ref(),
            ))
        }
        TypedAstDeclaration::VehicleProfile(declaration) => Some(vehicle_profile_declaration_len(
            &declaration.header.stable_key,
            &declaration.participant_class,
        )),
        TypedAstDeclaration::CanonicalFrame(declaration) => {
            Some(canonical_frame_declaration_len(declaration))
        }
        TypedAstDeclaration::AccessRule(declaration) => Some(access_rule_declaration_len(
            &declaration.header.stable_key,
            &declaration.target,
            &declaration.participant_classes,
            declaration.regulation.as_ref(),
        )),
        TypedAstDeclaration::ConflictZone(_) | TypedAstDeclaration::ParticipantStream(_) => {
            unreachable!("Synthetic frontend v4 does not construct conflict declarations")
        }
    }
}

#[inline]
pub(super) fn declaration_header_len(stable_key: &str) -> u64 {
    2_u64
        .saturating_add(4)
        .saturating_add(u64::try_from(stable_key.len()).unwrap_or(u64::MAX))
        .saturating_add(16)
}

#[inline]
pub(super) fn vehicle_profile_declaration_len(
    stable_key: &str,
    participant_class: &OwnedEntityReference<ParticipantClassKind>,
) -> u64 {
    declaration_header_len(stable_key)
        .saturating_add(encoded_reference_len(
            &participant_class.module_namespace,
            participant_class.declaration_key(),
        ))
        .saturating_add(7 * 4)
}

#[inline]
pub(super) fn canonical_frame_input_len(
    stable_key: &str,
    geometries: &[LaneEdgeGeometryInput<'_>],
    local_namespace: &str,
) -> u64 {
    geometries.iter().fold(
        declaration_header_len(stable_key).saturating_add(4),
        |total, geometry| {
            total
                .saturating_add(encoded_reference_len(
                    geometry
                        .lane_edge
                        .module_namespace()
                        .unwrap_or(local_namespace),
                    geometry.lane_edge.declaration_key(),
                ))
                .saturating_add(4)
                .saturating_add(
                    u64::try_from(geometry.centerline_points.len())
                        .unwrap_or(u64::MAX)
                        .saturating_mul(12),
                )
        },
    )
}

pub(super) fn canonical_frame_declaration_len(declaration: &CanonicalFrameDeclaration) -> u64 {
    declaration.lane_edge_geometries.iter().fold(
        declaration_header_len(&declaration.header.stable_key).saturating_add(4),
        |total, geometry| {
            total
                .saturating_add(encoded_reference_len(
                    &geometry.lane_edge.module_namespace,
                    geometry.lane_edge.declaration_key(),
                ))
                .saturating_add(4)
                .saturating_add(
                    u64::try_from(geometry.centerline_points.len())
                        .unwrap_or(u64::MAX)
                        .saturating_mul(12),
                )
        },
    )
}

#[inline]
pub(super) fn lane_edge_declaration_base_len(stable_key: &str) -> u64 {
    declaration_header_len(stable_key).saturating_add(4 + 4 + 4)
}

#[inline]
pub(super) fn facility_band_declaration_len(stable_key: &str, kind_id: &str) -> u64 {
    declaration_header_len(stable_key)
        .saturating_add(4)
        .saturating_add(u64::try_from(kind_id.len()).unwrap_or(u64::MAX))
}

#[inline]
pub(super) fn movement_declaration_len(
    stable_key: &str,
    junction: &OwnedEntityReference<JunctionKind>,
    directed_entry_approach_key: &str,
    directed_exit_approach_key: &str,
) -> u64 {
    declaration_header_len(stable_key)
        .saturating_add(encoded_reference_len(
            &junction.module_namespace,
            junction.declaration_key(),
        ))
        .saturating_add(4)
        .saturating_add(u64::try_from(directed_entry_approach_key.len()).unwrap_or(u64::MAX))
        .saturating_add(4)
        .saturating_add(u64::try_from(directed_exit_approach_key.len()).unwrap_or(u64::MAX))
}

#[inline]
pub(super) fn maneuver_path_declaration_len(
    stable_key: &str,
    movement: &OwnedEntityReference<MovementKind>,
    entry_edge: &OwnedEntityReference<LaneEdgeKind>,
    internal_edges: &[OwnedEntityReference<LaneEdgeKind>],
    exit_edge: &OwnedEntityReference<LaneEdgeKind>,
) -> u64 {
    let mut length = declaration_header_len(stable_key)
        .saturating_add(encoded_reference_len(
            &movement.module_namespace,
            movement.declaration_key(),
        ))
        .saturating_add(encoded_reference_len(
            &entry_edge.module_namespace,
            entry_edge.declaration_key(),
        ))
        .saturating_add(4);
    for edge in internal_edges {
        length = length.saturating_add(encoded_reference_len(
            &edge.module_namespace,
            edge.declaration_key(),
        ));
    }
    length.saturating_add(encoded_reference_len(
        &exit_edge.module_namespace,
        exit_edge.declaration_key(),
    ))
}

#[inline]
pub(super) fn stop_line_declaration_len(
    stable_key: &str,
    lane_edge: &OwnedEntityReference<LaneEdgeKind>,
) -> u64 {
    declaration_header_len(stable_key).saturating_add(encoded_reference_len(
        &lane_edge.module_namespace,
        lane_edge.declaration_key(),
    ))
}

#[inline]
pub(super) fn maneuver_gate_declaration_len(
    stable_key: &str,
    maneuver_path: &OwnedEntityReference<ManeuverPathKind>,
    _transition_index: u32,
    stop_line: &OwnedEntityReference<StopLineKind>,
    signal_control: &OwnedSignalControl,
) -> u64 {
    declaration_header_len(stable_key)
        .saturating_add(encoded_reference_len(
            &maneuver_path.module_namespace,
            maneuver_path.declaration_key(),
        ))
        .saturating_add(4)
        .saturating_add(encoded_reference_len(
            &stop_line.module_namespace,
            stop_line.declaration_key(),
        ))
        .saturating_add(1)
        .saturating_add(match signal_control {
            OwnedSignalControl::Group(group) => {
                encoded_reference_len(&group.module_namespace, group.declaration_key())
            }
            OwnedSignalControl::None => 0,
        })
}

#[inline]
pub(super) fn signal_controller_input_len(
    stable_key: &str,
    _offset_ms: u64,
    signal_groups: &[SignalGroupReference<'_>],
    phases: &[SignalPhaseInput<'_>],
    local_namespace: &str,
) -> u64 {
    let mut length = declaration_header_len(stable_key).saturating_add(8 + 4 + 4);
    for group in signal_groups {
        length = length.saturating_add(encoded_reference_len(
            group.module_namespace().unwrap_or(local_namespace),
            group.declaration_key(),
        ));
    }
    for phase in phases {
        length = length
            .saturating_add(declaration_header_len(phase.signal_phase_key))
            // controller_relation_span 与 synthetic phase 声明共用同一文本位置，但仍是
            // 独立来源语义，必须进入精确来源记录。
            .saturating_add(16 + 8 + 4);
        for state in phase.states {
            length = length
                .saturating_add(encoded_reference_len(
                    state
                        .signal_group
                        .module_namespace()
                        .unwrap_or(local_namespace),
                    state.signal_group.declaration_key(),
                ))
                .saturating_add(1);
        }
    }
    length
}

pub(super) fn signal_controller_declaration_len(declaration: &SignalControllerDeclaration) -> u64 {
    let mut length =
        declaration_header_len(&declaration.header.stable_key).saturating_add(8 + 4 + 4);
    for group in &declaration.signal_groups {
        length = length.saturating_add(encoded_reference_len(
            &group.module_namespace,
            group.declaration_key(),
        ));
    }
    for phase in &declaration.phases {
        length = length
            .saturating_add(declaration_header_len(&phase.header.stable_key))
            .saturating_add(16 + 8 + 4);
        for state in &phase.states {
            length = length
                .saturating_add(encoded_reference_len(
                    &state.signal_group.module_namespace,
                    state.signal_group.declaration_key(),
                ))
                .saturating_add(1);
        }
    }
    length
}

#[inline]
pub(super) fn parking_space_input_len(input: &ParkingSpaceInput<'_>, local_namespace: &str) -> u64 {
    let mut length = declaration_header_len(input.parking_space_key).saturating_add(1 + 4 * 6);
    if let Some(area) = input.parking_facility {
        length = length.saturating_add(encoded_reference_len(
            area.module_namespace().unwrap_or(local_namespace),
            area.declaration_key(),
        ));
    }
    for edge in [input.entry.lane_edge, input.exit.lane_edge] {
        length = length.saturating_add(encoded_reference_len(
            edge.module_namespace().unwrap_or(local_namespace),
            edge.declaration_key(),
        ));
    }
    length
}

#[inline]
pub(super) fn parking_facility_input_len(
    input: &crate::declaration::ParkingFacilityInput<'_>,
    local_namespace: &str,
) -> u64 {
    let mut length = declaration_header_len(input.parking_facility_key).saturating_add(12);
    for anchor in input
        .virtual_entries
        .iter()
        .chain(input.virtual_exits.iter())
    {
        length = length
            .saturating_add(encoded_reference_len(
                anchor
                    .lane_edge
                    .module_namespace()
                    .unwrap_or(local_namespace),
                anchor.lane_edge.declaration_key(),
            ))
            .saturating_add(4);
    }
    length
}

pub(super) fn parking_facility_declaration_len(
    declaration: &crate::declaration::ParkingFacilityDeclaration,
) -> u64 {
    let mut length = declaration_header_len(&declaration.header.stable_key).saturating_add(12);
    for anchor in declaration
        .virtual_entries
        .iter()
        .chain(declaration.virtual_exits.iter())
    {
        length = length
            .saturating_add(encoded_reference_len(
                &anchor.lane_edge.module_namespace,
                anchor.lane_edge.declaration_key(),
            ))
            .saturating_add(4);
    }
    length
}

pub(super) fn parking_space_declaration_len(declaration: &ParkingSpaceDeclaration) -> u64 {
    let mut length =
        declaration_header_len(&declaration.header.stable_key).saturating_add(1 + 4 * 6);
    if let Some(area) = &declaration.parking_facility {
        length = length.saturating_add(encoded_reference_len(
            &area.module_namespace,
            area.declaration_key(),
        ));
    }
    for anchor in [&declaration.entry, &declaration.exit] {
        length = length.saturating_add(encoded_reference_len(
            &anchor.lane_edge.module_namespace,
            anchor.lane_edge.declaration_key(),
        ));
    }
    length
}

#[inline]
pub(super) fn participant_class_declaration_len(
    stable_key: &str,
    extends: Option<&OwnedEntityReference<ParticipantClassKind>>,
) -> u64 {
    declaration_header_len(stable_key)
        .saturating_add(1)
        .saturating_add(extends.map_or(0, |reference| {
            encoded_reference_len(&reference.module_namespace, reference.declaration_key())
        }))
}

#[inline]
pub(super) fn access_target_input_parts(
    target: AccessRuleTargetInput<'_>,
) -> (EntityKind, Option<&str>, &str) {
    match target {
        AccessRuleTargetInput::LaneEdge(reference) => (
            EntityKind::LaneEdge,
            reference.module_namespace(),
            reference.declaration_key(),
        ),
        AccessRuleTargetInput::LaneGroup(reference) => (
            EntityKind::LaneGroup,
            reference.module_namespace(),
            reference.declaration_key(),
        ),
        AccessRuleTargetInput::RoadSection(reference) => (
            EntityKind::RoadSection,
            reference.module_namespace(),
            reference.declaration_key(),
        ),
        AccessRuleTargetInput::ManeuverPath(reference) => (
            EntityKind::ManeuverPath,
            reference.module_namespace(),
            reference.declaration_key(),
        ),
        AccessRuleTargetInput::FacilityBand(reference) => (
            EntityKind::FacilityBand,
            reference.module_namespace(),
            reference.declaration_key(),
        ),
    }
}

#[inline]
pub(super) fn access_rule_input_len(
    stable_key: &str,
    target: AccessRuleTargetInput<'_>,
    participant_classes: &[ParticipantClassReference<'_>],
    regulation: Option<AccessRegulationInput<'_>>,
    local_namespace: &str,
) -> u64 {
    let (_, target_namespace, target_key) = access_target_input_parts(target);
    let mut length = declaration_header_len(stable_key)
        .saturating_add(2)
        .saturating_add(encoded_reference_len(
            target_namespace.unwrap_or(local_namespace),
            target_key,
        ))
        .saturating_add(1 + 4 + 4);
    for class in participant_classes {
        length = length.saturating_add(encoded_reference_len(
            class.module_namespace().unwrap_or(local_namespace),
            class.declaration_key(),
        ));
    }
    length = length.saturating_add(1);
    if let Some(regulation) = regulation {
        for value in [
            Some(regulation.jurisdiction),
            Some(regulation.version),
            regulation.source,
        ] {
            length = length.saturating_add(1);
            if let Some(value) = value {
                length = length
                    .saturating_add(4)
                    .saturating_add(u64::try_from(value.len()).unwrap_or(u64::MAX));
            }
        }
    }
    length
}

pub(super) fn access_rule_declaration_len(
    stable_key: &str,
    target: &OwnedAccessRuleTarget,
    participant_classes: &[OwnedEntityReference<ParticipantClassKind>],
    regulation: Option<&OwnedAccessRegulation>,
) -> u64 {
    let mut length = declaration_header_len(stable_key)
        .saturating_add(2)
        .saturating_add(access_target_encoded_reference_len(target))
        .saturating_add(1 + 4 + 4);
    for class in participant_classes {
        length = length.saturating_add(encoded_reference_len(
            &class.module_namespace,
            class.declaration_key(),
        ));
    }
    length = length.saturating_add(1);
    if let Some(regulation) = regulation {
        for value in [
            Some(regulation.jurisdiction.as_ref()),
            Some(regulation.version.as_ref()),
            regulation.source.as_deref(),
        ] {
            length = length.saturating_add(1);
            if let Some(value) = value {
                length = length
                    .saturating_add(4)
                    .saturating_add(u64::try_from(value.len()).unwrap_or(u64::MAX));
            }
        }
    }
    length
}

pub(super) fn access_target_encoded_reference_len(target: &OwnedAccessRuleTarget) -> u64 {
    match target {
        OwnedAccessRuleTarget::LaneEdge(reference) => {
            encoded_reference_len(&reference.module_namespace, reference.declaration_key())
        }
        OwnedAccessRuleTarget::LaneGroup(reference) => {
            encoded_reference_len(&reference.module_namespace, reference.declaration_key())
        }
        OwnedAccessRuleTarget::RoadSection(reference) => {
            encoded_reference_len(&reference.module_namespace, reference.declaration_key())
        }
        OwnedAccessRuleTarget::ManeuverPath(reference) => {
            encoded_reference_len(&reference.module_namespace, reference.declaration_key())
        }
        OwnedAccessRuleTarget::FacilityBand(reference) => {
            encoded_reference_len(&reference.module_namespace, reference.declaration_key())
        }
    }
}

#[inline]
pub(super) fn waiting_zone_declaration_len(
    stable_key: &str,
    maneuver_path: &OwnedEntityReference<ManeuverPathKind>,
    entry_gate: &OwnedEntityReference<ManeuverGateKind>,
    release_gate: &OwnedEntityReference<ManeuverGateKind>,
) -> u64 {
    declaration_header_len(stable_key)
        .saturating_add(encoded_reference_len(
            &maneuver_path.module_namespace,
            maneuver_path.declaration_key(),
        ))
        .saturating_add(encoded_reference_len(
            &entry_gate.module_namespace,
            entry_gate.declaration_key(),
        ))
        .saturating_add(encoded_reference_len(
            &release_gate.module_namespace,
            release_gate.declaration_key(),
        ))
        .saturating_add(4)
}

#[inline]
pub(super) fn lane_group_declaration_len(
    stable_key: &str,
    road_section: &OwnedEntityReference<RoadSectionKind>,
) -> u64 {
    declaration_header_len(stable_key).saturating_add(encoded_reference_len(
        &road_section.module_namespace,
        road_section.declaration_key(),
    ))
}

#[inline]
pub(super) fn road_section_declaration_len(
    stable_key: &str,
    kind_id: &str,
    lanes: &[AuthoringLaneDeclaration],
) -> u64 {
    let mut length = declaration_header_len(stable_key)
        .saturating_add(4)
        .saturating_add(u64::try_from(kind_id.len()).unwrap_or(u64::MAX))
        .saturating_add(4);
    for lane in lanes {
        length = length
            .saturating_add(declaration_header_len(&lane.header.stable_key))
            .saturating_add(4)
            .saturating_add(1);
        for edge in &lane.edge_chain {
            length = length.saturating_add(encoded_reference_len(
                &edge.module_namespace,
                edge.declaration_key(),
            ));
        }
        if let Some(lane_group) = &lane.lane_group {
            length = length.saturating_add(encoded_reference_len(
                &lane_group.module_namespace,
                lane_group.declaration_key(),
            ));
        }
    }
    length
}

#[inline]
pub(super) fn road_corridor_declaration_len(
    stable_key: &str,
    reference_section: &OwnedEntityReference<RoadSectionKind>,
    elements: &[OwnedCorridorElementReference],
) -> u64 {
    let mut length = declaration_header_len(stable_key)
        .saturating_add(encoded_reference_len(
            &reference_section.module_namespace,
            reference_section.declaration_key(),
        ))
        .saturating_add(4);
    for element in elements {
        let reference_len = match element {
            OwnedCorridorElementReference::RoadSection(reference) => {
                encoded_reference_len(&reference.module_namespace, reference.declaration_key())
            }
            OwnedCorridorElementReference::FacilityBand(reference) => {
                encoded_reference_len(&reference.module_namespace, reference.declaration_key())
            }
        };
        length = length.saturating_add(2).saturating_add(reference_len);
    }
    length
}

#[inline]
pub(super) fn encoded_reference_len(module_namespace: &str, declaration_key: &str) -> u64 {
    4_u64
        .saturating_add(u64::try_from(module_namespace.len()).unwrap_or(u64::MAX))
        .saturating_add(4)
        .saturating_add(u64::try_from(declaration_key.len()).unwrap_or(u64::MAX))
        .saturating_add(16)
}

pub(super) fn put_declaration(output: &mut Vec<u8>, declaration: &TypedAstDeclaration) {
    match declaration {
        TypedAstDeclaration::LaneEdge(declaration) => {
            put_declaration_header(output, &declaration.header);
            output.extend_from_slice(
                &declaration
                    .geometry_authority
                    .direct_length()
                    .expect("synthetic source records only contain direct lane lengths")
                    .millimetres()
                    .to_le_bytes(),
            );
            output.extend_from_slice(
                &declaration
                    .speed_limit
                    .millimetres_per_second()
                    .to_le_bytes(),
            );
            output.extend_from_slice(
                &u32::try_from(declaration.successors.len())
                    .unwrap_or(u32::MAX)
                    .to_le_bytes(),
            );
            for successor in &declaration.successors {
                put_bytes(output, &successor.module_namespace);
                put_bytes(output, successor.declaration_key());
                put_source_location(output, &successor.span);
            }
        }
        TypedAstDeclaration::RoadCorridor(declaration) => {
            put_declaration_header(output, &declaration.header);
            put_owned_reference(output, &declaration.reference_section);
            output.extend_from_slice(
                &u32::try_from(declaration.elements.len())
                    .unwrap_or(u32::MAX)
                    .to_le_bytes(),
            );
            for element in &declaration.elements {
                match element {
                    OwnedCorridorElementReference::RoadSection(reference) => {
                        output.extend_from_slice(&(EntityKind::RoadSection as u16).to_le_bytes());
                        put_owned_reference(output, reference);
                    }
                    OwnedCorridorElementReference::FacilityBand(reference) => {
                        output.extend_from_slice(&(EntityKind::FacilityBand as u16).to_le_bytes());
                        put_owned_reference(output, reference);
                    }
                }
            }
        }
        TypedAstDeclaration::RoadSection(declaration) => {
            put_declaration_header(output, &declaration.header);
            put_bytes(output, &declaration.kind_id);
            output.extend_from_slice(
                &u32::try_from(declaration.lanes.len())
                    .unwrap_or(u32::MAX)
                    .to_le_bytes(),
            );
            for lane in &declaration.lanes {
                put_declaration_header(output, &lane.header);
                output.extend_from_slice(
                    &u32::try_from(lane.edge_chain.len())
                        .unwrap_or(u32::MAX)
                        .to_le_bytes(),
                );
                for edge in &lane.edge_chain {
                    put_owned_reference(output, edge);
                }
                output.push(u8::from(lane.lane_group.is_some()));
                if let Some(lane_group) = &lane.lane_group {
                    put_owned_reference(output, lane_group);
                }
            }
        }
        TypedAstDeclaration::LaneGroup(declaration) => {
            put_declaration_header(output, &declaration.header);
            put_owned_reference(output, &declaration.road_section);
        }
        TypedAstDeclaration::FacilityBand(declaration) => {
            put_declaration_header(output, &declaration.header);
            put_bytes(output, &declaration.kind_id);
        }
        TypedAstDeclaration::Junction(declaration) => {
            put_declaration_header(output, &declaration.header);
        }
        TypedAstDeclaration::Movement(declaration) => {
            put_declaration_header(output, &declaration.header);
            put_owned_reference(output, &declaration.junction);
            put_bytes(output, &declaration.directed_entry_approach_key);
            put_bytes(output, &declaration.directed_exit_approach_key);
        }
        TypedAstDeclaration::ManeuverPath(declaration) => {
            put_declaration_header(output, &declaration.header);
            put_owned_reference(output, &declaration.movement);
            put_owned_reference(output, &declaration.entry_edge);
            output.extend_from_slice(
                &u32::try_from(declaration.internal_edges.len())
                    .unwrap_or(u32::MAX)
                    .to_le_bytes(),
            );
            for edge in &declaration.internal_edges {
                put_owned_reference(output, edge);
            }
            put_owned_reference(output, &declaration.exit_edge);
        }
        TypedAstDeclaration::StopLine(declaration) => {
            put_declaration_header(output, &declaration.header);
            put_owned_reference(output, &declaration.lane_edge);
        }
        TypedAstDeclaration::ManeuverGate(declaration) => {
            put_declaration_header(output, &declaration.header);
            put_owned_reference(output, &declaration.maneuver_path);
            output.extend_from_slice(&declaration.transition_index.to_le_bytes());
            put_owned_reference(output, &declaration.stop_line);
            match &declaration.signal_control {
                OwnedSignalControl::None => output.push(0),
                OwnedSignalControl::Group(group) => {
                    output.push(1);
                    put_owned_reference(output, group);
                }
            }
        }
        TypedAstDeclaration::WaitingZone(declaration) => {
            put_declaration_header(output, &declaration.header);
            put_owned_reference(output, &declaration.maneuver_path);
            put_owned_reference(output, &declaration.entry_gate);
            put_owned_reference(output, &declaration.release_gate);
            output.extend_from_slice(&declaration.max_occupancy.to_le_bytes());
        }
        TypedAstDeclaration::SignalGroup(declaration) => {
            put_declaration_header(output, &declaration.header);
        }
        TypedAstDeclaration::SignalController(declaration) => {
            put_declaration_header(output, &declaration.header);
            output.extend_from_slice(&declaration.offset_ms.to_le_bytes());
            output.extend_from_slice(
                &u32::try_from(declaration.signal_groups.len())
                    .unwrap_or(u32::MAX)
                    .to_le_bytes(),
            );
            for group in &declaration.signal_groups {
                put_owned_reference(output, group);
            }
            output.extend_from_slice(
                &u32::try_from(declaration.phases.len())
                    .unwrap_or(u32::MAX)
                    .to_le_bytes(),
            );
            for phase in &declaration.phases {
                put_declaration_header(output, &phase.header);
                put_source_location(output, &phase.controller_relation_span);
                output.extend_from_slice(&phase.duration_ms.to_le_bytes());
                output.extend_from_slice(
                    &u32::try_from(phase.states.len())
                        .unwrap_or(u32::MAX)
                        .to_le_bytes(),
                );
                for state in &phase.states {
                    put_owned_reference(output, &state.signal_group);
                    output.push(signal_aspect_source_code(state.aspect));
                }
            }
        }
        TypedAstDeclaration::ParkingFacility(declaration) => {
            put_declaration_header(output, &declaration.header);
            output.extend_from_slice(&declaration.virtual_capacity.to_le_bytes());
            for anchors in [&declaration.virtual_entries, &declaration.virtual_exits] {
                output.extend_from_slice(
                    &u32::try_from(anchors.len())
                        .unwrap_or(u32::MAX)
                        .to_le_bytes(),
                );
                for anchor in anchors.iter() {
                    put_owned_reference(output, &anchor.lane_edge);
                    output.extend_from_slice(&anchor.progress_mm.to_le_bytes());
                }
            }
        }
        TypedAstDeclaration::ParkingSpace(declaration) => {
            put_declaration_header(output, &declaration.header);
            output.push(u8::from(declaration.parking_facility.is_some()));
            if let Some(area) = &declaration.parking_facility {
                put_owned_reference(output, area);
            }
            for anchor in [&declaration.entry, &declaration.exit] {
                put_owned_reference(output, &anchor.lane_edge);
                output.extend_from_slice(&anchor.progress_mm.to_le_bytes());
            }
            let geometry = declaration.geometry;
            output.extend_from_slice(&geometry.lateral_offset_mm.to_le_bytes());
            output.extend_from_slice(&geometry.heading_offset_radians.to_bits().to_le_bytes());
            output.extend_from_slice(&geometry.length_mm.to_le_bytes());
            output.extend_from_slice(&geometry.width_mm.to_le_bytes());
        }
        TypedAstDeclaration::ParticipantClass(declaration) => {
            put_declaration_header(output, &declaration.header);
            output.push(u8::from(declaration.extends.is_some()));
            if let Some(parent) = &declaration.extends {
                put_owned_reference(output, parent);
            }
        }
        TypedAstDeclaration::VehicleProfile(declaration) => {
            put_declaration_header(output, &declaration.header);
            put_owned_reference(output, &declaration.participant_class);
            let iidm = declaration.iidm;
            for value in [iidm.length_mm, iidm.desired_speed_mm_s, iidm.min_gap_mm] {
                output.extend_from_slice(&value.to_le_bytes());
            }
            for value in [
                iidm.time_headway_seconds,
                iidm.max_acceleration_meters_per_second_squared,
                iidm.comfortable_deceleration_meters_per_second_squared,
                iidm.emergency_deceleration_meters_per_second_squared,
            ] {
                output.extend_from_slice(&value.to_bits().to_le_bytes());
            }
        }
        TypedAstDeclaration::CanonicalFrame(declaration) => {
            put_declaration_header(output, &declaration.header);
            output.extend_from_slice(
                &u32::try_from(declaration.lane_edge_geometries.len())
                    .unwrap_or(u32::MAX)
                    .to_le_bytes(),
            );
            for geometry in &declaration.lane_edge_geometries {
                put_owned_reference(output, &geometry.lane_edge);
                output.extend_from_slice(
                    &u32::try_from(geometry.centerline_points.len())
                        .unwrap_or(u32::MAX)
                        .to_le_bytes(),
                );
                for point in &geometry.centerline_points {
                    for component in [point.x, point.y, point.z] {
                        output.extend_from_slice(&component.to_bits().to_le_bytes());
                    }
                }
            }
        }
        TypedAstDeclaration::AccessRule(declaration) => {
            put_declaration_header(output, &declaration.header);
            put_access_target(output, &declaration.target);
            output.push(access_effect_source_code(declaration.effect));
            output.extend_from_slice(
                &u32::try_from(declaration.participant_classes.len())
                    .unwrap_or(u32::MAX)
                    .to_le_bytes(),
            );
            for class in &declaration.participant_classes {
                put_owned_reference(output, class);
            }
            output.extend_from_slice(&declaration.priority.to_le_bytes());
            output.push(u8::from(declaration.regulation.is_some()));
            if let Some(regulation) = &declaration.regulation {
                put_optional_bytes(output, Some(&regulation.jurisdiction));
                put_optional_bytes(output, Some(&regulation.version));
                put_optional_bytes(output, regulation.source.as_deref());
            }
        }
        TypedAstDeclaration::ConflictZone(_) | TypedAstDeclaration::ParticipantStream(_) => {
            unreachable!("Synthetic frontend v4 does not construct conflict declarations")
        }
    }
}

#[allow(unreachable_patterns)]
pub(super) fn signal_aspect_source_code(aspect: SignalAspect) -> u8 {
    match aspect {
        SignalAspect::Red => 1,
        SignalAspect::Yellow => 2,
        SignalAspect::Green => 3,
        _ => unreachable!("SignalAspect extension requires a new synthetic frontend version"),
    }
}

#[allow(unreachable_patterns)]
pub(super) fn access_effect_source_code(effect: AccessEffect) -> u8 {
    match effect {
        AccessEffect::Allow => 1,
        AccessEffect::Deny => 2,
        _ => unreachable!("AccessEffect extension requires a new synthetic frontend version"),
    }
}

pub(super) fn put_declaration_header(output: &mut Vec<u8>, header: &DeclarationHeader) {
    output.extend_from_slice(&(header.entity_kind as u16).to_le_bytes());
    put_bytes(output, &header.stable_key);
    put_source_location(output, &header.span);
}

pub(super) fn put_owned_reference<K: laneflow_static_contract::EntityKindMarker>(
    output: &mut Vec<u8>,
    reference: &OwnedEntityReference<K>,
) {
    put_bytes(output, &reference.module_namespace);
    put_bytes(output, reference.declaration_key());
    put_source_location(output, &reference.span);
}

pub(super) fn put_access_target(output: &mut Vec<u8>, target: &OwnedAccessRuleTarget) {
    match target {
        OwnedAccessRuleTarget::LaneEdge(reference) => {
            output.extend_from_slice(&(EntityKind::LaneEdge as u16).to_le_bytes());
            put_owned_reference(output, reference);
        }
        OwnedAccessRuleTarget::LaneGroup(reference) => {
            output.extend_from_slice(&(EntityKind::LaneGroup as u16).to_le_bytes());
            put_owned_reference(output, reference);
        }
        OwnedAccessRuleTarget::RoadSection(reference) => {
            output.extend_from_slice(&(EntityKind::RoadSection as u16).to_le_bytes());
            put_owned_reference(output, reference);
        }
        OwnedAccessRuleTarget::ManeuverPath(reference) => {
            output.extend_from_slice(&(EntityKind::ManeuverPath as u16).to_le_bytes());
            put_owned_reference(output, reference);
        }
        OwnedAccessRuleTarget::FacilityBand(reference) => {
            output.extend_from_slice(&(EntityKind::FacilityBand as u16).to_le_bytes());
            put_owned_reference(output, reference);
        }
    }
}

pub(super) fn put_optional_bytes(output: &mut Vec<u8>, value: Option<&str>) {
    output.push(u8::from(value.is_some()));
    if let Some(value) = value {
        put_bytes(output, value);
    }
}

pub(super) fn put_bytes(output: &mut Vec<u8>, value: &str) {
    output.extend_from_slice(&u32::try_from(value.len()).unwrap_or(u32::MAX).to_le_bytes());
    output.extend_from_slice(value.as_bytes());
}

pub(super) fn put_span(output: &mut Vec<u8>, span: &SourceSpan) {
    output.extend_from_slice(&span.start().line().to_le_bytes());
    output.extend_from_slice(&span.start().column().to_le_bytes());
    output.extend_from_slice(&span.end().line().to_le_bytes());
    output.extend_from_slice(&span.end().column().to_le_bytes());
}

fn put_source_location(output: &mut Vec<u8>, location: &SourceLocation) {
    let span = location
        .text_span()
        .expect("the synthetic exact source record only contains text locations");
    put_span(output, span);
}
