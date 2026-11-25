//! 规范路网规模的统计定义与编译端 LIR 投影。
//!
//! 规范路网容量各维度的计数定义见编译器基础设计第 5.3 节：编译端按最终 LIR、消费端按最终
//! LFCA 字节各自独立统计，两侧对同一规范路网的结果逐维相等。本模块持有两侧共用的按角色
//! 计数与编译端的 LIR 投影；LFCA 投影在可移植发射器内。两侧投影都只计数、不分配，不为此
//! 建立和排序完整的关系元组。

use laneflow_static_contract::{
    CanonicalNetworkUsage, CanonicalRelationCapacity, SourceRelationRole,
};

/// 按 `sourceRelationRole` 登记的规范关联出现记录数。
///
/// 只统计计入 `CanonicalRelationCount` 的角色；路权策略局部成员（role 33–36）不计入，
/// 作为独立类别的规范统计进入资源计划，不在此统计。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RelationRoleCounts([u64; SourceRelationRole::ALL.len()]);

impl Default for RelationRoleCounts {
    fn default() -> Self {
        Self([0; SourceRelationRole::ALL.len()])
    }
}

impl RelationRoleCounts {
    /// 为某角色累加出现记录数；饱和累加使超大输入仍按容量拒绝而非回绕。
    pub(crate) fn add(&mut self, role: SourceRelationRole, count: u64) {
        let slot = &mut self.0[usize::from(role.code() - 1)];
        *slot = slot.saturating_add(count);
    }

    /// 某角色的出现记录数。
    pub(crate) fn get(&self, role: SourceRelationRole) -> u64 {
        self.0[usize::from(role.code() - 1)]
    }

    /// 按角色登记的容量归属求 `CanonicalRelationCount`。
    pub(crate) fn canonical_relation_count(&self) -> u64 {
        SourceRelationRole::ALL
            .into_iter()
            .filter(|role| {
                matches!(
                    role.canonical_relation_capacity(),
                    CanonicalRelationCapacity::Counted
                )
            })
            .fold(0_u64, |total, role| total.saturating_add(self.get(role)))
    }
}

/// 一份规范路网的规范统计：规范路网规模各维度，以及关系维度按角色的分解。
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct CanonicalStatistics {
    pub(crate) stable_entities: u64,
    pub(crate) maneuver_gates: u64,
    pub(crate) waiting_zones: u64,
    pub(crate) relation_roles: RelationRoleCounts,
    pub(crate) geometry_points: u64,
}

impl CanonicalStatistics {
    /// 规范路网规模。
    pub(crate) fn usage(&self) -> CanonicalNetworkUsage {
        CanonicalNetworkUsage::new(
            self.stable_entities,
            self.maneuver_gates,
            self.waiting_zones,
            self.relation_roles.canonical_relation_count(),
            self.geometry_points,
        )
    }
}

fn count(len: usize) -> u64 {
    u64::try_from(len).unwrap_or(u64::MAX)
}

/// 交通边是否为路口内部边；路口内部边表按边序号排序。
fn is_internal_edge(lir: &crate::lir::LirUnit, edge: u32) -> bool {
    lir.junction_internal_edges
        .binary_search_by_key(&edge, |entry| entry.edge.raw())
        .is_ok()
}

