use super::*;

/// 按实体种类与 typed ordinal 从冻结 LIR 取回 16 字节稳定标识。
///
/// 策略集不能作为拓扑或空间关系端点，遇到 `RightOfWayPolicySet` 时不可达。
pub(super) fn entity_stable_id(
    lir: &crate::lir::LirUnit,
    kind: EntityKind,
    ordinal: u32,
) -> [u8; 16] {
    let index = usize::try_from(ordinal)
        .expect("supported compiler targets can index a validated entity ordinal");
    match kind {
        EntityKind::RoadCorridor => stable_id_bytes(lir.road_corridors[index].stable_id),
        EntityKind::RoadSection => stable_id_bytes(lir.road_sections[index].stable_id),
        EntityKind::AuthoringLane => stable_id_bytes(lir.authoring_lanes[index].stable_id),
        EntityKind::LaneEdge => stable_id_bytes(lir.lane_edges[index].stable_id),
        EntityKind::Junction => stable_id_bytes(lir.junctions[index].stable_id),
        EntityKind::Movement => stable_id_bytes(lir.movements[index].stable_id),
        EntityKind::ManeuverPath => stable_id_bytes(lir.maneuver_paths[index].stable_id),
        EntityKind::ManeuverGate => stable_id_bytes(lir.maneuver_gates[index].stable_id),
        EntityKind::WaitingZone => stable_id_bytes(lir.waiting_zones[index].stable_id),
        EntityKind::StopLine => stable_id_bytes(lir.stop_lines[index].stable_id),
        EntityKind::SignalGroup => stable_id_bytes(lir.signal_groups[index].stable_id),
        EntityKind::SignalController => stable_id_bytes(lir.signal_controllers[index].stable_id),
        EntityKind::SignalPhase => stable_id_bytes(lir.signal_phases[index].stable_id),
        EntityKind::ParkingFacility => stable_id_bytes(lir.parking_facilities[index].stable_id),
        EntityKind::ParkingSpace => stable_id_bytes(lir.parking_spaces[index].stable_id),
        EntityKind::LaneGroup => stable_id_bytes(lir.lane_groups[index].stable_id),
        EntityKind::FacilityBand => stable_id_bytes(lir.facility_bands[index].stable_id),
        EntityKind::ParticipantClass => stable_id_bytes(lir.participant_classes[index].stable_id),
        EntityKind::AccessRule => stable_id_bytes(lir.access_rules[index].stable_id),
        EntityKind::VehicleProfile => stable_id_bytes(lir.vehicle_profiles[index].stable_id),
        EntityKind::ConflictZone => stable_id_bytes(lir.conflict_zones[index].stable_id),
        EntityKind::ParticipantStream => stable_id_bytes(lir.participant_streams[index].stable_id),
        EntityKind::CanonicalFrame => stable_id_bytes(lir.canonical_frames[index].stable_id),
        EntityKind::RightOfWayPolicySet => {
            // 合同 §4.4.3：策略局部成员走 OwnerLocalSource/PolicyLocalChange，
            // 不派生拓扑关系或空间几何关系；本函数只投影这两类关系的端点。
            unreachable!("policy sets cannot be topology or spatial relation endpoints")
        }
    }
}

#[allow(
    clippy::too_many_arguments,
    reason = "the compiler-private A.5 tuple projection keeps both typed endpoints explicit"
)]
fn push_relation_tuple(
    relations: &mut RelationSink<'_>,
    lir: &crate::lir::LirUnit,
    owner_kind: EntityKind,
    owner_ordinal: u32,
    role: u8,
    local_index: u32,
    subject_kind: EntityKind,
    subject_ordinal: u32,
) {
    (relations.0)(RelationTuple {
        owner_entity_kind: owner_kind,
        owner_stable_id: entity_stable_id(lir, owner_kind, owner_ordinal),
        role,
        local_index,
        subject_entity_kind: subject_kind,
        subject_stable_id: entity_stable_id(lir, subject_kind, subject_ordinal),
    });
}

/// 规范关系元组的接收端。
struct RelationSink<'s>(&'s mut dyn FnMut(RelationTuple));

/// 交通边是否为路口内部边；路口内部边表按边序号排序。
pub(super) fn is_internal_edge(lir: &crate::lir::LirUnit, edge: u32) -> bool {
    lir.junction_internal_edges
        .binary_search_by_key(&edge, |entry| entry.edge.raw())
        .is_ok()
}

