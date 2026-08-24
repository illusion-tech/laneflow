//! LIR 冻结计划：记录计数、容量与限额预检。

use laneflow_static_contract::{
    AuthoringLaneOrdinal, LaneEdgeOrdinal, ManeuverGateOrdinal, ManeuverPathOrdinal,
    MovementOrdinal, ParkingSpaceOrdinal, ParticipantClassOrdinal, SignalGroupOrdinal,
    SignalPhaseOrdinal, WaitingZoneOrdinal,
};

use crate::arena::ArenaKey;
use crate::diagnostic::DiagnosticCollector;
use crate::mir::{
    MirLaneEdgeConnection, MirLaneEdgeKey, MirSignalControllerGroup, MirSignalPhaseState, MirUnit,
};
use crate::{CompilationUnit, CompileLimitDimension, Diagnostic, DiagnosticBundle, SourceLocation};

use super::{
    LIR_ACCESS_RULE_LOGICAL_BYTES, LIR_BAND_LOGICAL_BYTES, LIR_CANONICAL_FRAME_LOGICAL_BYTES,
    LIR_CANONICAL_POINT_LOGICAL_BYTES, LIR_CORRIDOR_ELEMENT_LOGICAL_BYTES,
    LIR_CORRIDOR_LOGICAL_BYTES, LIR_FACILITY_BAND_GEOMETRY_LOGICAL_BYTES,
    LIR_GATE_OCCURRENCE_LOGICAL_BYTES, LIR_GEOMETRY_PROFILE_LOGICAL_BYTES, LIR_GROUP_LOGICAL_BYTES,
    LIR_IDENTITY_FIELD_LOGICAL_BYTES, LIR_JUNCTION_INTERNAL_EDGE_LOGICAL_BYTES,
    LIR_JUNCTION_LOGICAL_BYTES, LIR_LANE_EDGE_LOGICAL_BYTES, LIR_LANE_LOGICAL_BYTES,
    LIR_MANEUVER_GATE_LOGICAL_BYTES, LIR_MANEUVER_OCCURRENCE_LOGICAL_BYTES,
    LIR_MANEUVER_PATH_LOGICAL_BYTES, LIR_MOVEMENT_LOGICAL_BYTES, LIR_PARKING_AREA_LOGICAL_BYTES,
    LIR_PARKING_SPACE_LOGICAL_BYTES, LIR_PARTICIPANT_CLASS_LOGICAL_BYTES,
    LIR_ROUTE_OCCURRENCE_REF_LOGICAL_BYTES, LIR_SECTION_LOGICAL_BYTES, LIR_SEMANTIC_DIGEST_BYTES,
    LIR_SIGNAL_CONTROLLER_LOGICAL_BYTES, LIR_SIGNAL_GROUP_LOGICAL_BYTES,
    LIR_SIGNAL_PHASE_LOGICAL_BYTES, LIR_SIGNAL_PHASE_STATE_LOGICAL_BYTES,
    LIR_SPATIAL_GEOMETRY_LOGICAL_BYTES, LIR_SPATIAL_SEGMENT_LOGICAL_BYTES,
    LIR_STATIC_ROUTE_LOGICAL_BYTES, LIR_STOP_LINE_LOGICAL_BYTES, LIR_SUCCESSOR_LOGICAL_BYTES,
    LIR_TYPED_ORDINAL_LOGICAL_BYTES, LIR_VEHICLE_PROFILE_LOGICAL_BYTES,
    LIR_WAITING_OCCURRENCE_LOGICAL_BYTES, LIR_WAITING_ZONE_LOGICAL_BYTES, LirAccessRule,
    LirAuthoringLane, LirCanonicalFrame, LirCanonicalPoint3F32, LirCorridorElement,
    LirFacilityBand, LirFacilityBandGeometry, LirGateOccurrence, LirIdentityField, LirJunction,
    LirJunctionInternalEdge, LirLaneEdge, LirLaneEdgeGeometry, LirLaneGroup, LirManeuverGate,
    LirManeuverOccurrence, LirManeuverPath, LirMovement, LirParkingArea, LirParkingSpace,
    LirParticipantClass, LirRoadCorridor, LirRoadSection, LirRouteOccurrenceRef,
    LirSignalController, LirSignalGroup, LirSignalPhase, LirSignalPhaseState, LirSpatialSegment,
    LirStaticRoute, LirStaticRouteTransition, LirStopLine, LirVehicleProfile, LirWaitingZone,
    LirWaitingZoneOccurrence, identity_field_byte_count, requested_bytes,
};

