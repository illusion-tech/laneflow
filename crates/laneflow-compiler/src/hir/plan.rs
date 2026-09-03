//! HIR 构建计划（build plan）与内存计划（memory plan）。
//!
//! [`HirBuildPlan::analyze`] 在任何与记录数成正比的阶段分配前，一次性统计编译单元的
//! 公共基础计数与七个领域实体计数，并把持久表、lookup 预算与互斥阶段暂存区聚合为
//! [`HirMemoryPlan`]；[`HirBuildPlan::check_limits`] 在 base 构造前按所选资源配置档
//! 预检限额维度。各分量算术逐点复制自拆分前 `build_hir` 的资源统计段：persistent 与
//! lookup 跨领域求和，互斥 scratch 取峰值；saturating 加法与 max 均可交换结合，分组
//! 聚合与拆分前的单链求值逐字节等价。#374 已记录的估算与实际分配偏差不作修正，
//! 原样保留并在对应位置注释。

use std::collections::HashMap;
use std::sync::Arc;

use laneflow_static_contract::{SignalAspect, StableId128};

use crate::declaration::{
    LaneEdgeDeclaration, LaneEdgeGeometryAuthority, OwnedSignalControl, TypedAstDeclaration,
    TypedAstEntityAddress,
};
use crate::diagnostic::DiagnosticCollector;
use crate::identity::RegisteredCanonicalIdentity;
use crate::{CompilationUnit, CompileLimitDimension, Diagnostic, DiagnosticBundle, SourceLocation};

use super::{
    AccessCandidate, CanonicalAuthoringLaneSource, CanonicalDeclarationSource,
    CanonicalLaneEdgeSource, HirAccessRule, HirAccessRuleKey, HirAccessRuleParticipantClass,
    HirAuthoringLane, HirAuthoringLaneEdge, HirAuthoringLaneKey, HirCanonicalFrame,
    HirCanonicalFrameKey, HirCanonicalPoint2F32, HirCanonicalPoint3F32, HirConflictPassage,
    HirConflictZone, HirConflictZoneKey, HirConflictZoneRegion, HirConflictZoneStream,
    HirCorridorElement, HirDeclaredJunctionEdge, HirFacilityBand, HirFacilityBandGeometry,
    HirFacilityBandKey, HirGeometrySourceRange, HirImport, HirJunction, HirJunctionInternalEdge,
    HirJunctionKey, HirJunctionMovement, HirLaneEdge, HirLaneEdgeGeometry, HirLaneEdgeKey,
    HirLaneEdgeReference, HirLaneGroup, HirLaneGroupKey, HirLaneGroupMember, HirManeuverGate,
    HirManeuverGateKey, HirManeuverPath, HirManeuverPathEdge, HirManeuverPathGate,
    HirManeuverPathKey, HirManeuverPathWaitingZone, HirModule, HirModuleKey, HirMovement,
    HirMovementKey, HirMovementManeuverPath, HirParkingFacility, HirParkingFacilityKey,
    HirParkingFacilitySpace, HirParkingLaneAnchor, HirParkingSpace, HirParkingSpaceKey,
    HirParticipantClass, HirParticipantClassKey, HirParticipantStream, HirParticipantStreamKey,
    HirRoadCorridor, HirRoadCorridorKey, HirRoadSection, HirRoadSectionKey, HirSignalController,
    HirSignalControllerGroup, HirSignalControllerKey, HirSignalGroup, HirSignalGroupKey,
    HirSignalGroupManeuverGate, HirSignalPhase, HirSignalPhaseState, HirSpatialSegment,
    HirStopLine, HirStopLineKey, HirStopLineManeuverGate, HirVehicleProfile, HirVehicleProfileKey,
    HirWaitingZone, HirWaitingZoneKey, ManeuverPathSequence, PendingConflictZoneRegion,
    PendingSpatialGeometry, SpatialFrameAssignment, declaration_header, lane_edge_declaration,
};

/// 公共基础构造或单个领域的三类内存预算分量。
///
/// `persistent_bytes` 与 `lookup_bytes` 跨领域求和；`scratch_bytes` 是互斥工作集，
/// 跨领域取峰值。
pub(super) struct DomainBudget {
    pub(super) persistent_bytes: u64,
    pub(super) lookup_bytes: u64,
    pub(super) scratch_bytes: u64,
}

/// HIR 阶段的内存计划：记录数上界、互斥暂存峰值与三类存续字节。
pub(super) struct HirMemoryPlan {
    pub(super) hir_record_count: u64,
    pub(super) stage_scratch_bytes: u64,
    pub(super) persistent_bytes: u64,
    pub(super) lookup_bytes: u64,
    pub(super) controlled_live_bytes: u64,
}

/// 一次统计形成的 HIR 构建计划：公共基础计数、七领域计数与聚合内存计划。
pub(super) struct HirBuildPlan {
    pub(super) lane_edge_count: u64,
    pub(super) lane_edge_reference_count: u64,
    pub(super) cross_section: CrossSectionCounts,
    pub(super) junction: JunctionCounts,
    pub(super) control: ControlCounts,
    pub(super) signal: SignalCounts,
    pub(super) conflict: ConflictCounts,
    pub(super) parking: ParkingCounts,
    pub(super) spatial: SpatialCounts,
    pub(super) access: AccessCounts,
    pub(super) memory: HirMemoryPlan,
}