/// 规范关系遍历所需的路口局部序号暂存长度：路口内部边、冲突区与参与者流三组路口成员
/// 关系逐个路口编号，共用一段按路口序号索引的计数器。
pub(super) fn canonical_relation_scratch_len(lir: &crate::lir::LirUnit) -> usize {
    lir.junctions.len()
}

/// 依次产出全部规范关系元组（未排序）。路口局部序号的暂存由调用方从编译遍授予，长度为
/// [`canonical_relation_scratch_len`]；本函数自身不分配。
pub(super) fn for_each_canonical_relation_tuple(
    lir: &crate::lir::LirUnit,
    pass: &crate::ledger::PassBudget<'_>,
    f: &mut dyn FnMut(RelationTuple),
) -> Result<(), PortableEmissionError> {
    let mut junction_next = pass.vec::<u32>(canonical_relation_scratch_len(lir))?;
    junction_next.resize(canonical_relation_scratch_len(lir), 0)?;
    for_each_canonical_relation(lir, Some(&mut junction_next), &mut RelationSink(f));
    Ok(())
}

/// 规范关系元组的数量，即 [`canonical_relation_tuples`] 的元组计划容量。不分配，供准入前
/// 求计划使用。
pub(super) fn canonical_relation_count(lir: &crate::lir::LirUnit) -> usize {
    let mut count = 0_usize;
    for_each_canonical_relation(lir, None, &mut RelationSink(&mut |_| count += 1));
    count
}

/// 从冻结 LIR 投影全部规范关系元组并按规范顺序排序，存入编译遍授予的序列；路口内部边
/// 不参与 successor 关系。授予容量为 [`canonical_relation_count`] 个元组加
/// [`canonical_relation_scratch_len`] 个 `u32` 暂存。
pub(super) fn canonical_relation_tuples(
    lir: &crate::lir::LirUnit,
    pass: &crate::ledger::PassBudget<'_>,
) -> Result<crate::ledger::BoundedVec<RelationTuple>, PortableEmissionError> {
    let mut relations = pass.vec(canonical_relation_count(lir))?;
    let mut overflow = None;
    for_each_canonical_relation_tuple(lir, pass, &mut |relation| {
        if let Err(error) = relations.push(relation) {
            overflow.get_or_insert(error);
        }
    })?;
    if let Some(error) = overflow {
        return Err(error.into());
    }
    relations.sort_unstable();
    Ok(relations)
}

/// 不经账本的规范关系元组；只供 LFSD 构造代码直接写入对象树之前的差异路径使用。
pub(super) fn canonical_relation_tuple_vec(lir: &crate::lir::LirUnit) -> Vec<RelationTuple> {
    let mut relations = Vec::new();
    let mut junction_next = vec![0_u32; canonical_relation_scratch_len(lir)];
    for_each_canonical_relation(
        lir,
        Some(&mut junction_next),
        &mut RelationSink(&mut |relation| relations.push(relation)),
    );
    relations.sort_unstable();
    relations
}

/// 路口局部序号计数器清零；只计数时无暂存。
fn reset_junction_next(junction_next: &mut Option<&mut [u32]>) {
    if let Some(next) = junction_next {
        next.fill(0);
    }
}

/// 取路口的下一个局部序号；只计数时无暂存，返回 0。
fn next_junction_local(junction_next: &mut Option<&mut [u32]>, junction: usize) -> u32 {
    let Some(next) = junction_next else {
        return 0;
    };
    let local_index = next[junction];
    next[junction] += 1;
    local_index
}

