//! MIR→LIR 规范排列。

use core::cmp::Ordering;

use laneflow_static_contract::{
    AccessRuleOrdinal, AuthoringLaneOrdinal, CanonicalFrameOrdinal, FacilityBandOrdinal,
    JunctionOrdinal, LaneEdgeOrdinal, LaneGroupOrdinal, ManeuverGateOrdinal, ManeuverPathOrdinal,
    MovementOrdinal, ParkingAreaOrdinal, ParkingSpaceOrdinal, ParticipantClassOrdinal,
    RoadCorridorOrdinal, RoadSectionOrdinal, SignalControllerOrdinal, SignalGroupOrdinal,
    SignalPhaseOrdinal, StaticRouteOrdinal, StopLineOrdinal, VehicleProfileOrdinal,
    WaitingZoneOrdinal,
};

use crate::arena::ArenaKey;
use crate::mir::{
    MirAccessRuleKey, MirAuthoringLaneKey, MirCanonicalFrameKey, MirFacilityBandKey,
    MirJunctionKey, MirLaneEdgeKey, MirLaneGroupKey, MirManeuverGateKey, MirManeuverPathKey,
    MirMovementKey, MirParkingAreaKey, MirParkingSpaceKey, MirParticipantClassKey,
    MirRoadCorridorKey, MirRoadSectionKey, MirSignalControllerKey, MirSignalGroupKey,
    MirSignalPhaseKey, MirStaticRouteKey, MirStopLineKey, MirUnit, MirVehicleProfileKey,
    MirWaitingZoneKey,
};
use crate::{DiagnosticBundle, SourceLocation};

use super::{
    LirFreezePlan, compare_identity_parts, compare_identity_v1, compare_length_prefixed,
    dense_mir_keys, mapping_pair_bytes, ordinal_mapping, ordinal_overflow, requested_bytes,
};

/// 一类稳定实体从 MIR 致密地址到 LIR 规范序号的双向排列。
///
/// 两个方向在冻结输出中成对拥有，只在同一次 `Compiler::compile` 内跨越
/// LIR/source-map 阶段；它不是身份、公共 LIR 或跨编译缓存。
pub(crate) struct LirEntityOrder<K, O> {
    stage_keys_in_lir_order: Box<[K]>,
    ordinal_by_stage_key: Box<[O]>,
}

impl<K: Copy, O: Copy> LirEntityOrder<K, O> {
    pub(crate) fn stage_keys_in_lir_order(&self) -> &[K] {
        &self.stage_keys_in_lir_order
    }

    pub(crate) fn mapping_bytes(&self) -> u64 {
        mapping_pair_bytes::<K, O>(
            self.stage_keys_in_lir_order.len(),
            self.ordinal_by_stage_key.len(),
        )
    }
}

impl<Tag, O: Copy + Into<u32>> LirEntityOrder<ArenaKey<Tag>, O> {
    pub(crate) fn from_parts(
        stage_keys_in_lir_order: Vec<ArenaKey<Tag>>,
        ordinal_by_stage_key: Vec<O>,
    ) -> Self {
        debug_assert_eq!(
            stage_keys_in_lir_order.len(),
            ordinal_by_stage_key.len(),
            "dense stage-key and LIR-ordinal tables must describe the same entity set"
        );
        debug_assert!(
            stage_keys_in_lir_order
                .iter()
                .copied()
                .enumerate()
                .all(|(lir_index, stage_key)| {
                    ordinal_by_stage_key[stage_key.index()].into()
                        == u32::try_from(lir_index)
                            .expect("LIR precheck proved entity count fits u32")
                }),
            "both LIR entity-order directions must be exact inverses"
        );
        Self {
            stage_keys_in_lir_order: stage_keys_in_lir_order.into_boxed_slice(),
            ordinal_by_stage_key: ordinal_by_stage_key.into_boxed_slice(),
        }
    }

    pub(crate) fn stage_key_at_lir_index(&self, index: usize) -> ArenaKey<Tag> {
        self.stage_keys_in_lir_order[index]
    }

    pub(crate) fn ordinal(&self, stage_key: ArenaKey<Tag>) -> O {
        self.ordinal_by_stage_key[stage_key.index()]
    }
}

/// 已按 LIR owner-local 行顺序冻结的 MIR 关系行地址排列。
///
/// 关系行没有稳定身份；本值只让来源伴随数据经过与 LIR 语义行相同的 permutation。
pub(crate) struct OwnerLocalPermutation<Row> {
    mir_rows_in_lir_order: Box<[ArenaKey<Row>]>,
}