impl HirBuildPlan {
    /// 统计全部计数并聚合内存计划；不分配任何与记录数成正比的集合。
    pub(super) fn analyze(unit: &CompilationUnit) -> HirBuildPlan {
        let module_count = u64::try_from(unit.modules.len()).unwrap_or(u64::MAX);
        // 先装配计数，再以各领域预算回填内存计划；各分量公式逐点复制自拆分前
        // `build_hir` 的资源统计段。
        let mut plan = HirBuildPlan {
            lane_edge_count: lane_edge_count(unit),
            lane_edge_reference_count: lane_edge_reference_count(unit),
            cross_section: cross_section_counts(unit),
            junction: junction_counts(unit),
            control: control_counts(unit),
            signal: signal_counts(unit),
            conflict: conflict_counts(unit),
            parking: parking_counts(unit),
            spatial: spatial_counts(unit),
            access: access_counts(unit),
            memory: HirMemoryPlan {
                hir_record_count: 0,
                stage_scratch_bytes: 0,
                persistent_bytes: 0,
                lookup_bytes: 0,
                controlled_live_bytes: 0,
            },
        };
        plan.memory.hir_record_count = module_count
            .saturating_add(unit.import_edge_count)
            .saturating_add(unit.symbol_count)
            .saturating_add(unit.identity_field_occurrence_count)
            .saturating_add(unit.reference_count)
            .saturating_add(unit.relation_occurrence_count)
            // HIR 记录数必须使用实际冻结后的规范点，而不是 RoadEditing source curve 的
            // 控制点计数；细分可能让两者显著不同。
            .saturating_add(plan.spatial.lane_edge_geometries)
            .saturating_add(plan.spatial.facility_band_geometries)
            .saturating_add(plan.spatial.geometry_source_ranges)
            .saturating_add(plan.spatial.canonical_points)
            .saturating_add(plan.spatial.conflict_zone_regions)
            .saturating_add(plan.spatial.conflict_region_points)
            .saturating_add(plan.spatial.spatial_segments)
            // 信号组到机动门的反向使用关系由 HIR 派生，Typed AST 只计正向绑定。
            .saturating_add(plan.signal.controlled_gates)
            // 区域归属在 Typed AST 中按停车位正向引用计数；区域成员表是 HIR 派生反向关系。
            .saturating_add(plan.parking.memberships);
        plan.memory.hir_record_count = plan
            .memory
            .hir_record_count
            .saturating_add(plan.conflict.zone_streams);
        let base = base_budget(
            unit,
            module_count,
            plan.lane_edge_count,
            plan.lane_edge_reference_count,
        );
        let cross_section = cross_section_budget(
            unit,
            module_count,
            plan.lane_edge_count,
            &plan.cross_section,
        );
        let junction = junction_budget(unit, module_count, plan.lane_edge_count, &plan.junction);
        let control = control_budget(
            module_count,
            plan.lane_edge_count,
            plan.lane_edge_reference_count,
            &plan.control,
            &plan.junction,
        );
        let signal = signal_budget(unit, module_count, &plan.signal);
        let conflict = conflict_budget(unit, module_count, &plan.conflict);
        let parking = parking_budget(unit, module_count, &plan.parking);
        let spatial = spatial_budget(
            unit,
            module_count,
            plan.lane_edge_count,
            &plan.spatial,
            &plan.cross_section,
        );
        let access = access_budget(unit, module_count, &plan.access);
        plan.memory.stage_scratch_bytes = base
            .scratch_bytes
            .max(cross_section.scratch_bytes)
            .max(junction.scratch_bytes)
            .max(control.scratch_bytes)
            .max(signal.scratch_bytes)
            .max(conflict.scratch_bytes)
            .max(parking.scratch_bytes)
            .max(spatial.scratch_bytes)
            .max(access.scratch_bytes);
        plan.memory.persistent_bytes = base
            .persistent_bytes
            .saturating_add(cross_section.persistent_bytes)
            .saturating_add(junction.persistent_bytes)
            .saturating_add(control.persistent_bytes)
            .saturating_add(signal.persistent_bytes)
            .saturating_add(conflict.persistent_bytes)
            .saturating_add(parking.persistent_bytes)
            .saturating_add(spatial.persistent_bytes)
            .saturating_add(access.persistent_bytes);
        plan.memory.lookup_bytes = base
            .lookup_bytes
            .saturating_add(cross_section.lookup_bytes)
            .saturating_add(junction.lookup_bytes)
            .saturating_add(control.lookup_bytes)
            .saturating_add(signal.lookup_bytes)
            .saturating_add(conflict.lookup_bytes)
            .saturating_add(parking.lookup_bytes)
            .saturating_add(spatial.lookup_bytes)
            .saturating_add(access.lookup_bytes);
        plan.memory.controlled_live_bytes = unit
            .controlled_live_bytes
            .saturating_add(plan.memory.persistent_bytes)
            .saturating_add(plan.memory.lookup_bytes)
            .saturating_add(plan.memory.stage_scratch_bytes)
            .max(unit.admission_peak_live_bytes);
        plan
    }

    /// 在 base 构造前按所选配置档预检限额维度；任一维度超限即收集诊断并整体早退。
    ///
    /// 维度顺序、诊断构造（首模块 `primary_span`/`stable_key`）与早退语义同拆分前
    /// `build_hir` 的限额校验块。
    pub(super) fn check_limits(&self, unit: &CompilationUnit) -> Result<(), DiagnosticBundle> {
        let primary_span = unit
            .modules
            .first()
            .map(|module| module.declaration_span().clone());
        let stable_key = unit
            .modules
            .first()
            .map(|module| module.descriptor().authoring_namespace_id().into());
        let mut limit_diagnostics =
            DiagnosticCollector::new(unit.limits.value(CompileLimitDimension::DiagnosticCount));
        for (dimension, observed) in [
            (
                CompileLimitDimension::GeometryPointCount,
                self.spatial
                    .canonical_points
                    .saturating_add(self.spatial.conflict_region_points),
            ),
            (
                CompileLimitDimension::HirRecordCount,
                self.memory.hir_record_count,
            ),
            (
                CompileLimitDimension::StageScratchBytes,
                self.memory.stage_scratch_bytes,
            ),
            (
                CompileLimitDimension::CompilerControlledLiveBytes,
                self.memory.controlled_live_bytes,
            ),
        ] {
            if observed > unit.limits.value(dimension) {
                limit_diagnostics.push(Diagnostic::compile_limit_exceeded_at(
                    dimension,
                    unit.limits.value(dimension),
                    observed,
                    primary_span.clone(),
                    stable_key.clone(),
                ));
            }
        }
        if !limit_diagnostics.is_empty() {
            return Err(limit_diagnostics.finish());
        }
        Ok(())
    }
}

/// 公共基础构造（模块轴、导入、LaneEdge 表与规范身份）预算。
fn base_budget(
    unit: &CompilationUnit,
    module_count: u64,
    lane_edge_count: u64,
    lane_edge_reference_count: u64,
) -> DomainBudget {
    let canonical_source_scratch = requested_bytes::<CanonicalLaneEdgeSource>(lane_edge_count)
        .saturating_add(requested_bytes::<usize>(unit.declaration_count));
    // 估算按平坦表 (&str, &SourceLocation) × import_edge_count 计入；实际分配是 base 段
    // 逐模块 Vec collect、无 with_capacity。#374 已记录该估算与实际分配不一致，按行为
    // 不变约束原样保留。
    let import_sort_scratch = requested_bytes::<(&str, &SourceLocation)>(unit.import_edge_count);
    let (canonical_identity_bytes, largest_canonical_identity_bytes) = identity_byte_counts(unit);
    DomainBudget {
        persistent_bytes: requested_bytes::<HirModule>(module_count)
            .saturating_add(requested_bytes::<HirImport>(unit.import_edge_count))
            .saturating_add(requested_bytes::<HirLaneEdge>(lane_edge_count))
            .saturating_add(requested_bytes::<HirLaneEdgeReference>(
                lane_edge_reference_count,
            )),
        lookup_bytes: requested_hash_table_bytes::<Arc<str>, HirModuleKey>(module_count)
            .saturating_add(requested_bytes::<
                HashMap<TypedAstEntityAddress, HirLaneEdgeKey>,
            >(module_count))
            .saturating_add(requested_hash_table_bytes::<
                TypedAstEntityAddress,
                HirLaneEdgeKey,
            >(lane_edge_count))
            .saturating_add(requested_hash_table_bytes::<
                StableId128,
                RegisteredCanonicalIdentity,
            >(unit.declaration_count))
            .saturating_add(canonical_identity_bytes),
        scratch_bytes: canonical_source_scratch
            .max(import_sort_scratch)
            .max(largest_canonical_identity_bytes),
    }
}

