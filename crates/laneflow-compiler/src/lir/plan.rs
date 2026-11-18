//! LIR 冻结计划：记录计数、容量与限额预检。

use laneflow_static_contract::{
    AuthoringLaneOrdinal, LaneEdgeOrdinal, ManeuverGateOrdinal, ManeuverPathOrdinal,
    MovementOrdinal, ParkingSpaceOrdinal, ParticipantClassOrdinal, ParticipantStreamOrdinal,
    SignalGroupOrdinal, SignalPhaseOrdinal, WaitingZoneOrdinal,
};

use crate::arena::ArenaKey;
use crate::diagnostic::DiagnosticCollector;
use crate::ledger::{BoundedVec, Plan};
use crate::mir::{
    MirConflictPassage, MirLaneEdgeConnection, MirLaneEdgeKey, MirSignalControllerGroup,
    MirSignalPhaseState, MirUnit,
};
use crate::{CompilationUnit, Diagnostic, DiagnosticBundle, SourceLocation};

use super::{
    LIR_ACCESS_RULE_LOGICAL_BYTES, LIR_BAND_LOGICAL_BYTES, LIR_CANONICAL_FRAME_LOGICAL_BYTES,
    LIR_CANONICAL_POINT_LOGICAL_BYTES, LIR_CONFLICT_PASSAGE_LOGICAL_BYTES,
    LIR_CONFLICT_ZONE_LOGICAL_BYTES, LIR_CONFLICT_ZONE_REGION_LOGICAL_BYTES,
    LIR_CORRIDOR_ELEMENT_LOGICAL_BYTES, LIR_CORRIDOR_LOGICAL_BYTES,
    LIR_FACILITY_BAND_GEOMETRY_LOGICAL_BYTES, LIR_GEOMETRY_PROFILE_LOGICAL_BYTES,
    LIR_GROUP_LOGICAL_BYTES, LIR_IDENTITY_FIELD_LOGICAL_BYTES,
    LIR_JUNCTION_INTERNAL_EDGE_LOGICAL_BYTES, LIR_JUNCTION_LOGICAL_BYTES,
    LIR_LANE_EDGE_LOGICAL_BYTES, LIR_LANE_LOGICAL_BYTES, LIR_MANEUVER_GATE_LOGICAL_BYTES,
    LIR_MANEUVER_PATH_LOGICAL_BYTES, LIR_MOVEMENT_LOGICAL_BYTES,
    LIR_PARKING_FACILITY_LOGICAL_BYTES, LIR_PARKING_SPACE_LOGICAL_BYTES,
    LIR_PARTICIPANT_CLASS_LOGICAL_BYTES, LIR_PARTICIPANT_STREAM_LOGICAL_BYTES,
    LIR_POINT2_LOGICAL_BYTES, LIR_SECTION_LOGICAL_BYTES, LIR_SEMANTIC_DIGEST_BYTES,
    LIR_SIGNAL_CONTROLLER_LOGICAL_BYTES, LIR_SIGNAL_GROUP_LOGICAL_BYTES,
    LIR_SIGNAL_PHASE_LOGICAL_BYTES, LIR_SIGNAL_PHASE_STATE_LOGICAL_BYTES,
    LIR_SPATIAL_GEOMETRY_LOGICAL_BYTES, LIR_SPATIAL_SEGMENT_LOGICAL_BYTES,
    LIR_STOP_LINE_LOGICAL_BYTES, LIR_SUCCESSOR_LOGICAL_BYTES, LIR_TYPED_ORDINAL_LOGICAL_BYTES,
    LIR_VEHICLE_PROFILE_LOGICAL_BYTES, LIR_WAITING_ZONE_LOGICAL_BYTES, LirAccessRule,
    LirAuthoringLane, LirCanonicalFrame, LirCanonicalPoint2F32, LirCanonicalPoint3F32,
    LirConflictPassage, LirConflictZone, LirConflictZoneRegion, LirCorridorElement,
    LirFacilityBand, LirFacilityBandGeometry, LirIdentityField, LirJunction,
    LirJunctionInternalEdge, LirLaneEdge, LirLaneEdgeGeometry, LirLaneGroup, LirManeuverGate,
    LirManeuverPath, LirMovement, LirParkingFacility, LirParkingSpace, LirParticipantClass,
    LirParticipantStream, LirPolicy, LirRoadCorridor, LirRoadSection, LirSignalController,
    LirSignalGroup, LirSignalPhase, LirSignalPhaseState, LirSpatialSegment, LirStopLine,
    LirVehicleProfile, LirWaitingZone, identity_field_byte_count,
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
    pub(crate) conflict: LirConflictCounts,
    pub(crate) parking: LirParkingCounts,
    pub(crate) spatial: LirSpatialCounts,
    pub(crate) access: LirAccessCounts,
    pub(crate) lir_record_count: u64,
    /// 领域冻结内部暂存的授予字节；保留供测试核对。
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) stage_scratch_bytes: u64,
    pub(crate) output_bytes: u64,
    /// LIR 各表与内嵌载荷的授予字节，即 LIR 输出的受控存续。
    pub(crate) output_owned_bytes: u64,
    /// 记录内嵌变长载荷集中取得的授予字节。
    pub(crate) inner_payload_bytes: u64,
    /// 编译遍结束时转为输出存续的授予字节：LIR 输出与跨入源映射冻结的规范排列。
    pub(crate) retained_bytes: u64,
    /// 本遍计划峰值字节：转为输出的存储与暂存之和；上游存续由账本记录。
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) stage_bytes: u64,
    /// 本遍计划峰值。
    pub(crate) peak: Plan,
}

