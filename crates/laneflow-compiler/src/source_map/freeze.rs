//! 源映射的资源计量、限额校验与确定性记录冻结。

use super::*;
use crate::ledger::{BoundedVec, PassBudget, Plan};

/// 将受检来源文档键解析为冻结序号，并核对逻辑模块所有权。
struct SourceLocationResolver<'a> {
    unit: &'a CompilationUnit,
    logical_bytes: std::cell::Cell<u64>,
}

impl SourceLocationResolver<'_> {
    fn new(unit: &CompilationUnit) -> SourceLocationResolver<'_> {
        SourceLocationResolver {
            unit,
            logical_bytes: std::cell::Cell::new(0),
        }
    }

    #[inline]
    fn resolve(
        &self,
        owner_module: MirModuleKey,
        location: &SourceLocation,
    ) -> Result<SourceLocationRecord, DiagnosticBundle> {
        self.unit
            .resolve_source_location_for_module(owner_module.raw(), location)
            .map(|location| self.record(location))
    }

    #[inline]
    fn record(&self, location: ResolvedSourceLocation) -> SourceLocationRecord {
        let record = SourceLocationRecord::from(location);
        self.logical_bytes.set(
            self.logical_bytes
                .get()
                .saturating_add(record.logical_bytes()),
        );
        record
    }

    #[inline]
    fn logical_bytes(&self) -> u64 {
        self.logical_bytes.get()
    }
}