/// 一次冻结前形成的 LIR 计数、容量与限额观测值。
///
/// [`LirFreezePlan::analyze`] 在任何与记录数成正比的分配前一次统计；算术逐点复制
/// 自拆分前 `freeze_lir` 的资源段。#374 已记录的估算/实际偏差与 `lir_record_count`
/// 中重复累加项原样保留。规范排列与领域冻结的表容量从本计划派生，不再回读 MIR 长度。
pub(crate) struct LirFreezePlan {
    pub(crate) lane_edge_count: u64,
    pub(crate) successor_count: u64,
    pub(crate) identity_field_count: u64,
    pub(crate) identity_field_byte_count: u64,
    pub(crate) cross_section: LirCrossSectionCounts,
    pub(crate) junction: LirJunctionCounts,
    pub(crate) control: LirControlCounts,
    pub(crate) signal: LirSignalCounts,
    pub(crate) parking: LirParkingCounts,
    pub(crate) spatial: LirSpatialCounts,
    pub(crate) access: LirAccessCounts,
    pub(crate) route: LirRouteCounts,
    pub(crate) reverse_occurrence_count: u64,
    pub(crate) lir_record_count: u64,
    pub(crate) stage_scratch_bytes: u64,
    pub(crate) output_bytes: u64,
    pub(crate) output_owned_bytes: u64,
    pub(crate) controlled_live_bytes: u64,
}

pub(crate) struct LirCrossSectionCounts {
    pub(crate) road_corridors: u64,
    pub(crate) corridor_elements: u64,
    pub(crate) road_sections: u64,
    pub(crate) section_lanes: u64,
    pub(crate) authoring_lanes: u64,
    pub(crate) authoring_lane_edges: u64,
    pub(crate) lane_groups: u64,
    pub(crate) lane_group_members: u64,
    pub(crate) facility_bands: u64,
}

pub(crate) struct LirJunctionCounts {
    pub(crate) junctions: u64,
    pub(crate) junction_movements: u64,
    pub(crate) movements: u64,
    pub(crate) movement_paths: u64,
    pub(crate) maneuver_paths: u64,
    pub(crate) maneuver_path_edges: u64,
    pub(crate) maneuver_path_gates: u64,
    pub(crate) maneuver_path_waiting_zones: u64,
    pub(crate) junction_internal_edges: u64,
}

pub(crate) struct LirControlCounts {
    pub(crate) stop_lines: u64,
    pub(crate) stop_line_maneuver_gates: u64,
    pub(crate) maneuver_gates: u64,
    pub(crate) waiting_zones: u64,
}

pub(crate) struct LirSignalCounts {
    pub(crate) groups: u64,
    pub(crate) controllers: u64,
    pub(crate) controller_groups: u64,
    pub(crate) phases: u64,
    pub(crate) phase_states: u64,
    pub(crate) controlled_gates: u64,
}

pub(crate) struct LirParkingCounts {
    pub(crate) areas: u64,
    pub(crate) spaces: u64,
    pub(crate) memberships: u64,
}

pub(crate) struct LirSpatialCounts {
    pub(crate) canonical_frames: u64,
    pub(crate) lane_edge_geometries: u64,
    pub(crate) facility_band_geometries: u64,
    pub(crate) canonical_points: u64,
    pub(crate) spatial_segments: u64,
}

pub(crate) struct LirAccessCounts {
    pub(crate) participant_classes: u64,
    pub(crate) vehicle_profiles: u64,
    pub(crate) access_rules: u64,
    pub(crate) rule_class_references: u64,
}

pub(crate) struct LirRouteCounts {
    pub(crate) static_routes: u64,
    pub(crate) route_edges: u64,
    pub(crate) route_transitions: u64,
    pub(crate) maneuver_occurrences: u64,
    pub(crate) gate_occurrences: u64,
    pub(crate) waiting_occurrences: u64,
}

impl LirFreezePlan {
    pub(crate) fn capacity(
        count: u64,
        limits: &crate::CompileLimits,
        primary_span: Option<SourceLocation>,
    ) -> Result<usize, DiagnosticBundle> {
        usize::try_from(count).map_err(|_| super::ordinal_overflow(limits, primary_span))
    }