/// 横断面领域各 LIR 表与关系辅表的记录计数。
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

/// 路口领域各 LIR 表与关系辅表的记录计数。
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

/// 控制领域各 LIR 表与关系辅表的记录计数。
pub(crate) struct LirControlCounts {
    pub(crate) stop_lines: u64,
    pub(crate) stop_line_maneuver_gates: u64,
    pub(crate) maneuver_gates: u64,
    pub(crate) waiting_zones: u64,
}

/// 信号领域各 LIR 表与关系辅表的记录计数。
pub(crate) struct LirSignalCounts {
    pub(crate) groups: u64,
    pub(crate) controllers: u64,
    pub(crate) controller_groups: u64,
    pub(crate) phases: u64,
    pub(crate) phase_states: u64,
    pub(crate) controlled_gates: u64,
}

/// 停车领域各 LIR 表与关系辅表的记录计数。
pub(crate) struct LirParkingCounts {
    pub(crate) areas: u64,
    pub(crate) spaces: u64,
    pub(crate) memberships: u64,
    pub(crate) virtual_entries: u64,
    pub(crate) virtual_exits: u64,
}

/// 冲突领域各 LIR 表与关系辅表的记录计数。
pub(crate) struct LirConflictCounts {
    pub(crate) zones: u64,
    pub(crate) streams: u64,
    pub(crate) passages: u64,
    pub(crate) zone_streams: u64,
    pub(crate) max_zone_streams: u64,
}

/// 空间几何领域各 LIR 表的记录计数。
pub(crate) struct LirSpatialCounts {
    pub(crate) canonical_frames: u64,
    pub(crate) lane_edge_geometries: u64,
    pub(crate) facility_band_geometries: u64,
    pub(crate) conflict_zone_regions: u64,
    pub(crate) canonical_points: u64,
    pub(crate) conflict_region_points: u64,
    pub(crate) spatial_segments: u64,
}

/// 准入领域各 LIR 表与规则类别引用辅表的记录计数。
pub(crate) struct LirAccessCounts {
    pub(crate) participant_classes: u64,
    pub(crate) vehicle_profiles: u64,
    pub(crate) access_rules: u64,
    pub(crate) rule_class_references: u64,
}

impl LirFreezePlan {
    /// 把记录计数换算为可分配容量；计数超出目标平台 `usize` 范围时返回序号溢出诊断。
    pub(crate) fn capacity(
        count: u64,
        limits: &crate::CompileProfile,
        primary_span: Option<SourceLocation>,
    ) -> Result<usize, DiagnosticBundle> {
        usize::try_from(count).map_err(|_| super::ordinal_overflow(limits, primary_span))
    }