/// 把同次成功编译的编译单元、MIR 与已冻结 LIR 绑定为 `ValidatedSourceMapInput`；全程计入编译上限。
/// 在编译账本上冻结源映射；账本已计入编译单元、MIR 与 LIR 的存续。
pub(crate) fn freeze_source_map_in(
    unit: CompilationUnit,
    mir: &MirUnit,
    frozen_lir: &LirFreezeOutput,
    ledger: &crate::ledger::Ledger,
) -> Result<ValidatedSourceMapInput, DiagnosticBundle> {
    let module_count = u64::try_from(unit.modules.len()).unwrap_or(u64::MAX);
    let source_document_count = unit.source_document_count;
    let lane_edge_count = u64::try_from(mir.lane_edges.len()).unwrap_or(u64::MAX);
    let successor_count = u64::try_from(mir.lane_edge_connections.len()).unwrap_or(u64::MAX);
    let cross_entity_count = [
        mir.road_corridors.len(),
        mir.road_sections.len(),
        mir.authoring_lanes.len(),
        mir.lane_groups.len(),
        mir.facility_bands.len(),
    ]
    .into_iter()
    .fold(0_u64, |total, count| {
        total.saturating_add(u64::try_from(count).unwrap_or(u64::MAX))
    });
    let cross_relation_count = [
        mir.corridor_elements.len(),
        mir.authoring_lanes.len(),
        mir.authoring_lane_edges.len(),
        mir.lane_group_members.len(),
    ]
    .into_iter()
    .fold(0_u64, |total, count| {
        total.saturating_add(u64::try_from(count).unwrap_or(u64::MAX))
    });
    let junction_entity_count = [
        mir.junctions.len(),
        mir.movements.len(),
        mir.maneuver_paths.len(),
        mir.stop_lines.len(),
        mir.maneuver_gates.len(),
        mir.waiting_zones.len(),
    ]
    .into_iter()
    .fold(0_u64, |total, count| {
        total.saturating_add(u64::try_from(count).unwrap_or(u64::MAX))
    });
    let junction_relation_count = [
        mir.junction_movements.len(),
        mir.movement_maneuver_paths.len(),
        mir.maneuver_path_edges.len(),
        mir.junction_internal_edges.len(),
        mir.maneuver_path_gates.len(),
        mir.maneuver_path_waiting_zones.len(),
        mir.stop_line_maneuver_gates.len(),
        mir.conflict_zones.len(),
        mir.participant_streams.len(),
    ]
    .into_iter()
    .fold(0_u64, |total, count| {
        total.saturating_add(u64::try_from(count).unwrap_or(u64::MAX))
    });
    let signal_entity_count = [
        mir.signal_groups.len(),
        mir.signal_controllers.len(),
        mir.signal_phases.len(),
    ]
    .into_iter()
    .fold(0_u64, |total, count| {
        total.saturating_add(u64::try_from(count).unwrap_or(u64::MAX))
    });
    let signal_relation_count = [
        mir.signal_controller_groups.len(),
        mir.signal_phases.len(),
        mir.signal_phase_states.len(),
        mir.signal_group_maneuver_gates.len(),
    ]
    .into_iter()
    .fold(0_u64, |total, count| {
        total.saturating_add(u64::try_from(count).unwrap_or(u64::MAX))
    });
    let conflict_entity_count = u64::try_from(mir.conflict_zones.len())
        .unwrap_or(u64::MAX)
        .saturating_add(u64::try_from(mir.participant_streams.len()).unwrap_or(u64::MAX));
    let conflict_relation_count = u64::try_from(mir.participant_streams.len())
        .unwrap_or(u64::MAX)
        .saturating_add(u64::try_from(mir.conflict_passages.len()).unwrap_or(u64::MAX));
    let parking_entity_count = u64::try_from(mir.parking_facilities.len())
        .unwrap_or(u64::MAX)
        .saturating_add(u64::try_from(mir.parking_spaces.len()).unwrap_or(u64::MAX));
    let parking_relation_count = u64::try_from(mir.parking_spaces.len())
        .unwrap_or(u64::MAX)
        .saturating_mul(2)
        .saturating_add(u64::try_from(mir.parking_facility_spaces.len()).unwrap_or(u64::MAX));
    let parking_facility_relation_count = u64::try_from(
        mir.parking_facility_virtual_entries
            .len()
            .saturating_add(mir.parking_facility_virtual_exits.len()),
    )
    .unwrap_or(u64::MAX);
    let spatial_entity_count = u64::try_from(mir.canonical_frames.len()).unwrap_or(u64::MAX);
    let spatial_relation_count = u64::try_from(mir.lane_edge_geometries.len())
        .unwrap_or(u64::MAX)
        .saturating_add(u64::try_from(mir.facility_band_geometries.len()).unwrap_or(u64::MAX))
        .saturating_add(u64::try_from(mir.conflict_zone_regions.len()).unwrap_or(u64::MAX));
    let spatial_contributing_source_count = u64::try_from(mir.geometry_source_ranges.len())
        .unwrap_or(u64::MAX)
        .saturating_add(u64::try_from(mir.conflict_zone_regions.len()).unwrap_or(u64::MAX));
    let access_entity_count = u64::try_from(mir.participant_classes.len())
        .unwrap_or(u64::MAX)
        .saturating_add(u64::try_from(mir.vehicle_profiles.len()).unwrap_or(u64::MAX))
        .saturating_add(u64::try_from(mir.access_rules.len()).unwrap_or(u64::MAX));
    let access_relation_count = mir
        .participant_classes
        .iter()
        .filter(|participant_class| participant_class.parent.is_some())
        .count()
        .try_into()
        .unwrap_or(u64::MAX)
        .saturating_add(u64::try_from(mir.vehicle_profiles.len()).unwrap_or(u64::MAX))
        .saturating_add(u64::try_from(mir.access_rules.len()).unwrap_or(u64::MAX))
        .saturating_add(
            u64::try_from(mir.access_rule_participant_classes.len()).unwrap_or(u64::MAX),
        );
    let source_map_fixed_logical_bytes = unit
        .modules
        .iter()
        .fold(0_u64, |total, module| {
            let road_editing_context_bytes = module
                .declaration_span()
                .road_editing()
                .map_or(0, |location| location.context().source_map_logical_bytes());
            module.source_documents.iter().fold(
                total
                    .saturating_add(module.descriptor().source_map_logical_bytes())
                    .saturating_add(road_editing_context_bytes),
                |document_total, document| {
                    document_total.saturating_add(document.source_map_logical_bytes())
                },
            )
        })
        .saturating_add(lane_edge_count.saturating_mul(LANE_EDGE_SOURCE_FIXED_LOGICAL_BYTES))
        .saturating_add(
            successor_count.saturating_mul(LANE_EDGE_SUCCESSOR_SOURCE_FIXED_LOGICAL_BYTES),
        )
        .saturating_add(cross_entity_count.saturating_mul(STABLE_ENTITY_SOURCE_FIXED_LOGICAL_BYTES))
        .saturating_add(
            cross_relation_count.saturating_mul(OWNER_LOCAL_RELATION_SOURCE_FIXED_LOGICAL_BYTES),
        )
        .saturating_add(
            junction_entity_count.saturating_mul(STABLE_ENTITY_SOURCE_FIXED_LOGICAL_BYTES),
        )
        .saturating_add(
            junction_relation_count.saturating_mul(OWNER_LOCAL_RELATION_SOURCE_FIXED_LOGICAL_BYTES),
        )
        .saturating_add(
            spatial_contributing_source_count
                .saturating_mul(SPATIAL_GEOMETRY_SOURCE_RANGE_FIXED_LOGICAL_BYTES),
        )
        .saturating_add(
            signal_entity_count.saturating_mul(STABLE_ENTITY_SOURCE_FIXED_LOGICAL_BYTES),
        )
        .saturating_add(
            signal_relation_count.saturating_mul(OWNER_LOCAL_RELATION_SOURCE_FIXED_LOGICAL_BYTES),
        )
        .saturating_add(
            conflict_entity_count.saturating_mul(STABLE_ENTITY_SOURCE_FIXED_LOGICAL_BYTES),
        )
        .saturating_add(
            conflict_relation_count.saturating_mul(OWNER_LOCAL_RELATION_SOURCE_FIXED_LOGICAL_BYTES),
        )
        .saturating_add(
            parking_entity_count.saturating_mul(STABLE_ENTITY_SOURCE_FIXED_LOGICAL_BYTES),
        )
        .saturating_add(
            parking_relation_count.saturating_mul(OWNER_LOCAL_RELATION_SOURCE_FIXED_LOGICAL_BYTES),
        )
        .saturating_add(
            parking_facility_relation_count
                .saturating_mul(OWNER_LOCAL_RELATION_SOURCE_FIXED_LOGICAL_BYTES),
        )
        .saturating_add(
            spatial_entity_count.saturating_mul(STABLE_ENTITY_SOURCE_FIXED_LOGICAL_BYTES),
        )
        .saturating_add(
            spatial_relation_count.saturating_mul(OWNER_LOCAL_RELATION_SOURCE_FIXED_LOGICAL_BYTES),
        )
        .saturating_add(
            access_entity_count.saturating_mul(STABLE_ENTITY_SOURCE_FIXED_LOGICAL_BYTES),
        )
        .saturating_add(
            access_relation_count.saturating_mul(OWNER_LOCAL_RELATION_SOURCE_FIXED_LOGICAL_BYTES),
        );
    // 描述符字段与 import backing 已由 CompilationUnit 持有；冻结时新增模块/文档
    // 描述符平坦表、各稳定实体来源表及 owner-local 关系来源表的连续存储。峰值仍保留
    // 完整 unit，直到全部伴随表构造成功。
    let tables =
        Plan::ZERO
            .array::<SourceModuleDescriptor>(cap(module_count))
            .array::<SourceLocationRecord>(cap(module_count))
            .array::<SourceDocumentDescriptor>(cap(source_document_count))
            .array::<LaneEdgeSourceRecord>(cap(lane_edge_count))
            .array::<LaneEdgeSuccessorSourceRecord>(cap(successor_count))
            .array::<StableEntitySourceRecord<RoadCorridorOrdinal, RoadCorridorId>>(cap(mir
                .road_corridors
                .len()
                .try_into()
                .unwrap_or(u64::MAX)))
            .array::<StableEntitySourceRecord<RoadSectionOrdinal, RoadSectionId>>(cap(mir
                .road_sections
                .len()
                .try_into()
                .unwrap_or(u64::MAX)))
            .array::<StableEntitySourceRecord<AuthoringLaneOrdinal, AuthoringLaneId>>(cap(mir
                .authoring_lanes
                .len()
                .try_into()
                .unwrap_or(u64::MAX)))
            .array::<StableEntitySourceRecord<LaneGroupOrdinal, LaneGroupId>>(cap(mir
                .lane_groups
                .len()
                .try_into()
                .unwrap_or(u64::MAX)))
            .array::<StableEntitySourceRecord<FacilityBandOrdinal, FacilityBandId>>(cap(mir
                .facility_bands
                .len()
                .try_into()
                .unwrap_or(u64::MAX)))
            .array::<CrossSectionRelationSourceRecord>(cap(cross_relation_count))
            .array::<StableEntitySourceRecord<JunctionOrdinal, JunctionId>>(cap(mir
                .junctions
                .len()
                .try_into()
                .unwrap_or(u64::MAX)))
            .array::<StableEntitySourceRecord<MovementOrdinal, MovementId>>(cap(mir
                .movements
                .len()
                .try_into()
                .unwrap_or(u64::MAX)))
            .array::<StableEntitySourceRecord<ManeuverPathOrdinal, ManeuverPathId>>(cap(mir
                .maneuver_paths
                .len()
                .try_into()
                .unwrap_or(u64::MAX)))
            .array::<StableEntitySourceRecord<StopLineOrdinal, StopLineId>>(cap(mir
                .stop_lines
                .len()
                .try_into()
                .unwrap_or(u64::MAX)))
            .array::<StableEntitySourceRecord<ManeuverGateOrdinal, ManeuverGateId>>(cap(mir
                .maneuver_gates
                .len()
                .try_into()
                .unwrap_or(u64::MAX)))
            .array::<StableEntitySourceRecord<WaitingZoneOrdinal, WaitingZoneId>>(cap(mir
                .waiting_zones
                .len()
                .try_into()
                .unwrap_or(u64::MAX)))
            .array::<JunctionRelationSourceRecord>(cap(junction_relation_count))
            .array::<StableEntitySourceRecord<SignalGroupOrdinal, SignalGroupId>>(cap(mir
                .signal_groups
                .len()
                .try_into()
                .unwrap_or(u64::MAX)))
            .array::<StableEntitySourceRecord<SignalControllerOrdinal, SignalControllerId>>(cap(
                mir.signal_controllers.len().try_into().unwrap_or(u64::MAX),
            ))
            .array::<StableEntitySourceRecord<SignalPhaseOrdinal, SignalPhaseId>>(cap(mir
                .signal_phases
                .len()
                .try_into()
                .unwrap_or(u64::MAX)))
            .array::<SignalRelationSourceRecord>(cap(signal_relation_count))
            .array::<StableEntitySourceRecord<ConflictZoneOrdinal, ConflictZoneId>>(cap(mir
                .conflict_zones
                .len()
                .try_into()
                .unwrap_or(u64::MAX)))
            .array::<StableEntitySourceRecord<ParticipantStreamOrdinal, ParticipantStreamId>>(cap(
                mir.participant_streams.len().try_into().unwrap_or(u64::MAX),
            ))
            .array::<ConflictRelationSourceRecord>(cap(conflict_relation_count))
            .array::<StableEntitySourceRecord<ParkingFacilityOrdinal, ParkingFacilityId>>(cap(mir
                .parking_facilities
                .len()
                .try_into()
                .unwrap_or(u64::MAX)))
            .array::<StableEntitySourceRecord<ParkingSpaceOrdinal, ParkingSpaceId>>(cap(mir
                .parking_spaces
                .len()
                .try_into()
                .unwrap_or(u64::MAX)))
            .array::<ParkingRelationSourceRecord>(cap(parking_relation_count))
            .array::<ParkingFacilityRelationSourceRecord>(cap(parking_facility_relation_count))
            .array::<StableEntitySourceRecord<CanonicalFrameOrdinal, CanonicalFrameId>>(cap(
                spatial_entity_count,
            ))
            .array::<SpatialRelationSourceRecord>(cap(spatial_relation_count))
            .array::<SpatialGeometrySourceRangeRecord>(cap(spatial_contributing_source_count))
            .array::<StableEntitySourceRecord<ParticipantClassOrdinal, ParticipantClassId>>(cap(
                mir.participant_classes.len().try_into().unwrap_or(u64::MAX),
            ))
            .array::<StableEntitySourceRecord<VehicleProfileOrdinal, VehicleProfileId>>(cap(mir
                .vehicle_profiles
                .len()
                .try_into()
                .unwrap_or(u64::MAX)))
            .array::<StableEntitySourceRecord<AccessRuleOrdinal, AccessRuleId>>(cap(mir
                .access_rules
                .len()
                .try_into()
                .unwrap_or(u64::MAX)))
            .array::<AccessRelationSourceRecord>(cap(access_relation_count));
    // 派生内部边需要按 owner 生成稠密 local index；空间关系还需要把 MIR 几何按最终
    // LaneEdgeOrdinal 重排，并为每个 frame 生成独立 local index。四张空间暂存表在空间
    // 关系冻结期间同时存续；各阶段暂存在阶段结束时释放，互不重叠，取峰值即可。
    let junction_scratch = Plan::ZERO.array::<u32>(mir.junctions.len());
    let spatial_scratch = Plan::ZERO
        .array::<Option<usize>>(mir.lane_edges.len())
        .array::<u32>(mir.canonical_frames.len())
        .array::<Option<usize>>(mir.conflict_zones.len())
        .array::<u32>(mir.canonical_frames.len());
    // 准入规则的类别选择器逐条规则排序后释放，峰值取单条规则的最大选择器数。
    let selector_scratch = Plan::ZERO.array::<(ParticipantClassOrdinal, SourceLocation)>(
        mir.access_rules
            .iter()
            .map(|rule| rule.participant_classes.as_usize_range().len())
            .max()
            .unwrap_or(0),
    );
    // 策略来源按来源声明逐项计数：每个输入冻结为一条记录，伴随来源随记录逐项授予，成员
    // 键在输入与记录中各有一份，集中取得授予。
    let policy_counts = PolicySourceCounts::of(&unit, mir);
    let policy_input_scratch = Plan::ZERO
        .array::<policy::PolicySourceInput<'_>>(policy_counts.inputs)
        .plus(Plan::of(policy_counts.member_key_bytes));
    let policy_records = Plan::ZERO
        .array::<policy::PolicySourceRecord>(policy_counts.inputs)
        .array::<SourceLocationRecord>(policy_counts.contributing)
        .plus(Plan::of(policy_counts.member_key_bytes));
    let mut rows = 0;
    let mut contributors = 0;
    for c in &frozen_lir.lir.road_corridors {
        let Ok(()) = station::visit(&frozen_lir.lir, c, |_, _, _, _, _, _, s| {
            rows += 1;
            contributors += s.contributing.len();
            Ok::<_, core::convert::Infallible>(())
        });
    }
    let output = tables
        .plus(policy_records)
        .array::<station::StationSource>(rows)
        .array::<SourceLocationRecord>(contributors);
    let peak = output.plus(
        junction_scratch
            .max(spatial_scratch)
            .max(selector_scratch)
            .max(policy_input_scratch),
    );
    let primary_span = unit
        .modules
        .first()
        .map(|module| module.declaration_span().clone());
    let stable_key = unit
        .modules
        .first()
        .map(|module| module.descriptor().authoring_namespace_id().into());
    let pass = ledger.admit_pass(SOURCE_MAP_PASS, peak).map_err(|error| {
        DiagnosticBundle::single(Diagnostic::from_ledger_error(
            error,
            primary_span.clone(),
            stable_key,
        ))
    })?;
    let committed = ledger.live().saturating_add(ledger.reserved());

    // 真实文档序号由共同准入保留的全局唯一文档键绑定解析，同时核对该文档确实属于
    // 产生语义记录的逻辑模块。缺失键和跨模块错绑都以结构化诊断失败关闭，不能 panic
    // 或借另一个模块的同名/现存文档静默归因。
    let location = SourceLocationResolver::new(&unit);

    let module_capacity =
        usize::try_from(module_count).map_err(|_| output_overflow(&unit, primary_span.clone()))?;
    let mut source_module_declaration_sources = pass.vec(module_capacity)?;
    for (module_index, module) in unit.modules.iter().enumerate() {
        let module_ordinal = u32::try_from(module_index)
            .expect("compile limits bound canonical module ordinals to u32");
        source_module_declaration_sources.push(location.resolve(
            MirModuleKey::from_raw(module_ordinal),
            module.declaration_span(),
        )?)?;
    }

    let edge_capacity = usize::try_from(lane_edge_count)
        .map_err(|_| output_overflow(&unit, primary_span.clone()))?;
    let successor_capacity = usize::try_from(successor_count)
        .map_err(|_| output_overflow(&unit, primary_span.clone()))?;
    let mut lane_edge_sources = pass.vec(edge_capacity)?;
    let mut lane_edge_successor_sources = pass.vec(successor_capacity)?;
    for mir_key in frozen_lir
        .lane_edges
        .stage_keys_in_lir_order()
        .iter()
        .copied()
    {
        let edge = &mir.lane_edges[mir_key.index()];
        let ordinal = frozen_lir.lane_edges.ordinal(mir_key);
        lane_edge_sources.push(LaneEdgeSourceRecord {
            ordinal,
            stable_id: edge.stable_id,
            primary: location.resolve(edge.module, &edge.source_span)?,
        })?;
        let lir_edge = &frozen_lir.lir.lane_edges[ordinal.index()];
        let lir_successors =
            &frozen_lir.lir.lane_edge_successors[lir_edge.successors.as_usize_range()];
        let mir_rows = &frozen_lir.lane_edge_successors.mir_rows_in_lir_order()
            [lir_edge.successors.as_usize_range()];
        debug_assert_eq!(mir_rows.len(), lir_successors.len());
        for (local_index, (mir_row, lir_successor)) in
            mir_rows.iter().zip(lir_successors).enumerate()
        {
            let connection = &mir.lane_edge_connections[mir_row.index()];
            debug_assert_eq!(
                frozen_lir.lane_edges.ordinal(connection.target),
                *lir_successor
            );
            lane_edge_successor_sources.push(LaneEdgeSuccessorSourceRecord {
                owner_ordinal: ordinal,
                owner_stable_id: edge.stable_id,
                role: SourceRelationRole::LaneEdgeSuccessor,
                local_index: u32::try_from(local_index)
                    .expect("LIR relation range precheck proved local index fits u32"),
                primary: location.resolve(edge.module, &connection.source_span)?,
            })?;
        }
    }

    let mut road_corridor_sources = pass.vec(mir.road_corridors.len())?;
    let mut road_section_sources = pass.vec(mir.road_sections.len())?;
    let mut authoring_lane_sources = pass.vec(mir.authoring_lanes.len())?;
    let mut lane_group_sources = pass.vec(mir.lane_groups.len())?;
    let mut facility_band_sources = pass.vec(mir.facility_bands.len())?;
    let mut cross_section_relation_sources = pass.vec(
        usize::try_from(cross_relation_count)
            .map_err(|_| output_overflow(&unit, primary_span.clone()))?,
    )?;
    let mut junction_sources = pass.vec(mir.junctions.len())?;
    let mut movement_sources = pass.vec(mir.movements.len())?;
    let mut maneuver_path_sources = pass.vec(mir.maneuver_paths.len())?;
    let mut stop_line_sources = pass.vec(mir.stop_lines.len())?;
    let mut maneuver_gate_sources = pass.vec(mir.maneuver_gates.len())?;
    let mut waiting_zone_sources = pass.vec(mir.waiting_zones.len())?;
    let mut signal_group_sources = pass.vec(mir.signal_groups.len())?;
    let mut signal_controller_sources = pass.vec(mir.signal_controllers.len())?;
    let mut signal_phase_sources = pass.vec(mir.signal_phases.len())?;
    let mut signal_relation_sources = pass.vec(
        usize::try_from(signal_relation_count)
            .map_err(|_| output_overflow(&unit, primary_span.clone()))?,
    )?;
    let mut conflict_zone_sources = pass.vec(mir.conflict_zones.len())?;
    let mut participant_stream_sources = pass.vec(mir.participant_streams.len())?;
    let mut conflict_relation_sources = pass.vec(
        usize::try_from(conflict_relation_count)
            .map_err(|_| output_overflow(&unit, primary_span.clone()))?,
    )?;
    let mut parking_facility_sources = pass.vec(mir.parking_facilities.len())?;
    let mut parking_space_sources = pass.vec(mir.parking_spaces.len())?;
    let mut parking_relation_sources = pass.vec(
        usize::try_from(parking_relation_count)
            .map_err(|_| output_overflow(&unit, primary_span.clone()))?,
    )?;
    let mut parking_facility_relation_sources = pass.vec(
        usize::try_from(parking_facility_relation_count)
            .map_err(|_| output_overflow(&unit, primary_span.clone()))?,
    )?;
    let mut participant_class_sources = pass.vec(mir.participant_classes.len())?;
    let mut vehicle_profile_sources = pass.vec(mir.vehicle_profiles.len())?;
    let mut canonical_frame_sources = pass.vec(mir.canonical_frames.len())?;
    let mut spatial_relation_sources = pass.vec(
        usize::try_from(spatial_relation_count)
            .map_err(|_| output_overflow(&unit, primary_span.clone()))?,
    )?;
    let mut access_rule_sources = pass.vec(mir.access_rules.len())?;
    let mut access_relation_sources = pass.vec(
        usize::try_from(access_relation_count)
            .map_err(|_| output_overflow(&unit, primary_span.clone()))?,
    )?;
    let mut junction_relation_sources = pass.vec(
        usize::try_from(junction_relation_count)
            .map_err(|_| output_overflow(&unit, primary_span.clone()))?,
    )?;

    for mir_key in frozen_lir
        .road_corridors
        .stage_keys_in_lir_order()
        .iter()
        .copied()
    {
        let corridor = &mir.road_corridors[mir_key.index()];
        let ordinal = frozen_lir.road_corridors.ordinal(mir_key);
        road_corridor_sources.push(StableEntitySourceRecord {
            ordinal,
            stable_id: corridor.stable_id,
            primary: location.resolve(corridor.module, &corridor.source_span)?,
        })?;
        for (local_index, element) in mir.corridor_elements[corridor.elements.as_usize_range()]
            .iter()
            .enumerate()
        {
            let source_location = match element {
                crate::mir::MirCorridorElement::RoadSection {
                    source_location, ..
                }
                | crate::mir::MirCorridorElement::FacilityBand {
                    source_location, ..
                } => source_location.clone(),
            };
            cross_section_relation_sources.push(CrossSectionRelationSourceRecord {
                owner: CrossSectionRelationOwnerRecord::RoadCorridor(ordinal, corridor.stable_id),
                role: SourceRelationRole::RoadCorridorElement,
                local_index: u32::try_from(local_index)
                    .expect("MIR range precheck proved local index fits u32"),
                primary: location.record(source_location),
            })?;
        }
    }
    for mir_key in frozen_lir
        .road_sections
        .stage_keys_in_lir_order()
        .iter()
        .copied()
    {
        let section = &mir.road_sections[mir_key.index()];
        let ordinal = frozen_lir.road_sections.ordinal(mir_key);
        road_section_sources.push(StableEntitySourceRecord {
            ordinal,
            stable_id: section.stable_id,
            primary: location.resolve(section.module, &section.source_span)?,
        })?;
        for (local_index, lane_index) in section.lanes.as_usize_range().enumerate() {
            let lane = &mir.authoring_lanes[lane_index];
            cross_section_relation_sources.push(CrossSectionRelationSourceRecord {
                owner: CrossSectionRelationOwnerRecord::RoadSection(ordinal, section.stable_id),
                role: SourceRelationRole::RoadSectionLane,
                local_index: u32::try_from(local_index)
                    .expect("MIR range precheck proved local index fits u32"),
                primary: location.resolve(lane.module, &lane.source_span)?,
            })?;
        }
    }
    for mir_key in frozen_lir
        .authoring_lanes
        .stage_keys_in_lir_order()
        .iter()
        .copied()
    {
        let lane = &mir.authoring_lanes[mir_key.index()];
        let ordinal = frozen_lir.authoring_lanes.ordinal(mir_key);
        authoring_lane_sources.push(StableEntitySourceRecord {
            ordinal,
            stable_id: lane.stable_id,
            primary: location.resolve(lane.module, &lane.source_span)?,
        })?;
        for (local_index, edge) in mir.authoring_lane_edges[lane.edge_chain.as_usize_range()]
            .iter()
            .enumerate()
        {
            cross_section_relation_sources.push(CrossSectionRelationSourceRecord {
                owner: CrossSectionRelationOwnerRecord::AuthoringLane(ordinal, lane.stable_id),
                role: SourceRelationRole::AuthoringLaneEdge,
                local_index: u32::try_from(local_index)
                    .expect("MIR range precheck proved local index fits u32"),
                primary: location.resolve(lane.module, &edge.source_span)?,
            })?;
        }
    }
    for mir_key in frozen_lir
        .lane_groups
        .stage_keys_in_lir_order()
        .iter()
        .copied()
    {
        let group = &mir.lane_groups[mir_key.index()];
        let ordinal = frozen_lir.lane_groups.ordinal(mir_key);
        lane_group_sources.push(StableEntitySourceRecord {
            ordinal,
            stable_id: group.stable_id,
            primary: location.resolve(group.module, &group.source_span)?,
        })?;
        for (local_index, member) in mir.lane_group_members[group.members.as_usize_range()]
            .iter()
            .enumerate()
        {
            let lane = &mir.authoring_lanes[member.lane.index()];
            cross_section_relation_sources.push(CrossSectionRelationSourceRecord {
                owner: CrossSectionRelationOwnerRecord::LaneGroup(ordinal, group.stable_id),
                role: SourceRelationRole::LaneGroupMember,
                local_index: u32::try_from(local_index)
                    .expect("MIR range precheck proved local index fits u32"),
                primary: location.record(
                    lane.lane_group_source_location
                        .as_ref()
                        .expect("resolved lane-group member retains its reference source")
                        .clone(),
                ),
            })?;
        }
    }
    for mir_key in frozen_lir
        .facility_bands
        .stage_keys_in_lir_order()
        .iter()
        .copied()
    {
        let band = &mir.facility_bands[mir_key.index()];
        facility_band_sources.push(StableEntitySourceRecord {
            ordinal: frozen_lir.facility_bands.ordinal(mir_key),
            stable_id: band.stable_id,
            primary: location.resolve(band.module, &band.source_span)?,
        })?;
    }

    for mir_key in frozen_lir
        .junctions
        .stage_keys_in_lir_order()
        .iter()
        .copied()
    {
        let junction = &mir.junctions[mir_key.index()];
        let ordinal = frozen_lir.junctions.ordinal(mir_key);
        junction_sources.push(StableEntitySourceRecord {
            ordinal,
            stable_id: junction.stable_id,
            primary: location.resolve(junction.module, &junction.source_span)?,
        })?;
        let lir_junction = &frozen_lir.lir.junctions[ordinal.index()];
        for (local_index, movement_ordinal) in frozen_lir.lir.junction_movements
            [lir_junction.movements.as_usize_range()]
        .iter()
        .copied()
        .enumerate()
        {
            let movement_key = frozen_lir
                .movements
                .stage_key_at_lir_index(movement_ordinal.index());
            let movement = &mir.movements[movement_key.index()];
            junction_relation_sources.push(JunctionRelationSourceRecord {
                owner: JunctionRelationOwnerRecord::Junction(ordinal, junction.stable_id),
                role: SourceRelationRole::JunctionMovement,
                local_index: u32::try_from(local_index)
                    .expect("LIR relation range precheck proved local index fits u32"),
                primary: location.record(
                    movement
                        .junction_source_location
                        .as_ref()
                        .expect("resolved junction member retains its reference source")
                        .clone(),
                ),
            })?;
        }
    }

    for mir_key in frozen_lir
        .movements
        .stage_keys_in_lir_order()
        .iter()
        .copied()
    {
        let movement = &mir.movements[mir_key.index()];
        let ordinal = frozen_lir.movements.ordinal(mir_key);
        movement_sources.push(StableEntitySourceRecord {
            ordinal,
            stable_id: movement.stable_id,
            primary: location.resolve(movement.module, &movement.source_span)?,
        })?;
        let lir_movement = &frozen_lir.lir.movements[ordinal.index()];
        for (local_index, path_ordinal) in frozen_lir.lir.movement_maneuver_paths
            [lir_movement.maneuver_paths.as_usize_range()]
        .iter()
        .copied()
        .enumerate()
        {
            let path_key = frozen_lir
                .maneuver_paths
                .stage_key_at_lir_index(path_ordinal.index());
            let path = &mir.maneuver_paths[path_key.index()];
            junction_relation_sources.push(JunctionRelationSourceRecord {
                owner: JunctionRelationOwnerRecord::Movement(ordinal, movement.stable_id),
                role: SourceRelationRole::MovementManeuverPath,
                local_index: u32::try_from(local_index)
                    .expect("LIR relation range precheck proved local index fits u32"),
                primary: location.record(
                    path.movement_source_location
                        .as_ref()
                        .expect("resolved movement path retains its parent reference source")
                        .clone(),
                ),
            })?;
        }
    }

    for mir_key in frozen_lir
        .maneuver_paths
        .stage_keys_in_lir_order()
        .iter()
        .copied()
    {
        let path = &mir.maneuver_paths[mir_key.index()];
        let ordinal = frozen_lir.maneuver_paths.ordinal(mir_key);
        maneuver_path_sources.push(StableEntitySourceRecord {
            ordinal,
            stable_id: path.stable_id,
            primary: location.resolve(path.module, &path.source_span)?,
        })?;
        for (local_index, edge) in mir.maneuver_path_edges[path.edges.as_usize_range()]
            .iter()
            .enumerate()
        {
            junction_relation_sources.push(JunctionRelationSourceRecord {
                owner: JunctionRelationOwnerRecord::ManeuverPath(ordinal, path.stable_id),
                role: SourceRelationRole::ManeuverPathEdge,
                local_index: u32::try_from(local_index)
                    .expect("LIR relation range precheck proved local index fits u32"),
                primary: location.resolve(path.module, &edge.source_span)?,
            })?;
        }
        let lir_path = &frozen_lir.lir.maneuver_paths[ordinal.index()];
        for (local_index, gate_ordinal) in frozen_lir.lir.maneuver_path_gates
            [lir_path.maneuver_gates.as_usize_range()]
        .iter()
        .copied()
        .enumerate()
        {
            let gate_key = frozen_lir
                .maneuver_gates
                .stage_key_at_lir_index(gate_ordinal.index());
            let gate = &mir.maneuver_gates[gate_key.index()];
            junction_relation_sources.push(JunctionRelationSourceRecord {
                owner: JunctionRelationOwnerRecord::ManeuverPath(ordinal, path.stable_id),
                role: SourceRelationRole::ManeuverPathGate,
                local_index: u32::try_from(local_index)
                    .expect("LIR relation range precheck proved local index fits u32"),
                primary: location.record(
                    gate.maneuver_path_source_location
                        .as_ref()
                        .expect("resolved maneuver gate retains its path reference source")
                        .clone(),
                ),
            })?;
        }
        for (local_index, waiting_ordinal) in frozen_lir.lir.maneuver_path_waiting_zones
            [lir_path.waiting_zones.as_usize_range()]
        .iter()
        .copied()
        .enumerate()
        {
            let waiting_key = frozen_lir
                .waiting_zones
                .stage_key_at_lir_index(waiting_ordinal.index());
            let waiting = &mir.waiting_zones[waiting_key.index()];
            junction_relation_sources.push(JunctionRelationSourceRecord {
                owner: JunctionRelationOwnerRecord::ManeuverPath(ordinal, path.stable_id),
                role: SourceRelationRole::ManeuverPathWaitingZone,
                local_index: u32::try_from(local_index)
                    .expect("LIR relation range precheck proved local index fits u32"),
                primary: location.record(
                    waiting
                        .maneuver_path_source_location
                        .as_ref()
                        .expect("resolved waiting zone retains its path reference source")
                        .clone(),
                ),
            })?;
        }
    }

    for mir_key in frozen_lir
        .stop_lines
        .stage_keys_in_lir_order()
        .iter()
        .copied()
    {
        let stop_line = &mir.stop_lines[mir_key.index()];
        let ordinal = frozen_lir.stop_lines.ordinal(mir_key);
        stop_line_sources.push(StableEntitySourceRecord {
            ordinal,
            stable_id: stop_line.stable_id,
            primary: location.resolve(stop_line.module, &stop_line.source_span)?,
        })?;
        let lir_stop_line = &frozen_lir.lir.stop_lines[ordinal.index()];
        for (local_index, gate_ordinal) in frozen_lir.lir.stop_line_maneuver_gates
            [lir_stop_line.maneuver_gates.as_usize_range()]
        .iter()
        .copied()
        .enumerate()
        {
            let gate_key = frozen_lir
                .maneuver_gates
                .stage_key_at_lir_index(gate_ordinal.index());
            let gate = &mir.maneuver_gates[gate_key.index()];
            junction_relation_sources.push(JunctionRelationSourceRecord {
                owner: JunctionRelationOwnerRecord::StopLine(ordinal, stop_line.stable_id),
                role: SourceRelationRole::StopLineManeuverGate,
                local_index: u32::try_from(local_index)
                    .expect("LIR relation range precheck proved local index fits u32"),
                primary: location.record(
                    gate.stop_line_source_location
                        .as_ref()
                        .expect("resolved maneuver gate retains its stop-line reference source")
                        .clone(),
                ),
            })?;
        }
    }
    for mir_key in frozen_lir
        .maneuver_gates
        .stage_keys_in_lir_order()
        .iter()
        .copied()
    {
        let gate = &mir.maneuver_gates[mir_key.index()];
        maneuver_gate_sources.push(StableEntitySourceRecord {
            ordinal: frozen_lir.maneuver_gates.ordinal(mir_key),
            stable_id: gate.stable_id,
            primary: location.resolve(gate.module, &gate.source_span)?,
        })?;
    }
    for mir_key in frozen_lir
        .waiting_zones
        .stage_keys_in_lir_order()
        .iter()
        .copied()
    {
        let waiting = &mir.waiting_zones[mir_key.index()];
        waiting_zone_sources.push(StableEntitySourceRecord {
            ordinal: frozen_lir.waiting_zones.ordinal(mir_key),
            stable_id: waiting.stable_id,
            primary: location.resolve(waiting.module, &waiting.source_span)?,
        })?;
    }

    for mir_key in frozen_lir
        .signal_groups
        .stage_keys_in_lir_order()
        .iter()
        .copied()
    {
        let group = &mir.signal_groups[mir_key.index()];
        signal_group_sources.push(StableEntitySourceRecord {
            ordinal: frozen_lir.signal_groups.ordinal(mir_key),
            stable_id: group.stable_id,
            primary: location.resolve(group.module, &group.source_span)?,
        })?;
    }
    for mir_key in frozen_lir
        .signal_controllers
        .stage_keys_in_lir_order()
        .iter()
        .copied()
    {
        let controller = &mir.signal_controllers[mir_key.index()];
        let ordinal = frozen_lir.signal_controllers.ordinal(mir_key);
        signal_controller_sources.push(StableEntitySourceRecord {
            ordinal,
            stable_id: controller.stable_id,
            primary: location.resolve(controller.module, &controller.source_span)?,
        })?;
        let lir_controller = &frozen_lir.lir.signal_controllers[ordinal.index()];
        let lir_groups =
            &frozen_lir.lir.signal_controller_groups[lir_controller.signal_groups.as_usize_range()];
        let mir_group_rows = &frozen_lir.signal_controller_groups.mir_rows_in_lir_order()
            [lir_controller.signal_groups.as_usize_range()];
        debug_assert_eq!(mir_group_rows.len(), lir_groups.len());
        for (local_index, (mir_row, lir_group)) in mir_group_rows.iter().zip(lir_groups).enumerate()
        {
            let group = &mir.signal_controller_groups[mir_row.index()];
            debug_assert_eq!(
                frozen_lir.signal_groups.ordinal(group.signal_group),
                *lir_group
            );
            signal_relation_sources.push(SignalRelationSourceRecord {
                owner: SignalRelationOwnerRecord::SignalController(ordinal, controller.stable_id),
                role: SourceRelationRole::SignalControllerGroup,
                local_index: u32::try_from(local_index)
                    .expect("LIR relation range precheck proved local index fits u32"),
                primary: location.record(group.source_location.clone()),
            })?;
        }
        for (local_index, phase_ordinal) in frozen_lir.lir.signal_controller_phases
            [lir_controller.phases.as_usize_range()]
        .iter()
        .copied()
        .enumerate()
        {
            let phase_key = frozen_lir
                .signal_phases
                .stage_key_at_lir_index(phase_ordinal.index());
            let phase = &mir.signal_phases[phase_key.index()];
            signal_relation_sources.push(SignalRelationSourceRecord {
                owner: SignalRelationOwnerRecord::SignalController(ordinal, controller.stable_id),
                role: SourceRelationRole::SignalControllerPhase,
                local_index: u32::try_from(local_index)
                    .expect("LIR relation range precheck proved local index fits u32"),
                primary: phase.controller_relation_source_location.clone().into(),
            })?;
        }
    }
    for mir_key in frozen_lir
        .signal_phases
        .stage_keys_in_lir_order()
        .iter()
        .copied()
    {
        let phase = &mir.signal_phases[mir_key.index()];
        let ordinal = frozen_lir.signal_phases.ordinal(mir_key);
        signal_phase_sources.push(StableEntitySourceRecord {
            ordinal,
            stable_id: phase.stable_id,
            primary: location.resolve(phase.module, &phase.source_span)?,
        })?;
        let lir_phase = &frozen_lir.lir.signal_phases[ordinal.index()];
        let lir_states = &frozen_lir.lir.signal_phase_states[lir_phase.states.as_usize_range()];
        let mir_state_rows = &frozen_lir.signal_phase_states.mir_rows_in_lir_order()
            [lir_phase.states.as_usize_range()];
        debug_assert_eq!(mir_state_rows.len(), lir_states.len());
        for (local_index, (mir_row, lir_state)) in mir_state_rows.iter().zip(lir_states).enumerate()
        {
            let state = &mir.signal_phase_states[mir_row.index()];
            debug_assert_eq!(
                frozen_lir.signal_groups.ordinal(state.signal_group),
                lir_state.signal_group
            );
            signal_relation_sources.push(SignalRelationSourceRecord {
                owner: SignalRelationOwnerRecord::SignalPhase(ordinal, phase.stable_id),
                role: SourceRelationRole::SignalPhaseState,
                local_index: u32::try_from(local_index)
                    .expect("LIR relation range precheck proved local index fits u32"),
                primary: location.record(state.source_location.clone()),
            })?;
        }
    }
    for mir_key in frozen_lir
        .maneuver_gates
        .stage_keys_in_lir_order()
        .iter()
        .copied()
    {
        let gate = &mir.maneuver_gates[mir_key.index()];
        if let MirSignalControl::Group {
            source_location, ..
        } = &gate.signal_control
        {
            signal_relation_sources.push(SignalRelationSourceRecord {
                owner: SignalRelationOwnerRecord::ManeuverGate(
                    frozen_lir.maneuver_gates.ordinal(mir_key),
                    gate.stable_id,
                ),
                role: SourceRelationRole::ManeuverGateSignalGroup,
                local_index: 0,
                primary: location.record(source_location.clone()),
            })?;
        }
    }

    let mut next_conflict_zone_index_by_junction = filled(&pass, mir.junctions.len(), 0_u32)?;
    for mir_key in frozen_lir
        .conflict_zones
        .stage_keys_in_lir_order()
        .iter()
        .copied()
    {
        let zone = &mir.conflict_zones[mir_key.index()];
        let ordinal = frozen_lir.conflict_zones.ordinal(mir_key);
        conflict_zone_sources.push(StableEntitySourceRecord {
            ordinal,
            stable_id: zone.stable_id,
            primary: location.resolve(zone.module, &zone.source_span)?,
        })?;
        let local_index = next_conflict_zone_index_by_junction[zone.junction.index()];
        next_conflict_zone_index_by_junction[zone.junction.index()] = local_index
            .checked_add(1)
            .expect("LIR relation count precheck proved local index fits u32");
        let junction = &mir.junctions[zone.junction.index()];
        junction_relation_sources.push(JunctionRelationSourceRecord {
            owner: JunctionRelationOwnerRecord::Junction(
                frozen_lir.junctions.ordinal(zone.junction),
                junction.stable_id,
            ),
            role: SourceRelationRole::JunctionConflictZone,
            local_index,
            primary: location.record(
                zone.junction_source_location
                    .as_ref()
                    .expect("resolved conflict zone retains its junction reference source")
                    .clone(),
            ),
        })?;
    }

    drop(next_conflict_zone_index_by_junction);
    let mut next_participant_stream_index_by_junction = filled(&pass, mir.junctions.len(), 0_u32)?;
    for mir_key in frozen_lir
        .participant_streams
        .stage_keys_in_lir_order()
        .iter()
        .copied()
    {
        let stream = &mir.participant_streams[mir_key.index()];
        let ordinal = frozen_lir.participant_streams.ordinal(mir_key);
        participant_stream_sources.push(StableEntitySourceRecord {
            ordinal,
            stable_id: stream.stable_id,
            primary: location.resolve(stream.module, &stream.source_span)?,
        })?;
        let local_index = next_participant_stream_index_by_junction[stream.junction.index()];
        next_participant_stream_index_by_junction[stream.junction.index()] = local_index
            .checked_add(1)
            .expect("LIR relation count precheck proved local index fits u32");
        let junction = &mir.junctions[stream.junction.index()];
        junction_relation_sources.push(JunctionRelationSourceRecord {
            owner: JunctionRelationOwnerRecord::Junction(
                frozen_lir.junctions.ordinal(stream.junction),
                junction.stable_id,
            ),
            role: SourceRelationRole::JunctionParticipantStream,
            local_index,
            primary: location.record(
                stream
                    .junction_source_location
                    .as_ref()
                    .expect("resolved participant stream retains its junction reference source")
                    .clone(),
            ),
        })?;
        conflict_relation_sources.push(ConflictRelationSourceRecord {
            owner_ordinal: ordinal,
            owner_stable_id: stream.stable_id,
            role: SourceRelationRole::ParticipantStreamManeuverPath,
            local_index: 0,
            primary: location.record(
                stream
                    .maneuver_path_source_location
                    .as_ref()
                    .expect("resolved participant stream retains its maneuver-path source")
                    .clone(),
            ),
        })?;
        let lir_stream = &frozen_lir.lir.participant_streams[ordinal.index()];
        let mir_rows = &frozen_lir.conflict_passages.mir_rows_in_lir_order()
            [lir_stream.passages.as_usize_range()];
        let lir_passages = &frozen_lir.lir.conflict_passages[lir_stream.passages.as_usize_range()];
        debug_assert_eq!(mir_rows.len(), lir_passages.len());
        for (local_index, (mir_row, lir_passage)) in mir_rows.iter().zip(lir_passages).enumerate() {
            let passage = &mir.conflict_passages[mir_row.index()];
            debug_assert_eq!(
                frozen_lir.conflict_zones.ordinal(passage.conflict_zone),
                lir_passage.conflict_zone
            );
            conflict_relation_sources.push(ConflictRelationSourceRecord {
                owner_ordinal: ordinal,
                owner_stable_id: stream.stable_id,
                role: SourceRelationRole::ParticipantStreamConflictPassage,
                local_index: u32::try_from(local_index)
                    .expect("LIR relation range precheck proved local index fits u32"),
                primary: location.record(passage.source_location.clone()),
            })?;
        }
    }

    for mir_key in frozen_lir
        .parking_facilities
        .stage_keys_in_lir_order()
        .iter()
        .copied()
    {
        let area = &mir.parking_facilities[mir_key.index()];
        parking_facility_sources.push(StableEntitySourceRecord {
            ordinal: frozen_lir.parking_facilities.ordinal(mir_key),
            stable_id: area.stable_id,
            primary: location.resolve(area.module, &area.source_span)?,
        })?;
        for (role, anchors) in [
            (
                SourceRelationRole::ParkingFacilityVirtualEntry,
                &mir.parking_facility_virtual_entries[area.virtual_entries.as_usize_range()],
            ),
            (
                SourceRelationRole::ParkingFacilityVirtualExit,
                &mir.parking_facility_virtual_exits[area.virtual_exits.as_usize_range()],
            ),
        ] {
            for (local_index, anchor) in anchors.iter().enumerate() {
                parking_facility_relation_sources.push(ParkingFacilityRelationSourceRecord {
                    owner_ordinal: frozen_lir.parking_facilities.ordinal(mir_key),
                    owner_stable_id: area.stable_id,
                    role,
                    local_index: u32::try_from(local_index)
                        .expect("MIR relation range precheck proved local index fits u32"),
                    primary: location.record(anchor.source_location.clone()),
                })?;
            }
        }
    }
    for mir_key in frozen_lir
        .parking_spaces
        .stage_keys_in_lir_order()
        .iter()
        .copied()
    {
        let space = &mir.parking_spaces[mir_key.index()];
        let ordinal = frozen_lir.parking_spaces.ordinal(mir_key);
        let primary = location.resolve(space.module, &space.source_span)?;
        parking_space_sources.push(StableEntitySourceRecord {
            ordinal,
            stable_id: space.stable_id,
            primary,
        })?;
        if space.parking_facility.is_some() {
            parking_relation_sources.push(ParkingRelationSourceRecord {
                owner_ordinal: ordinal,
                owner_stable_id: space.stable_id,
                role: SourceRelationRole::ParkingSpaceFacility,
                local_index: 0,
                primary: location.record(
                    space
                        .parking_facility_source_location
                        .as_ref()
                        .expect("resolved parking-facility member retains its reference source")
                        .clone(),
                ),
            })?;
        }
        for (role, source_location) in [
            (
                SourceRelationRole::ParkingSpaceEntry,
                space.entry.source_location.clone(),
            ),
            (
                SourceRelationRole::ParkingSpaceExit,
                space.exit.source_location.clone(),
            ),
        ] {
            parking_relation_sources.push(ParkingRelationSourceRecord {
                owner_ordinal: ordinal,
                owner_stable_id: space.stable_id,
                role,
                local_index: 0,
                primary: location.record(source_location),
            })?;
        }
    }

    for mir_key in frozen_lir
        .participant_classes
        .stage_keys_in_lir_order()
        .iter()
        .copied()
    {
        let participant_class = &mir.participant_classes[mir_key.index()];
        let ordinal = frozen_lir.participant_classes.ordinal(mir_key);
        participant_class_sources.push(StableEntitySourceRecord {
            ordinal,
            stable_id: participant_class.stable_id,
            primary: location.resolve(participant_class.module, &participant_class.source_span)?,
        })?;
        if participant_class.parent.is_some() {
            access_relation_sources.push(AccessRelationSourceRecord {
                owner: AccessRelationOwnerRecord::ParticipantClass(
                    ordinal,
                    participant_class.stable_id,
                ),
                role: SourceRelationRole::ParticipantClassExtends,
                local_index: 0,
                primary: location.resolve(
                    participant_class.module,
                    participant_class
                        .parent_source_span
                        .as_ref()
                        .expect("resolved parent retains its reference source"),
                )?,
            })?;
        }
    }
    for mir_key in frozen_lir
        .vehicle_profiles
        .stage_keys_in_lir_order()
        .iter()
        .copied()
    {
        let profile = &mir.vehicle_profiles[mir_key.index()];
        let ordinal = frozen_lir.vehicle_profiles.ordinal(mir_key);
        vehicle_profile_sources.push(StableEntitySourceRecord {
            ordinal,
            stable_id: profile.stable_id,
            primary: location.resolve(profile.module, &profile.source_span)?,
        })?;
        access_relation_sources.push(AccessRelationSourceRecord {
            owner: AccessRelationOwnerRecord::VehicleProfile(ordinal, profile.stable_id),
            role: SourceRelationRole::VehicleProfileParticipantClass,
            local_index: 0,
            primary: location.resolve(profile.module, &profile.participant_class_source_span)?,
        })?;
    }
    for mir_key in frozen_lir
        .canonical_frames
        .stage_keys_in_lir_order()
        .iter()
        .copied()
    {
        let frame = &mir.canonical_frames[mir_key.index()];
        let ordinal = frozen_lir.canonical_frames.ordinal(mir_key);
        canonical_frame_sources.push(StableEntitySourceRecord {
            ordinal,
            stable_id: frame.stable_id,
            primary: location.resolve(frame.module, &frame.source_span)?,
        })?;
    }
    drop(next_participant_stream_index_by_junction);
    let mut mir_edge_to_geometry = filled(&pass, mir.lane_edges.len(), None)?;
    for (geometry_index, geometry) in mir.lane_edge_geometries.iter().enumerate() {
        debug_assert!(mir_edge_to_geometry[geometry.lane_edge.index()].is_none());
        mir_edge_to_geometry[geometry.lane_edge.index()] = Some(geometry_index);
    }
    let mut next_local_index_by_frame = filled(&pass, mir.canonical_frames.len(), 0_u32)?;
    for mir_edge in frozen_lir
        .lane_edges
        .stage_keys_in_lir_order()
        .iter()
        .copied()
    {
        let Some(geometry_index) = mir_edge_to_geometry[mir_edge.index()] else {
            debug_assert!(mir.lane_edge_geometries.is_empty());
            continue;
        };
        let geometry = &mir.lane_edge_geometries[geometry_index];
        let frame = &mir.canonical_frames[geometry.canonical_frame.index()];
        let owner_ordinal = frozen_lir
            .canonical_frames
            .ordinal(geometry.canonical_frame);
        let local_index = next_local_index_by_frame[geometry.canonical_frame.index()];
        next_local_index_by_frame[geometry.canonical_frame.index()] = local_index
            .checked_add(1)
            .expect("MIR range precheck proved local index fits u32");
        let geometry_point_start = geometry.points.start();
        let mir_source_ranges =
            &mir.geometry_source_ranges[geometry.source_ranges.as_usize_range()];
        let mut source_ranges = pass.vec(mir_source_ranges.len())?;
        for range in mir_source_ranges {
            source_ranges.push(SpatialGeometrySourceRangeRecord {
                point_start: range.points.start().saturating_sub(geometry_point_start),
                point_end_exclusive: range
                    .points
                    .start()
                    .saturating_sub(geometry_point_start)
                    .saturating_add(range.points.len()),
                source_segment_ordinal: range.source_segment_ordinal,
                source: location.resolve(range.source_module, &range.source)?,
            })?;
        }
        spatial_relation_sources.push(SpatialRelationSourceRecord {
            owner_ordinal,
            owner_stable_id: frame.stable_id,
            role: SourceRelationRole::CanonicalFrameLaneEdgeGeometry,
            local_index,
            primary: location.resolve(geometry.source_module, &geometry.source_span)?,
            source_ranges: source_ranges.freeze()?,
        })?;
    }
    for mir_frame in frozen_lir
        .canonical_frames
        .stage_keys_in_lir_order()
        .iter()
        .copied()
    {
        let frame = &mir.canonical_frames[mir_frame.index()];
        let owner_ordinal = frozen_lir.canonical_frames.ordinal(mir_frame);
        // FacilityBand 的 LIR 几何是按完整规范身份排序的稀疏表；来源关系的
        // frame-local index 必须复用同一顺序，不能沿用 HIR/MIR 的阶段局部 key
        // 顺序。否则不同 owner 下的 band 在两种顺序分歧时会互换来源。
        for (local_index, geometry) in frozen_lir
            .facility_bands
            .stage_keys_in_lir_order()
            .iter()
            .filter_map(|mir_band| {
                mir.facility_band_geometries[frame.facility_band_geometries.as_usize_range()]
                    .iter()
                    .find(|geometry| geometry.facility_band == *mir_band)
            })
            .enumerate()
        {
            let source_module = mir.facility_bands[geometry.facility_band.index()].module;
            let geometry_point_start = geometry.points.start();
            let mir_source_ranges =
                &mir.geometry_source_ranges[geometry.source_ranges.as_usize_range()];
            let mut source_ranges = pass.vec(mir_source_ranges.len())?;
            for range in mir_source_ranges {
                source_ranges.push(SpatialGeometrySourceRangeRecord {
                    point_start: range.points.start().saturating_sub(geometry_point_start),
                    point_end_exclusive: range
                        .points
                        .start()
                        .saturating_sub(geometry_point_start)
                        .saturating_add(range.points.len()),
                    source_segment_ordinal: range.source_segment_ordinal,
                    source: location.resolve(range.source_module, &range.source)?,
                })?;
            }
            spatial_relation_sources.push(SpatialRelationSourceRecord {
                owner_ordinal,
                owner_stable_id: frame.stable_id,
                role: SourceRelationRole::CanonicalFrameFacilityBandGeometry,
                local_index: u32::try_from(local_index)
                    .expect("MIR range precheck proved local index fits u32"),
                primary: location.resolve(source_module, &geometry.source_span)?,
                source_ranges: source_ranges.freeze()?,
            })?;
        }
    }
    let mut mir_zone_to_region = filled(&pass, mir.conflict_zones.len(), None)?;
    for (region_index, region) in mir.conflict_zone_regions.iter().enumerate() {
        debug_assert!(mir_zone_to_region[region.conflict_zone.index()].is_none());
        mir_zone_to_region[region.conflict_zone.index()] = Some(region_index);
    }
    let mut next_conflict_region_index_by_frame = filled(&pass, mir.canonical_frames.len(), 0_u32)?;
    for lir_region in &frozen_lir.lir.conflict_zone_regions {
        let mir_zone = frozen_lir
            .conflict_zones
            .stage_key_at_lir_index(lir_region.conflict_zone.index());
        let region_index = mir_zone_to_region[mir_zone.index()]
            .expect("LIR conflict region must map back to its unique MIR zone region");
        let region = &mir.conflict_zone_regions[region_index];
        let frame = &mir.canonical_frames[region.canonical_frame.index()];
        let owner_ordinal = frozen_lir.canonical_frames.ordinal(region.canonical_frame);
        let local_index = next_conflict_region_index_by_frame[region.canonical_frame.index()];
        next_conflict_region_index_by_frame[region.canonical_frame.index()] = local_index
            .checked_add(1)
            .expect("MIR range precheck proved local index fits u32");
        spatial_relation_sources.push(SpatialRelationSourceRecord {
            owner_ordinal,
            owner_stable_id: frame.stable_id,
            role: SourceRelationRole::CanonicalFrameConflictZoneRegion,
            local_index,
            primary: location.record(region.canonical_frame_source_location.clone()),
            source_ranges: {
                let mut source_ranges = pass.vec(1)?;
                source_ranges.push(SpatialGeometrySourceRangeRecord {
                    point_start: 0,
                    point_end_exclusive: region.ring_xz.len(),
                    source_segment_ordinal: 0,
                    source: location.record(region.source_location.clone()),
                })?;
                source_ranges.freeze()?
            },
        })?;
    }
    for (frame_index, frame) in mir.canonical_frames.iter().enumerate() {
        debug_assert_eq!(
            next_local_index_by_frame[frame_index],
            frame.lane_edge_geometries.len(),
        );
    }
    spatial_relation_sources
        .sort_unstable_by_key(|source| (source.owner_ordinal, source.role, source.local_index));
    drop(mir_edge_to_geometry);
    drop(next_local_index_by_frame);
    drop(mir_zone_to_region);
    drop(next_conflict_region_index_by_frame);
    for mir_key in frozen_lir
        .access_rules
        .stage_keys_in_lir_order()
        .iter()
        .copied()
    {
        let rule = &mir.access_rules[mir_key.index()];
        let ordinal = frozen_lir.access_rules.ordinal(mir_key);
        access_rule_sources.push(StableEntitySourceRecord {
            ordinal,
            stable_id: rule.stable_id,
            primary: location.resolve(rule.module, &rule.source_span)?,
        })?;
        access_relation_sources.push(AccessRelationSourceRecord {
            owner: AccessRelationOwnerRecord::AccessRule(ordinal, rule.stable_id),
            role: SourceRelationRole::AccessRuleTarget,
            local_index: 0,
            primary: location.resolve(rule.module, &rule.target_source_span)?,
        })?;
        let rule_selectors =
            &mir.access_rule_participant_classes[rule.participant_classes.as_usize_range()];
        let mut selectors = pass.vec(rule_selectors.len())?;
        selectors.try_extend(rule_selectors.iter().map(|selector| {
            (
                frozen_lir
                    .participant_classes
                    .ordinal(selector.participant_class),
                selector.source_span.clone(),
            )
        }))?;
        selectors.sort_unstable_by_key(|(participant_class, _)| *participant_class);
        for (local_index, (_, source_span)) in selectors.iter().enumerate() {
            access_relation_sources.push(AccessRelationSourceRecord {
                owner: AccessRelationOwnerRecord::AccessRule(ordinal, rule.stable_id),
                role: SourceRelationRole::AccessRuleParticipantClass,
                local_index: u32::try_from(local_index)
                    .expect("LIR relation range precheck proved local index fits u32"),
                primary: location.resolve(rule.module, source_span)?,
            })?;
        }
    }

    let mut internal_edge_local_indexes = filled(&pass, mir.junctions.len(), 0_u32)?;
    for relation_index in frozen_lir.canonical_mir_internal_edge_order.iter().copied() {
        let relation = &mir.junction_internal_edges[relation_index as usize];
        let junction = &mir.junctions[relation.junction.index()];
        let junction_ordinal = frozen_lir.junctions.ordinal(relation.junction);
        let local_index = internal_edge_local_indexes[junction_ordinal.index()];
        internal_edge_local_indexes[junction_ordinal.index()] = local_index
            .checked_add(1)
            .expect("LIR relation count precheck proved local index fits u32");
        junction_relation_sources.push(JunctionRelationSourceRecord {
            owner: JunctionRelationOwnerRecord::Junction(junction_ordinal, junction.stable_id),
            role: SourceRelationRole::JunctionInternalEdge,
            local_index,
            primary: location.resolve(relation.module, &relation.source_span)?,
        })?;
    }

    drop(internal_edge_local_indexes);
    debug_assert_eq!(lane_edge_sources.len(), edge_capacity);
    debug_assert_eq!(lane_edge_successor_sources.len(), successor_capacity);
    debug_assert_eq!(
        junction_relation_sources.len(),
        usize::try_from(junction_relation_count).unwrap_or(usize::MAX)
    );
    debug_assert_eq!(
        signal_relation_sources.len(),
        usize::try_from(signal_relation_count).unwrap_or(usize::MAX)
    );
    debug_assert_eq!(
        conflict_relation_sources.len(),
        usize::try_from(conflict_relation_count).unwrap_or(usize::MAX)
    );
    debug_assert_eq!(
        parking_relation_sources.len(),
        usize::try_from(parking_relation_count).unwrap_or(usize::MAX)
    );
    debug_assert_eq!(
        access_relation_sources.len(),
        usize::try_from(access_relation_count).unwrap_or(usize::MAX)
    );
    debug_assert_eq!(
        spatial_relation_sources.len(),
        usize::try_from(spatial_relation_count).unwrap_or(usize::MAX)
    );
    // 输出逻辑字节不再是限额（ADR 0035）；资源账本第二步把它并入报告，此前保留计算。
    let _output_bytes = frozen_lir
        .lir
        .output_bytes
        .saturating_add(source_map_fixed_logical_bytes)
        .saturating_add(location.logical_bytes());
    // 输入的成员键在输入存续期间集中持有一笔授予。
    let input_member_keys = pass.grant_bytes(policy_counts.member_key_bytes)?;
    let mut policy_inputs = pass.vec(policy_counts.inputs)?;
    for movement in &mir.movements {
        if let Some(source) = &movement.direction_source {
            policy_inputs.push(policy::PolicySourceInput {
                target: PolicySourceTarget::MovementDirection {
                    id: movement.stable_id,
                },
                owner_module: movement.module.raw(),
                primary: source,
                contributing: &[],
            })?;
        }
    }
    for (index, p) in mir.policies.iter().enumerate() {
        let crate::declaration::TypedAstDeclaration::RightOfWayPolicySet(source) =
            &unit.modules[p.origin.module as usize].declarations[p.origin.declaration as usize]
        else {
            unreachable!("policy origin bound by HIR");
        };
        debug_assert_eq!(p.value.id, frozen_lir.lir.policies[index].value.id);
        policy_inputs.push(policy::PolicySourceInput {
            target: PolicySourceTarget::Declaration {
                id: p.value.id,
                ordinal: frozen_lir.lir.policies[index].ordinal,
            },
            owner_module: p.origin.module,
            primary: &source.header.span,
            contributing: &source.contributing,
        })?;
        macro_rules! members {
            ($values:expr, $kind:ident) => {
                for member in $values {
                    policy_inputs.push(policy::PolicySourceInput {
                        target: PolicySourceTarget::Member {
                            owner: p.value.id,
                            kind: laneflow_static_contract::PolicyLocalMemberKind::$kind,
                            key: member.key.as_ref().into(),
                        },
                        owner_module: p.origin.module,
                        primary: &member.source.primary,
                        contributing: &member.source.contributing,
                    })?;
                }
            };
        }
        members!(&source.evidence, Evidence);
        members!(&source.gap_profiles, GapProfile);
        members!(&source.stream_rules, StreamRule);
        members!(&source.gate_rules, GateRule);
    }
    // 记录的成员键与输入的成员键各有一份；记录的授予先于复制取得，输入随后释放。
    let record_member_keys = pass.grant_bytes(policy_counts.member_key_bytes)?;
    let policy_sources = policy::freeze_policy_sources(&unit, &policy_inputs, &pass)?;
    drop(policy_inputs);
    drop(input_member_keys);
    let station_sources = station::freeze(frozen_lir, mir, &unit, rows, &pass)?;
    let (source_modules, source_documents) = unit.into_source_descriptors(&pass)?;
    let input = ValidatedSourceMapInput {
        station_sources,
        policy_sources,
        inner_payloads: crate::ledger::OutputGrant::new(record_member_keys),
        source_modules,
        source_module_declaration_sources: source_module_declaration_sources.freeze()?,
        source_documents,
        lane_edge_sources: lane_edge_sources.freeze()?,
        lane_edge_successor_sources: lane_edge_successor_sources.freeze()?,
        road_corridor_sources: road_corridor_sources.freeze()?,
        road_section_sources: road_section_sources.freeze()?,
        authoring_lane_sources: authoring_lane_sources.freeze()?,
        lane_group_sources: lane_group_sources.freeze()?,
        facility_band_sources: facility_band_sources.freeze()?,
        cross_section_relation_sources: cross_section_relation_sources.freeze()?,
        junction_sources: junction_sources.freeze()?,
        movement_sources: movement_sources.freeze()?,
        maneuver_path_sources: maneuver_path_sources.freeze()?,
        stop_line_sources: stop_line_sources.freeze()?,
        maneuver_gate_sources: maneuver_gate_sources.freeze()?,
        waiting_zone_sources: waiting_zone_sources.freeze()?,
        signal_group_sources: signal_group_sources.freeze()?,
        signal_controller_sources: signal_controller_sources.freeze()?,
        signal_phase_sources: signal_phase_sources.freeze()?,
        signal_relation_sources: signal_relation_sources.freeze()?,
        conflict_zone_sources: conflict_zone_sources.freeze()?,
        participant_stream_sources: participant_stream_sources.freeze()?,
        conflict_relation_sources: conflict_relation_sources.freeze()?,
        parking_facility_sources: parking_facility_sources.freeze()?,
        parking_space_sources: parking_space_sources.freeze()?,
        parking_relation_sources: parking_relation_sources.freeze()?,
        parking_facility_relation_sources: parking_facility_relation_sources.freeze()?,
        participant_class_sources: participant_class_sources.freeze()?,
        vehicle_profile_sources: vehicle_profile_sources.freeze()?,
        canonical_frame_sources: canonical_frame_sources.freeze()?,
        spatial_relation_sources: spatial_relation_sources.freeze()?,
        access_rule_sources: access_rule_sources.freeze()?,
        access_relation_sources: access_relation_sources.freeze()?,
        junction_relation_sources: junction_relation_sources.freeze()?,
        peak_controlled_live_bytes: committed,
    };
    // 编译遍结束：来源表、策略来源记录与描述符平坦表转为输出存续，暂存已全部释放。
    pass.finish(output.bytes().unwrap_or(u64::MAX))?;
    Ok(input)
}