fn cross_section_budget(
    unit: &CompilationUnit,
    module_count: u64,
    lane_edge_count: u64,
    counts: &CrossSectionCounts,
) -> DomainBudget {
    let lookup_module_count = if counts.entity_count() == 0 {
        0
    } else {
        module_count
    };
    let scratch_bytes = if counts.entity_count() == 0 {
        0
    } else {
        requested_bytes::<CanonicalDeclarationSource<HirRoadCorridorKey>>(counts.road_corridors)
            .saturating_add(requested_bytes::<
                CanonicalDeclarationSource<HirRoadSectionKey>,
            >(counts.road_sections))
            .saturating_add(requested_bytes::<CanonicalAuthoringLaneSource>(
                counts.authoring_lanes,
            ))
            .saturating_add(
                requested_bytes::<CanonicalDeclarationSource<HirLaneGroupKey>>(counts.lane_groups),
            )
            .saturating_add(requested_bytes::<
                CanonicalDeclarationSource<HirFacilityBandKey>,
            >(counts.facility_bands))
            .saturating_add(requested_bytes::<
                Option<(HirRoadCorridorKey, SourceLocation)>,
            >(
                counts.road_sections.saturating_add(counts.facility_bands)
            ))
            .saturating_add(requested_bytes::<Option<HirAuthoringLaneKey>>(
                lane_edge_count,
            ))
            .saturating_add(requested_bytes::<usize>(
                counts.lane_groups.saturating_mul(2),
            ))
            .saturating_add(requested_bytes::<usize>(unit.declaration_count))
    };
    DomainBudget {
        persistent_bytes: requested_bytes::<HirRoadCorridor>(counts.road_corridors)
            .saturating_add(requested_bytes::<HirCorridorElement>(
                counts.corridor_elements,
            ))
            .saturating_add(requested_bytes::<HirRoadSection>(counts.road_sections))
            .saturating_add(requested_bytes::<HirAuthoringLane>(counts.authoring_lanes))
            .saturating_add(requested_bytes::<HirAuthoringLaneEdge>(
                counts.authoring_lane_edges,
            ))
            .saturating_add(requested_bytes::<HirLaneGroup>(counts.lane_groups))
            .saturating_add(requested_bytes::<HirLaneGroupMember>(
                counts.authoring_lanes,
            ))
            .saturating_add(requested_bytes::<HirFacilityBand>(counts.facility_bands)),
        lookup_bytes: requested_bytes::<HashMap<TypedAstEntityAddress, HirRoadSectionKey>>(
            lookup_module_count,
        )
        .saturating_add(requested_bytes::<
            HashMap<TypedAstEntityAddress, HirLaneGroupKey>,
        >(lookup_module_count))
        .saturating_add(requested_bytes::<
            HashMap<TypedAstEntityAddress, HirFacilityBandKey>,
        >(lookup_module_count))
        .saturating_add(requested_hash_table_bytes::<
            TypedAstEntityAddress,
            HirRoadSectionKey,
        >(counts.road_sections))
        .saturating_add(requested_hash_table_bytes::<
            TypedAstEntityAddress,
            HirLaneGroupKey,
        >(counts.lane_groups))
        .saturating_add(requested_hash_table_bytes::<
            TypedAstEntityAddress,
            HirFacilityBandKey,
        >(counts.facility_bands)),
        scratch_bytes,
    }
}

fn junction_budget(
    unit: &CompilationUnit,
    module_count: u64,
    lane_edge_count: u64,
    counts: &JunctionCounts,
) -> DomainBudget {
    let lookup_module_count = if counts.entity_count() == 0 {
        0
    } else {
        module_count
    };
    let scratch_bytes = if counts.entity_count() == 0 {
        0
    } else {
        requested_bytes::<CanonicalDeclarationSource<HirMovementKey>>(counts.movements)
            .saturating_add(requested_bytes::<
                CanonicalDeclarationSource<HirManeuverPathKey>,
            >(counts.maneuver_paths))
            .saturating_add(requested_bytes::<HirDeclaredJunctionEdge>(
                counts
                    .declared_approach_edges
                    .saturating_add(counts.declared_internal_edges),
            ))
            .saturating_add(requested_bytes::<u8>(lane_edge_count))
            .saturating_add(requested_bytes::<usize>(
                counts
                    .junctions
                    .saturating_add(counts.movements)
                    .saturating_mul(2),
            ))
            .saturating_add(requested_hash_table_bytes::<
                ManeuverPathSequence<'static>,
                HirManeuverPathKey,
            >(counts.maneuver_paths))
            .saturating_add(requested_bytes::<Option<HirJunctionInternalEdge>>(
                lane_edge_count,
            ))
            .saturating_add(requested_bytes::<
                Option<(HirManeuverPathKey, SourceLocation)>,
            >(lane_edge_count))
            .saturating_add(requested_bytes::<usize>(unit.declaration_count))
    };
    DomainBudget {
        persistent_bytes: requested_bytes::<HirJunction>(counts.junctions)
            .saturating_add(requested_bytes::<HirMovement>(counts.movements))
            .saturating_add(requested_bytes::<HirJunctionMovement>(counts.movements))
            .saturating_add(requested_bytes::<HirManeuverPath>(counts.maneuver_paths))
            .saturating_add(requested_bytes::<HirMovementManeuverPath>(
                counts.maneuver_paths,
            ))
            .saturating_add(requested_bytes::<HirManeuverPathEdge>(
                counts.maneuver_path_edges,
            ))
            .saturating_add(requested_bytes::<HirJunctionInternalEdge>(
                lane_edge_count.min(counts.maneuver_path_edges),
            )),
        lookup_bytes: requested_bytes::<HashMap<TypedAstEntityAddress, HirJunctionKey>>(
            lookup_module_count,
        )
        .saturating_add(requested_bytes::<
            HashMap<TypedAstEntityAddress, HirMovementKey>,
        >(lookup_module_count))
        .saturating_add(requested_hash_table_bytes::<
            TypedAstEntityAddress,
            HirJunctionKey,
        >(counts.junctions))
        .saturating_add(requested_hash_table_bytes::<
            TypedAstEntityAddress,
            HirMovementKey,
        >(counts.movements)),
        scratch_bytes,
    }
}