    /// 统计全部计数并聚合预算；不分配任何与记录数成正比的集合。
    pub(crate) fn analyze(mir: &MirUnit) -> Self {
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
            areas: mir_len(mir.parking_facilities.len()),
            spaces: mir_len(mir.parking_spaces.len()),
            memberships: mir_len(mir.parking_facility_spaces.len()),
            virtual_entries: mir_len(mir.parking_facility_virtual_entries.len()),
            virtual_exits: mir_len(mir.parking_facility_virtual_exits.len()),
        };
        let conflict = LirConflictCounts {
            zones: mir_len(mir.conflict_zones.len()),
            streams: mir_len(mir.participant_streams.len()),
            passages: mir_len(mir.conflict_passages.len()),
            zone_streams: mir_len(mir.conflict_zone_streams.len()),
            max_zone_streams: mir
                .conflict_zones
                .iter()
                .map(|zone| u64::from(zone.participant_streams.len()))
                .max()
                .unwrap_or(0),
        };
        let spatial = LirSpatialCounts {
            canonical_frames: mir_len(mir.canonical_frames.len()),
            lane_edge_geometries: mir_len(mir.lane_edge_geometries.len()),
            facility_band_geometries: mir_len(mir.facility_band_geometries.len()),
            conflict_zone_regions: mir_len(mir.conflict_zone_regions.len()),
            canonical_points: mir_len(mir.canonical_points.len()),
            conflict_region_points: mir_len(mir.conflict_region_points.len()),
            spatial_segments: mir_len(mir.spatial_segments.len()),
        };
        let access = LirAccessCounts {
            participant_classes: mir_len(mir.participant_classes.len()),
            vehicle_profiles: mir_len(mir.vehicle_profiles.len()),
            access_rules: mir_len(mir.access_rules.len()),
            rule_class_references: mir_len(mir.access_rule_participant_classes.len()),
        };
        // Identity 字段出现记录有独立资源维度；LIR record 指标计实体行和关系出现行，与 MIR
        // 当前已支持实体与关系的计数对象保持一致。
        // `authoring_lanes` 按拆分前 `section_lane_count` + `lane_count` 加两次；
        // `waiting_zones` / `maneuver_gates` 在信号段前与数组末尾重复累加。#374 重建
        // 精确账本前原样保留。
        let lir_record_count = [
            mir.policies
                .iter()
                .fold(0_u64, |n, p| n.saturating_add(p.value.records())),
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
            control.waiting_zones,
            control.maneuver_gates,
            signal.groups,
            signal.controllers,
            signal.controller_groups,
            signal.phases,
            signal.phases,
            signal.phase_states,
            signal.controlled_gates,
            conflict.zones,
            conflict.streams,
            conflict.passages,
            parking.areas,
            parking.spaces,
            parking.memberships,
            parking.virtual_entries,
            parking.virtual_exits,
            access.participant_classes,
            access.vehicle_profiles,
            spatial.canonical_frames,
            spatial.lane_edge_geometries,
            spatial.facility_band_geometries,
            spatial.conflict_zone_regions,
            spatial.canonical_points,
            spatial.conflict_region_points,
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
            .saturating_add((mir.policies.len() as u64).saturating_mul(2))
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
            .saturating_add(conflict.zones.saturating_mul(3))
            .saturating_add(conflict.streams.saturating_mul(3))
            .saturating_add(parking.areas.saturating_mul(2))
            .saturating_add(parking.spaces.saturating_mul(2))
            .saturating_add(access.participant_classes.saturating_mul(2))
            .saturating_add(access.vehicle_profiles.saturating_mul(2))
            .saturating_add(spatial.canonical_frames.saturating_mul(2))
            .saturating_add(access.access_rules.saturating_mul(2));
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

        // 暂存只在领域冻结内部存续，随领域冻结返回释放：zone→stream 反向闭包只在冻结时证明
        // MIR 没有丢失 passage authority（外层序列、各 zone 精确容量的成员表与复用的单-zone
        // 对照缓冲）；MIR 行到几何行的稀疏映射；ParticipantClass 的 HIR DFS 区间不能跨
        // MIR→LIR 身份重排复用，最终 ordinal 上重建 first-child/next-sibling 与显式 DFS 栈。
        let scratch = Plan::ZERO
            .array::<BoundedVec<ParticipantStreamOrdinal>>(cap(conflict.zones))
            .array::<ParticipantStreamOrdinal>(cap(conflict.zone_streams))
            .array::<ParticipantStreamOrdinal>(cap(conflict.max_zone_streams))
            .array::<Option<usize>>(cap(lane_edge_count))
            .array::<Option<usize>>(cap(cross_section.facility_bands))
            .array::<Option<usize>>(cap(conflict.zones))
            .array::<u32>(cap(access.participant_classes))
            .array::<u32>(cap(access.participant_classes))
            .array::<(u32, bool)>(cap(access.participant_classes.saturating_mul(2)));
        // 规范排列与 owner-local 行排列随冻结输出跨入源映射冻结：排序序列与 MIR→LIR 映射
        // 各只保存一个有类型 `u32` 值，完整身份原像通过借用 MIR 字段作分段比较。
        let stable_entity_counts = [
            cross_section.road_corridors,
            cross_section.road_sections,
            cross_section.authoring_lanes,
            cross_section.lane_groups,
            cross_section.facility_bands,
            junction.junctions,
            junction.movements,
            junction.maneuver_paths,
            control.stop_lines,
            control.maneuver_gates,
            control.waiting_zones,
            signal.groups,
            signal.controllers,
            signal.phases,
            conflict.zones,
            conflict.streams,
            parking.areas,
            parking.spaces,
            access.participant_classes,
            access.vehicle_profiles,
            spatial.canonical_frames,
            access.access_rules,
        ];
        let orders = stable_entity_counts
            .into_iter()
            .fold(
                Plan::ZERO
                    .array::<MirLaneEdgeKey>(cap(lane_edge_count))
                    .array::<LaneEdgeOrdinal>(cap(lane_edge_count)),
                // 每类实体一张 MIR 阶段键表与一张 LIR 序号表，两者都是 `u32` 宽。
                |plan, count| plan.array::<u32>(cap(count)).array::<u32>(cap(count)),
            )
            .array::<u32>(cap(junction.junction_internal_edges))
            .array::<ArenaKey<MirLaneEdgeConnection>>(cap(successor_count))
            .array::<ArenaKey<MirSignalControllerGroup>>(cap(signal.controller_groups))
            .array::<ArenaKey<MirSignalPhaseState>>(cap(signal.phase_states))
            .array::<ArenaKey<MirConflictPassage>>(cap(conflict.passages));
        // 输出逻辑字节使用设计冻结的目标布局中立字段宽度，不能把 Rust struct padding 或
        // 当前平台对齐冒充规范输出量；受控存续内存则按真实堆容量请求单独计算。
        let output_bytes = lane_edge_count
            .saturating_mul(LIR_LANE_EDGE_LOGICAL_BYTES)
            .saturating_add(
                mir.policies
                    .iter()
                    .fold(0_u64, |n, p| n.saturating_add(p.value.logical_bytes())),
            )
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
            .saturating_add(
                conflict
                    .zones
                    .saturating_mul(LIR_CONFLICT_ZONE_LOGICAL_BYTES),
            )
            .saturating_add(
                conflict
                    .streams
                    .saturating_mul(LIR_PARTICIPANT_STREAM_LOGICAL_BYTES),
            )
            .saturating_add(
                conflict
                    .passages
                    .saturating_mul(LIR_CONFLICT_PASSAGE_LOGICAL_BYTES),
            )
            .saturating_add(
                parking
                    .areas
                    .saturating_mul(LIR_PARKING_FACILITY_LOGICAL_BYTES),
            )
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
                parking
                    .virtual_entries
                    .saturating_add(parking.virtual_exits)
                    .saturating_mul(8),
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
                    .conflict_zone_regions
                    .saturating_mul(LIR_CONFLICT_ZONE_REGION_LOGICAL_BYTES),
            )
            .saturating_add(
                spatial
                    .canonical_points
                    .saturating_mul(LIR_CANONICAL_POINT_LOGICAL_BYTES),
            )
            .saturating_add(
                spatial
                    .conflict_region_points
                    .saturating_mul(LIR_POINT2_LOGICAL_BYTES),
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
        // LIR 各表按领域冻结的实际计数逐表授予；记录内嵌的变长载荷集中取得一笔授予。
        let tables = Plan::ZERO
            .array::<LirPolicy>(mir.policies.len())
            .array::<LirLaneEdge>(cap(lane_edge_count))
            .array::<LaneEdgeOrdinal>(cap(successor_count))
            .array::<LirIdentityField>(cap(identity_field_count))
            .array::<u8>(cap(identity_field_byte_count))
            .array::<LirRoadCorridor>(cap(cross_section.road_corridors))
            .array::<LirCorridorElement>(cap(cross_section.corridor_elements))
            .array::<LirRoadSection>(cap(cross_section.road_sections))
            .array::<AuthoringLaneOrdinal>(cap(cross_section.section_lanes))
            .array::<LirAuthoringLane>(cap(cross_section.authoring_lanes))
            .array::<LaneEdgeOrdinal>(cap(cross_section.authoring_lane_edges))
            .array::<LirLaneGroup>(cap(cross_section.lane_groups))
            .array::<AuthoringLaneOrdinal>(cap(cross_section.lane_group_members))
            .array::<LirFacilityBand>(cap(cross_section.facility_bands))
            .array::<LirJunction>(cap(junction.junctions))
            .array::<MovementOrdinal>(cap(junction.junction_movements))
            .array::<LirMovement>(cap(junction.movements))
            .array::<ManeuverPathOrdinal>(cap(junction.movement_paths))
            .array::<LirManeuverPath>(cap(junction.maneuver_paths))
            .array::<LaneEdgeOrdinal>(cap(junction.maneuver_path_edges))
            .array::<ManeuverGateOrdinal>(cap(junction.maneuver_path_gates))
            .array::<WaitingZoneOrdinal>(cap(junction.maneuver_path_waiting_zones))
            .array::<LirJunctionInternalEdge>(cap(junction.junction_internal_edges))
            .array::<LirStopLine>(cap(control.stop_lines))
            .array::<ManeuverGateOrdinal>(cap(control.stop_line_maneuver_gates))
            .array::<LirManeuverGate>(cap(control.maneuver_gates))
            .array::<LirWaitingZone>(cap(control.waiting_zones))
            .array::<LirSignalGroup>(cap(signal.groups))
            .array::<ManeuverGateOrdinal>(cap(signal.controlled_gates))
            .array::<LirSignalController>(cap(signal.controllers))
            .array::<SignalGroupOrdinal>(cap(signal.controller_groups))
            .array::<SignalPhaseOrdinal>(cap(signal.phases))
            .array::<LirSignalPhase>(cap(signal.phases))
            .array::<LirSignalPhaseState>(cap(signal.phase_states))
            .array::<LirConflictZone>(cap(conflict.zones))
            .array::<LirParticipantStream>(cap(conflict.streams))
            .array::<LirConflictPassage>(cap(conflict.passages))
            .array::<LirParkingFacility>(cap(parking.areas))
            .array::<ParkingSpaceOrdinal>(cap(parking.memberships))
            .array::<super::parking::LirParkingLaneAnchor>(cap(parking.virtual_entries))
            .array::<super::parking::LirParkingLaneAnchor>(cap(parking.virtual_exits))
            .array::<LirParkingSpace>(cap(parking.spaces))
            .array::<LirParticipantClass>(cap(access.participant_classes))
            .array::<LirVehicleProfile>(cap(access.vehicle_profiles))
            .array::<LirAccessRule>(cap(access.access_rules))
            .array::<ParticipantClassOrdinal>(cap(access.rule_class_references))
            .array::<LirCanonicalFrame>(cap(spatial.canonical_frames))
            .array::<LirLaneEdgeGeometry>(cap(spatial.lane_edge_geometries))
            .array::<LirFacilityBandGeometry>(cap(spatial.facility_band_geometries))
            .array::<LirConflictZoneRegion>(cap(spatial.conflict_zone_regions))
            .array::<LirCanonicalPoint3F32>(cap(spatial.canonical_points))
            .array::<LirCanonicalPoint2F32>(cap(spatial.conflict_region_points))
            .array::<LirSpatialSegment>(cap(spatial.spatial_segments));
        let inner_payload_bytes = super::policy::bytes(mir)
            .saturating_add(kind_id_byte_count)
            .saturating_add(movement_approach_key_byte_count)
            .saturating_add(access_regulation_byte_count);
        let output = tables.plus(Plan::of(inner_payload_bytes));
        let retained = output.plus(orders);
        let peak = retained.plus(scratch);
        // 无法表示的计划由准入按宿主预算拒绝；以下字节量只用于报告与测试核对。
        let output_owned_bytes = output.bytes().unwrap_or(u64::MAX);
        Self {
            lane_edge_count,
            successor_count,
            identity_field_count,
            identity_field_byte_count,
            cross_section,
            junction,
            control,
            signal,
            conflict,
            parking,
            spatial,
            access,
            lir_record_count,
            stage_scratch_bytes: scratch.bytes().unwrap_or(u64::MAX),
            output_bytes,
            output_owned_bytes,
            inner_payload_bytes,
            retained_bytes: retained.bytes().unwrap_or(u64::MAX),
            stage_bytes: peak.bytes().unwrap_or(u64::MAX),
            peak,
        }
    }