/// 策略来源输入与冻结记录的计数：输入条数、伴随来源总数与成员键字节数。
///
/// 与输入构造按同一顺序遍历 Movement 方向字段与策略声明的具名成员，不分配内存。
struct PolicySourceCounts {
    inputs: usize,
    contributing: usize,
    member_key_bytes: u64,
}

impl PolicySourceCounts {
    fn of(unit: &CompilationUnit, mir: &MirUnit) -> Self {
        let mut counts = Self {
            inputs: mir
                .movements
                .iter()
                .filter(|movement| movement.direction_source.is_some())
                .count(),
            contributing: 0,
            member_key_bytes: 0,
        };
        for p in &mir.policies {
            let crate::declaration::TypedAstDeclaration::RightOfWayPolicySet(source) =
                &unit.modules[p.origin.module as usize].declarations[p.origin.declaration as usize]
            else {
                unreachable!("policy origin bound by HIR");
            };
            counts.inputs = counts.inputs.saturating_add(1);
            counts.contributing = counts
                .contributing
                .saturating_add(source.contributing.len());
            macro_rules! members {
                ($values:expr) => {
                    for member in $values {
                        counts.inputs = counts.inputs.saturating_add(1);
                        counts.contributing = counts
                            .contributing
                            .saturating_add(member.source.contributing.len());
                        counts.member_key_bytes = counts
                            .member_key_bytes
                            .saturating_add(u64::try_from(member.key.len()).unwrap_or(u64::MAX));
                    }
                };
            }
            members!(&source.evidence);
            members!(&source.gap_profiles);
            members!(&source.stream_rules);
            members!(&source.gate_rules);
        }
        counts
    }
}