fn control_budget(
    module_count: u64,
    lane_edge_count: u64,
    lane_edge_reference_count: u64,
    counts: &ControlCounts,
    junction_counts: &JunctionCounts,
) -> DomainBudget {
    let lookup_module_count = if counts.entity_count() == 0 {
        0
    } else {
        module_count
    };
    let scratch_bytes = if counts.entity_count() == 0 {
        0
    } else {
        requested_bytes::<CanonicalDeclarationSource<HirStopLineKey>>(counts.stop_lines)
            .saturating_add(requested_bytes::<
                CanonicalDeclarationSource<HirManeuverGateKey>,
            >(counts.maneuver_gates))
            .saturating_add(requested_bytes::<
                CanonicalDeclarationSource<HirWaitingZoneKey>,
            >(counts.waiting_zones))
            .saturating_add(requested_bytes::<usize>(
                counts
                    .stop_lines
                    .saturating_add(junction_counts.maneuver_paths)
                    .saturating_mul(2),
            ))
            .saturating_add(requested_bytes::<Option<HirStopLineKey>>(lane_edge_count))
            .saturating_add(requested_bytes::<u8>(
                counts
                    .stop_lines
                    .saturating_add(junction_counts.maneuver_paths)
                    .saturating_add(lane_edge_reference_count)
                    .saturating_add(lane_edge_count),
            ))
            .saturating_add(requested_bytes::<HirManeuverGateKey>(
                counts.maneuver_gates.saturating_mul(2),
            ))
            .saturating_add(requested_bytes::<HirWaitingZoneKey>(counts.waiting_zones))
    };
    DomainBudget {
        persistent_bytes: requested_bytes::<HirStopLine>(counts.stop_lines)
            .saturating_add(requested_bytes::<HirManeuverGate>(counts.maneuver_gates))
            .saturating_add(requested_bytes::<HirWaitingZone>(counts.waiting_zones))
            .saturating_add(requested_bytes::<HirManeuverPathGate>(
                counts.maneuver_gates,
            ))
            .saturating_add(requested_bytes::<HirManeuverPathWaitingZone>(
                counts.waiting_zones,
            ))
            .saturating_add(requested_bytes::<HirStopLineManeuverGate>(
                counts.maneuver_gates,
            )),
        lookup_bytes: requested_bytes::<HashMap<TypedAstEntityAddress, HirManeuverPathKey>>(
            lookup_module_count,
        )
        .saturating_add(requested_bytes::<
            HashMap<TypedAstEntityAddress, HirStopLineKey>,
        >(lookup_module_count))
        .saturating_add(requested_bytes::<
            HashMap<TypedAstEntityAddress, HirManeuverGateKey>,
        >(lookup_module_count))
        .saturating_add(requested_hash_table_bytes::<
            TypedAstEntityAddress,
            HirManeuverPathKey,
        >(junction_counts.maneuver_paths))
        .saturating_add(requested_hash_table_bytes::<
            TypedAstEntityAddress,
            HirStopLineKey,
        >(counts.stop_lines))
        .saturating_add(requested_hash_table_bytes::<
            TypedAstEntityAddress,
            HirManeuverGateKey,
        >(counts.maneuver_gates)),
        scratch_bytes,
    }
}

fn signal_budget(unit: &CompilationUnit, module_count: u64, counts: &SignalCounts) -> DomainBudget {
    let lookup_module_count = if counts.entity_count() == 0 {
        0
    } else {
        module_count
    };
    let scratch_bytes = if counts.entity_count() == 0 {
        0
    } else {
        requested_bytes::<CanonicalDeclarationSource<HirSignalGroupKey>>(counts.groups)
            .saturating_add(requested_bytes::<
                CanonicalDeclarationSource<HirSignalControllerKey>,
            >(counts.controllers))
            .saturating_add(requested_bytes::<
                Option<(HirSignalControllerKey, SourceLocation)>,
            >(counts.groups))
            // 相位状态暂存按 Option<(SignalAspect, SourceLocation)> × groups 估算；实际按
            // 单控制器 resolved_groups.len() 逐相位分配。#374 已记录该估算与实际分配不
            // 一致，按行为不变约束原样保留。
            .saturating_add(requested_bytes::<Option<(SignalAspect, SourceLocation)>>(
                counts.groups,
            ))
            .saturating_add(requested_bytes::<usize>(counts.groups.saturating_mul(3)))
            .saturating_add(requested_bytes::<HirSignalGroupKey>(
                counts.controller_groups,
            ))
            .saturating_add(requested_bytes::<(HirSignalGroupKey, HirManeuverGateKey)>(
                counts.controlled_gates,
            ))
            // seen 表按 controller_groups 总量估算；实际容量取单声明的
            // source.signal_groups.len()。#374 已记录该口径偏差，原样保留。
            .saturating_add(requested_hash_table_bytes::<
                HirSignalGroupKey,
                SourceLocation,
            >(counts.controller_groups))
            .saturating_add(requested_hash_table_bytes::<HirSignalGroupKey, usize>(
                counts.controller_groups,
            ))
            .saturating_add(requested_hash_table_bytes::<Arc<str>, SourceLocation>(
                counts.phases,
            ))
            .saturating_add(requested_bytes::<usize>(unit.declaration_count))
    };
    DomainBudget {
        persistent_bytes: requested_bytes::<HirSignalGroup>(counts.groups)
            .saturating_add(requested_bytes::<HirSignalController>(counts.controllers))
            .saturating_add(requested_bytes::<HirSignalControllerGroup>(
                counts.controller_groups,
            ))
            .saturating_add(requested_bytes::<HirSignalPhase>(counts.phases))
            .saturating_add(requested_bytes::<HirSignalPhaseState>(counts.phase_states))
            .saturating_add(requested_bytes::<HirSignalGroupManeuverGate>(
                counts.controlled_gates,
            )),
        lookup_bytes: requested_bytes::<HashMap<TypedAstEntityAddress, HirSignalGroupKey>>(
            lookup_module_count,
        )
        .saturating_add(requested_hash_table_bytes::<
            TypedAstEntityAddress,
            HirSignalGroupKey,
        >(counts.groups)),
        scratch_bytes,
    }
}

fn parking_budget(
    unit: &CompilationUnit,
    module_count: u64,
    counts: &ParkingCounts,
) -> DomainBudget {
    let lookup_module_count = if counts.entity_count() == 0 {
        0
    } else {
        module_count
    };
    let scratch_bytes = if counts.entity_count() == 0 {
        0
    } else {
        requested_bytes::<CanonicalDeclarationSource<HirParkingFacilityKey>>(counts.areas)
            // 来源暂存按 12 字节 CanonicalDeclarationSource<HirParkingSpaceKey> 估算；
            // 实际为 8 字节 Vec<(u32, u32)>、键按下标重建，方向为估算偏保守。#374 已
            // 记录该偏差，原样保留。
            .saturating_add(requested_bytes::<
                CanonicalDeclarationSource<HirParkingSpaceKey>,
            >(counts.spaces))
            .saturating_add(requested_bytes::<bool>(counts.areas))
            .saturating_add(
                requested_bytes::<(HirParkingFacilityKey, HirParkingSpaceKey)>(counts.memberships),
            )
            .saturating_add(requested_bytes::<usize>(unit.declaration_count))
    };
    DomainBudget {
        persistent_bytes: requested_bytes::<HirParkingFacility>(counts.areas)
            .saturating_add(requested_bytes::<HirParkingSpace>(counts.spaces))
            .saturating_add(requested_bytes::<HirParkingFacilitySpace>(
                counts.memberships,
            ))
            .saturating_add(requested_bytes::<HirParkingLaneAnchor>(
                counts.virtual_entries.saturating_add(counts.virtual_exits),
            )),
        lookup_bytes: requested_bytes::<HashMap<TypedAstEntityAddress, HirParkingFacilityKey>>(
            lookup_module_count,
        )
        .saturating_add(requested_hash_table_bytes::<
            TypedAstEntityAddress,
            HirParkingFacilityKey,
        >(counts.areas)),
        scratch_bytes,
    }
}

