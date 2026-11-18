//! HIR 构建计划（build plan）与内存计划（memory plan）。
//!
//! [`HirBuildPlan::analyze`] 在任何与记录数成正比的阶段分配前，一次性统计编译单元的
//! 公共基础计数与七个领域实体计数，并按授予请求的类型与计数把持久表、查找表与互斥
//! 阶段暂存聚合为 HIR 编译遍的计划峰值；[`HirBuildPlan::check_limits`] 在 base 构造前
//! 按所选资源配置档预检限额维度并准入编译遍。persistent 与 lookup 跨领域求和，互斥
//! scratch 取峰值，策略绑定子编译遍与领域暂存互斥。哈希表按 [`Plan::hash_table`] 的
//! 保守请求计入。持久表项是计数上界：编译遍结束时以 HIR 各表实际持有的授予核对输出。

use std::sync::Arc;

use laneflow_static_contract::{SignalAspect, StableId128};

use crate::declaration::{
    LaneEdgeDeclaration, LaneEdgeGeometryAuthority, OwnedSignalControl, TypedAstDeclaration,
    TypedAstEntityAddress,
};
use crate::diagnostic::DiagnosticCollector;
use crate::identity::RegisteredCanonicalIdentity;
use crate::ledger::{
    BoundedHashMap, Ledger, LedgerError, LedgerStage, PassBudget, PassLabel, Plan,
};
use crate::{CompilationUnit, Diagnostic, DiagnosticBundle, SourceLocation};

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
    pub(super) persistent_bytes: Plan,
    pub(super) lookup_bytes: Plan,
    pub(super) scratch_bytes: Plan,
}

