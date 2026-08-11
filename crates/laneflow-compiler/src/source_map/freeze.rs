//! 源映射的资源计量、限额校验与确定性记录冻结。

use super::*;

/// 源映射阶段的三个受限资源观察值。
///
/// 该值对象只执行饱和算术，不扫描额外集合，也不分配内存；观察值顺序与原限额诊断
/// 顺序一致。
#[derive(Clone, Copy)]
struct SourceMapSizing {
    output_bytes: u64,
    scratch_bytes: u64,
    controlled_live_bytes: u64,
}

impl SourceMapSizing {
    #[inline]
    fn from_components(
        unit: &CompilationUnit,
        mir: &MirUnit,
        frozen_lir: &LirFreezeOutput,
        source_map_logical_bytes: u64,
        source_map_new_owned_bytes: u64,
        scratch_bytes: u64,
    ) -> Self {
        Self {
            output_bytes: frozen_lir
                .lir
                .output_bytes
                .saturating_add(source_map_logical_bytes),
            scratch_bytes,
            controlled_live_bytes: unit
                .controlled_live_bytes
                .saturating_add(mir.controlled_live_bytes)
                .saturating_add(frozen_lir.lir.controlled_live_bytes)
                .saturating_add(frozen_lir.mapping_bytes())
                .saturating_add(source_map_new_owned_bytes)
                .saturating_add(scratch_bytes),
        }
    }

    #[inline]
    const fn limit_observations(self) -> [(CompileLimitDimension, u64); 3] {
        [
            (CompileLimitDimension::OutputBytes, self.output_bytes),
            (CompileLimitDimension::StageScratchBytes, self.scratch_bytes),
            (
                CompileLimitDimension::CompilerControlledLiveBytes,
                self.controlled_live_bytes,
            ),
        ]
    }
}

/// 将受检来源文档键解析为冻结序号，并核对逻辑模块所有权。
struct SourceLocationResolver<'a> {
    unit: &'a CompilationUnit,
}

impl SourceLocationResolver<'_> {
    #[inline]
    fn resolve(
        &self,
        owner_module: MirModuleKey,
        location: &SourceLocation,
    ) -> Result<SourceLocationRecord, DiagnosticBundle> {
        self.unit
            .resolve_source_location_for_module(owner_module.raw(), location)
            .map(Into::into)
    }
}