fn conflict_budget(
    unit: &CompilationUnit,
    module_count: u64,
    counts: &ConflictCounts,
) -> DomainBudget {
    let lookup_module_count = if counts.entity_count() == 0 {
        0
    } else {
        module_count
    };
    let scratch_bytes = if counts.entity_count() == 0 {
        0
    } else {
        requested_bytes::<(u32, u32, HirConflictZoneKey)>(counts.zones)
            .saturating_add(requested_bytes::<(u32, u32, HirParticipantStreamKey)>(
                counts.streams,
            ))
            .saturating_add(requested_bytes::<HirParticipantStreamKey>(counts.passages))
            .saturating_add(requested_bytes::<(
                HirConflictZoneKey,
                super::HirManeuverPathKey,
                (u32, u32),
                (u32, u32),
                laneflow_static_contract::ParticipantStreamId,
                HirParticipantStreamKey,
            )>(counts.passages))
            .saturating_add(requested_bytes::<(
                HirConflictZoneKey,
                HirParticipantStreamKey,
            )>(counts.passages))
            .saturating_add(requested_bytes::<HirConflictZoneKey>(counts.passages))
            .saturating_add(requested_bytes::<usize>(unit.declaration_count))
            .saturating_add(requested_bytes::<
                HashMap<TypedAstEntityAddress, HirJunctionKey>,
            >(lookup_module_count))
            .saturating_add(requested_bytes::<
                HashMap<TypedAstEntityAddress, super::HirManeuverPathKey>,
            >(lookup_module_count))
            .saturating_add(requested_bytes::<
                HashMap<TypedAstEntityAddress, HirManeuverGateKey>,
            >(lookup_module_count))
            .saturating_add(requested_hash_table_bytes::<
                TypedAstEntityAddress,
                HirJunctionKey,
            >(unit.declaration_count))
            .saturating_add(requested_hash_table_bytes::<
                TypedAstEntityAddress,
                super::HirManeuverPathKey,
            >(unit.declaration_count))
            .saturating_add(requested_hash_table_bytes::<
                TypedAstEntityAddress,
                HirManeuverGateKey,
            >(unit.declaration_count))
    };
    DomainBudget {
        persistent_bytes: requested_bytes::<HirConflictZone>(counts.zones)
            .saturating_add(requested_bytes::<HirParticipantStream>(counts.streams))
            .saturating_add(requested_bytes::<HirConflictPassage>(counts.passages))
            .saturating_add(requested_bytes::<HirConflictZoneStream>(
                counts.zone_streams,
            )),
        lookup_bytes: requested_bytes::<HashMap<TypedAstEntityAddress, HirConflictZoneKey>>(
            lookup_module_count,
        )
        .saturating_add(requested_hash_table_bytes::<
            TypedAstEntityAddress,
            HirConflictZoneKey,
        >(counts.zones)),
        scratch_bytes,
    }
}

fn spatial_budget(
    unit: &CompilationUnit,
    module_count: u64,
    lane_edge_count: u64,
    counts: &SpatialCounts,
    cross_section_counts: &CrossSectionCounts,
) -> DomainBudget {
    let scratch_bytes = if counts.canonical_frames == 0
        && counts.lane_edge_geometries == 0
        && counts.facility_band_geometries == 0
        && counts.conflict_zone_regions == 0
    {
        0
    } else {
        requested_bytes::<usize>(unit.declaration_count)
            .saturating_add(requested_bytes::<Option<PendingSpatialGeometry<'static>>>(
                lane_edge_count,
            ))
            .saturating_add(requested_bytes::<Option<PendingSpatialGeometry<'static>>>(
                cross_section_counts.facility_bands,
            ))
            .saturating_add(requested_bytes::<Option<SpatialFrameAssignment>>(
                lane_edge_count,
            ))
            .saturating_add(requested_bytes::<Option<SpatialFrameAssignment>>(
                cross_section_counts.facility_bands,
            ))
            .saturating_add(requested_bytes::<Option<usize>>(lane_edge_count))
            .saturating_add(requested_bytes::<u8>(lane_edge_count))
            .saturating_add(requested_bytes::<HirLaneEdgeKey>(
                counts.lane_edge_geometries,
            ))
            .saturating_add(requested_bytes::<HirFacilityBandKey>(
                counts.facility_band_geometries,
            ))
            // spatial 的 per-module HashMap 计入 scratch；本领域不在 lookup 模块清单。
            .saturating_add(requested_bytes::<
                HashMap<TypedAstEntityAddress, HirCanonicalFrameKey>,
            >(module_count))
            .saturating_add(requested_bytes::<
                HashMap<TypedAstEntityAddress, HirFacilityBandKey>,
            >(module_count))
            .saturating_add(requested_hash_table_bytes::<
                TypedAstEntityAddress,
                HirCanonicalFrameKey,
            >(counts.canonical_frames))
            .saturating_add(requested_hash_table_bytes::<
                TypedAstEntityAddress,
                HirFacilityBandKey,
            >(cross_section_counts.facility_bands))
            .saturating_add(requested_bytes::<PendingConflictZoneRegion<'static>>(
                counts.conflict_zone_regions,
            ))
            .saturating_add(requested_bytes::<Option<SourceLocation>>(
                counts.conflict_zone_regions,
            ))
            .saturating_add(requested_bytes::<HirCanonicalPoint2F32>(
                counts.conflict_region_points,
            ))
            .saturating_add(requested_bytes::<(usize, HirCanonicalPoint2F32)>(
                counts.conflict_region_points,
            ))
    };
    DomainBudget {
        persistent_bytes: requested_bytes::<HirCanonicalFrame>(counts.canonical_frames)
            .saturating_add(requested_bytes::<HirLaneEdgeGeometry>(
                counts.lane_edge_geometries,
            ))
            .saturating_add(requested_bytes::<HirFacilityBandGeometry>(
                counts.facility_band_geometries,
            ))
            .saturating_add(requested_bytes::<HirGeometrySourceRange>(
                counts.geometry_source_ranges,
            ))
            .saturating_add(requested_bytes::<HirCanonicalPoint3F32>(
                counts.canonical_points,
            ))
            .saturating_add(requested_bytes::<HirSpatialSegment>(
                counts.spatial_segments,
            ))
            .saturating_add(requested_bytes::<HirConflictZoneRegion>(
                counts.conflict_zone_regions,
            ))
            .saturating_add(requested_bytes::<HirCanonicalPoint2F32>(
                counts.conflict_region_points,
            )),
        // spatial 不在 lookup 模块清单。
        lookup_bytes: 0,
        scratch_bytes,
    }
}

fn access_budget(unit: &CompilationUnit, module_count: u64, counts: &AccessCounts) -> DomainBudget {
    let lookup_module_count = if counts.entity_count() == 0 {
        0
    } else {
        module_count
    };
    let scratch_bytes = if counts.entity_count() == 0 {
        0
    } else {
        requested_bytes::<CanonicalDeclarationSource<HirParticipantClassKey>>(
            counts.participant_classes,
        )
        .saturating_add(requested_bytes::<
            CanonicalDeclarationSource<HirVehicleProfileKey>,
        >(counts.vehicle_profiles))
        .saturating_add(requested_bytes::<
            CanonicalDeclarationSource<HirAccessRuleKey>,
        >(counts.access_rules))
        .saturating_add(requested_bytes::<Option<HirParticipantClassKey>>(
            counts.participant_classes.saturating_mul(2),
        ))
        .saturating_add(requested_bytes::<u8>(counts.participant_classes))
        .saturating_add(requested_bytes::<(HirParticipantClassKey, bool)>(
            counts.participant_classes.saturating_mul(2),
        ))
        .saturating_add(requested_bytes::<HirAccessRuleParticipantClass>(
            counts.rule_class_references,
        ))
        .saturating_add(requested_bytes::<AccessCandidate>(
            counts.rule_class_references,
        ))
        .saturating_add(requested_bytes::<usize>(unit.declaration_count))
    };
    DomainBudget {
        persistent_bytes: requested_bytes::<HirParticipantClass>(counts.participant_classes)
            .saturating_add(requested_bytes::<HirVehicleProfile>(
                counts.vehicle_profiles,
            ))
            .saturating_add(requested_bytes::<HirAccessRule>(counts.access_rules))
            .saturating_add(requested_bytes::<HirAccessRuleParticipantClass>(
                counts.rule_class_references,
            )),
        lookup_bytes: requested_bytes::<HashMap<TypedAstEntityAddress, HirParticipantClassKey>>(
            lookup_module_count,
        )
        .saturating_add(requested_hash_table_bytes::<
            TypedAstEntityAddress,
            HirParticipantClassKey,
        >(counts.participant_classes)),
        scratch_bytes,
    }
}