/// HIR 阶段的内存计划：记录数上界、互斥暂存峰值与三类存续字节。
pub(super) struct HirMemoryPlan {
    pub(super) hir_record_count: u64,
    pub(super) stage_scratch_bytes: u64,
    pub(super) persistent_bytes: u64,
    pub(super) lookup_bytes: u64,
    /// 本阶段计划峰值字节：持久表、查找表与互斥暂存峰值之和；上游存续由账本记录。
    pub(super) stage_bytes: u64,
    /// 编译遍结束时转为输出存续的授予字节：HIR 各表与策略存储。
    pub(super) output_bytes: u64,
    /// 本阶段计划峰值。
    pub(super) peak: Plan,
    /// 策略绑定子编译遍的计划。
    pub(super) policy: super::policy::PolicyBindPlan,
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
        // 先装配计数，再以各领域预算回填内存计划。
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
                stage_bytes: 0,
                output_bytes: 0,
                peak: Plan::ZERO,
                policy: super::policy::PolicyBindPlan::EMPTY,
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
        let conflict = conflict_budget(
            unit,
            module_count,
            &plan.conflict,
            &plan.junction,
            &plan.control,
        );
        let parking = parking_budget(unit, module_count, &plan.parking);
        let spatial = spatial_budget(
            unit,
            module_count,
            plan.lane_edge_count,
            &plan.spatial,
            &plan.cross_section,
        );
        let access = access_budget(
            unit,
            module_count,
            &plan.access,
            plan.lane_edge_count,
            &plan.cross_section,
            &plan.junction,
        );
        let scratch = base
            .scratch_bytes
            .max(cross_section.scratch_bytes)
            .max(junction.scratch_bytes)
            .max(control.scratch_bytes)
            .max(signal.scratch_bytes)
            .max(conflict.scratch_bytes)
            .max(parking.scratch_bytes)
            .max(spatial.scratch_bytes)
            .max(access.scratch_bytes);
        let persistent = [
            cross_section.persistent_bytes,
            junction.persistent_bytes,
            control.persistent_bytes,
            signal.persistent_bytes,
            conflict.persistent_bytes,
            parking.persistent_bytes,
            spatial.persistent_bytes,
            access.persistent_bytes,
        ]
        .into_iter()
        .fold(base.persistent_bytes, Plan::plus);
        let lookup = [
            cross_section.lookup_bytes,
            junction.lookup_bytes,
            control.lookup_bytes,
            signal.lookup_bytes,
            conflict.lookup_bytes,
            parking.lookup_bytes,
            spatial.lookup_bytes,
            access.lookup_bytes,
        ]
        .into_iter()
        .fold(base.lookup_bytes, Plan::plus);
        // 策略绑定在各领域暂存释放后、身份登记表仍存续时进行，与领域暂存互斥。
        let policy = super::policy::PolicyBindPlan::analyze(
            unit,
            cap(plan.control.maneuver_gates),
            cap(plan.conflict.streams),
            cap(plan.access.participant_classes),
        );
        let output = persistent.plus(Plan::of(policy.output_bytes()));
        let peak = persistent.plus(lookup).plus(scratch.max(policy.peak));
        plan.memory.stage_scratch_bytes = scratch.bytes().unwrap_or(u64::MAX);
        plan.memory.persistent_bytes = persistent.bytes().unwrap_or(u64::MAX);
        plan.memory.lookup_bytes = lookup.bytes().unwrap_or(u64::MAX);
        plan.memory.output_bytes = output.bytes().unwrap_or(u64::MAX);
        plan.memory.stage_bytes = peak.bytes().unwrap_or(u64::MAX);
        plan.memory.peak = peak;
        plan.memory.policy = policy;
        plan
    }

    /// 在编译账本上准入 HIR 编译遍。编译单元的准入峰值仍是 HIR 阶段的下界：计划峰值
    /// 取本阶段工作集与“准入峰值减去当前存续”的较大者。
    pub(super) fn admit<'l>(
        &self,
        unit: &CompilationUnit,
        ledger: &'l Ledger,
    ) -> Result<PassBudget<'l>, LedgerError> {
        let admission_floor =
            Plan::of(unit.admission_peak_live_bytes.saturating_sub(ledger.live()));
        ledger.admit_pass(HIR_PASS, self.memory.peak.max(admission_floor))
    }

    /// 在 base 构造前按所选配置档预检限额维度；任一维度超限即收集诊断并整体早退。
    ///
    /// 维度顺序、诊断构造（首模块 `primary_span`/`stable_key`）与早退语义同拆分前
    /// `build_hir` 的限额校验块。
    pub(super) fn check_limits<'l>(
        &self,
        unit: &CompilationUnit,
        ledger: &'l Ledger,
    ) -> Result<PassBudget<'l>, DiagnosticBundle> {
        let primary_span = unit
            .modules
            .first()
            .map(|module| module.declaration_span().clone());
        let stable_key = unit
            .modules
            .first()
            .map(|module| module.descriptor().authoring_namespace_id().into());
        let mut limit_diagnostics = DiagnosticCollector::new(
            unit.limits
                .source_limits()
                .get(crate::SourceProcessingDimension::DiagnosticCount),
        );
        let admission = self.admit(unit, ledger);
        for violation in [
            unit.limits.early_canonical_exceeded(
                laneflow_static_contract::CanonicalNetworkDimension::GeometryPointCount,
                self.spatial
                    .canonical_points
                    .saturating_add(self.spatial.conflict_region_points),
            ),
            unit.limits.source_limits().exceeded(
                crate::SourceProcessingDimension::HirRecordCount,
                self.memory.hir_record_count,
            ),
        ]
        .into_iter()
        .flatten()
        {
            let crate::limits::ResourceLimitExceeded {
                dimension,
                limit,
                observed,
            } = violation;

            limit_diagnostics.push(Diagnostic::compile_limit_exceeded_at(
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
                limit_diagnostics.push(Diagnostic::from_ledger_error(
                    error,
                    primary_span.clone(),
                    stable_key.clone(),
                ));
                None
            }
        };
        match pass {
            Some(pass) if limit_diagnostics.is_empty() => Ok(pass),
            _ => Err(limit_diagnostics.finish()),
        }
    }
}

/// HIR 主编译遍的记账点。
pub(super) const HIR_PASS: PassLabel = PassLabel::new(LedgerStage::CompilePipeline, "hir");