/// 规范关系遍历，自身不分配。`junction_next` 为 `None` 时只计数：路口内部边、冲突区与
/// 参与者流关系的局部序号一律为 0，接收端得到的元组只能用于计数。
fn for_each_canonical_relation(
    lir: &crate::lir::LirUnit,
    mut junction_next: Option<&mut [u32]>,
    relations: &mut RelationSink<'_>,
) {
    for edge in &lir.lane_edges {
        let mut local_index = 0_u32;
        for successor in &lir.lane_edge_successors[edge.successors.as_usize_range()] {
            if is_internal_edge(lir, edge.ordinal.raw()) || is_internal_edge(lir, successor.raw()) {
                continue;
            }
            push_relation_tuple(
                relations,
                lir,
                EntityKind::LaneEdge,
                edge.ordinal.raw(),
                1,
                local_index,
                EntityKind::LaneEdge,
                successor.raw(),
            );
            local_index += 1;
        }
    }
    for corridor in &lir.road_corridors {
        for (local_index, element) in lir.corridor_elements[corridor.elements.as_usize_range()]
            .iter()
            .enumerate()
        {
            let (kind, ordinal) = match element {
                crate::lir::LirCorridorElement::RoadSection(ordinal) => {
                    (EntityKind::RoadSection, ordinal.raw())
                }
                crate::lir::LirCorridorElement::FacilityBand(ordinal) => {
                    (EntityKind::FacilityBand, ordinal.raw())
                }
            };
            push_relation_tuple(
                relations,
                lir,
                EntityKind::RoadCorridor,
                corridor.ordinal.raw(),
                2,
                u32::try_from(local_index).expect("compile limits cap relation counts at u32"),
                kind,
                ordinal,
            );
        }
    }
    macro_rules! append_vector_relations {
        ($owners:expr, $range_field:ident, $values:expr, $owner_kind:expr, $role:expr, $subject_kind:expr) => {
            for owner in $owners {
                for (local_index, subject) in $values[owner.$range_field.as_usize_range()]
                    .iter()
                    .enumerate()
                {
                    push_relation_tuple(
                        relations,
                        lir,
                        $owner_kind,
                        owner.ordinal.raw(),
                        $role,
                        u32::try_from(local_index)
                            .expect("compile limits cap relation counts at u32"),
                        $subject_kind,
                        subject.raw(),
                    );
                }
            }
        };
    }
    append_vector_relations!(
        &lir.road_sections,
        lanes,
        lir.road_section_lanes,
        EntityKind::RoadSection,
        3,
        EntityKind::AuthoringLane
    );
    append_vector_relations!(
        &lir.authoring_lanes,
        edge_chain,
        lir.authoring_lane_edges,
        EntityKind::AuthoringLane,
        4,
        EntityKind::LaneEdge
    );
    append_vector_relations!(
        &lir.lane_groups,
        members,
        lir.lane_group_members,
        EntityKind::LaneGroup,
        5,
        EntityKind::AuthoringLane
    );
    append_vector_relations!(
        &lir.junctions,
        movements,
        lir.junction_movements,
        EntityKind::Junction,
        6,
        EntityKind::Movement
    );
    append_vector_relations!(
        &lir.movements,
        maneuver_paths,
        lir.movement_maneuver_paths,
        EntityKind::Movement,
        7,
        EntityKind::ManeuverPath
    );
    append_vector_relations!(
        &lir.maneuver_paths,
        edges,
        lir.maneuver_path_edges,
        EntityKind::ManeuverPath,
        8,
        EntityKind::LaneEdge
    );
    reset_junction_next(&mut junction_next);
    for relation in &lir.junction_internal_edges {
        let local_index = next_junction_local(&mut junction_next, relation.junction.index());
        push_relation_tuple(
            relations,
            lir,
            EntityKind::Junction,
            relation.junction.raw(),
            9,
            local_index,
            EntityKind::LaneEdge,
            relation.edge.raw(),
        );
    }
    append_vector_relations!(
        &lir.maneuver_paths,
        maneuver_gates,
        lir.maneuver_path_gates,
        EntityKind::ManeuverPath,
        10,
        EntityKind::ManeuverGate
    );
    append_vector_relations!(
        &lir.maneuver_paths,
        waiting_zones,
        lir.maneuver_path_waiting_zones,
        EntityKind::ManeuverPath,
        11,
        EntityKind::WaitingZone
    );
    append_vector_relations!(
        &lir.stop_lines,
        maneuver_gates,
        lir.stop_line_maneuver_gates,
        EntityKind::StopLine,
        12,
        EntityKind::ManeuverGate
    );
    for facility in &lir.parking_facilities {
        for (role, anchors) in [
            (
                13,
                &lir.parking_facility_virtual_entries[facility.virtual_entries.as_usize_range()],
            ),
            (
                14,
                &lir.parking_facility_virtual_exits[facility.virtual_exits.as_usize_range()],
            ),
        ] {
            for (local_index, anchor) in anchors.iter().enumerate() {
                push_relation_tuple(
                    relations,
                    lir,
                    EntityKind::ParkingFacility,
                    facility.ordinal.raw(),
                    role,
                    u32::try_from(local_index).expect("compile limits cap relation counts at u32"),
                    EntityKind::LaneEdge,
                    anchor.lane_edge.raw(),
                );
            }
        }
    }
    reset_junction_next(&mut junction_next);
    for zone in &lir.conflict_zones {
        let local_index = next_junction_local(&mut junction_next, zone.junction.index());
        push_relation_tuple(
            relations,
            lir,
            EntityKind::Junction,
            zone.junction.raw(),
            15,
            local_index,
            EntityKind::ConflictZone,
            zone.ordinal.raw(),
        );
    }
    reset_junction_next(&mut junction_next);
    for stream in &lir.participant_streams {
        let local_index = next_junction_local(&mut junction_next, stream.junction.index());
        push_relation_tuple(
            relations,
            lir,
            EntityKind::Junction,
            stream.junction.raw(),
            16,
            local_index,
            EntityKind::ParticipantStream,
            stream.ordinal.raw(),
        );
        push_relation_tuple(
            relations,
            lir,
            EntityKind::ParticipantStream,
            stream.ordinal.raw(),
            30,
            0,
            EntityKind::ManeuverPath,
            stream.maneuver_path.raw(),
        );
        for (local_index, passage) in lir.conflict_passages[stream.passages.as_usize_range()]
            .iter()
            .enumerate()
        {
            push_relation_tuple(
                relations,
                lir,
                EntityKind::ParticipantStream,
                stream.ordinal.raw(),
                31,
                u32::try_from(local_index).expect("compile limits cap relation counts at u32"),
                EntityKind::ConflictZone,
                passage.conflict_zone.raw(),
            );
        }
    }
    append_vector_relations!(
        &lir.signal_controllers,
        signal_groups,
        lir.signal_controller_groups,
        EntityKind::SignalController,
        17,
        EntityKind::SignalGroup
    );
    append_vector_relations!(
        &lir.signal_controllers,
        phases,
        lir.signal_controller_phases,
        EntityKind::SignalController,
        18,
        EntityKind::SignalPhase
    );
    for gate in &lir.maneuver_gates {
        if let crate::lir::LirSignalControl::Group(group) = gate.signal_control {
            push_relation_tuple(
                relations,
                lir,
                EntityKind::ManeuverGate,
                gate.ordinal.raw(),
                20,
                0,
                EntityKind::SignalGroup,
                group.raw(),
            );
        }
    }
    for space in &lir.parking_spaces {
        if let Some(area) = space.parking_facility {
            push_relation_tuple(
                relations,
                lir,
                EntityKind::ParkingSpace,
                space.ordinal.raw(),
                21,
                0,
                EntityKind::ParkingFacility,
                area.raw(),
            );
        }
        for (role, edge) in [(22, space.entry.lane_edge), (23, space.exit.lane_edge)] {
            push_relation_tuple(
                relations,
                lir,
                EntityKind::ParkingSpace,
                space.ordinal.raw(),
                role,
                0,
                EntityKind::LaneEdge,
                edge.raw(),
            );
        }
    }
    for class in &lir.participant_classes {
        if let Some(parent) = class.parent {
            push_relation_tuple(
                relations,
                lir,
                EntityKind::ParticipantClass,
                class.ordinal.raw(),
                24,
                0,
                EntityKind::ParticipantClass,
                parent.raw(),
            );
        }
    }
    for rule in &lir.access_rules {
        let (target_kind, target_ordinal) = match rule.target {
            crate::lir::LirAccessTarget::LaneEdge(target) => (EntityKind::LaneEdge, target.raw()),
            crate::lir::LirAccessTarget::LaneGroup(target) => (EntityKind::LaneGroup, target.raw()),
            crate::lir::LirAccessTarget::RoadSection(target) => {
                (EntityKind::RoadSection, target.raw())
            }
            crate::lir::LirAccessTarget::ManeuverPath(target) => {
                (EntityKind::ManeuverPath, target.raw())
            }
        };
        push_relation_tuple(
            relations,
            lir,
            EntityKind::AccessRule,
            rule.ordinal.raw(),
            25,
            0,
            target_kind,
            target_ordinal,
        );
        for (index, class) in lir.access_rule_participant_classes
            [rule.participant_classes.as_usize_range()]
        .iter()
        .enumerate()
        {
            push_relation_tuple(
                relations,
                lir,
                EntityKind::AccessRule,
                rule.ordinal.raw(),
                26,
                u32::try_from(index).expect("compile limits cap relation counts at u32"),
                EntityKind::ParticipantClass,
                class.raw(),
            );
        }
    }
    for profile in &lir.vehicle_profiles {
        push_relation_tuple(
            relations,
            lir,
            EntityKind::VehicleProfile,
            profile.ordinal.raw(),
            27,
            0,
            EntityKind::ParticipantClass,
            profile.participant_class.raw(),
        );
    }
}