impl<Row> OwnerLocalPermutation<Row> {
    pub(crate) fn from_rows(mir_rows_in_lir_order: Vec<ArenaKey<Row>>) -> Self {
        Self {
            mir_rows_in_lir_order: mir_rows_in_lir_order.into_boxed_slice(),
        }
    }

    pub(crate) fn mir_rows_in_lir_order(&self) -> &[ArenaKey<Row>] {
        &self.mir_rows_in_lir_order
    }

    pub(crate) fn mapping_bytes(&self) -> u64 {
        requested_bytes::<ArenaKey<Row>>(
            u64::try_from(self.mir_rows_in_lir_order.len()).unwrap_or(u64::MAX),
        )
    }
}

/// 全部稳定实体的 MIR→LIR 双向排列。
///
/// [`CanonicalOrders::build`] 只读 MIR 做 Identity v1 排序；owner-local permutation
/// 依赖已冻结 LIR 序号，仍在领域冻结中生成。
pub(crate) struct CanonicalOrders {
    pub(crate) lane_edges: LirEntityOrder<MirLaneEdgeKey, LaneEdgeOrdinal>,
    pub(crate) road_corridors: LirEntityOrder<MirRoadCorridorKey, RoadCorridorOrdinal>,
    pub(crate) road_sections: LirEntityOrder<MirRoadSectionKey, RoadSectionOrdinal>,
    pub(crate) authoring_lanes: LirEntityOrder<MirAuthoringLaneKey, AuthoringLaneOrdinal>,
    pub(crate) lane_groups: LirEntityOrder<MirLaneGroupKey, LaneGroupOrdinal>,
    pub(crate) facility_bands: LirEntityOrder<MirFacilityBandKey, FacilityBandOrdinal>,
    pub(crate) junctions: LirEntityOrder<MirJunctionKey, JunctionOrdinal>,
    pub(crate) movements: LirEntityOrder<MirMovementKey, MovementOrdinal>,
    pub(crate) maneuver_paths: LirEntityOrder<MirManeuverPathKey, ManeuverPathOrdinal>,
    pub(crate) stop_lines: LirEntityOrder<MirStopLineKey, StopLineOrdinal>,
    pub(crate) maneuver_gates: LirEntityOrder<MirManeuverGateKey, ManeuverGateOrdinal>,
    pub(crate) waiting_zones: LirEntityOrder<MirWaitingZoneKey, WaitingZoneOrdinal>,
    pub(crate) signal_groups: LirEntityOrder<MirSignalGroupKey, SignalGroupOrdinal>,
    pub(crate) signal_controllers: LirEntityOrder<MirSignalControllerKey, SignalControllerOrdinal>,
    pub(crate) signal_phases: LirEntityOrder<MirSignalPhaseKey, SignalPhaseOrdinal>,
    pub(crate) parking_areas: LirEntityOrder<MirParkingAreaKey, ParkingAreaOrdinal>,
    pub(crate) parking_spaces: LirEntityOrder<MirParkingSpaceKey, ParkingSpaceOrdinal>,
    pub(crate) participant_classes: LirEntityOrder<MirParticipantClassKey, ParticipantClassOrdinal>,
    pub(crate) vehicle_profiles: LirEntityOrder<MirVehicleProfileKey, VehicleProfileOrdinal>,
    pub(crate) canonical_frames: LirEntityOrder<MirCanonicalFrameKey, CanonicalFrameOrdinal>,
    pub(crate) access_rules: LirEntityOrder<MirAccessRuleKey, AccessRuleOrdinal>,
    pub(crate) static_routes: LirEntityOrder<MirStaticRouteKey, StaticRouteOrdinal>,
}