    /// 对照编译资源上限预检 LIR 记录数、阶段暂存字节、输出字节与编译器控制存续字节；任一维度超限即聚合返回资源诊断。
    pub(crate) fn check_limits<'l>(
        &self,
        unit: &CompilationUnit,
        mir: &MirUnit,
        ledger: &'l crate::ledger::Ledger,
    ) -> Result<crate::ledger::PassBudget<'l>, DiagnosticBundle> {
        let primary_span = mir.modules.first().map(|module| module.source_span.clone());
        let stable_key = mir
            .modules
            .first()
            .map(|module| module.authoring_namespace_id.as_ref().into());
        let mut diagnostics = DiagnosticCollector::new(
            unit.limits
                .source_limits()
                .get(crate::SourceProcessingDimension::DiagnosticCount),
        );
        let admission = ledger.admit_pass(LIR_PASS, self.peak);
        for violation in [unit.limits.source_limits().exceeded(
            crate::SourceProcessingDimension::LirRecordCount,
            self.lir_record_count,
        )]
        .into_iter()
        .flatten()
        {
            let crate::limits::ResourceLimitExceeded {
                dimension,
                limit,
                observed,
            } = violation;

            diagnostics.push(Diagnostic::compile_limit_exceeded_at(
                dimension,
                limit,
                observed,
                primary_span.clone(),
                stable_key.clone(),
            ));
        }
        let pass = match admission {
            Ok(pass) => Some(pass),
            Err(error) => {
                diagnostics.push(Diagnostic::from_ledger_error(
                    error,
                    primary_span.clone(),
                    stable_key.clone(),
                ));
                None
            }
        };
        match pass {
            Some(pass) if diagnostics.is_empty() => Ok(pass),
            _ => Err(diagnostics.finish()),
        }
    }
}

/// LIR 冻结编译遍的记账点。
const LIR_PASS: crate::ledger::PassLabel =
    crate::ledger::PassLabel::new(crate::ledger::LedgerStage::CompilePipeline, "lir");

/// 计数换算为授予请求的元素数；超出 `usize` 时取最大值，使计划无法表示。
fn cap(count: u64) -> usize {
    usize::try_from(count).unwrap_or(usize::MAX)
}

fn mir_len(len: usize) -> u64 {
    u64::try_from(len).unwrap_or(u64::MAX)
}