#[derive(Default)]
pub(super) struct CrossSectionCounts {
    pub(super) road_corridors: u64,
    pub(super) corridor_elements: u64,
    pub(super) road_sections: u64,
    pub(super) authoring_lanes: u64,
    pub(super) authoring_lane_edges: u64,
    pub(super) lane_groups: u64,
    pub(super) facility_bands: u64,
}

impl CrossSectionCounts {
    pub(super) fn entity_count(&self) -> u64 {
        self.road_corridors
            .saturating_add(self.road_sections)
            .saturating_add(self.authoring_lanes)
            .saturating_add(self.lane_groups)
            .saturating_add(self.facility_bands)
    }
}

#[derive(Default)]
pub(super) struct JunctionCounts {
    pub(super) junctions: u64,
    pub(super) movements: u64,
    pub(super) maneuver_paths: u64,
    pub(super) maneuver_path_edges: u64,
    pub(super) declared_approach_edges: u64,
    pub(super) declared_internal_edges: u64,
}

impl JunctionCounts {
    pub(super) fn entity_count(&self) -> u64 {
        self.junctions
            .saturating_add(self.movements)
            .saturating_add(self.maneuver_paths)
    }
}

#[derive(Default)]
pub(super) struct ControlCounts {
    pub(super) stop_lines: u64,
    pub(super) maneuver_gates: u64,
    pub(super) waiting_zones: u64,
}

impl ControlCounts {
    pub(super) fn entity_count(&self) -> u64 {
        self.stop_lines
            .saturating_add(self.maneuver_gates)
            .saturating_add(self.waiting_zones)
    }
}

#[derive(Default)]
pub(super) struct SignalCounts {
    pub(super) groups: u64,
    pub(super) controllers: u64,
    pub(super) controller_groups: u64,
    pub(super) phases: u64,
    pub(super) phase_states: u64,
    pub(super) controlled_gates: u64,
}

impl SignalCounts {
    pub(super) fn entity_count(&self) -> u64 {
        self.groups
            .saturating_add(self.controllers)
            .saturating_add(self.phases)
    }
}

#[derive(Default)]
pub(super) struct ParkingCounts {
    pub(super) areas: u64,
    pub(super) spaces: u64,
    pub(super) memberships: u64,
    pub(super) virtual_entries: u64,
    pub(super) virtual_exits: u64,
}

#[derive(Default)]
pub(super) struct ConflictCounts {
    pub(super) zones: u64,
    pub(super) streams: u64,
    pub(super) passages: u64,
    pub(super) zone_streams: u64,
}

impl ConflictCounts {
    pub(super) fn entity_count(&self) -> u64 {
        self.zones.saturating_add(self.streams)
    }
}

impl ParkingCounts {
    pub(super) fn entity_count(&self) -> u64 {
        self.areas.saturating_add(self.spaces)
    }
}

#[derive(Default)]
pub(super) struct SpatialCounts {
    pub(super) canonical_frames: u64,
    pub(super) lane_edge_geometries: u64,
    pub(super) facility_band_geometries: u64,
    pub(super) geometry_source_ranges: u64,
    pub(super) canonical_points: u64,
    pub(super) spatial_segments: u64,
    pub(super) conflict_zone_regions: u64,
    pub(super) conflict_region_points: u64,
}

#[derive(Default)]
pub(super) struct AccessCounts {
    pub(super) participant_classes: u64,
    pub(super) vehicle_profiles: u64,
    pub(super) access_rules: u64,
    pub(super) rule_class_references: u64,
}

impl AccessCounts {
    pub(super) fn entity_count(&self) -> u64 {
        self.participant_classes
            .saturating_add(self.access_rules)
            .saturating_add(self.vehicle_profiles)
    }
}

fn lane_edge_count(unit: &CompilationUnit) -> u64 {
    unit.modules
        .iter()
        .flat_map(|module| module.declarations.iter())
        .filter(|declaration| matches!(declaration, TypedAstDeclaration::LaneEdge(_)))
        .count()
        .try_into()
        .unwrap_or(u64::MAX)
}

fn lane_edge_reference_count(unit: &CompilationUnit) -> u64 {
    unit.modules
        .iter()
        .flat_map(|module| module.declarations.iter())
        .filter_map(lane_edge_declaration)
        .fold(0_u64, |total, declaration| {
            total.saturating_add(u64::try_from(declaration.successors.len()).unwrap_or(u64::MAX))
        })
}

pub(super) fn cross_section_counts(unit: &CompilationUnit) -> CrossSectionCounts {
    let mut counts = CrossSectionCounts::default();
    for declaration in unit
        .modules
        .iter()
        .flat_map(|module| module.declarations.iter())
    {
        match declaration {
            TypedAstDeclaration::LaneEdge(_) => {}
            TypedAstDeclaration::RoadCorridor(corridor) => {
                counts.road_corridors = counts.road_corridors.saturating_add(1);
                counts.corridor_elements = counts
                    .corridor_elements
                    .saturating_add(u64::try_from(corridor.elements.len()).unwrap_or(u64::MAX));
            }
            TypedAstDeclaration::RoadSection(section) => {
                counts.road_sections = counts.road_sections.saturating_add(1);
                counts.authoring_lanes = counts
                    .authoring_lanes
                    .saturating_add(u64::try_from(section.lanes.len()).unwrap_or(u64::MAX));
                counts.authoring_lane_edges =
                    counts
                        .authoring_lane_edges
                        .saturating_add(section.lanes.iter().fold(0_u64, |total, lane| {
                            total.saturating_add(
                                u64::try_from(lane.edge_chain.len()).unwrap_or(u64::MAX),
                            )
                        }));
            }
            TypedAstDeclaration::LaneGroup(_) => {
                counts.lane_groups = counts.lane_groups.saturating_add(1);
            }
            TypedAstDeclaration::FacilityBand(_) => {
                counts.facility_bands = counts.facility_bands.saturating_add(1);
            }
            TypedAstDeclaration::RightOfWayPolicySet(_)
            | TypedAstDeclaration::Junction(_)
            | TypedAstDeclaration::Movement(_)
            | TypedAstDeclaration::ManeuverPath(_)
            | TypedAstDeclaration::StopLine(_)
            | TypedAstDeclaration::ManeuverGate(_)
            | TypedAstDeclaration::WaitingZone(_)
            | TypedAstDeclaration::SignalGroup(_)
            | TypedAstDeclaration::SignalController(_)
            | TypedAstDeclaration::ParkingFacility(_)
            | TypedAstDeclaration::ParkingSpace(_)
            | TypedAstDeclaration::ParticipantClass(_)
            | TypedAstDeclaration::VehicleProfile(_)
            | TypedAstDeclaration::CanonicalFrame(_)
            | TypedAstDeclaration::ConflictZone(_)
            | TypedAstDeclaration::ParticipantStream(_)
            | TypedAstDeclaration::AccessRule(_) => {}
        }
    }
    counts
}