/// 公共基础构造（模块轴、导入、LaneEdge 表与规范身份）预算。
fn base_budget(
    unit: &CompilationUnit,
    module_count: u64,
    lane_edge_count: u64,
    lane_edge_reference_count: u64,
) -> DomainBudget {
    let canonical_source_scratch = Plan::ZERO
        .array::<CanonicalLaneEdgeSource>(cap(lane_edge_count))
        .array::<usize>(cap(unit.declaration_count));
    // 估算按平坦表 (&str, &SourceLocation) × import_edge_count 计入；实际分配是 base 段
    // 逐模块 Vec collect、无 with_capacity。#374 已记录该估算与实际分配不一致，按行为
    // 不变约束原样保留。
    let import_sort_scratch =
        Plan::ZERO.array::<(&str, &SourceLocation)>(cap(unit.import_edge_count));
    let (canonical_identity_bytes, largest_canonical_identity_bytes) = identity_byte_counts(unit);
    DomainBudget {
        persistent_bytes: Plan::ZERO
            .array::<HirModule>(cap(module_count))
            .array::<HirImport>(cap(unit.import_edge_count))
            .array::<HirLaneEdge>(cap(lane_edge_count))
            .array::<HirLaneEdgeReference>(cap(lane_edge_reference_count)),
        lookup_bytes: Plan::ZERO
            .hash_table::<Arc<str>, HirModuleKey>(cap(module_count))
            .plus(symbol_table_heads::<HirLaneEdgeKey>(module_count))
            .hash_table::<TypedAstEntityAddress, HirLaneEdgeKey>(cap(lane_edge_count))
            .hash_table::<StableId128, RegisteredCanonicalIdentity>(cap(unit.stable_entity_count))
            .plus(Plan::of(canonical_identity_bytes)),
        scratch_bytes: canonical_source_scratch
            .max(import_sort_scratch)
            .max(Plan::of(largest_canonical_identity_bytes)),
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
        Plan::ZERO
    } else {
        Plan::ZERO
            .array::<CanonicalDeclarationSource<HirRoadCorridorKey>>(cap(counts.road_corridors))
            .array::<CanonicalDeclarationSource<HirRoadSectionKey>>(cap(counts.road_sections))
            .array::<CanonicalAuthoringLaneSource>(cap(counts.authoring_lanes))
            .array::<CanonicalDeclarationSource<HirLaneGroupKey>>(cap(counts.lane_groups))
            .array::<CanonicalDeclarationSource<HirFacilityBandKey>>(cap(counts.facility_bands))
            .array::<Option<(HirRoadCorridorKey, SourceLocation)>>(cap(counts
                .road_sections
                .saturating_add(counts.facility_bands)))
            .array::<Option<HirAuthoringLaneKey>>(cap(lane_edge_count))
            .array::<usize>(cap(counts.lane_groups.saturating_mul(2)))
            .array::<usize>(cap(unit.declaration_count))
    };
    DomainBudget {
        persistent_bytes: Plan::ZERO
            .array::<HirRoadCorridor>(cap(counts.road_corridors))
            .array::<HirCorridorElement>(cap(counts.corridor_elements))
            .array::<HirRoadSection>(cap(counts.road_sections))
            .array::<HirAuthoringLane>(cap(counts.authoring_lanes))
            .array::<HirAuthoringLaneEdge>(cap(counts.authoring_lane_edges))
            .array::<HirLaneGroup>(cap(counts.lane_groups))
            .array::<HirLaneGroupMember>(cap(counts.authoring_lanes))
            .array::<HirFacilityBand>(cap(counts.facility_bands)),
        lookup_bytes: Plan::ZERO
            .plus(symbol_table_heads::<HirRoadSectionKey>(lookup_module_count))
            .plus(symbol_table_heads::<HirLaneGroupKey>(lookup_module_count))
            .plus(symbol_table_heads::<HirFacilityBandKey>(
                lookup_module_count,
            ))
            .hash_table::<TypedAstEntityAddress, HirRoadSectionKey>(cap(counts.road_sections))
            .hash_table::<TypedAstEntityAddress, HirLaneGroupKey>(cap(counts.lane_groups))
            .hash_table::<TypedAstEntityAddress, HirFacilityBandKey>(cap(counts.facility_bands)),
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
        Plan::ZERO
    } else {
        Plan::ZERO
            .array::<CanonicalDeclarationSource<HirMovementKey>>(cap(counts.movements))
            .array::<CanonicalDeclarationSource<HirManeuverPathKey>>(cap(counts.maneuver_paths))
            .array::<HirDeclaredJunctionEdge>(cap(counts
                .declared_approach_edges
                .saturating_add(counts.declared_internal_edges)))
            .array::<u8>(cap(lane_edge_count))
            .array::<usize>(cap(counts
                .junctions
                .saturating_add(counts.movements)
                .saturating_mul(2)))
            .hash_table::<ManeuverPathSequence<'static>, HirManeuverPathKey>(cap(
                counts.maneuver_paths
            ))
            .array::<Option<HirJunctionInternalEdge>>(cap(lane_edge_count))
            .array::<Option<(HirManeuverPathKey, SourceLocation)>>(cap(lane_edge_count))
            // 路径成员索引 CSR（#1219 P2-6）：starts(N+1) + 构造游标(N) + 连续
            // 路径键；路径键按成员出现记录上界（maneuver_path_edges）入账，越界
            // 成员跳过时只高估，宁可高估不低估。
            .array::<usize>(cap(lane_edge_count.saturating_add(1)))
            .array::<usize>(cap(lane_edge_count))
            .array::<HirManeuverPathKey>(cap(counts.maneuver_path_edges))
            .array::<usize>(cap(unit.declaration_count))
    };
    DomainBudget {
        persistent_bytes: Plan::ZERO
            .array::<HirJunction>(cap(counts.junctions))
            .array::<HirMovement>(cap(counts.movements))
            .array::<HirJunctionMovement>(cap(counts.movements))
            .array::<HirManeuverPath>(cap(counts.maneuver_paths))
            .array::<HirMovementManeuverPath>(cap(counts.maneuver_paths))
            .array::<HirManeuverPathEdge>(cap(counts.maneuver_path_edges))
            .array::<HirJunctionInternalEdge>(cap(lane_edge_count.min(counts.maneuver_path_edges))),
        lookup_bytes: Plan::ZERO
            .plus(symbol_table_heads::<HirJunctionKey>(lookup_module_count))
            .plus(symbol_table_heads::<HirMovementKey>(lookup_module_count))
            .hash_table::<TypedAstEntityAddress, HirJunctionKey>(cap(counts.junctions))
            .hash_table::<TypedAstEntityAddress, HirMovementKey>(cap(counts.movements)),
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
        Plan::ZERO
    } else {
        Plan::ZERO
            .array::<CanonicalDeclarationSource<HirStopLineKey>>(cap(counts.stop_lines))
            .array::<CanonicalDeclarationSource<HirManeuverGateKey>>(cap(counts.maneuver_gates))
            .array::<CanonicalDeclarationSource<HirWaitingZoneKey>>(cap(counts.waiting_zones))
            .array::<usize>(cap(counts
                .stop_lines
                .saturating_add(junction_counts.maneuver_paths)
                .saturating_mul(2)))
            .array::<Option<HirStopLineKey>>(cap(lane_edge_count))
            .array::<u8>(cap(counts
                .stop_lines
                .saturating_add(junction_counts.maneuver_paths)
                .saturating_add(lane_edge_reference_count)
                .saturating_add(lane_edge_count)))
            .array::<HirManeuverGateKey>(cap(counts.maneuver_gates.saturating_mul(2)))
            .array::<HirWaitingZoneKey>(cap(counts.waiting_zones))
    };
    DomainBudget {
        persistent_bytes: Plan::ZERO
            .array::<HirStopLine>(cap(counts.stop_lines))
            .array::<HirManeuverGate>(cap(counts.maneuver_gates))
            .array::<HirWaitingZone>(cap(counts.waiting_zones))
            .array::<HirManeuverPathGate>(cap(counts.maneuver_gates))
            .array::<HirManeuverPathWaitingZone>(cap(counts.waiting_zones))
            .array::<HirStopLineManeuverGate>(cap(counts.maneuver_gates)),
        lookup_bytes: Plan::ZERO
            .plus(symbol_table_heads::<HirManeuverPathKey>(
                lookup_module_count,
            ))
            .plus(symbol_table_heads::<HirStopLineKey>(lookup_module_count))
            .plus(symbol_table_heads::<HirManeuverGateKey>(
                lookup_module_count,
            ))
            .hash_table::<TypedAstEntityAddress, HirManeuverPathKey>(cap(
                junction_counts.maneuver_paths
            ))
            .hash_table::<TypedAstEntityAddress, HirStopLineKey>(cap(counts.stop_lines))
            .hash_table::<TypedAstEntityAddress, HirManeuverGateKey>(cap(counts.maneuver_gates)),
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
        Plan::ZERO
    } else {
        Plan::ZERO
            .array::<CanonicalDeclarationSource<HirSignalGroupKey>>(cap(counts.groups))
            .array::<CanonicalDeclarationSource<HirSignalControllerKey>>(cap(counts.controllers))
            .array::<Option<(HirSignalControllerKey, SourceLocation)>>(cap(counts.groups))
            // 相位状态暂存按 Option<(SignalAspect, SourceLocation)> × groups 估算；实际按
            // 单控制器 resolved_groups.len() 逐相位分配。#374 已记录该估算与实际分配不
            // 一致，按行为不变约束原样保留。
            .array::<Option<(SignalAspect, SourceLocation)>>(cap(counts.groups))
            .array::<usize>(cap(counts.groups.saturating_mul(3)))
            .array::<HirSignalGroupKey>(cap(counts.controller_groups))
            .array::<(HirSignalGroupKey, HirManeuverGateKey)>(cap(counts.controlled_gates))
            // seen 表按 controller_groups 总量估算；实际容量取单声明的
            // source.signal_groups.len()。#374 已记录该口径偏差，原样保留。
            .hash_table::<HirSignalGroupKey, SourceLocation>(cap(counts.controller_groups))
            .hash_table::<HirSignalGroupKey, usize>(cap(counts.controller_groups))
            .hash_table::<Arc<str>, SourceLocation>(cap(counts.phases))
            .array::<usize>(cap(unit.declaration_count))
    };
    DomainBudget {
        persistent_bytes: Plan::ZERO
            .array::<HirSignalGroup>(cap(counts.groups))
            .array::<HirSignalController>(cap(counts.controllers))
            .array::<HirSignalControllerGroup>(cap(counts.controller_groups))
            .array::<HirSignalPhase>(cap(counts.phases))
            .array::<HirSignalPhaseState>(cap(counts.phase_states))
            .array::<HirSignalGroupManeuverGate>(cap(counts.controlled_gates)),
        lookup_bytes: Plan::ZERO
            .plus(symbol_table_heads::<HirSignalGroupKey>(lookup_module_count))
            .hash_table::<TypedAstEntityAddress, HirSignalGroupKey>(cap(counts.groups)),
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
        Plan::ZERO
    } else {
        Plan::ZERO
            .array::<CanonicalDeclarationSource<HirParkingFacilityKey>>(cap(counts.areas))
            // 来源暂存按 12 字节 CanonicalDeclarationSource<HirParkingSpaceKey> 估算；
            // 实际为 8 字节 Vec<(u32, u32)>、键按下标重建，方向为估算偏保守。#374 已
            // 记录该偏差，原样保留。
            .array::<CanonicalDeclarationSource<HirParkingSpaceKey>>(cap(counts.spaces))
            .array::<bool>(cap(counts.areas))
            .array::<(HirParkingFacilityKey, HirParkingSpaceKey)>(cap(counts.memberships))
            .array::<usize>(cap(unit.declaration_count))
    };
    DomainBudget {
        persistent_bytes: Plan::ZERO
            .array::<HirParkingFacility>(cap(counts.areas))
            .array::<HirParkingSpace>(cap(counts.spaces))
            .array::<HirParkingFacilitySpace>(cap(counts.memberships))
            .array::<HirParkingLaneAnchor>(cap(counts
                .virtual_entries
                .saturating_add(counts.virtual_exits))),
        lookup_bytes: Plan::ZERO
            .plus(symbol_table_heads::<HirParkingFacilityKey>(
                lookup_module_count,
            ))
            .hash_table::<TypedAstEntityAddress, HirParkingFacilityKey>(cap(counts.areas)),
        scratch_bytes,
    }
}