/// 某个计入容量的角色在最终 LIR 中的出现记录数；不计入的角色返回 `None`。
///
/// 每个角色按可移植规范制品第 4.3 节登记的唯一投影计数，与 LFCA 一侧逐角色对应。
fn lir_role_count(lir: &crate::lir::LirUnit, role: SourceRelationRole) -> Option<u64> {
    macro_rules! ranges {
        ($owners:expr, $field:ident) => {
            $owners
                .iter()
                .map(|owner| count(owner.$field.as_usize_range().len()))
                .fold(0_u64, u64::saturating_add)
        };
    }
    let value = match role {
        SourceRelationRole::LaneEdgeSuccessor => lir
            .lane_edges
            .iter()
            .map(|edge| {
                if is_internal_edge(lir, edge.ordinal.raw()) {
                    return 0;
                }
                count(
                    lir.lane_edge_successors[edge.successors.as_usize_range()]
                        .iter()
                        .filter(|successor| !is_internal_edge(lir, successor.raw()))
                        .count(),
                )
            })
            .fold(0_u64, u64::saturating_add),
        SourceRelationRole::RoadCorridorElement => ranges!(lir.road_corridors, elements),
        SourceRelationRole::RoadSectionLane => ranges!(lir.road_sections, lanes),
        SourceRelationRole::AuthoringLaneEdge => ranges!(lir.authoring_lanes, edge_chain),
        SourceRelationRole::LaneGroupMember => ranges!(lir.lane_groups, members),
        SourceRelationRole::JunctionMovement => ranges!(lir.junctions, movements),
        SourceRelationRole::MovementManeuverPath => ranges!(lir.movements, maneuver_paths),
        SourceRelationRole::ManeuverPathEdge => ranges!(lir.maneuver_paths, edges),
        SourceRelationRole::JunctionInternalEdge => count(lir.junction_internal_edges.len()),
        SourceRelationRole::ManeuverPathGate => ranges!(lir.maneuver_paths, maneuver_gates),
        SourceRelationRole::ManeuverPathWaitingZone => ranges!(lir.maneuver_paths, waiting_zones),
        SourceRelationRole::StopLineManeuverGate => ranges!(lir.stop_lines, maneuver_gates),
        SourceRelationRole::ParkingFacilityVirtualEntry => {
            ranges!(lir.parking_facilities, virtual_entries)
        }
        SourceRelationRole::ParkingFacilityVirtualExit => {
            ranges!(lir.parking_facilities, virtual_exits)
        }
        SourceRelationRole::JunctionConflictZone => count(lir.conflict_zones.len()),
        SourceRelationRole::JunctionParticipantStream => count(lir.participant_streams.len()),
        SourceRelationRole::SignalControllerGroup => {
            ranges!(lir.signal_controllers, signal_groups)
        }
        SourceRelationRole::SignalControllerPhase => ranges!(lir.signal_controllers, phases),
        SourceRelationRole::SignalPhaseState => ranges!(lir.signal_phases, states),
        SourceRelationRole::ManeuverGateSignalGroup => count(
            lir.maneuver_gates
                .iter()
                .filter(|gate| {
                    matches!(gate.signal_control, crate::lir::LirSignalControl::Group(_))
                })
                .count(),
        ),
        SourceRelationRole::ParkingSpaceFacility => count(
            lir.parking_spaces
                .iter()
                .filter(|space| space.parking_facility.is_some())
                .count(),
        ),
        SourceRelationRole::ParkingSpaceEntry | SourceRelationRole::ParkingSpaceExit => {
            count(lir.parking_spaces.len())
        }
        SourceRelationRole::ParticipantClassExtends => count(
            lir.participant_classes
                .iter()
                .filter(|class| class.parent.is_some())
                .count(),
        ),
        SourceRelationRole::AccessRuleTarget => count(lir.access_rules.len()),
        SourceRelationRole::AccessRuleParticipantClass => {
            ranges!(lir.access_rules, participant_classes)
        }
        SourceRelationRole::VehicleProfileParticipantClass => count(lir.vehicle_profiles.len()),
        SourceRelationRole::CanonicalFrameLaneEdgeGeometry => count(lir.lane_edge_geometries.len()),
        SourceRelationRole::CanonicalFrameFacilityBandGeometry => {
            count(lir.facility_band_geometries.len())
        }
        SourceRelationRole::ParticipantStreamManeuverPath => count(lir.participant_streams.len()),
        SourceRelationRole::ParticipantStreamConflictPassage => {
            ranges!(lir.participant_streams, passages)
        }
        SourceRelationRole::CanonicalFrameConflictZoneRegion => {
            count(lir.conflict_zone_regions.len())
        }
        SourceRelationRole::PolicyEvidence
        | SourceRelationRole::PolicyGapProfile
        | SourceRelationRole::PolicyStreamRule
        | SourceRelationRole::PolicyGateRule
        | SourceRelationRole::FacilityBandBounds
        | SourceRelationRole::BoundaryExtent
        | SourceRelationRole::LaneFootprint
        | SourceRelationRole::SurfaceSpan
        | SourceRelationRole::BoundaryRule
        | SourceRelationRole::BoundaryObstacle => return None,
    };
    Some(value)
}

/// 最终 LIR 的规范统计。只读冻结 LIR，不分配。
pub(crate) fn lir_statistics(lir: &crate::lir::LirUnit) -> CanonicalStatistics {
    let mut stable_entities = count(lir.policies.len());
    macro_rules! entities {
        ($kind:expr, $records:expr) => {
            stable_entities = stable_entities.saturating_add(count($records.len()));
        };
    }
    crate::lir::for_each_lir_entity_table!(lir, entities);
    let mut relation_roles = RelationRoleCounts::default();
    for role in SourceRelationRole::ALL {
        if let Some(value) = lir_role_count(lir, role) {
            relation_roles.add(role, value);
        }
    }
    CanonicalStatistics {
        stable_entities,
        maneuver_gates: count(lir.maneuver_gates.len()),
        waiting_zones: count(lir.waiting_zones.len()),
        relation_roles,
        geometry_points: count(lir.canonical_points.len())
            .saturating_add(count(lir.conflict_region_points.len())),
    }
}