    /// 统计全部计数并聚合预算；不分配任何与记录数成正比的集合。
    pub(crate) fn analyze(unit: &CompilationUnit, mir: &MirUnit) -> Self {
        let lane_edge_count = mir_len(mir.lane_edges.len());
        let successor_count = mir_len(mir.lane_edge_connections.len());
        let cross_section = LirCrossSectionCounts {
            road_corridors: mir_len(mir.road_corridors.len()),
            corridor_elements: mir_len(mir.corridor_elements.len()),
            road_sections: mir_len(mir.road_sections.len()),
            section_lanes: mir_len(mir.authoring_lanes.len()),
            authoring_lanes: mir_len(mir.authoring_lanes.len()),
            authoring_lane_edges: mir_len(mir.authoring_lane_edges.len()),
            lane_groups: mir_len(mir.lane_groups.len()),
            lane_group_members: mir_len(mir.lane_group_members.len()),
            facility_bands: mir_len(mir.facility_bands.len()),
        };
        let junction = LirJunctionCounts {
            junctions: mir_len(mir.junctions.len()),
            junction_movements: mir_len(mir.junction_movements.len()),
            movements: mir_len(mir.movements.len()),
            movement_paths: mir_len(mir.movement_maneuver_paths.len()),
            maneuver_paths: mir_len(mir.maneuver_paths.len()),
            maneuver_path_edges: mir_len(mir.maneuver_path_edges.len()),
            maneuver_path_gates: mir_len(mir.maneuver_path_gates.len()),
            maneuver_path_waiting_zones: mir_len(mir.maneuver_path_waiting_zones.len()),
            junction_internal_edges: mir_len(mir.junction_internal_edges.len()),
        };
        let control = LirControlCounts {
            stop_lines: mir_len(mir.stop_lines.len()),
            stop_line_maneuver_gates: mir_len(mir.stop_line_maneuver_gates.len()),
            maneuver_gates: mir_len(mir.maneuver_gates.len()),
            waiting_zones: mir_len(mir.waiting_zones.len()),
        };
        let signal = LirSignalCounts {
            groups: mir_len(mir.signal_groups.len()),
            controllers: mir_len(mir.signal_controllers.len()),
            controller_groups: mir_len(mir.signal_controller_groups.len()),
            phases: mir_len(mir.signal_phases.len()),
            phase_states: mir_len(mir.signal_phase_states.len()),
            controlled_gates: mir_len(mir.signal_group_maneuver_gates.len()),
        };
        let parking = LirParkingCounts {
            areas: mir_len(mir.parking_areas.len()),
            spaces: mir_len(mir.parking_spaces.len()),
            memberships: mir_len(mir.parking_area_spaces.len()),
        };
        let spatial = LirSpatialCounts {
            canonical_frames: mir_len(mir.canonical_frames.len()),
            lane_edge_geometries: mir_len(mir.lane_edge_geometries.len()),
            facility_band_geometries: mir_len(mir.facility_band_geometries.len()),
            canonical_points: mir_len(mir.canonical_points.len()),
            spatial_segments: mir_len(mir.spatial_segments.len()),
        };
        let access = LirAccessCounts {
            participant_classes: mir_len(mir.participant_classes.len()),
            vehicle_profiles: mir_len(mir.vehicle_profiles.len()),
            access_rules: mir_len(mir.access_rules.len()),
            rule_class_references: mir_len(mir.access_rule_participant_classes.len()),
        };
        let route = LirRouteCounts {
            static_routes: mir_len(mir.static_routes.len()),
            route_edges: mir_len(mir.static_route_edges.len()),
            route_transitions: mir_len(mir.static_route_transitions.len()),
            maneuver_occurrences: mir_len(mir.maneuver_occurrences.len()),
            gate_occurrences: mir_len(mir.gate_occurrences.len()),
            waiting_occurrences: mir_len(mir.waiting_zone_occurrences.len()),
        };
        let reverse_occurrence_count = route
            .route_edges
            .saturating_add(route.maneuver_occurrences)
            .saturating_add(route.gate_occurrences)
            .saturating_add(route.waiting_occurrences);
        // Identity 字段出现项有独立资源维度；LIR record 指标计实体行和关系出现行，与 MIR
        // 当前已支持实体与关系的计数对象保持一致。
        // `authoring_lanes` 按拆分前 `section_lane_count` + `lane_count` 加两次；
        // `waiting_zones` / `maneuver_gates` 在信号段前与数组末尾重复累加。#374 重建
        // 精确账本前原样保留。
        let lir_record_count = [
            lane_edge_count,
            successor_count,
            cross_section.road_corridors,
            cross_section.corridor_elements,
            cross_section.road_sections,
            cross_section.section_lanes,
            cross_section.authoring_lanes,
            cross_section.authoring_lane_edges,
            cross_section.lane_groups,
            cross_section.lane_group_members,
            cross_section.facility_bands,
            junction.junctions,
            junction.junction_movements,
            junction.movements,
            junction.movement_paths,
            junction.maneuver_paths,
            junction.maneuver_path_edges,
            junction.junction_internal_edges,
            control.stop_lines,
            control.maneuver_gates,
            route.static_routes,
            route.route_edges,
            route.route_transitions,
            route.maneuver_occurrences,
            route.gate_occurrences,
            route.waiting_occurrences,
            reverse_occurrence_count,
            control.waiting_zones,
            control.maneuver_gates,
            signal.groups,
            signal.controllers,
            signal.controller_groups,
            signal.phases,
            signal.phases,
            signal.phase_states,
            signal.controlled_gates,
            parking.areas,
            parking.spaces,
            parking.memberships,
            access.participant_classes,
            access.vehicle_profiles,
            spatial.canonical_frames,
            spatial.lane_edge_geometries,
            spatial.facility_band_geometries,
            spatial.canonical_points,
            spatial.spatial_segments,
            access.access_rules,
            access.rule_class_references,
            control.waiting_zones,
            control.maneuver_gates,
        ]
        .into_iter()
        .fold(0_u64, u64::saturating_add);
        let identity_field_count = lane_edge_count
            .saturating_mul(2)
            .saturating_add(cross_section.road_corridors.saturating_mul(2))
            .saturating_add(
                cross_section
                    .road_sections
                    .saturating_add(cross_section.authoring_lanes)
                    .saturating_add(cross_section.lane_groups)
                    .saturating_add(cross_section.facility_bands)
                    .saturating_mul(3),
            )
            .saturating_add(junction.junctions.saturating_mul(2))
            .saturating_add(junction.movements.saturating_mul(5))
            .saturating_add(junction.maneuver_paths.saturating_mul(5))
            .saturating_add(control.stop_lines.saturating_mul(2))
            .saturating_add(control.maneuver_gates.saturating_mul(3))
            .saturating_add(control.waiting_zones.saturating_mul(3))
            .saturating_add(signal.groups.saturating_mul(2))
            .saturating_add(signal.controllers.saturating_mul(2))
            .saturating_add(signal.phases.saturating_mul(3))
            .saturating_add(parking.areas.saturating_mul(2))
            .saturating_add(parking.spaces.saturating_mul(2))
            .saturating_add(access.participant_classes.saturating_mul(2))
            .saturating_add(access.vehicle_profiles.saturating_mul(2))
            .saturating_add(spatial.canonical_frames.saturating_mul(2))
            .saturating_add(access.access_rules.saturating_mul(2))
            .saturating_add(route.static_routes.saturating_mul(2));
        let identity_field_byte_count = identity_field_byte_count(mir);
        let kind_id_byte_count = mir
            .road_sections
            .iter()
            .map(|section| section.kind_id.len())
            .chain(mir.facility_bands.iter().map(|band| band.kind_id.len()))
            .fold(0_u64, |total, len| {
                total.saturating_add(u64::try_from(len).unwrap_or(u64::MAX))
            });
        let movement_approach_key_byte_count =
            mir.movements.iter().fold(0_u64, |total, movement| {
                total
                    .saturating_add(
                        u64::try_from(movement.directed_entry_approach_key.len())
                            .unwrap_or(u64::MAX),
                    )
                    .saturating_add(
                        u64::try_from(movement.directed_exit_approach_key.len())
                            .unwrap_or(u64::MAX),
                    )
            });
        let access_regulation_byte_count = mir.access_rules.iter().fold(0_u64, |total, rule| {
            let Some(regulation) = &rule.regulation else {
                return total;
            };
            total
                .saturating_add(u64::try_from(regulation.jurisdiction.len()).unwrap_or(u64::MAX))
                .saturating_add(u64::try_from(regulation.version.len()).unwrap_or(u64::MAX))
                .saturating_add(
                    regulation
                        .source
                        .as_ref()
                        .map_or(0, |source| u64::try_from(source.len()).unwrap_or(u64::MAX)),
                )
        });

        // 排序序列与 MIR→LIR 映射各只保存一个有类型 `u32` 值；完整身份前像通过借用 MIR
        // 字段作分段比较，避免为排序再复制一份变长编码。
        let stage_scratch_bytes = requested_bytes::<MirLaneEdgeKey>(lane_edge_count)
            .saturating_add(requested_bytes::<LaneEdgeOrdinal>(lane_edge_count))
            .saturating_add(requested_bytes::<u32>(
                cross_section
                    .road_corridors
                    .saturating_add(cross_section.road_sections)
                    .saturating_add(cross_section.authoring_lanes)
                    .saturating_add(cross_section.lane_groups)
                    .saturating_add(cross_section.facility_bands)
                    .saturating_add(junction.junctions)
                    .saturating_add(junction.movements)
                    .saturating_add(junction.maneuver_paths)
                    .saturating_add(control.stop_lines)
                    .saturating_add(control.maneuver_gates)
                    .saturating_add(control.waiting_zones)
                    .saturating_add(signal.groups)
                    .saturating_add(signal.controllers)
                    .saturating_add(signal.phases)
                    .saturating_add(parking.areas)
                    .saturating_add(parking.spaces)
                    .saturating_add(access.participant_classes)
                    .saturating_add(access.vehicle_profiles)
                    .saturating_add(spatial.canonical_frames)
                    .saturating_add(access.access_rules)
                    .saturating_add(route.static_routes)
                    .saturating_mul(2),
            ))
            .saturating_add(requested_bytes::<u32>(junction.junction_internal_edges))
            // owner-local 关系没有稳定身份；保留其 MIR 行地址排列，使来源与 LIR 语义行
            // 共享同一次规范重排。
            .saturating_add(requested_bytes::<ArenaKey<MirSignalControllerGroup>>(
                signal.controller_groups,
            ))
            .saturating_add(requested_bytes::<ArenaKey<MirSignalPhaseState>>(
                signal.phase_states,
            ))
            .saturating_add(requested_bytes::<ArenaKey<MirLaneEdgeConnection>>(
                successor_count,
            ))
            .saturating_add(requested_bytes::<Option<usize>>(lane_edge_count))
            .saturating_add(requested_bytes::<Option<usize>>(
                cross_section.facility_bands,
            ))
            // 四类反向索引先以 `(targetOrdinal, occurrence)` 排序，再复制进最终连续表；
            // 最终表已计入 output-owned bytes，这里只补临时排序对。
            .saturating_add(requested_bytes::<(u32, LirRouteOccurrenceRef)>(
                reverse_occurrence_count,
            ));
        // OutputBytes 使用设计冻结的目标布局中立字段宽度，不能把 Rust struct padding 或
        // 当前平台对齐冒充规范输出量；受控存续内存则按真实堆容量请求单独计算。
        let output_bytes = lane_edge_count
            .saturating_mul(LIR_LANE_EDGE_LOGICAL_BYTES)
            .saturating_add(successor_count.saturating_mul(LIR_SUCCESSOR_LOGICAL_BYTES))
            .saturating_add(identity_field_count.saturating_mul(LIR_IDENTITY_FIELD_LOGICAL_BYTES))
            .saturating_add(identity_field_byte_count)
            .saturating_add(
                cross_section
                    .road_corridors
                    .saturating_mul(LIR_CORRIDOR_LOGICAL_BYTES),
            )
            .saturating_add(
                cross_section
                    .corridor_elements
                    .saturating_mul(LIR_CORRIDOR_ELEMENT_LOGICAL_BYTES),
            )
            .saturating_add(
                cross_section
                    .road_sections
                    .saturating_mul(LIR_SECTION_LOGICAL_BYTES),
            )
            .saturating_add(
                cross_section
                    .section_lanes
                    .saturating_mul(LIR_TYPED_ORDINAL_LOGICAL_BYTES),
            )
            .saturating_add(
                cross_section
                    .authoring_lanes
                    .saturating_mul(LIR_LANE_LOGICAL_BYTES),
            )
            .saturating_add(
                cross_section
                    .authoring_lane_edges
                    .saturating_mul(LIR_TYPED_ORDINAL_LOGICAL_BYTES),
            )
            .saturating_add(
                cross_section
                    .lane_groups
                    .saturating_mul(LIR_GROUP_LOGICAL_BYTES),
            )
            .saturating_add(
                cross_section
                    .lane_group_members
                    .saturating_mul(LIR_TYPED_ORDINAL_LOGICAL_BYTES),
            )
            .saturating_add(
                cross_section
                    .facility_bands
                    .saturating_mul(LIR_BAND_LOGICAL_BYTES),
            )
            .saturating_add(
                junction
                    .junctions
                    .saturating_mul(LIR_JUNCTION_LOGICAL_BYTES),
            )
            .saturating_add(
                junction
                    .junction_movements
                    .saturating_mul(LIR_TYPED_ORDINAL_LOGICAL_BYTES),
            )
            .saturating_add(
                junction
                    .movements
                    .saturating_mul(LIR_MOVEMENT_LOGICAL_BYTES),
            )
            .saturating_add(movement_approach_key_byte_count)
            .saturating_add(
                junction
                    .movement_paths
                    .saturating_mul(LIR_TYPED_ORDINAL_LOGICAL_BYTES),
            )
            .saturating_add(
                junction
                    .maneuver_paths
                    .saturating_mul(LIR_MANEUVER_PATH_LOGICAL_BYTES),
            )
            .saturating_add(
                junction
                    .maneuver_path_edges
                    .saturating_mul(LIR_TYPED_ORDINAL_LOGICAL_BYTES),
            )
            .saturating_add(
                junction
                    .junction_internal_edges
                    .saturating_mul(LIR_JUNCTION_INTERNAL_EDGE_LOGICAL_BYTES),
            )
            .saturating_add(
                control
                    .stop_lines
                    .saturating_mul(LIR_STOP_LINE_LOGICAL_BYTES),
            )
            .saturating_add(
                control
                    .maneuver_gates
                    .saturating_mul(LIR_MANEUVER_GATE_LOGICAL_BYTES),
            )
            .saturating_add(
                control
                    .waiting_zones
                    .saturating_mul(LIR_WAITING_ZONE_LOGICAL_BYTES),
            )
            .saturating_add(
                control
                    .maneuver_gates
                    .saturating_mul(LIR_TYPED_ORDINAL_LOGICAL_BYTES),
            )
            .saturating_add(signal.groups.saturating_mul(LIR_SIGNAL_GROUP_LOGICAL_BYTES))
            .saturating_add(
                signal
                    .controllers
                    .saturating_mul(LIR_SIGNAL_CONTROLLER_LOGICAL_BYTES),
            )
            .saturating_add(
                signal
                    .controller_groups
                    .saturating_mul(LIR_TYPED_ORDINAL_LOGICAL_BYTES),
            )
            .saturating_add(
                signal
                    .phases
                    .saturating_mul(LIR_TYPED_ORDINAL_LOGICAL_BYTES),
            )
            .saturating_add(signal.phases.saturating_mul(LIR_SIGNAL_PHASE_LOGICAL_BYTES))
            .saturating_add(
                signal
                    .phase_states
                    .saturating_mul(LIR_SIGNAL_PHASE_STATE_LOGICAL_BYTES),
            )
            .saturating_add(
                signal
                    .controlled_gates
                    .saturating_mul(LIR_TYPED_ORDINAL_LOGICAL_BYTES),
            )
            .saturating_add(parking.areas.saturating_mul(LIR_PARKING_AREA_LOGICAL_BYTES))
            .saturating_add(
                parking
                    .spaces
                    .saturating_mul(LIR_PARKING_SPACE_LOGICAL_BYTES),
            )
            .saturating_add(
                parking
                    .memberships
                    .saturating_mul(LIR_TYPED_ORDINAL_LOGICAL_BYTES),
            )
            .saturating_add(
                access
                    .participant_classes
                    .saturating_mul(LIR_PARTICIPANT_CLASS_LOGICAL_BYTES),
            )
            .saturating_add(
                access
                    .vehicle_profiles
                    .saturating_mul(LIR_VEHICLE_PROFILE_LOGICAL_BYTES),
            )
            .saturating_add(
                spatial
                    .canonical_frames
                    .saturating_mul(LIR_CANONICAL_FRAME_LOGICAL_BYTES),
            )
            .saturating_add(
                spatial
                    .lane_edge_geometries
                    .saturating_mul(LIR_SPATIAL_GEOMETRY_LOGICAL_BYTES),
            )
            .saturating_add(
                spatial
                    .facility_band_geometries
                    .saturating_mul(LIR_FACILITY_BAND_GEOMETRY_LOGICAL_BYTES),
            )
            .saturating_add(
                spatial
                    .canonical_points
                    .saturating_mul(LIR_CANONICAL_POINT_LOGICAL_BYTES),
            )
            .saturating_add(
                spatial
                    .spatial_segments
                    .saturating_mul(LIR_SPATIAL_SEGMENT_LOGICAL_BYTES),
            )
            .saturating_add(
                access
                    .access_rules
                    .saturating_mul(LIR_ACCESS_RULE_LOGICAL_BYTES),
            )
            .saturating_add(
                access
                    .rule_class_references
                    .saturating_mul(LIR_TYPED_ORDINAL_LOGICAL_BYTES),
            )
            .saturating_add(access_regulation_byte_count)
            .saturating_add(
                route
                    .static_routes
                    .saturating_mul(LIR_STATIC_ROUTE_LOGICAL_BYTES),
            )
            .saturating_add(
                route
                    .route_edges
                    .saturating_mul(LIR_TYPED_ORDINAL_LOGICAL_BYTES),
            )
            .saturating_add(
                route
                    .route_transitions
                    .saturating_mul(1 + LIR_TYPED_ORDINAL_LOGICAL_BYTES),
            )
            .saturating_add(
                route
                    .maneuver_occurrences
                    .saturating_mul(LIR_MANEUVER_OCCURRENCE_LOGICAL_BYTES),
            )
            .saturating_add(
                route
                    .gate_occurrences
                    .saturating_mul(LIR_GATE_OCCURRENCE_LOGICAL_BYTES),
            )
            .saturating_add(
                route
                    .waiting_occurrences
                    .saturating_mul(LIR_WAITING_OCCURRENCE_LOGICAL_BYTES),
            )
            .saturating_add(
                reverse_occurrence_count.saturating_mul(LIR_ROUTE_OCCURRENCE_REF_LOGICAL_BYTES),
            )
            .saturating_add(
                control
                    .waiting_zones
                    .saturating_mul(LIR_TYPED_ORDINAL_LOGICAL_BYTES),
            )
            .saturating_add(
                control
                    .maneuver_gates
                    .saturating_mul(LIR_TYPED_ORDINAL_LOGICAL_BYTES),
            )
            .saturating_add(kind_id_byte_count)
            .saturating_add(LIR_SEMANTIC_DIGEST_BYTES)
            .saturating_add(LIR_GEOMETRY_PROFILE_LOGICAL_BYTES);
        let output_owned_bytes = requested_bytes::<LirLaneEdge>(lane_edge_count)
            .saturating_add(requested_bytes::<LaneEdgeOrdinal>(successor_count))
            .saturating_add(requested_bytes::<LirIdentityField>(identity_field_count))
            .saturating_add(identity_field_byte_count)
            .saturating_add(requested_bytes::<LirRoadCorridor>(
                cross_section.road_corridors,
            ))
            .saturating_add(requested_bytes::<LirCorridorElement>(
                cross_section.corridor_elements,
            ))
            .saturating_add(requested_bytes::<LirRoadSection>(
                cross_section.road_sections,
            ))
            .saturating_add(requested_bytes::<AuthoringLaneOrdinal>(
                cross_section.section_lanes,
            ))
            .saturating_add(requested_bytes::<LirAuthoringLane>(
                cross_section.authoring_lanes,
            ))
            .saturating_add(requested_bytes::<LaneEdgeOrdinal>(
                cross_section.authoring_lane_edges,
            ))
            .saturating_add(requested_bytes::<LirLaneGroup>(cross_section.lane_groups))
            .saturating_add(requested_bytes::<AuthoringLaneOrdinal>(
                cross_section.lane_group_members,
            ))
            .saturating_add(requested_bytes::<LirFacilityBand>(
                cross_section.facility_bands,
            ))
            .saturating_add(kind_id_byte_count)
            .saturating_add(requested_bytes::<LirJunction>(junction.junctions))
            .saturating_add(requested_bytes::<MovementOrdinal>(
                junction.junction_movements,
            ))
            .saturating_add(requested_bytes::<LirMovement>(junction.movements))
            .saturating_add(movement_approach_key_byte_count)
            .saturating_add(requested_bytes::<ManeuverPathOrdinal>(
                junction.movement_paths,
            ))
            .saturating_add(requested_bytes::<LirManeuverPath>(junction.maneuver_paths))
            .saturating_add(requested_bytes::<LaneEdgeOrdinal>(
                junction.maneuver_path_edges,
            ))
            .saturating_add(requested_bytes::<LirJunctionInternalEdge>(
                junction.junction_internal_edges,
            ))
            .saturating_add(requested_bytes::<LirStopLine>(control.stop_lines))
            .saturating_add(requested_bytes::<LirManeuverGate>(control.maneuver_gates))
            .saturating_add(requested_bytes::<LirWaitingZone>(control.waiting_zones))
            .saturating_add(requested_bytes::<ManeuverGateOrdinal>(
                control.maneuver_gates,
            ))
            .saturating_add(requested_bytes::<WaitingZoneOrdinal>(control.waiting_zones))
            .saturating_add(requested_bytes::<ManeuverGateOrdinal>(
                control.maneuver_gates,
            ))
            .saturating_add(requested_bytes::<LirSignalGroup>(signal.groups))
            .saturating_add(requested_bytes::<LirSignalController>(signal.controllers))
            .saturating_add(requested_bytes::<SignalGroupOrdinal>(
                signal.controller_groups,
            ))
            .saturating_add(requested_bytes::<SignalPhaseOrdinal>(signal.phases))
            .saturating_add(requested_bytes::<LirSignalPhase>(signal.phases))
            .saturating_add(requested_bytes::<LirSignalPhaseState>(signal.phase_states))
            .saturating_add(requested_bytes::<ManeuverGateOrdinal>(
                signal.controlled_gates,
            ))
            .saturating_add(requested_bytes::<LirParkingArea>(parking.areas))
            .saturating_add(requested_bytes::<LirParkingSpace>(parking.spaces))
            .saturating_add(requested_bytes::<ParkingSpaceOrdinal>(parking.memberships))
            .saturating_add(requested_bytes::<LirParticipantClass>(
                access.participant_classes,
            ))
            .saturating_add(requested_bytes::<LirVehicleProfile>(
                access.vehicle_profiles,
            ))
            .saturating_add(requested_bytes::<LirCanonicalFrame>(
                spatial.canonical_frames,
            ))
            .saturating_add(requested_bytes::<LirLaneEdgeGeometry>(
                spatial.lane_edge_geometries,
            ))
            .saturating_add(requested_bytes::<LirFacilityBandGeometry>(
                spatial.facility_band_geometries,
            ))
            .saturating_add(requested_bytes::<LirCanonicalPoint3F32>(
                spatial.canonical_points,
            ))
            .saturating_add(requested_bytes::<LirSpatialSegment>(
                spatial.spatial_segments,
            ))
            .saturating_add(requested_bytes::<LirAccessRule>(access.access_rules))
            .saturating_add(requested_bytes::<ParticipantClassOrdinal>(
                access.rule_class_references,
            ))
            .saturating_add(access_regulation_byte_count)
            .saturating_add(requested_bytes::<LirStaticRoute>(route.static_routes))
            .saturating_add(requested_bytes::<LaneEdgeOrdinal>(route.route_edges))
            .saturating_add(requested_bytes::<LirStaticRouteTransition>(
                route.route_transitions,
            ))
            .saturating_add(requested_bytes::<LirManeuverOccurrence>(
                route.maneuver_occurrences,
            ))
            .saturating_add(requested_bytes::<LirGateOccurrence>(route.gate_occurrences))
            .saturating_add(requested_bytes::<LirWaitingZoneOccurrence>(
                route.waiting_occurrences,
            ))
            .saturating_add(requested_bytes::<LirRouteOccurrenceRef>(
                reverse_occurrence_count,
            ));
        let controlled_live_bytes = unit
            .controlled_live_bytes
            .saturating_add(mir.controlled_live_bytes)
            .saturating_add(stage_scratch_bytes)
            .saturating_add(output_owned_bytes);
        Self {
            lane_edge_count,
            successor_count,
            identity_field_count,
            identity_field_byte_count,
            cross_section,
            junction,
            control,
            signal,
            parking,
            spatial,
            access,
            route,
            reverse_occurrence_count,
            lir_record_count,
            stage_scratch_bytes,
            output_bytes,
            output_owned_bytes,
            controlled_live_bytes,
        }
    }