fn conflict_budget(
    unit: &CompilationUnit,
    module_count: u64,
    counts: &ConflictCounts,
    junction: &JunctionCounts,
    control: &ControlCounts,
) -> DomainBudget {
    let lookup_module_count = if counts.entity_count() == 0 {
        0
    } else {
        module_count
    };
    let scratch_bytes = if counts.entity_count() == 0 {
        Plan::ZERO
    } else {
        Plan::ZERO
            .array::<(u32, u32, HirConflictZoneKey)>(cap(counts.zones))
            .array::<(u32, u32, HirParticipantStreamKey)>(cap(counts.streams))
            .array::<HirParticipantStreamKey>(cap(counts.passages))
            .array::<(
                HirConflictZoneKey,
                super::HirManeuverPathKey,
                (u32, u32),
                (u32, u32),
                laneflow_static_contract::ParticipantStreamId,
                HirParticipantStreamKey,
            )>(cap(counts.passages))
            .array::<(HirConflictZoneKey, HirParticipantStreamKey)>(cap(counts.passages))
            .array::<HirConflictZoneKey>(cap(counts.passages))
            .array::<usize>(cap(unit.declaration_count))
            .plus(symbol_table_heads::<HirJunctionKey>(lookup_module_count))
            .plus(symbol_table_heads::<super::HirManeuverPathKey>(
                lookup_module_count,
            ))
            .plus(symbol_table_heads::<HirManeuverGateKey>(
                lookup_module_count,
            ))
            // 与 build_conflict_hir 的三张 typed 表一致；其他种类声明只参与上面的
            // 声明排序视图，不占用这些查找表的桶。
            .hash_table::<TypedAstEntityAddress, HirJunctionKey>(cap(junction.junctions))
            .hash_table::<TypedAstEntityAddress, super::HirManeuverPathKey>(cap(
                junction.maneuver_paths
            ))
            .hash_table::<TypedAstEntityAddress, HirManeuverGateKey>(cap(control.maneuver_gates))
    };
    DomainBudget {
        persistent_bytes: Plan::ZERO
            .array::<HirConflictZone>(cap(counts.zones))
            .array::<HirParticipantStream>(cap(counts.streams))
            .array::<HirConflictPassage>(cap(counts.passages))
            .array::<HirConflictZoneStream>(cap(counts.zone_streams)),
        lookup_bytes: Plan::ZERO
            .plus(symbol_table_heads::<HirConflictZoneKey>(
                lookup_module_count,
            ))
            .hash_table::<TypedAstEntityAddress, HirConflictZoneKey>(cap(counts.zones)),
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
        Plan::ZERO
    } else {
        Plan::ZERO
            .array::<usize>(cap(unit.declaration_count))
            .array::<Option<PendingSpatialGeometry<'static>>>(cap(lane_edge_count))
            .array::<Option<PendingSpatialGeometry<'static>>>(cap(
                cross_section_counts.facility_bands
            ))
            .array::<Option<SpatialFrameAssignment>>(cap(lane_edge_count))
            .array::<Option<SpatialFrameAssignment>>(cap(cross_section_counts.facility_bands))
            .array::<Option<usize>>(cap(lane_edge_count))
            .array::<u8>(cap(lane_edge_count))
            .array::<HirLaneEdgeKey>(cap(counts.lane_edge_geometries))
            .array::<HirFacilityBandKey>(cap(counts.facility_band_geometries))
            // spatial 的 per-module HashMap 计入 scratch；本领域不在 lookup 模块清单。
            .plus(symbol_table_heads::<HirCanonicalFrameKey>(module_count))
            .plus(symbol_table_heads::<HirFacilityBandKey>(module_count))
            .hash_table::<TypedAstEntityAddress, HirCanonicalFrameKey>(cap(counts.canonical_frames))
            .hash_table::<TypedAstEntityAddress, HirFacilityBandKey>(cap(
                cross_section_counts.facility_bands
            ))
            .array::<PendingConflictZoneRegion<'static>>(cap(counts.conflict_zone_regions))
            .array::<Option<SourceLocation>>(cap(counts.conflict_zone_regions))
            .array::<HirCanonicalPoint2F32>(cap(counts.conflict_region_points))
            .array::<(usize, HirCanonicalPoint2F32)>(cap(counts.conflict_region_points))
    };
    DomainBudget {
        persistent_bytes: Plan::ZERO
            .array::<HirCanonicalFrame>(cap(counts.canonical_frames))
            .array::<HirLaneEdgeGeometry>(cap(counts.lane_edge_geometries))
            .array::<HirFacilityBandGeometry>(cap(counts.facility_band_geometries))
            .array::<HirGeometrySourceRange>(cap(counts.geometry_source_ranges))
            .array::<HirCanonicalPoint3F32>(cap(counts.canonical_points))
            .array::<HirSpatialSegment>(cap(counts.spatial_segments))
            .array::<HirConflictZoneRegion>(cap(counts.conflict_zone_regions))
            .array::<HirCanonicalPoint2F32>(cap(counts.conflict_region_points)),
        // spatial 不在 lookup 模块清单。
        lookup_bytes: Plan::ZERO,
        scratch_bytes,
    }
}