pub(crate) fn freeze_source_map(
    unit: CompilationUnit,
    mir: &MirUnit,
    frozen_lir: &LirFreezeOutput,
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
    let parking_entity_count = u64::try_from(mir.parking_areas.len())
        .unwrap_or(u64::MAX)
        .saturating_add(u64::try_from(mir.parking_spaces.len()).unwrap_or(u64::MAX));
    let parking_relation_count = u64::try_from(mir.parking_spaces.len())
        .unwrap_or(u64::MAX)
        .saturating_mul(2)
        .saturating_add(u64::try_from(mir.parking_area_spaces.len()).unwrap_or(u64::MAX));
    let spatial_entity_count = u64::try_from(mir.canonical_frames.len()).unwrap_or(u64::MAX);
    let spatial_relation_count = u64::try_from(mir.lane_edge_geometries.len())
        .unwrap_or(u64::MAX)
        .saturating_add(u64::try_from(mir.facility_band_geometries.len()).unwrap_or(u64::MAX));
    let spatial_contributing_source_count =
        u64::try_from(mir.geometry_source_ranges.len()).unwrap_or(u64::MAX);
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
    let static_route_count = u64::try_from(mir.static_routes.len()).unwrap_or(u64::MAX);
    let route_relation_count = [
        mir.static_route_edges.len(),
        mir.maneuver_occurrences.len(),
        mir.gate_occurrences.len(),
        mir.waiting_zone_occurrences.len(),
    ]
    .into_iter()
    .fold(0_u64, |total, count| {
        total.saturating_add(u64::try_from(count).unwrap_or(u64::MAX))
    });
    let route_contributing_source_count = [
        mir.maneuver_occurrences.len(),
        mir.gate_occurrences.len(),
        mir.waiting_zone_occurrences.len(),
    ]
    .into_iter()
    .fold(0_u64, |total, count| {
        total.saturating_add(u64::try_from(count).unwrap_or(u64::MAX))
    });
    let source_map_logical_bytes = unit
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
        .saturating_add(module_count.saturating_mul(SOURCE_LOCATION_LOGICAL_BYTES))
        .saturating_add(lane_edge_count.saturating_mul(LANE_EDGE_SOURCE_LOGICAL_BYTES))
        .saturating_add(successor_count.saturating_mul(LANE_EDGE_SUCCESSOR_SOURCE_LOGICAL_BYTES))
        .saturating_add(cross_entity_count.saturating_mul(STABLE_ENTITY_SOURCE_LOGICAL_BYTES))
        .saturating_add(
            cross_relation_count.saturating_mul(CROSS_SECTION_RELATION_SOURCE_LOGICAL_BYTES),
        )
        .saturating_add(junction_entity_count.saturating_mul(STABLE_ENTITY_SOURCE_LOGICAL_BYTES))
        .saturating_add(
            junction_relation_count.saturating_mul(JUNCTION_RELATION_SOURCE_LOGICAL_BYTES),
        )
        .saturating_add(signal_entity_count.saturating_mul(STABLE_ENTITY_SOURCE_LOGICAL_BYTES))
        .saturating_add(signal_relation_count.saturating_mul(SIGNAL_RELATION_SOURCE_LOGICAL_BYTES))
        .saturating_add(parking_entity_count.saturating_mul(STABLE_ENTITY_SOURCE_LOGICAL_BYTES))
        .saturating_add(
            parking_relation_count.saturating_mul(PARKING_RELATION_SOURCE_LOGICAL_BYTES),
        )
        .saturating_add(spatial_entity_count.saturating_mul(STABLE_ENTITY_SOURCE_LOGICAL_BYTES))
        .saturating_add(
            spatial_relation_count.saturating_mul(SPATIAL_RELATION_SOURCE_LOGICAL_BYTES),
        )
        .saturating_add(
            spatial_contributing_source_count
                .saturating_mul(SPATIAL_GEOMETRY_SOURCE_RANGE_LOGICAL_BYTES),
        )
        .saturating_add(access_entity_count.saturating_mul(STABLE_ENTITY_SOURCE_LOGICAL_BYTES))
        .saturating_add(access_relation_count.saturating_mul(ACCESS_RELATION_SOURCE_LOGICAL_BYTES))
        .saturating_add(static_route_count.saturating_mul(STABLE_ENTITY_SOURCE_LOGICAL_BYTES))
        .saturating_add(route_relation_count.saturating_mul(ROUTE_RELATION_SOURCE_LOGICAL_BYTES))
        .saturating_add(
            route_contributing_source_count.saturating_mul(SOURCE_LOCATION_LOGICAL_BYTES),
        );
    // 描述符字段与 import backing 已由 CompilationUnit 持有；冻结时新增模块/文档
    // 描述符平坦表、各稳定实体来源表及 owner-local 关系来源表的连续存储。峰值仍保留
    // 完整 unit，直到全部伴随表构造成功。
    let source_map_new_owned_bytes = requested_bytes::<SourceModuleDescriptor>(module_count)
        .saturating_add(requested_bytes::<SourceLocationRecord>(module_count))
        .saturating_add(requested_bytes::<SourceDocumentDescriptor>(
            source_document_count,
        ))
        .saturating_add(requested_bytes::<LaneEdgeSourceRecord>(lane_edge_count))
        .saturating_add(requested_bytes::<LaneEdgeSuccessorSourceRecord>(
            successor_count,
        ))
        .saturating_add(requested_bytes::<
            StableEntitySourceRecord<RoadCorridorOrdinal, RoadCorridorId>,
        >(
            mir.road_corridors.len().try_into().unwrap_or(u64::MAX)
        ))
        .saturating_add(requested_bytes::<
            StableEntitySourceRecord<RoadSectionOrdinal, RoadSectionId>,
        >(
            mir.road_sections.len().try_into().unwrap_or(u64::MAX)
        ))
        .saturating_add(requested_bytes::<
            StableEntitySourceRecord<AuthoringLaneOrdinal, AuthoringLaneId>,
        >(
            mir.authoring_lanes.len().try_into().unwrap_or(u64::MAX)
        ))
        .saturating_add(requested_bytes::<
            StableEntitySourceRecord<LaneGroupOrdinal, LaneGroupId>,
        >(
            mir.lane_groups.len().try_into().unwrap_or(u64::MAX)
        ))
        .saturating_add(requested_bytes::<
            StableEntitySourceRecord<FacilityBandOrdinal, FacilityBandId>,
        >(
            mir.facility_bands.len().try_into().unwrap_or(u64::MAX)
        ))
        .saturating_add(requested_bytes::<CrossSectionRelationSourceRecord>(
            cross_relation_count,
        ))
        .saturating_add(requested_bytes::<
            StableEntitySourceRecord<JunctionOrdinal, JunctionId>,
        >(mir.junctions.len().try_into().unwrap_or(u64::MAX)))
        .saturating_add(requested_bytes::<
            StableEntitySourceRecord<MovementOrdinal, MovementId>,
        >(mir.movements.len().try_into().unwrap_or(u64::MAX)))
        .saturating_add(requested_bytes::<
            StableEntitySourceRecord<ManeuverPathOrdinal, ManeuverPathId>,
        >(
            mir.maneuver_paths.len().try_into().unwrap_or(u64::MAX)
        ))
        .saturating_add(requested_bytes::<
            StableEntitySourceRecord<StopLineOrdinal, StopLineId>,
        >(
            mir.stop_lines.len().try_into().unwrap_or(u64::MAX)
        ))
        .saturating_add(requested_bytes::<
            StableEntitySourceRecord<ManeuverGateOrdinal, ManeuverGateId>,
        >(
            mir.maneuver_gates.len().try_into().unwrap_or(u64::MAX)
        ))
        .saturating_add(requested_bytes::<
            StableEntitySourceRecord<WaitingZoneOrdinal, WaitingZoneId>,
        >(
            mir.waiting_zones.len().try_into().unwrap_or(u64::MAX)
        ))
        .saturating_add(requested_bytes::<JunctionRelationSourceRecord>(
            junction_relation_count,
        ))
        .saturating_add(requested_bytes::<
            StableEntitySourceRecord<SignalGroupOrdinal, SignalGroupId>,
        >(
            mir.signal_groups.len().try_into().unwrap_or(u64::MAX)
        ))
        .saturating_add(requested_bytes::<
            StableEntitySourceRecord<SignalControllerOrdinal, SignalControllerId>,
        >(
            mir.signal_controllers.len().try_into().unwrap_or(u64::MAX),
        ))
        .saturating_add(requested_bytes::<
            StableEntitySourceRecord<SignalPhaseOrdinal, SignalPhaseId>,
        >(
            mir.signal_phases.len().try_into().unwrap_or(u64::MAX)
        ))
        .saturating_add(requested_bytes::<SignalRelationSourceRecord>(
            signal_relation_count,
        ))
        .saturating_add(requested_bytes::<
            StableEntitySourceRecord<ParkingAreaOrdinal, ParkingAreaId>,
        >(
            mir.parking_areas.len().try_into().unwrap_or(u64::MAX)
        ))
        .saturating_add(requested_bytes::<
            StableEntitySourceRecord<ParkingSpaceOrdinal, ParkingSpaceId>,
        >(
            mir.parking_spaces.len().try_into().unwrap_or(u64::MAX)
        ))
        .saturating_add(requested_bytes::<ParkingRelationSourceRecord>(
            parking_relation_count,
        ))
        .saturating_add(requested_bytes::<
            StableEntitySourceRecord<CanonicalFrameOrdinal, CanonicalFrameId>,
        >(spatial_entity_count))
        .saturating_add(requested_bytes::<SpatialRelationSourceRecord>(
            spatial_relation_count,
        ))
        .saturating_add(requested_bytes::<SpatialGeometrySourceRangeRecord>(
            spatial_contributing_source_count,
        ))
        .saturating_add(requested_bytes::<
            StableEntitySourceRecord<ParticipantClassOrdinal, ParticipantClassId>,
        >(
            mir.participant_classes.len().try_into().unwrap_or(u64::MAX),
        ))
        .saturating_add(requested_bytes::<
            StableEntitySourceRecord<VehicleProfileOrdinal, VehicleProfileId>,
        >(
            mir.vehicle_profiles.len().try_into().unwrap_or(u64::MAX)
        ))
        .saturating_add(requested_bytes::<
            StableEntitySourceRecord<AccessRuleOrdinal, AccessRuleId>,
        >(
            mir.access_rules.len().try_into().unwrap_or(u64::MAX)
        ))
        .saturating_add(requested_bytes::<AccessRelationSourceRecord>(
            access_relation_count,
        ))
        .saturating_add(requested_bytes::<
            StableEntitySourceRecord<StaticRouteOrdinal, StaticRouteId>,
        >(static_route_count))
        .saturating_add(requested_bytes::<RouteRelationSourceRecord>(
            route_relation_count,
        ));
    // 派生内部边需要按 owner 生成稠密 local index；计数器只活到源映射冻结完成。
    let source_map_scratch_bytes =
        requested_bytes::<u32>(mir.junctions.len().try_into().unwrap_or(u64::MAX)).max(
            requested_bytes::<(ParticipantClassOrdinal, SourceLocation)>(
                mir.access_rule_participant_classes
                    .len()
                    .try_into()
                    .unwrap_or(u64::MAX),
            ),
        );
    let sizing = SourceMapSizing::from_components(
        &unit,
        mir,
        frozen_lir,
        source_map_logical_bytes,
        source_map_new_owned_bytes,
        source_map_scratch_bytes,
    );
    let primary_span = unit
        .modules
        .first()
        .map(|module| module.declaration_span().clone());
    let stable_key = unit
        .modules
        .first()
        .map(|module| module.descriptor().authoring_namespace_id().into());
    let mut diagnostics =
        DiagnosticCollector::new(unit.limits.value(CompileLimitDimension::DiagnosticCount));
    for (dimension, observed) in sizing.limit_observations() {
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

    // 真实文档序号由共同准入保留的全局唯一文档键绑定解析，同时核对该文档确实属于
    // 产生语义记录的逻辑模块。缺失键和跨模块错绑都以结构化诊断失败关闭，不能 panic
    // 或借另一个模块的同名/现存文档静默归因。
    let location = SourceLocationResolver { unit: &unit };

    let module_capacity =
        usize::try_from(module_count).map_err(|_| output_overflow(&unit, primary_span.clone()))?;
    let mut source_module_declaration_sources = Vec::with_capacity(module_capacity);
    for (module_index, module) in unit.modules.iter().enumerate() {
        let module_ordinal = u32::try_from(module_index)
            .expect("compile limits bound canonical module ordinals to u32");
        source_module_declaration_sources.push(location.resolve(
            MirModuleKey::from_raw(module_ordinal),
            module.declaration_span(),
        )?);
    }

    let edge_capacity = usize::try_from(lane_edge_count)
        .map_err(|_| output_overflow(&unit, primary_span.clone()))?;
    let successor_capacity = usize::try_from(successor_count)
        .map_err(|_| output_overflow(&unit, primary_span.clone()))?;
    let mut lane_edge_sources = Vec::with_capacity(edge_capacity);
    let mut lane_edge_successor_sources = Vec::with_capacity(successor_capacity);
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
        });
        for (local_index, connection) in mir.lane_edge_connections
            [edge.connections.as_usize_range()]
        .iter()
        .enumerate()
        {
            lane_edge_successor_sources.push(LaneEdgeSuccessorSourceRecord {
                owner_ordinal: ordinal,
                owner_stable_id: edge.stable_id,
                role: SourceRelationRole::LaneEdgeSuccessor,
                local_index: u32::try_from(local_index)
                    .expect("LIR relation range precheck proved local index fits u32"),
                primary: location.resolve(edge.module, &connection.source_span)?,
            });
        }
    }

    let mut road_corridor_sources = Vec::with_capacity(mir.road_corridors.len());
    let mut road_section_sources = Vec::with_capacity(mir.road_sections.len());
    let mut authoring_lane_sources = Vec::with_capacity(mir.authoring_lanes.len());
    let mut lane_group_sources = Vec::with_capacity(mir.lane_groups.len());
    let mut facility_band_sources = Vec::with_capacity(mir.facility_bands.len());
    let mut cross_section_relation_sources = Vec::with_capacity(
        usize::try_from(cross_relation_count)
            .map_err(|_| output_overflow(&unit, primary_span.clone()))?,
    );
    let mut junction_sources = Vec::with_capacity(mir.junctions.len());
    let mut movement_sources = Vec::with_capacity(mir.movements.len());
    let mut maneuver_path_sources = Vec::with_capacity(mir.maneuver_paths.len());
    let mut stop_line_sources = Vec::with_capacity(mir.stop_lines.len());
    let mut maneuver_gate_sources = Vec::with_capacity(mir.maneuver_gates.len());
    let mut waiting_zone_sources = Vec::with_capacity(mir.waiting_zones.len());
    let mut signal_group_sources = Vec::with_capacity(mir.signal_groups.len());
    let mut signal_controller_sources = Vec::with_capacity(mir.signal_controllers.len());
    let mut signal_phase_sources = Vec::with_capacity(mir.signal_phases.len());
    let mut signal_relation_sources = Vec::with_capacity(
        usize::try_from(signal_relation_count)
            .map_err(|_| output_overflow(&unit, primary_span.clone()))?,
    );
    let mut parking_area_sources = Vec::with_capacity(mir.parking_areas.len());
    let mut parking_space_sources = Vec::with_capacity(mir.parking_spaces.len());
    let mut parking_relation_sources = Vec::with_capacity(
        usize::try_from(parking_relation_count)
            .map_err(|_| output_overflow(&unit, primary_span.clone()))?,
    );
    let mut participant_class_sources = Vec::with_capacity(mir.participant_classes.len());
    let mut vehicle_profile_sources = Vec::with_capacity(mir.vehicle_profiles.len());
    let mut canonical_frame_sources = Vec::with_capacity(mir.canonical_frames.len());
    let mut spatial_relation_sources = Vec::with_capacity(
        usize::try_from(spatial_relation_count)
            .map_err(|_| output_overflow(&unit, primary_span.clone()))?,
    );
    let mut access_rule_sources = Vec::with_capacity(mir.access_rules.len());
    let mut access_relation_sources = Vec::with_capacity(
        usize::try_from(access_relation_count)
            .map_err(|_| output_overflow(&unit, primary_span.clone()))?,
    );
    let mut junction_relation_sources = Vec::with_capacity(
        usize::try_from(junction_relation_count)
            .map_err(|_| output_overflow(&unit, primary_span.clone()))?,
    );
    let mut static_route_sources = Vec::with_capacity(mir.static_routes.len());
    let mut route_relation_sources = Vec::with_capacity(
        usize::try_from(route_relation_count)
            .map_err(|_| output_overflow(&unit, primary_span.clone()))?,
    );

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
        });
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
                primary: source_location.into(),
            });
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
        });
        for (local_index, lane_index) in section.lanes.as_usize_range().enumerate() {
            let lane = &mir.authoring_lanes[lane_index];
            cross_section_relation_sources.push(CrossSectionRelationSourceRecord {
                owner: CrossSectionRelationOwnerRecord::RoadSection(ordinal, section.stable_id),
                role: SourceRelationRole::RoadSectionLane,
                local_index: u32::try_from(local_index)
                    .expect("MIR range precheck proved local index fits u32"),
                primary: location.resolve(lane.module, &lane.source_span)?,
            });
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
        });
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
            });
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
        });
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
                primary: lane
                    .lane_group_source_location
                    .as_ref()
                    .expect("resolved lane-group member retains its reference source")
                    .clone()
                    .into(),
            });
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
        });
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
        });
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
                primary: movement
                    .junction_source_location
                    .as_ref()
                    .expect("resolved junction member retains its reference source")
                    .clone()
                    .into(),
            });
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
        });
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
                primary: path
                    .movement_source_location
                    .as_ref()
                    .expect("resolved movement path retains its parent reference source")
                    .clone()
                    .into(),
            });
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
        });
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
            });
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
                primary: gate
                    .maneuver_path_source_location
                    .as_ref()
                    .expect("resolved maneuver gate retains its path reference source")
                    .clone()
                    .into(),
            });
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
                primary: waiting
                    .maneuver_path_source_location
                    .as_ref()
                    .expect("resolved waiting zone retains its path reference source")
                    .clone()
                    .into(),
            });
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
        });
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
                primary: gate
                    .stop_line_source_location
                    .as_ref()
                    .expect("resolved maneuver gate retains its stop-line reference source")
                    .clone()
                    .into(),
            });
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
        });
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
        });
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
        });
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
        });
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
                primary: group.source_location.clone().into(),
            });
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
            });
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
        });
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
                primary: state.source_location.clone().into(),
            });
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
                primary: source_location.clone().into(),
            });
        }
    }

    for mir_key in frozen_lir
        .parking_areas
        .stage_keys_in_lir_order()
        .iter()
        .copied()
    {
        let area = &mir.parking_areas[mir_key.index()];
        parking_area_sources.push(StableEntitySourceRecord {
            ordinal: frozen_lir.parking_areas.ordinal(mir_key),
            stable_id: area.stable_id,
            primary: location.resolve(area.module, &area.source_span)?,
        });
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
        });
        if space.parking_area.is_some() {
            parking_relation_sources.push(ParkingRelationSourceRecord {
                owner_ordinal: ordinal,
                owner_stable_id: space.stable_id,
                role: SourceRelationRole::ParkingSpaceArea,
                local_index: 0,
                primary: space
                    .parking_area_source_location
                    .as_ref()
                    .expect("resolved parking-area member retains its reference source")
                    .clone()
                    .into(),
            });
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
                primary: source_location.into(),
            });
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
        });
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
            });
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
        });
        access_relation_sources.push(AccessRelationSourceRecord {
            owner: AccessRelationOwnerRecord::VehicleProfile(ordinal, profile.stable_id),
            role: SourceRelationRole::VehicleProfileParticipantClass,
            local_index: 0,
            primary: location.resolve(profile.module, &profile.participant_class_source_span)?,
        });
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
        });
        for (local_index, geometry) in mir.lane_edge_geometries
            [frame.lane_edge_geometries.as_usize_range()]
        .iter()
        .enumerate()
        {
            let geometry_point_start = geometry.points.start();
            let mir_source_ranges =
                &mir.geometry_source_ranges[geometry.source_ranges.as_usize_range()];
            let mut source_ranges = Vec::with_capacity(mir_source_ranges.len());
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
                });
            }
            let source_ranges = source_ranges.into_boxed_slice();
            spatial_relation_sources.push(SpatialRelationSourceRecord {
                owner_ordinal: ordinal,
                owner_stable_id: frame.stable_id,
                role: SourceRelationRole::CanonicalFrameLaneEdgeGeometry,
                local_index: u32::try_from(local_index)
                    .expect("MIR range precheck proved local index fits u32"),
                primary: location.resolve(geometry.source_module, &geometry.source_span)?,
                source_ranges,
            });
        }
        for (local_index, geometry) in mir.facility_band_geometries
            [frame.facility_band_geometries.as_usize_range()]
        .iter()
        .enumerate()
        {
            let source_module = mir.facility_bands[geometry.facility_band.index()].module;
            spatial_relation_sources.push(SpatialRelationSourceRecord {
                owner_ordinal: ordinal,
                owner_stable_id: frame.stable_id,
                role: SourceRelationRole::CanonicalFrameFacilityBandGeometry,
                local_index: u32::try_from(local_index)
                    .expect("MIR range precheck proved local index fits u32"),
                primary: location.resolve(source_module, &geometry.source_span)?,
            });
        }
    }
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
        });
        access_relation_sources.push(AccessRelationSourceRecord {
            owner: AccessRelationOwnerRecord::AccessRule(ordinal, rule.stable_id),
            role: SourceRelationRole::AccessRuleTarget,
            local_index: 0,
            primary: location.resolve(rule.module, &rule.target_source_span)?,
        });
        let mut selectors = mir.access_rule_participant_classes
            [rule.participant_classes.as_usize_range()]
        .iter()
        .map(|selector| {
            (
                frozen_lir
                    .participant_classes
                    .ordinal(selector.participant_class),
                selector.source_span.clone(),
            )
        })
        .collect::<Vec<_>>();
        selectors.sort_unstable_by_key(|(participant_class, _)| *participant_class);
        for (local_index, (_, source_span)) in selectors.into_iter().enumerate() {
            access_relation_sources.push(AccessRelationSourceRecord {
                owner: AccessRelationOwnerRecord::AccessRule(ordinal, rule.stable_id),
                role: SourceRelationRole::AccessRuleParticipantClass,
                local_index: u32::try_from(local_index)
                    .expect("LIR relation range precheck proved local index fits u32"),
                primary: location.resolve(rule.module, &source_span)?,
            });
        }
    }

    for mir_key in frozen_lir
        .static_routes
        .stage_keys_in_lir_order()
        .iter()
        .copied()
    {
        let route = &mir.static_routes[mir_key.index()];
        let ordinal = frozen_lir.static_routes.ordinal(mir_key);
        static_route_sources.push(StableEntitySourceRecord {
            ordinal,
            stable_id: route.stable_id,
            primary: location.resolve(route.module, &route.source_span)?,
        });
        let route_edges = &mir.static_route_edges[route.edges.as_usize_range()];
        for (local_index, edge) in route_edges.iter().enumerate() {
            route_relation_sources.push(RouteRelationSourceRecord {
                owner_ordinal: ordinal,
                owner_stable_id: route.stable_id,
                role: SourceRelationRole::StaticRouteEdge,
                local_index: u32::try_from(local_index)
                    .expect("MIR route range precheck proved local index fits u32"),
                primary: location.resolve(route.module, &edge.source_span)?,
                contributing: None,
            });
        }
        for (local_index, occurrence) in mir.maneuver_occurrences
            [route.maneuver_occurrences.as_usize_range()]
        .iter()
        .enumerate()
        {
            let source_edge = &route_edges[occurrence.entry_route_edge_index as usize];
            route_relation_sources.push(RouteRelationSourceRecord {
                owner_ordinal: ordinal,
                owner_stable_id: route.stable_id,
                role: SourceRelationRole::StaticRouteManeuverOccurrence,
                local_index: u32::try_from(local_index)
                    .expect("MIR route range precheck proved local index fits u32"),
                primary: location.resolve(route.module, &source_edge.source_span)?,
                contributing: Some(location.resolve(
                    mir.maneuver_paths[occurrence.maneuver_path.index()].module,
                    &mir.maneuver_paths[occurrence.maneuver_path.index()].source_span,
                )?),
            });
        }
        for (local_index, occurrence) in mir.gate_occurrences
            [route.gate_occurrences.as_usize_range()]
        .iter()
        .enumerate()
        {
            let source_edge = &route_edges[occurrence.from_route_edge_index as usize];
            route_relation_sources.push(RouteRelationSourceRecord {
                owner_ordinal: ordinal,
                owner_stable_id: route.stable_id,
                role: SourceRelationRole::StaticRouteGateOccurrence,
                local_index: u32::try_from(local_index)
                    .expect("MIR route range precheck proved local index fits u32"),
                primary: location.resolve(route.module, &source_edge.source_span)?,
                contributing: Some(location.resolve(
                    mir.maneuver_gates[occurrence.maneuver_gate.index()].module,
                    &mir.maneuver_gates[occurrence.maneuver_gate.index()].source_span,
                )?),
            });
        }
        for (local_index, occurrence) in mir.waiting_zone_occurrences
            [route.waiting_zone_occurrences.as_usize_range()]
        .iter()
        .enumerate()
        {
            let source_edge = &route_edges[occurrence.entry_route_edge_index as usize];
            route_relation_sources.push(RouteRelationSourceRecord {
                owner_ordinal: ordinal,
                owner_stable_id: route.stable_id,
                role: SourceRelationRole::StaticRouteWaitingZoneOccurrence,
                local_index: u32::try_from(local_index)
                    .expect("MIR route range precheck proved local index fits u32"),
                primary: location.resolve(route.module, &source_edge.source_span)?,
                contributing: Some(location.resolve(
                    mir.waiting_zones[occurrence.waiting_zone.index()].module,
                    &mir.waiting_zones[occurrence.waiting_zone.index()].source_span,
                )?),
            });
        }
    }

    let mut internal_edge_local_indexes = vec![0_u32; mir.junctions.len()];
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
        });
    }

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
    let (source_modules, source_documents) = unit.into_source_descriptors();
    Ok(ValidatedSourceMapInput {
        source_modules,
        source_module_declaration_sources: source_module_declaration_sources.into_boxed_slice(),
        source_documents,
        lane_edge_sources: lane_edge_sources.into_boxed_slice(),
        lane_edge_successor_sources: lane_edge_successor_sources.into_boxed_slice(),
        road_corridor_sources: road_corridor_sources.into_boxed_slice(),
        road_section_sources: road_section_sources.into_boxed_slice(),
        authoring_lane_sources: authoring_lane_sources.into_boxed_slice(),
        lane_group_sources: lane_group_sources.into_boxed_slice(),
        facility_band_sources: facility_band_sources.into_boxed_slice(),
        cross_section_relation_sources: cross_section_relation_sources.into_boxed_slice(),
        junction_sources: junction_sources.into_boxed_slice(),
        movement_sources: movement_sources.into_boxed_slice(),
        maneuver_path_sources: maneuver_path_sources.into_boxed_slice(),
        stop_line_sources: stop_line_sources.into_boxed_slice(),
        maneuver_gate_sources: maneuver_gate_sources.into_boxed_slice(),
        waiting_zone_sources: waiting_zone_sources.into_boxed_slice(),
        signal_group_sources: signal_group_sources.into_boxed_slice(),
        signal_controller_sources: signal_controller_sources.into_boxed_slice(),
        signal_phase_sources: signal_phase_sources.into_boxed_slice(),
        signal_relation_sources: signal_relation_sources.into_boxed_slice(),
        parking_area_sources: parking_area_sources.into_boxed_slice(),
        parking_space_sources: parking_space_sources.into_boxed_slice(),
        parking_relation_sources: parking_relation_sources.into_boxed_slice(),
        participant_class_sources: participant_class_sources.into_boxed_slice(),
        vehicle_profile_sources: vehicle_profile_sources.into_boxed_slice(),
        canonical_frame_sources: canonical_frame_sources.into_boxed_slice(),
        spatial_relation_sources: spatial_relation_sources.into_boxed_slice(),
        access_rule_sources: access_rule_sources.into_boxed_slice(),
        access_relation_sources: access_relation_sources.into_boxed_slice(),
        junction_relation_sources: junction_relation_sources.into_boxed_slice(),
        static_route_sources: static_route_sources.into_boxed_slice(),
        route_relation_sources: route_relation_sources.into_boxed_slice(),
        peak_controlled_live_bytes: sizing.controlled_live_bytes,
    })
}

fn requested_bytes<T>(count: u64) -> u64 {
    count.saturating_mul(u64::try_from(size_of::<T>()).unwrap_or(u64::MAX))
}

fn output_overflow(
    unit: &CompilationUnit,
    primary_span: Option<SourceLocation>,
) -> DiagnosticBundle {
    DiagnosticBundle::single(Diagnostic::compile_limit_exceeded_at(
        CompileLimitDimension::OutputBytes,
        unit.limits.value(CompileLimitDimension::OutputBytes),
        u64::MAX,
        primary_span,
        None,
    ))
}