pub(super) fn junction_counts(unit: &CompilationUnit) -> JunctionCounts {
    let mut counts = JunctionCounts::default();
    for declaration in unit
        .modules
        .iter()
        .flat_map(|module| module.declarations.iter())
    {
        match declaration {
            TypedAstDeclaration::Junction(junction) => {
                counts.junctions = counts.junctions.saturating_add(1);
                counts.declared_approach_edges = counts.declared_approach_edges.saturating_add(
                    u64::try_from(junction.approach_edges.len()).unwrap_or(u64::MAX),
                );
                counts.declared_internal_edges = counts.declared_internal_edges.saturating_add(
                    u64::try_from(junction.internal_edges.len()).unwrap_or(u64::MAX),
                );
            }
            TypedAstDeclaration::Movement(_) => {
                counts.movements = counts.movements.saturating_add(1);
            }
            TypedAstDeclaration::ManeuverPath(path) => {
                counts.maneuver_paths = counts.maneuver_paths.saturating_add(1);
                counts.maneuver_path_edges = counts.maneuver_path_edges.saturating_add(
                    u64::try_from(path.internal_edges.len())
                        .unwrap_or(u64::MAX)
                        .saturating_add(2),
                );
            }
            _ => {}
        }
    }
    counts
}

pub(super) fn control_counts(unit: &CompilationUnit) -> ControlCounts {
    let mut counts = ControlCounts {
        maneuver_gates: unit.maneuver_gate_count,
        waiting_zones: unit.waiting_zone_count,
        ..ControlCounts::default()
    };
    for declaration in unit
        .modules
        .iter()
        .flat_map(|module| module.declarations.iter())
    {
        match declaration {
            TypedAstDeclaration::StopLine(_) => {
                counts.stop_lines = counts.stop_lines.saturating_add(1);
            }
            TypedAstDeclaration::ManeuverGate(_) | TypedAstDeclaration::WaitingZone(_) => {}
            _ => {}
        }
    }
    counts
}

pub(super) fn signal_counts(unit: &CompilationUnit) -> SignalCounts {
    let mut counts = SignalCounts::default();
    for declaration in unit
        .modules
        .iter()
        .flat_map(|module| module.declarations.iter())
    {
        match declaration {
            TypedAstDeclaration::SignalGroup(_) => {
                counts.groups = counts.groups.saturating_add(1);
            }
            TypedAstDeclaration::SignalController(controller) => {
                counts.controllers = counts.controllers.saturating_add(1);
                counts.controller_groups = counts.controller_groups.saturating_add(
                    u64::try_from(controller.signal_groups.len()).unwrap_or(u64::MAX),
                );
                counts.phases = counts
                    .phases
                    .saturating_add(u64::try_from(controller.phases.len()).unwrap_or(u64::MAX));
                counts.phase_states =
                    counts
                        .phase_states
                        .saturating_add(controller.phases.iter().fold(0_u64, |total, phase| {
                            total.saturating_add(
                                u64::try_from(phase.states.len()).unwrap_or(u64::MAX),
                            )
                        }));
            }
            TypedAstDeclaration::ManeuverGate(gate)
                if matches!(gate.signal_control, OwnedSignalControl::Group(_)) =>
            {
                counts.controlled_gates = counts.controlled_gates.saturating_add(1);
            }
            _ => {}
        }
    }
    counts
}

pub(super) fn parking_counts(unit: &CompilationUnit) -> ParkingCounts {
    let mut counts = ParkingCounts::default();
    for declaration in unit
        .modules
        .iter()
        .flat_map(|module| module.declarations.iter())
    {
        match declaration {
            TypedAstDeclaration::ParkingFacility(facility) => {
                counts.areas = counts.areas.saturating_add(1);
                counts.virtual_entries = counts.virtual_entries.saturating_add(
                    u64::try_from(facility.virtual_entries.len()).unwrap_or(u64::MAX),
                );
                counts.virtual_exits = counts.virtual_exits.saturating_add(
                    u64::try_from(facility.virtual_exits.len()).unwrap_or(u64::MAX),
                );
            }
            TypedAstDeclaration::ParkingSpace(space) => {
                counts.spaces = counts.spaces.saturating_add(1);
                if space.parking_facility.is_some() {
                    counts.memberships = counts.memberships.saturating_add(1);
                }
            }
            _ => {}
        }
    }
    counts
}

pub(super) fn conflict_counts(unit: &CompilationUnit) -> ConflictCounts {
    let mut counts = ConflictCounts::default();
    for declaration in unit
        .modules
        .iter()
        .flat_map(|module| module.declarations.iter())
    {
        match declaration {
            TypedAstDeclaration::ConflictZone(_) => {
                counts.zones = counts.zones.saturating_add(1);
            }
            TypedAstDeclaration::ParticipantStream(stream) => {
                counts.streams = counts.streams.saturating_add(1);
                let passages = u64::try_from(stream.passages.len()).unwrap_or(u64::MAX);
                counts.passages = counts.passages.saturating_add(passages);
                counts.zone_streams = counts.zone_streams.saturating_add(passages);
            }
            _ => {}
        }
    }
    counts
}

pub(super) fn access_counts(unit: &CompilationUnit) -> AccessCounts {
    let mut counts = AccessCounts::default();
    for declaration in unit
        .modules
        .iter()
        .flat_map(|module| module.declarations.iter())
    {
        match declaration {
            TypedAstDeclaration::ParticipantClass(_) => {
                counts.participant_classes = counts.participant_classes.saturating_add(1);
            }
            TypedAstDeclaration::VehicleProfile(_) => {
                counts.vehicle_profiles = counts.vehicle_profiles.saturating_add(1);
            }
            TypedAstDeclaration::AccessRule(rule) => {
                counts.access_rules = counts.access_rules.saturating_add(1);
                counts.rule_class_references = counts.rule_class_references.saturating_add(
                    u64::try_from(rule.participant_classes.len()).unwrap_or(u64::MAX),
                );
            }
            _ => {}
        }
    }
    counts
}