fn access_budget(
    unit: &CompilationUnit,
    module_count: u64,
    counts: &AccessCounts,
    lane_edge_count: u64,
    cross_section: &CrossSectionCounts,
    junction: &JunctionCounts,
) -> DomainBudget {
    let lookup_module_count = if counts.entity_count() == 0 {
        0
    } else {
        module_count
    };
    let scratch_bytes = if counts.entity_count() == 0 {
        Plan::ZERO
    } else {
        Plan::ZERO
            .array::<CanonicalDeclarationSource<HirParticipantClassKey>>(cap(
                counts.participant_classes
            ))
            .array::<CanonicalDeclarationSource<HirVehicleProfileKey>>(cap(counts.vehicle_profiles))
            .array::<CanonicalDeclarationSource<HirAccessRuleKey>>(cap(counts.access_rules))
            .array::<Option<HirParticipantClassKey>>(cap(counts
                .participant_classes
                .saturating_mul(2)))
            .array::<u8>(cap(counts.participant_classes))
            .array::<HirParticipantClassKey>(cap(counts.participant_classes))
            .array::<(HirParticipantClassKey, bool)>(cap(counts
                .participant_classes
                .saturating_mul(2)))
            .array::<HirAccessRuleParticipantClass>(cap(counts.rule_class_references))
            .array::<AccessCandidate>(cap(counts.rule_class_references))
            .array::<usize>(cap(unit.declaration_count))
            // 准入目标解析的五类目标符号表只在本领域存续。
            .plus(symbol_table_heads::<HirLaneEdgeKey>(module_count))
            .hash_table::<TypedAstEntityAddress, HirLaneEdgeKey>(cap(lane_edge_count))
            .plus(symbol_table_heads::<HirLaneGroupKey>(module_count))
            .hash_table::<TypedAstEntityAddress, HirLaneGroupKey>(cap(cross_section.lane_groups))
            .plus(symbol_table_heads::<HirRoadSectionKey>(module_count))
            .hash_table::<TypedAstEntityAddress, HirRoadSectionKey>(
                cap(cross_section.road_sections),
            )
            .plus(symbol_table_heads::<HirManeuverPathKey>(module_count))
            .hash_table::<TypedAstEntityAddress, HirManeuverPathKey>(cap(junction.maneuver_paths))
            .plus(symbol_table_heads::<HirFacilityBandKey>(module_count))
            .hash_table::<TypedAstEntityAddress, HirFacilityBandKey>(cap(
                cross_section.facility_bands
            ))
    };
    DomainBudget {
        persistent_bytes: Plan::ZERO
            .array::<HirParticipantClass>(cap(counts.participant_classes))
            .array::<HirVehicleProfile>(cap(counts.vehicle_profiles))
            .array::<HirAccessRule>(cap(counts.access_rules))
            .array::<HirAccessRuleParticipantClass>(cap(counts.rule_class_references)),
        lookup_bytes: Plan::ZERO
            .plus(symbol_table_heads::<HirParticipantClassKey>(
                lookup_module_count,
            ))
            .hash_table::<TypedAstEntityAddress, HirParticipantClassKey>(cap(
                counts.participant_classes
            )),
        scratch_bytes,
    }
}