/// 以编译遍授予创建长度为 `len`、每项为 `value` 的暂存序列。
fn filled<T: Clone>(
    pass: &PassBudget<'_>,
    len: usize,
    value: T,
) -> Result<BoundedVec<T>, DiagnosticBundle> {
    let mut items = pass.vec(len)?;
    items.resize(len, value)?;
    Ok(items)
}

/// 计数换算为授予请求的元素数；超出 `usize` 时取最大值，使计划无法表示。
fn cap(count: u64) -> usize {
    usize::try_from(count).unwrap_or(usize::MAX)
}

/// 源映射冻结编译遍的记账点。
const SOURCE_MAP_PASS: crate::ledger::PassLabel =
    crate::ledger::PassLabel::new(crate::ledger::LedgerStage::CompilePipeline, "source-map");

fn output_overflow(
    unit: &CompilationUnit,
    primary_span: Option<SourceLocation>,
) -> DiagnosticBundle {
    // 输出字节无法表示时，其存续必然超出宿主预算，按宿主预算维度报告。
    DiagnosticBundle::single(Diagnostic::compile_limit_exceeded_at(
        CompileLimitDimension::CompilerControlledLiveBytes,
        unit.limits
            .operation_budget()
            .get(crate::OperationResourceDimension::CompilerControlledLiveBytes),
        u64::MAX,
        primary_span,
        None,
    ))
}