pub(super) fn spatial_counts(unit: &CompilationUnit) -> SpatialCounts {
    let mut counts = SpatialCounts::default();
    for declaration in unit
        .modules
        .iter()
        .flat_map(|module| module.declarations.iter())
    {
        match declaration {
            TypedAstDeclaration::CanonicalFrame(frame) => {
                counts.canonical_frames = counts.canonical_frames.saturating_add(1);
                counts.lane_edge_geometries = counts.lane_edge_geometries.saturating_add(
                    u64::try_from(frame.lane_edge_geometries.len()).unwrap_or(u64::MAX),
                );
                for geometry in &frame.lane_edge_geometries {
                    let points =
                        u64::try_from(geometry.centerline_points.len()).unwrap_or(u64::MAX);
                    counts.canonical_points = counts.canonical_points.saturating_add(points);
                    counts.spatial_segments = counts
                        .spatial_segments
                        .saturating_add(points.saturating_sub(1));
                }
            }
            TypedAstDeclaration::LaneEdge(LaneEdgeDeclaration {
                geometry_authority: LaneEdgeGeometryAuthority::Compiled(geometry),
                ..
            }) => {
                counts.lane_edge_geometries = counts.lane_edge_geometries.saturating_add(1);
                counts.geometry_source_ranges = counts.geometry_source_ranges.saturating_add(
                    u64::try_from(geometry.source_ranges.len()).unwrap_or(u64::MAX),
                );
                let points = u64::try_from(geometry.centerline_points.len()).unwrap_or(u64::MAX);
                counts.canonical_points = counts.canonical_points.saturating_add(points);
                counts.spatial_segments = counts
                    .spatial_segments
                    .saturating_add(points.saturating_sub(1));
            }
            TypedAstDeclaration::FacilityBand(band) => {
                if let Some(geometry) = &band.compiled_geometry {
                    counts.facility_band_geometries =
                        counts.facility_band_geometries.saturating_add(1);
                    counts.geometry_source_ranges = counts.geometry_source_ranges.saturating_add(
                        u64::try_from(geometry.source_ranges.len()).unwrap_or(u64::MAX),
                    );
                    counts.canonical_points = counts.canonical_points.saturating_add(
                        u64::try_from(geometry.centerline_points.len()).unwrap_or(u64::MAX),
                    );
                }
            }
            _ => {}
        }
    }
    for module in &unit.modules {
        counts.conflict_zone_regions = counts
            .conflict_zone_regions
            .saturating_add(u64::try_from(module.conflict_zone_regions.len()).unwrap_or(u64::MAX));
        counts.conflict_region_points =
            counts
                .conflict_region_points
                .saturating_add(module.conflict_zone_regions.iter().fold(
                    0_u64,
                    |total, region| {
                        total
                            .saturating_add(u64::try_from(region.ring_xz.len()).unwrap_or(u64::MAX))
                    },
                ));
    }
    counts
}

fn identity_byte_counts(unit: &CompilationUnit) -> (u64, u64) {
    let mut total = 0_u64;
    let mut largest = 0_u64;
    for module in &unit.modules {
        let namespace_bytes =
            u64::try_from(module.descriptor().authoring_namespace_id().len()).unwrap_or(u64::MAX);
        for source_declaration in &module.declarations {
            let header = declaration_header(source_declaration);
            let bytes = match source_declaration {
                TypedAstDeclaration::LaneEdge(_)
                | TypedAstDeclaration::RoadCorridor(_)
                | TypedAstDeclaration::Junction(_) => 22_u64
                    .saturating_add(namespace_bytes)
                    .saturating_add(u64::try_from(header.stable_key.len()).unwrap_or(u64::MAX)),
                TypedAstDeclaration::RoadSection(_)
                | TypedAstDeclaration::LaneGroup(_)
                | TypedAstDeclaration::FacilityBand(_) => 44_u64
                    .saturating_add(namespace_bytes)
                    .saturating_add(u64::try_from(header.stable_key.len()).unwrap_or(u64::MAX)),
                TypedAstDeclaration::Movement(movement) => 56_u64
                    .saturating_add(namespace_bytes)
                    .saturating_add(u64::try_from(header.stable_key.len()).unwrap_or(u64::MAX))
                    .saturating_add(
                        u64::try_from(movement.directed_entry_approach_key.len())
                            .unwrap_or(u64::MAX),
                    )
                    .saturating_add(
                        u64::try_from(movement.directed_exit_approach_key.len())
                            .unwrap_or(u64::MAX),
                    ),
                TypedAstDeclaration::ManeuverPath(_) => 88_u64
                    .saturating_add(namespace_bytes)
                    .saturating_add(u64::try_from(header.stable_key.len()).unwrap_or(u64::MAX)),
                TypedAstDeclaration::StopLine(_) => 22_u64
                    .saturating_add(namespace_bytes)
                    .saturating_add(u64::try_from(header.stable_key.len()).unwrap_or(u64::MAX)),
                TypedAstDeclaration::ManeuverGate(_) | TypedAstDeclaration::WaitingZone(_) => {
                    44_u64
                        .saturating_add(namespace_bytes)
                        .saturating_add(u64::try_from(header.stable_key.len()).unwrap_or(u64::MAX))
                }
                TypedAstDeclaration::SignalGroup(_)
                | TypedAstDeclaration::SignalController(_)
                | TypedAstDeclaration::ParkingFacility(_)
                | TypedAstDeclaration::ParkingSpace(_)
                | TypedAstDeclaration::ParticipantClass(_)
                | TypedAstDeclaration::VehicleProfile(_)
                | TypedAstDeclaration::CanonicalFrame(_)
                | TypedAstDeclaration::RightOfWayPolicySet(_)
                | TypedAstDeclaration::AccessRule(_) => 22_u64
                    .saturating_add(namespace_bytes)
                    .saturating_add(u64::try_from(header.stable_key.len()).unwrap_or(u64::MAX)),
                TypedAstDeclaration::ConflictZone(_)
                | TypedAstDeclaration::ParticipantStream(_) => 44_u64
                    .saturating_add(namespace_bytes)
                    .saturating_add(u64::try_from(header.stable_key.len()).unwrap_or(u64::MAX)),
            };
            total = total.saturating_add(bytes);
            largest = largest.max(bytes);
            if let TypedAstDeclaration::RoadSection(section) = source_declaration {
                for lane in &section.lanes {
                    let lane_bytes = 44_u64.saturating_add(namespace_bytes).saturating_add(
                        u64::try_from(lane.header.stable_key.len()).unwrap_or(u64::MAX),
                    );
                    total = total.saturating_add(lane_bytes);
                    largest = largest.max(lane_bytes);
                }
            }
            if let TypedAstDeclaration::SignalController(controller) = source_declaration {
                for phase in &controller.phases {
                    let phase_bytes = 44_u64.saturating_add(namespace_bytes).saturating_add(
                        u64::try_from(phase.header.stable_key.len()).unwrap_or(u64::MAX),
                    );
                    total = total.saturating_add(phase_bytes);
                    largest = largest.max(phase_bytes);
                }
            }
        }
    }
    (total, largest)
}

fn requested_bytes<T>(count: u64) -> u64 {
    count.saturating_mul(u64::try_from(size_of::<T>()).unwrap_or(u64::MAX))
}

fn requested_hash_table_bytes<K, V>(entry_count: u64) -> u64 {
    if entry_count == 0 {
        return 0;
    }
    // 标准库不公开桶分配布局。预检为每个请求项预留八个桶，并额外计入每桶控制字节
    // 与一组尾部控制区，覆盖小表最小桶数和负载因子取整，而不依赖哈希表迭代顺序。
    // 真实生产基准仍须另报实际容量和进程内存，不能用本预算冒充测量。
    let bucket_bytes = u64::try_from(size_of::<(K, V)>())
        .unwrap_or(u64::MAX)
        .saturating_add(1);
    entry_count
        .saturating_mul(8)
        .saturating_mul(bucket_bytes)
        .saturating_add(16)
}