    pub(crate) fn check_limits(
        &self,
        unit: &CompilationUnit,
        mir: &MirUnit,
    ) -> Result<(), DiagnosticBundle> {
        let primary_span = mir.modules.first().map(|module| module.source_span.clone());
        let stable_key = mir
            .modules
            .first()
            .map(|module| module.authoring_namespace_id.as_ref().into());
        let mut diagnostics =
            DiagnosticCollector::new(unit.limits.value(CompileLimitDimension::DiagnosticCount));
        for (dimension, observed) in [
            (CompileLimitDimension::LirRecordCount, self.lir_record_count),
            (
                CompileLimitDimension::StageScratchBytes,
                self.stage_scratch_bytes,
            ),
            (CompileLimitDimension::OutputBytes, self.output_bytes),
            (
                CompileLimitDimension::CompilerControlledLiveBytes,
                self.controlled_live_bytes,
            ),
        ] {
            if observed > unit.limits.value(dimension) {
                diagnostics.push(Diagnostic::compile_limit_exceeded_at(
                    dimension,
                    unit.limits.value(dimension),
                    observed,
                    primary_span.clone(),
                    stable_key.clone(),
                ));
            }
        }
        if !diagnostics.is_empty() {
            return Err(diagnostics.finish());
        }
        Ok(())
    }
}

fn mir_len(len: usize) -> u64 {
    u64::try_from(len).unwrap_or(u64::MAX)
}