/// 横断面领域记录计数：道路走廊、走廊成员、道路区段、编制车道及边链、车道组与设施带。
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
    /// 需要符号登记与身份派生的横断面实体总数（不含成员行）。
    pub(super) fn entity_count(&self) -> u64 {
        self.road_corridors
            .saturating_add(self.road_sections)
            .saturating_add(self.authoring_lanes)
            .saturating_add(self.lane_groups)
            .saturating_add(self.facility_bands)
    }
}

/// 路口领域记录计数：路口、通行流向、机动路径、路径边与声明的进口道/出口道与内部边。
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
    /// 路口领域实体总数（路口 + 通行流向 + 机动路径）。
    pub(super) fn entity_count(&self) -> u64 {
        self.junctions
            .saturating_add(self.movements)
            .saturating_add(self.maneuver_paths)
    }
}

/// 通行权控制领域记录计数：停止线、机动门与等待区。
#[derive(Default)]
pub(super) struct ControlCounts {
    pub(super) stop_lines: u64,
    pub(super) maneuver_gates: u64,
    pub(super) waiting_zones: u64,
}

impl ControlCounts {
    /// 控制领域实体总数（停止线 + 机动门 + 等待区）。
    pub(super) fn entity_count(&self) -> u64 {
        self.stop_lines
            .saturating_add(self.maneuver_gates)
            .saturating_add(self.waiting_zones)
    }
}