impl CanonicalOrders {
    pub(crate) fn build(
        mir: &MirUnit,
        plan: &LirFreezePlan,
        limits: &crate::CompileLimits,
        primary_span: Option<SourceLocation>,
    ) -> Result<Self, DiagnosticBundle> {
        let mut canonical_order = mir
            .lane_edges
            .iter()
            .enumerate()
            .map(|(index, _)| {
                let raw = u32::try_from(index).expect("LIR precheck proved every MIR key fits u32");
                MirLaneEdgeKey::from_raw(raw)
            })
            .collect::<Vec<_>>();
        canonical_order.sort_unstable_by(|left, right| compare_identity_v1(mir, *left, *right));
        debug_assert!(
            canonical_order
                .windows(2)
                .all(|pair| { compare_identity_v1(mir, pair[0], pair[1]) == Ordering::Less })
        );

        let mut mir_to_lir =
            vec![
                LaneEdgeOrdinal::from_raw(0);
                LirFreezePlan::capacity(plan.lane_edge_count, limits, primary_span.clone())?
            ];
        for (index, mir_key) in canonical_order.iter().copied().enumerate() {
            mir_to_lir[mir_key.index()] = LaneEdgeOrdinal::try_from_usize(index)
                .map_err(|_| ordinal_overflow(limits, primary_span.clone()))?;
        }

        let mut canonical_mir_corridor_order: Vec<MirRoadCorridorKey> =
            dense_mir_keys(LirFreezePlan::capacity(
                plan.cross_section.road_corridors,
                limits,
                primary_span.clone(),
            )?);
        canonical_mir_corridor_order.sort_unstable_by(|left, right| {
            let left = &mir.road_corridors[left.index()];
            let right = &mir.road_corridors[right.index()];
            compare_identity_parts(
                &mir.modules[left.module.index()].authoring_namespace_id,
                &left.stable_key,
                None,
                &mir.modules[right.module.index()].authoring_namespace_id,
                &right.stable_key,
                None,
            )
        });
        let mir_corridor_to_lir = ordinal_mapping(
            LirFreezePlan::capacity(
                plan.cross_section.road_corridors,
                limits,
                primary_span.clone(),
            )?,
            &canonical_mir_corridor_order,
            RoadCorridorOrdinal::try_from_usize,
            limits,
            primary_span.clone(),
        )?;

        let mut canonical_mir_section_order: Vec<MirRoadSectionKey> =
            dense_mir_keys(LirFreezePlan::capacity(
                plan.cross_section.road_sections,
                limits,
                primary_span.clone(),
            )?);
        canonical_mir_section_order.sort_unstable_by(|left, right| {
            let left = &mir.road_sections[left.index()];
            let right = &mir.road_sections[right.index()];
            compare_identity_parts(
                &mir.modules[left.module.index()].authoring_namespace_id,
                &left.stable_key,
                Some(
                    mir.road_corridors[left.road_corridor.index()]
                        .stable_id
                        .as_untyped()
                        .as_bytes(),
                ),
                &mir.modules[right.module.index()].authoring_namespace_id,
                &right.stable_key,
                Some(
                    mir.road_corridors[right.road_corridor.index()]
                        .stable_id
                        .as_untyped()
                        .as_bytes(),
                ),
            )
        });
        let mir_section_to_lir = ordinal_mapping(
            LirFreezePlan::capacity(
                plan.cross_section.road_sections,
                limits,
                primary_span.clone(),
            )?,
            &canonical_mir_section_order,
            RoadSectionOrdinal::try_from_usize,
            limits,
            primary_span.clone(),
        )?;

        let mut canonical_mir_lane_order: Vec<MirAuthoringLaneKey> =
            dense_mir_keys(LirFreezePlan::capacity(
                plan.cross_section.authoring_lanes,
                limits,
                primary_span.clone(),
            )?);
        canonical_mir_lane_order.sort_unstable_by(|left, right| {
            let left = &mir.authoring_lanes[left.index()];
            let right = &mir.authoring_lanes[right.index()];
            compare_identity_parts(
                &mir.modules[left.module.index()].authoring_namespace_id,
                &left.stable_key,
                Some(
                    mir.road_sections[left.road_section.index()]
                        .stable_id
                        .as_untyped()
                        .as_bytes(),
                ),
                &mir.modules[right.module.index()].authoring_namespace_id,
                &right.stable_key,
                Some(
                    mir.road_sections[right.road_section.index()]
                        .stable_id
                        .as_untyped()
                        .as_bytes(),
                ),
            )
        });
        let mir_lane_to_lir = ordinal_mapping(
            LirFreezePlan::capacity(
                plan.cross_section.authoring_lanes,
                limits,
                primary_span.clone(),
            )?,
            &canonical_mir_lane_order,
            AuthoringLaneOrdinal::try_from_usize,
            limits,
            primary_span.clone(),
        )?;

        let mut canonical_mir_group_order: Vec<MirLaneGroupKey> = dense_mir_keys(
            LirFreezePlan::capacity(plan.cross_section.lane_groups, limits, primary_span.clone())?,
        );
        canonical_mir_group_order.sort_unstable_by(|left, right| {
            let left = &mir.lane_groups[left.index()];
            let right = &mir.lane_groups[right.index()];
            compare_identity_parts(
                &mir.modules[left.module.index()].authoring_namespace_id,
                &left.stable_key,
                Some(
                    mir.road_sections[left.road_section.index()]
                        .stable_id
                        .as_untyped()
                        .as_bytes(),
                ),
                &mir.modules[right.module.index()].authoring_namespace_id,
                &right.stable_key,
                Some(
                    mir.road_sections[right.road_section.index()]
                        .stable_id
                        .as_untyped()
                        .as_bytes(),
                ),
            )
        });
        let mir_group_to_lir = ordinal_mapping(
            LirFreezePlan::capacity(plan.cross_section.lane_groups, limits, primary_span.clone())?,
            &canonical_mir_group_order,
            LaneGroupOrdinal::try_from_usize,
            limits,
            primary_span.clone(),
        )?;

        let mut canonical_mir_band_order: Vec<MirFacilityBandKey> =
            dense_mir_keys(LirFreezePlan::capacity(
                plan.cross_section.facility_bands,
                limits,
                primary_span.clone(),
            )?);
        canonical_mir_band_order.sort_unstable_by(|left, right| {
            let left = &mir.facility_bands[left.index()];
            let right = &mir.facility_bands[right.index()];
            compare_identity_parts(
                &mir.modules[left.module.index()].authoring_namespace_id,
                &left.stable_key,
                Some(
                    mir.road_corridors[left.road_corridor.index()]
                        .stable_id
                        .as_untyped()
                        .as_bytes(),
                ),
                &mir.modules[right.module.index()].authoring_namespace_id,
                &right.stable_key,
                Some(
                    mir.road_corridors[right.road_corridor.index()]
                        .stable_id
                        .as_untyped()
                        .as_bytes(),
                ),
            )
        });
        let mir_band_to_lir = ordinal_mapping(
            LirFreezePlan::capacity(
                plan.cross_section.facility_bands,
                limits,
                primary_span.clone(),
            )?,
            &canonical_mir_band_order,
            FacilityBandOrdinal::try_from_usize,
            limits,
            primary_span.clone(),
        )?;

        let mut canonical_mir_junction_order: Vec<MirJunctionKey> = dense_mir_keys(
            LirFreezePlan::capacity(plan.junction.junctions, limits, primary_span.clone())?,
        );
        canonical_mir_junction_order.sort_unstable_by(|left, right| {
            let left = &mir.junctions[left.index()];
            let right = &mir.junctions[right.index()];
            compare_identity_parts(
                &mir.modules[left.module.index()].authoring_namespace_id,
                &left.stable_key,
                None,
                &mir.modules[right.module.index()].authoring_namespace_id,
                &right.stable_key,
                None,
            )
        });
        let mir_junction_to_lir = ordinal_mapping(
            LirFreezePlan::capacity(plan.junction.junctions, limits, primary_span.clone())?,
            &canonical_mir_junction_order,
            JunctionOrdinal::try_from_usize,
            limits,
            primary_span.clone(),
        )?;

        let mut canonical_mir_movement_order: Vec<MirMovementKey> = dense_mir_keys(
            LirFreezePlan::capacity(plan.junction.movements, limits, primary_span.clone())?,
        );
        canonical_mir_movement_order.sort_unstable_by(|left, right| {
            let left = &mir.movements[left.index()];
            let right = &mir.movements[right.index()];
            compare_length_prefixed(
                mir.modules[left.module.index()]
                    .authoring_namespace_id
                    .as_bytes(),
                mir.modules[right.module.index()]
                    .authoring_namespace_id
                    .as_bytes(),
            )
            .then_with(|| {
                compare_length_prefixed(left.stable_key.as_bytes(), right.stable_key.as_bytes())
            })
            .then_with(|| {
                compare_length_prefixed(
                    left.directed_entry_approach_key.as_bytes(),
                    right.directed_entry_approach_key.as_bytes(),
                )
            })
            .then_with(|| {
                compare_length_prefixed(
                    left.directed_exit_approach_key.as_bytes(),
                    right.directed_exit_approach_key.as_bytes(),
                )
            })
            .then_with(|| {
                mir.junctions[left.junction.index()]
                    .stable_id
                    .as_untyped()
                    .as_bytes()
                    .cmp(
                        mir.junctions[right.junction.index()]
                            .stable_id
                            .as_untyped()
                            .as_bytes(),
                    )
            })
        });
        let mir_movement_to_lir = ordinal_mapping(
            LirFreezePlan::capacity(plan.junction.movements, limits, primary_span.clone())?,
            &canonical_mir_movement_order,
            MovementOrdinal::try_from_usize,
            limits,
            primary_span.clone(),
        )?;

        let mut canonical_mir_maneuver_path_order: Vec<MirManeuverPathKey> = dense_mir_keys(
            LirFreezePlan::capacity(plan.junction.maneuver_paths, limits, primary_span.clone())?,
        );
        canonical_mir_maneuver_path_order.sort_unstable_by(|left, right| {
            let left = &mir.maneuver_paths[left.index()];
            let right = &mir.maneuver_paths[right.index()];
            let left_edges = &mir.maneuver_path_edges[left.edges.as_usize_range()];
            let right_edges = &mir.maneuver_path_edges[right.edges.as_usize_range()];
            compare_length_prefixed(
                mir.modules[left.module.index()]
                    .authoring_namespace_id
                    .as_bytes(),
                mir.modules[right.module.index()]
                    .authoring_namespace_id
                    .as_bytes(),
            )
            .then_with(|| {
                compare_length_prefixed(left.stable_key.as_bytes(), right.stable_key.as_bytes())
            })
            .then_with(|| {
                mir.movements[left.movement.index()]
                    .stable_id
                    .as_untyped()
                    .as_bytes()
                    .cmp(
                        mir.movements[right.movement.index()]
                            .stable_id
                            .as_untyped()
                            .as_bytes(),
                    )
            })
            .then_with(|| {
                mir.lane_edges[left_edges[0].target.index()]
                    .stable_id
                    .as_untyped()
                    .as_bytes()
                    .cmp(
                        mir.lane_edges[right_edges[0].target.index()]
                            .stable_id
                            .as_untyped()
                            .as_bytes(),
                    )
            })
            .then_with(|| {
                mir.lane_edges[left_edges[left_edges.len() - 1].target.index()]
                    .stable_id
                    .as_untyped()
                    .as_bytes()
                    .cmp(
                        mir.lane_edges[right_edges[right_edges.len() - 1].target.index()]
                            .stable_id
                            .as_untyped()
                            .as_bytes(),
                    )
            })
        });
        let mir_maneuver_path_to_lir = ordinal_mapping(
            LirFreezePlan::capacity(plan.junction.maneuver_paths, limits, primary_span.clone())?,
            &canonical_mir_maneuver_path_order,
            ManeuverPathOrdinal::try_from_usize,
            limits,
            primary_span.clone(),
        )?;

        let mut canonical_mir_stop_line_order: Vec<MirStopLineKey> = dense_mir_keys(
            LirFreezePlan::capacity(plan.control.stop_lines, limits, primary_span.clone())?,
        );
        canonical_mir_stop_line_order.sort_unstable_by(|left, right| {
            let left = &mir.stop_lines[left.index()];
            let right = &mir.stop_lines[right.index()];
            compare_identity_parts(
                &mir.modules[left.module.index()].authoring_namespace_id,
                &left.stable_key,
                None,
                &mir.modules[right.module.index()].authoring_namespace_id,
                &right.stable_key,
                None,
            )
        });
        let mir_stop_line_to_lir = ordinal_mapping(
            LirFreezePlan::capacity(plan.control.stop_lines, limits, primary_span.clone())?,
            &canonical_mir_stop_line_order,
            StopLineOrdinal::try_from_usize,
            limits,
            primary_span.clone(),
        )?;

        let mut canonical_mir_maneuver_gate_order: Vec<MirManeuverGateKey> = dense_mir_keys(
            LirFreezePlan::capacity(plan.control.maneuver_gates, limits, primary_span.clone())?,
        );
        canonical_mir_maneuver_gate_order.sort_unstable_by(|left, right| {
            let left = &mir.maneuver_gates[left.index()];
            let right = &mir.maneuver_gates[right.index()];
            compare_length_prefixed(
                mir.modules[left.module.index()]
                    .authoring_namespace_id
                    .as_bytes(),
                mir.modules[right.module.index()]
                    .authoring_namespace_id
                    .as_bytes(),
            )
            .then_with(|| {
                mir.maneuver_paths[left.maneuver_path.index()]
                    .stable_id
                    .as_untyped()
                    .as_bytes()
                    .cmp(
                        mir.maneuver_paths[right.maneuver_path.index()]
                            .stable_id
                            .as_untyped()
                            .as_bytes(),
                    )
            })
            .then_with(|| {
                compare_length_prefixed(left.stable_key.as_bytes(), right.stable_key.as_bytes())
            })
        });
        let mir_maneuver_gate_to_lir = ordinal_mapping(
            LirFreezePlan::capacity(plan.control.maneuver_gates, limits, primary_span.clone())?,
            &canonical_mir_maneuver_gate_order,
            ManeuverGateOrdinal::try_from_usize,
            limits,
            primary_span.clone(),
        )?;

        let mut canonical_mir_waiting_zone_order: Vec<MirWaitingZoneKey> = dense_mir_keys(
            LirFreezePlan::capacity(plan.control.waiting_zones, limits, primary_span.clone())?,
        );
        canonical_mir_waiting_zone_order.sort_unstable_by(|left, right| {
            let left = &mir.waiting_zones[left.index()];
            let right = &mir.waiting_zones[right.index()];
            compare_length_prefixed(
                mir.modules[left.module.index()]
                    .authoring_namespace_id
                    .as_bytes(),
                mir.modules[right.module.index()]
                    .authoring_namespace_id
                    .as_bytes(),
            )
            .then_with(|| {
                mir.maneuver_paths[left.maneuver_path.index()]
                    .stable_id
                    .as_untyped()
                    .as_bytes()
                    .cmp(
                        mir.maneuver_paths[right.maneuver_path.index()]
                            .stable_id
                            .as_untyped()
                            .as_bytes(),
                    )
            })
            .then_with(|| {
                compare_length_prefixed(left.stable_key.as_bytes(), right.stable_key.as_bytes())
            })
        });
        let mir_waiting_zone_to_lir = ordinal_mapping(
            LirFreezePlan::capacity(plan.control.waiting_zones, limits, primary_span.clone())?,
            &canonical_mir_waiting_zone_order,
            WaitingZoneOrdinal::try_from_usize,
            limits,
            primary_span.clone(),
        )?;

        let mut canonical_mir_signal_group_order: Vec<MirSignalGroupKey> = dense_mir_keys(
            LirFreezePlan::capacity(plan.signal.groups, limits, primary_span.clone())?,
        );
        canonical_mir_signal_group_order.sort_unstable_by(|left, right| {
            let left = &mir.signal_groups[left.index()];
            let right = &mir.signal_groups[right.index()];
            compare_identity_parts(
                &mir.modules[left.module.index()].authoring_namespace_id,
                &left.stable_key,
                None,
                &mir.modules[right.module.index()].authoring_namespace_id,
                &right.stable_key,
                None,
            )
        });
        let mir_signal_group_to_lir = ordinal_mapping(
            LirFreezePlan::capacity(plan.signal.groups, limits, primary_span.clone())?,
            &canonical_mir_signal_group_order,
            SignalGroupOrdinal::try_from_usize,
            limits,
            primary_span.clone(),
        )?;

        let mut canonical_mir_signal_controller_order: Vec<MirSignalControllerKey> = dense_mir_keys(
            LirFreezePlan::capacity(plan.signal.controllers, limits, primary_span.clone())?,
        );
        canonical_mir_signal_controller_order.sort_unstable_by(|left, right| {
            let left = &mir.signal_controllers[left.index()];
            let right = &mir.signal_controllers[right.index()];
            compare_identity_parts(
                &mir.modules[left.module.index()].authoring_namespace_id,
                &left.stable_key,
                None,
                &mir.modules[right.module.index()].authoring_namespace_id,
                &right.stable_key,
                None,
            )
        });
        let mir_signal_controller_to_lir = ordinal_mapping(
            LirFreezePlan::capacity(plan.signal.controllers, limits, primary_span.clone())?,
            &canonical_mir_signal_controller_order,
            SignalControllerOrdinal::try_from_usize,
            limits,
            primary_span.clone(),
        )?;

        let mut canonical_mir_signal_phase_order: Vec<MirSignalPhaseKey> = dense_mir_keys(
            LirFreezePlan::capacity(plan.signal.phases, limits, primary_span.clone())?,
        );
        canonical_mir_signal_phase_order.sort_unstable_by(|left, right| {
            let left = &mir.signal_phases[left.index()];
            let right = &mir.signal_phases[right.index()];
            compare_length_prefixed(
                mir.modules[left.module.index()]
                    .authoring_namespace_id
                    .as_bytes(),
                mir.modules[right.module.index()]
                    .authoring_namespace_id
                    .as_bytes(),
            )
            .then_with(|| {
                mir.signal_controllers[left.controller.index()]
                    .stable_id
                    .as_untyped()
                    .as_bytes()
                    .cmp(
                        mir.signal_controllers[right.controller.index()]
                            .stable_id
                            .as_untyped()
                            .as_bytes(),
                    )
            })
            .then_with(|| {
                compare_length_prefixed(left.stable_key.as_bytes(), right.stable_key.as_bytes())
            })
        });
        let mir_signal_phase_to_lir = ordinal_mapping(
            LirFreezePlan::capacity(plan.signal.phases, limits, primary_span.clone())?,
            &canonical_mir_signal_phase_order,
            SignalPhaseOrdinal::try_from_usize,
            limits,
            primary_span.clone(),
        )?;

        let mut canonical_mir_parking_area_order: Vec<MirParkingAreaKey> = dense_mir_keys(
            LirFreezePlan::capacity(plan.parking.areas, limits, primary_span.clone())?,
        );
        canonical_mir_parking_area_order.sort_unstable_by(|left, right| {
            let left = &mir.parking_areas[left.index()];
            let right = &mir.parking_areas[right.index()];
            compare_identity_parts(
                &mir.modules[left.module.index()].authoring_namespace_id,
                &left.stable_key,
                None,
                &mir.modules[right.module.index()].authoring_namespace_id,
                &right.stable_key,
                None,
            )
        });
        let mir_parking_area_to_lir = ordinal_mapping(
            LirFreezePlan::capacity(plan.parking.areas, limits, primary_span.clone())?,
            &canonical_mir_parking_area_order,
            ParkingAreaOrdinal::try_from_usize,
            limits,
            primary_span.clone(),
        )?;

        let mut canonical_mir_parking_space_order: Vec<MirParkingSpaceKey> = dense_mir_keys(
            LirFreezePlan::capacity(plan.parking.spaces, limits, primary_span.clone())?,
        );
        canonical_mir_parking_space_order.sort_unstable_by(|left, right| {
            let left = &mir.parking_spaces[left.index()];
            let right = &mir.parking_spaces[right.index()];
            compare_identity_parts(
                &mir.modules[left.module.index()].authoring_namespace_id,
                &left.stable_key,
                None,
                &mir.modules[right.module.index()].authoring_namespace_id,
                &right.stable_key,
                None,
            )
        });
        let mir_parking_space_to_lir = ordinal_mapping(
            LirFreezePlan::capacity(plan.parking.spaces, limits, primary_span.clone())?,
            &canonical_mir_parking_space_order,
            ParkingSpaceOrdinal::try_from_usize,
            limits,
            primary_span.clone(),
        )?;

        let mut canonical_mir_participant_class_order: Vec<MirParticipantClassKey> =
            dense_mir_keys(LirFreezePlan::capacity(
                plan.access.participant_classes,
                limits,
                primary_span.clone(),
            )?);
        canonical_mir_participant_class_order.sort_unstable_by(|left, right| {
            let left = &mir.participant_classes[left.index()];
            let right = &mir.participant_classes[right.index()];
            compare_identity_parts(
                &mir.modules[left.module.index()].authoring_namespace_id,
                &left.stable_key,
                None,
                &mir.modules[right.module.index()].authoring_namespace_id,
                &right.stable_key,
                None,
            )
        });
        let mir_participant_class_to_lir = ordinal_mapping(
            LirFreezePlan::capacity(
                plan.access.participant_classes,
                limits,
                primary_span.clone(),
            )?,
            &canonical_mir_participant_class_order,
            ParticipantClassOrdinal::try_from_usize,
            limits,
            primary_span.clone(),
        )?;

        let mut canonical_mir_vehicle_profile_order: Vec<MirVehicleProfileKey> = dense_mir_keys(
            LirFreezePlan::capacity(plan.access.vehicle_profiles, limits, primary_span.clone())?,
        );
        canonical_mir_vehicle_profile_order.sort_unstable_by(|left, right| {
            let left = &mir.vehicle_profiles[left.index()];
            let right = &mir.vehicle_profiles[right.index()];
            compare_identity_parts(
                &mir.modules[left.module.index()].authoring_namespace_id,
                &left.stable_key,
                None,
                &mir.modules[right.module.index()].authoring_namespace_id,
                &right.stable_key,
                None,
            )
        });
        let mir_vehicle_profile_to_lir = ordinal_mapping(
            LirFreezePlan::capacity(plan.access.vehicle_profiles, limits, primary_span.clone())?,
            &canonical_mir_vehicle_profile_order,
            VehicleProfileOrdinal::try_from_usize,
            limits,
            primary_span.clone(),
        )?;

        let mut canonical_mir_canonical_frame_order: Vec<MirCanonicalFrameKey> = dense_mir_keys(
            LirFreezePlan::capacity(plan.spatial.canonical_frames, limits, primary_span.clone())?,
        );
        canonical_mir_canonical_frame_order.sort_unstable_by(|left, right| {
            let left = &mir.canonical_frames[left.index()];
            let right = &mir.canonical_frames[right.index()];
            compare_identity_parts(
                &mir.modules[left.module.index()].authoring_namespace_id,
                &left.stable_key,
                None,
                &mir.modules[right.module.index()].authoring_namespace_id,
                &right.stable_key,
                None,
            )
        });
        let mir_canonical_frame_to_lir = ordinal_mapping(
            LirFreezePlan::capacity(plan.spatial.canonical_frames, limits, primary_span.clone())?,
            &canonical_mir_canonical_frame_order,
            CanonicalFrameOrdinal::try_from_usize,
            limits,
            primary_span.clone(),
        )?;

        let mut canonical_mir_access_rule_order: Vec<MirAccessRuleKey> = dense_mir_keys(
            LirFreezePlan::capacity(plan.access.access_rules, limits, primary_span.clone())?,
        );
        canonical_mir_access_rule_order.sort_unstable_by(|left, right| {
            let left = &mir.access_rules[left.index()];
            let right = &mir.access_rules[right.index()];
            compare_identity_parts(
                &mir.modules[left.module.index()].authoring_namespace_id,
                &left.stable_key,
                None,
                &mir.modules[right.module.index()].authoring_namespace_id,
                &right.stable_key,
                None,
            )
        });
        let mir_access_rule_to_lir = ordinal_mapping(
            LirFreezePlan::capacity(plan.access.access_rules, limits, primary_span.clone())?,
            &canonical_mir_access_rule_order,
            AccessRuleOrdinal::try_from_usize,
            limits,
            primary_span.clone(),
        )?;

        let mut canonical_mir_static_route_order: Vec<MirStaticRouteKey> = dense_mir_keys(
            LirFreezePlan::capacity(plan.route.static_routes, limits, primary_span.clone())?,
        );
        canonical_mir_static_route_order.sort_unstable_by(|left, right| {
            let left = &mir.static_routes[left.index()];
            let right = &mir.static_routes[right.index()];
            compare_identity_parts(
                &mir.modules[left.module.index()].authoring_namespace_id,
                &left.stable_key,
                None,
                &mir.modules[right.module.index()].authoring_namespace_id,
                &right.stable_key,
                None,
            )
        });
        let mir_static_route_to_lir = ordinal_mapping(
            LirFreezePlan::capacity(plan.route.static_routes, limits, primary_span.clone())?,
            &canonical_mir_static_route_order,
            StaticRouteOrdinal::try_from_usize,
            limits,
            primary_span.clone(),
        )?;

        Ok(Self {
            lane_edges: LirEntityOrder::from_parts(canonical_order, mir_to_lir),
            road_corridors: LirEntityOrder::from_parts(
                canonical_mir_corridor_order,
                mir_corridor_to_lir,
            ),
            road_sections: LirEntityOrder::from_parts(
                canonical_mir_section_order,
                mir_section_to_lir,
            ),
            authoring_lanes: LirEntityOrder::from_parts(canonical_mir_lane_order, mir_lane_to_lir),
            lane_groups: LirEntityOrder::from_parts(canonical_mir_group_order, mir_group_to_lir),
            facility_bands: LirEntityOrder::from_parts(canonical_mir_band_order, mir_band_to_lir),
            junctions: LirEntityOrder::from_parts(
                canonical_mir_junction_order,
                mir_junction_to_lir,
            ),
            movements: LirEntityOrder::from_parts(
                canonical_mir_movement_order,
                mir_movement_to_lir,
            ),
            maneuver_paths: LirEntityOrder::from_parts(
                canonical_mir_maneuver_path_order,
                mir_maneuver_path_to_lir,
            ),
            stop_lines: LirEntityOrder::from_parts(
                canonical_mir_stop_line_order,
                mir_stop_line_to_lir,
            ),
            maneuver_gates: LirEntityOrder::from_parts(
                canonical_mir_maneuver_gate_order,
                mir_maneuver_gate_to_lir,
            ),
            waiting_zones: LirEntityOrder::from_parts(
                canonical_mir_waiting_zone_order,
                mir_waiting_zone_to_lir,
            ),
            signal_groups: LirEntityOrder::from_parts(
                canonical_mir_signal_group_order,
                mir_signal_group_to_lir,
            ),
            signal_controllers: LirEntityOrder::from_parts(
                canonical_mir_signal_controller_order,
                mir_signal_controller_to_lir,
            ),
            signal_phases: LirEntityOrder::from_parts(
                canonical_mir_signal_phase_order,
                mir_signal_phase_to_lir,
            ),
            parking_areas: LirEntityOrder::from_parts(
                canonical_mir_parking_area_order,
                mir_parking_area_to_lir,
            ),
            parking_spaces: LirEntityOrder::from_parts(
                canonical_mir_parking_space_order,
                mir_parking_space_to_lir,
            ),
            participant_classes: LirEntityOrder::from_parts(
                canonical_mir_participant_class_order,
                mir_participant_class_to_lir,
            ),
            vehicle_profiles: LirEntityOrder::from_parts(
                canonical_mir_vehicle_profile_order,
                mir_vehicle_profile_to_lir,
            ),
            canonical_frames: LirEntityOrder::from_parts(
                canonical_mir_canonical_frame_order,
                mir_canonical_frame_to_lir,
            ),
            access_rules: LirEntityOrder::from_parts(
                canonical_mir_access_rule_order,
                mir_access_rule_to_lir,
            ),
            static_routes: LirEntityOrder::from_parts(
                canonical_mir_static_route_order,
                mir_static_route_to_lir,
            ),
        })
    }
}