/// 信号领域记录计数：信号组、控制器、控制器-组成员、相位、相位状态与受控门。
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
    /// 信号领域实体总数（信号组 + 控制器 + 相位）。
    pub(super) fn entity_count(&self) -> u64 {
        self.groups
            .saturating_add(self.controllers)
            .saturating_add(self.phases)
    }
}

/// 停车领域记录计数：停车设施、停车位、成员关系与虚拟入口/出口。
#[derive(Default)]
pub(super) struct ParkingCounts {
    pub(super) areas: u64,
    pub(super) spaces: u64,
    pub(super) memberships: u64,
    pub(super) virtual_entries: u64,
    pub(super) virtual_exits: u64,
}

/// 冲突领域记录计数：冲突区、参与者流、穿越记录与冲突区-流关系。
#[derive(Default)]
pub(super) struct ConflictCounts {
    pub(super) zones: u64,
    pub(super) streams: u64,
    pub(super) passages: u64,
    pub(super) zone_streams: u64,
}

impl ConflictCounts {
    /// 冲突领域实体总数（冲突区 + 参与者流）。
    pub(super) fn entity_count(&self) -> u64 {
        self.zones.saturating_add(self.streams)
    }
}

impl ParkingCounts {
    /// 停车领域实体总数（停车设施 + 停车位）。
    pub(super) fn entity_count(&self) -> u64 {
        self.areas.saturating_add(self.spaces)
    }
}

/// 空间领域记录计数：规范坐标框架、车道边/设施带几何、来源区间、规范点、线段与冲突区空间区域。
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

/// 准入领域记录计数：参与者类别、车辆配置、准入规则与规则-类别引用。
#[derive(Default)]
pub(super) struct AccessCounts {
    pub(super) participant_classes: u64,
    pub(super) vehicle_profiles: u64,
    pub(super) access_rules: u64,
    pub(super) rule_class_references: u64,
}

impl AccessCounts {
    /// 准入领域实体总数（参与者类别 + 准入规则 + 车辆配置）。
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

/// 统计编译单元的横断面领域实体与成员计数。
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

/// 统计编译单元的路口领域实体、路径边与声明边集合计数。
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

/// 统计编译单元的通行权控制领域实体计数。
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

/// 统计编译单元的信号领域实体与成员计数。
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

/// 统计编译单元的停车领域实体、成员与虚拟锚点计数。
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

/// 统计编译单元的冲突领域实体与穿越记录计数。
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

/// 统计编译单元的准入领域实体与规则-类别引用计数。
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

/// 统计编译单元的空间领域几何、来源区间与点/线段计数。
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

/// 全部稳定实体完整原像副本的字节数，即身份登记表集中授予的载荷字节。
pub(super) fn canonical_identity_bytes(unit: &CompilationUnit) -> u64 {
    identity_byte_counts(unit).0
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

/// 逐模块符号表的外层序列与各表固定控制区：每个模块一张按声明计数预留的哈希表，
/// 每张非空表另计一组尾部控制区（见 [`Plan::hash_table`]）；各表的桶由调用方按总项数计入。
fn symbol_table_heads<K>(module_count: u64) -> Plan {
    Plan::ZERO
        .array::<BoundedHashMap<TypedAstEntityAddress, K>>(cap(module_count))
        .plus(Plan::of(module_count.saturating_mul(16)))
}

/// 计数换算为授予请求的元素数；超出 `usize` 时取最大值，使计划无法表示。
fn cap(count: u64) -> usize {
    usize::try_from(count).unwrap_or(usize::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CompilationUnitBuilder, CompileProfile};

    #[test]
    fn unrelated_declarations_only_grow_the_conflict_sort_view() {
        let mut unit =
            CompilationUnitBuilder::new(CompileProfile::single_network_10k_stable_entities())
                .build()
                .unwrap();
        let counts = ConflictCounts {
            zones: 1,
            ..Default::default()
        };
        let junction = JunctionCounts {
            junctions: 2,
            maneuver_paths: 3,
            ..Default::default()
        };
        let control = ControlCounts {
            maneuver_gates: 4,
            ..Default::default()
        };
        let before = conflict_budget(&unit, 1, &counts, &junction, &control);
        unit.declaration_count += 1_000;
        let after = conflict_budget(&unit, 1, &counts, &junction, &control);
        assert_eq!(after.persistent_bytes, before.persistent_bytes);
        assert_eq!(after.lookup_bytes, before.lookup_bytes);
        assert_eq!(
            after.scratch_bytes,
            before.scratch_bytes.plus(Plan::ZERO.array::<usize>(1_000))
        );

        let empty = conflict_budget(&unit, 1, &ConflictCounts::default(), &junction, &control);
        assert_eq!(
            empty.scratch_bytes,
            Plan::ZERO,
            "absent conflict domain allocates no lookup tables"
        );
    }

    #[test]
    fn junction_budget_accounts_member_index_layout() {
        // P2-6（#1219）：成员索引 CSR 三笔全入 scratch（starts(N+1) + 构造游标(N) +
        // 路径键(M)）；边际增量钉住"不得低估"，M 按出现记录上界入账。
        let unit =
            CompilationUnitBuilder::new(CompileProfile::single_network_10k_stable_entities())
                .build()
                .unwrap();
        let counts = JunctionCounts {
            junctions: 1,
            movements: 1,
            maneuver_paths: 2,
            maneuver_path_edges: 5,
            declared_approach_edges: 2,
            declared_internal_edges: 1,
        };
        let base = junction_budget(&unit, 1, 3, &counts);
        let more_members = JunctionCounts {
            junctions: 1,
            movements: 1,
            maneuver_paths: 2,
            maneuver_path_edges: counts.maneuver_path_edges + 1,
            declared_approach_edges: 2,
            declared_internal_edges: 1,
        };
        assert_eq!(
            junction_budget(&unit, 1, 3, &more_members).scratch_bytes,
            base.scratch_bytes
                .plus(Plan::ZERO.array::<HirManeuverPathKey>(1)),
            "path_keys 按成员出现记录入账"
        );
        let n_terms = Plan::ZERO
            .array::<u8>(1)
            .array::<Option<HirJunctionInternalEdge>>(1)
            .array::<Option<(HirManeuverPathKey, SourceLocation)>>(1)
            .array::<usize>(2);
        assert_eq!(
            junction_budget(&unit, 1, 4, &counts).scratch_bytes,
            base.scratch_bytes.plus(n_terms),
            "starts+游标随车道边数入账"
        );
    }
}
