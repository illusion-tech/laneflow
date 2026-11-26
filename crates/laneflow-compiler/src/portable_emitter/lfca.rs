use super::budget::Accounting;
use super::tree::{ObjectTree, SectionRef, TableRef, TreeBuilder, Value, build_tree, field};
use super::*;
use crate::ledger::{BoundedVec, Plan};
mod policy;
mod station;

/// LFCA 构造前一次求出、两遍构造共用的派生输入，全部由发射账本授予。
struct LfcaInputs<'a> {
    station: station::Inputs<'a>,
    /// 每条交通边是否为路口内部边。
    internal_edges: BoundedVec<bool>,
    /// 每份交通边几何是否有空间来源区间。
    lane_geometry_applies: BoundedVec<bool>,
    /// 每份设施带几何是否有空间来源区间。
    facility_geometry_applies: BoundedVec<bool>,
}

impl LfcaInputs<'_> {
    const fn granted_bytes(&self) -> u64 {
        self.station.bytes()
            + self.internal_edges.granted_bytes()
            + self.lane_geometry_applies.granted_bytes()
            + self.facility_geometry_applies.granted_bytes()
    }
}

/// 空间来源键：（规范坐标系序号, 关系角色, 局部下标, 来源序号, 是否有几何来源区间）。
/// 来源序号使同键来源按出现顺序排列，查找取最后一条。
type GeometryFlag = (u32, u8, u32, usize, bool);

fn geometry_applies(flags: &[GeometryFlag], frame: u32, role: u8, local_index: u32) -> bool {
    let key = (frame, role, local_index);
    let end = flags.partition_point(|flag| (flag.0, flag.1, flag.2) <= key);
    end.checked_sub(1)
        .map(|index| flags[index])
        .filter(|flag| (flag.0, flag.1, flag.2) == key)
        .is_some_and(|flag| flag.4)
}

fn prepare_inputs<'a>(
    output: &'a CompilationOutput,
    accounting: Accounting<'_>,
) -> Result<LfcaInputs<'a>, PortableEmissionError> {
    let lir = output.lir().unit();
    let spatial = output.source_map_input().spatial_relation_sources();
    let frames = lir.canonical_frames.len();
    // 输出三组标志；来源键表与逐坐标系计数器只在本遍内存续。
    let (station_sizes, station_plan) = station::sizes(lir)?;
    let plan = station_plan
        .array::<bool>(lir.lane_edges.len())
        .array::<bool>(lir.lane_edge_geometries.len())
        .array::<bool>(lir.facility_band_geometries.len())
        .array::<GeometryFlag>(spatial.len())
        .array::<u32>(frames);
    let pass = accounting.admit("lfca-inputs", plan)?;

    let mut internal_edges = pass.vec::<bool>(lir.lane_edges.len())?;
    internal_edges.try_extend((0..lir.lane_edges.len()).map(|ordinal| {
        lir.junction_internal_edges
            .binary_search_by_key(
                &u32::try_from(ordinal).expect("compile limits cap entity counts at u32"),
                |entry| entry.edge.raw(),
            )
            .is_ok()
    }))?;

    let mut flags = pass.vec::<GeometryFlag>(spatial.len())?;
    flags.try_extend(spatial.enumerate().map(|(index, source)| {
        (
            source.owner_ordinal().raw(),
            source.role().code(),
            source.local_index(),
            index,
            source.geometry_source_ranges().len() != 0,
        )
    }))?;
    flags.sort_unstable();
    let direction_profile = lir.geometry_profiles.map_or(0, |profiles| {
        geometry_direction_profile_code(profiles.direction)
    });

    let mut counters = pass.vec::<u32>(frames)?;
    counters.resize(frames, 0)?;
    let lane_role = crate::source_map::SourceRelationRole::CanonicalFrameLaneEdgeGeometry.code();
    let mut lane_geometry_applies = pass.vec::<bool>(lir.lane_edge_geometries.len())?;
    for geometry in &lir.lane_edge_geometries {
        let local_index = counters[geometry.canonical_frame.index()];
        counters[geometry.canonical_frame.index()] += 1;
        let applies = geometry_applies(
            &flags,
            geometry.canonical_frame.raw(),
            lane_role,
            local_index,
        );
        debug_assert!(direction_profile != 0 || !applies);
        lane_geometry_applies.push(applies)?;
    }
    counters.fill(0);
    let facility_role =
        crate::source_map::SourceRelationRole::CanonicalFrameFacilityBandGeometry.code();
    let mut facility_geometry_applies = pass.vec::<bool>(lir.facility_band_geometries.len())?;
    for geometry in &lir.facility_band_geometries {
        let local_index = counters[geometry.canonical_frame.index()];
        counters[geometry.canonical_frame.index()] += 1;
        let applies = geometry_applies(
            &flags,
            geometry.canonical_frame.raw(),
            facility_role,
            local_index,
        );
        debug_assert!(direction_profile != 0 || !applies);
        facility_geometry_applies.push(applies)?;
    }
    drop((flags, counters));
    let inputs = LfcaInputs {
        station: station::prepare(lir, station_sizes, &pass)?,
        internal_edges,
        lane_geometry_applies,
        facility_geometry_applies,
    };
    pass.finish(inputs.granted_bytes())?;
    Ok(inputs)
}

/// 从冻结 LIR、显式发射 provenance 与声明的网络修订构造 LFCA 可移植规范制品的
/// 对象树。
///
/// 覆盖格式版本、规范身份、实体、关系、空间几何、执行契约、provenance 与网络修订各
/// section；派生输入与对象树均由发射账本授予，字节编码由对象树完成。
pub(super) fn build_lfca(
    output: &CompilationOutput,
    provenance: &PortableEmissionProvenance,
    source_collection_digest: [u8; 32],
    declared_network_revision: NetworkRevisionId,
    accounting: Accounting<'_>,
) -> Result<ObjectTree, PortableEmissionError> {
    let inputs = prepare_inputs(output, accounting)?;
    let tree = build_tree(
        accounting,
        "lfca-tree",
        PortableObjectKind::CanonicalArtifact,
        |t| {
            emit_lfca(
                t,
                output,
                provenance,
                &inputs,
                source_collection_digest,
                declared_network_revision,
            )
        },
    )?;
    drop(inputs);
    Ok(tree)
}

/// 改写 LFCA 第 8 节的声明网络修订。
pub(super) fn set_lfca_tree_revision(
    tree: &mut ObjectTree,
    revision: NetworkRevisionId,
) -> Result<(), PortableEmissionError> {
    tree.set_sha256(7, 0, 0, 1, revision.into_digest().into_bytes())
}

fn emit_lfca(
    t: &TreeBuilder,
    output: &CompilationOutput,
    provenance: &PortableEmissionProvenance,
    inputs: &LfcaInputs<'_>,
    source_collection_digest: [u8; 32],
    declared_network_revision: NetworkRevisionId,
) -> Result<[SectionRef; 9], PortableEmissionError> {
    let lir = output.lir().unit();
    let direction_profile = lir.geometry_profiles.map_or(0, |profiles| {
        geometry_direction_profile_code(profiles.direction)
    });
    let accuracy_profile = lir.geometry_profiles.map_or(0, |profiles| {
        geometry_accuracy_profile_code(profiles.accuracy)
    });
    let spatial_present = u8::from(
        lir.geometry_profiles.is_some()
            || !lir.canonical_frames.is_empty()
            || !lir.lane_edge_geometries.is_empty()
            || !lir.facility_band_geometries.is_empty()
            || !lir.conflict_zone_regions.is_empty(),
    );

    Ok([
        t.section(
            1,
            [t.table(
                1,
                [t.row([
                    field(1, Value::U16(CANONICAL_ARTIFACT_FORMAT_VERSION)),
                    field(2, Value::U16(IDENTITY_ENCODING_VERSION)),
                    field(3, Value::U16(IDENTITY_REGISTRY_REVISION)),
                    field(4, Value::U16(NETWORK_REVISION_DERIVATION_VERSION)),
                    field(5, Value::U16(CONSTRAINT_CONTRACT_VERSION)),
                    field(6, Value::U16(STATIC_EXECUTION_CONTRACT_VERSION)),
                ])],
            )],
        ),
        t.section(2, [canonical_identity_table(t, lir)]),
        t.section_with(3, || canonical_entity_tables(t, lir, inputs))?,
        t.section(4, canonical_relation_tables(t, lir)),
        t.section(
            5,
            [
                t.table(
                    1,
                    [t.row([
                        field(1, Value::U8(spatial_present)),
                        field(2, Value::U8(direction_profile)),
                    ])],
                ),
                lane_edge_geometry_table(t, lir, inputs),
                facility_band_geometry_table(t, lir, inputs),
                conflict_zone_region_table(t, lir),
            ],
        ),
        t.section(
            6,
            [t.table(
                1,
                [t.row([
                    field(1, Value::U16(STATIC_EXECUTION_CONTRACT_VERSION)),
                    field(2, Value::U16(CONSTRAINT_CONTRACT_VERSION)),
                ])],
            )],
        ),
        t.section(
            7,
            [t.table(
                1,
                [t.row([
                    field(1, t.utf8(&provenance.compiler_build_id)),
                    field(2, Value::U16(SOURCE_COLLECTION_DIGEST_VERSION_V1)),
                    field(3, t.sha256(source_collection_digest)),
                    field(4, t.sha256(PORTABLE_COMPILE_OPTIONS_DIGEST_V1)),
                    field(5, Value::U16(CHUNKED_EMITTER_VERSION)),
                    field(6, Value::U8(accuracy_profile)),
                ])],
            )],
        ),
        t.section(
            8,
            [t.table(
                1,
                [t.row([field(
                    1,
                    t.sha256(declared_network_revision.into_digest().into_bytes()),
                )])],
            )],
        ),
        inputs.station.section(t),
    ])
}

fn identity_fields(
    t: &TreeBuilder,
    lir: &crate::lir::LirUnit,
    range: crate::arena::TableRange<crate::lir::LirIdentityField>,
) -> Value {
    t.records(
        lir.identity_fields[range.as_usize_range()]
            .iter()
            .map(|identity| {
                t.record([
                    field(1, Value::U16(identity.tag.code())),
                    field(
                        2,
                        t.bytes(&lir.identity_field_bytes[identity.value_bytes.as_usize_range()]),
                    ),
                ])
            }),
    )
}

fn canonical_identity_table(t: &TreeBuilder, lir: &crate::lir::LirUnit) -> TableRef {
    let written: Result<_, core::convert::Infallible> = t.table_with(1, || {
        macro_rules! append {
            ($kind:expr, $records:expr) => {
                for record in &$records {
                    t.row([
                        field(1, Value::U16($kind.code())),
                        field(2, Value::U32(record.ordinal.raw())),
                        field(3, Value::StableId128(stable_id_bytes(record.stable_id))),
                        field(4, identity_fields(t, lir, record.identity_fields)),
                    ]);
                }
            };
        }
        append!(EntityKind::RoadCorridor, lir.road_corridors);
        append!(EntityKind::RoadSection, lir.road_sections);
        append!(EntityKind::AuthoringLane, lir.authoring_lanes);
        append!(EntityKind::LaneEdge, lir.lane_edges);
        append!(EntityKind::Junction, lir.junctions);
        append!(EntityKind::Movement, lir.movements);
        append!(EntityKind::ManeuverPath, lir.maneuver_paths);
        append!(EntityKind::ManeuverGate, lir.maneuver_gates);
        append!(EntityKind::WaitingZone, lir.waiting_zones);
        append!(EntityKind::StopLine, lir.stop_lines);
        append!(EntityKind::SignalGroup, lir.signal_groups);
        append!(EntityKind::SignalController, lir.signal_controllers);
        append!(EntityKind::SignalPhase, lir.signal_phases);
        append!(EntityKind::ParkingFacility, lir.parking_facilities);
        append!(EntityKind::ParkingSpace, lir.parking_spaces);
        append!(EntityKind::LaneGroup, lir.lane_groups);
        append!(EntityKind::FacilityBand, lir.facility_bands);
        append!(EntityKind::ParticipantClass, lir.participant_classes);
        append!(EntityKind::AccessRule, lir.access_rules);
        append!(EntityKind::VehicleProfile, lir.vehicle_profiles);
        append!(EntityKind::ConflictZone, lir.conflict_zones);
        append!(EntityKind::CanonicalFrame, lir.canonical_frames);
        append!(EntityKind::ParticipantStream, lir.participant_streams);
        policy::identities(t, lir);
        Ok(())
    });
    match written {
        Ok(table) => table,
        Err(never) => match never {},
    }
}

fn canonical_entity_tables(
    t: &TreeBuilder,
    lir: &crate::lir::LirUnit,
    inputs: &LfcaInputs<'_>,
) -> Result<(), PortableEmissionError> {
    let internal_edges = &inputs.internal_edges;

    t.table(
        1,
        lir.road_corridors.iter().map(|record| {
            let elements = t.records(
                lir.corridor_elements[record.elements.as_usize_range()]
                    .iter()
                    .map(|element| match element {
                        crate::lir::LirCorridorElement::RoadSection(ordinal) => {
                            t.record([field(1, Value::U8(0)), field(2, Value::U32(ordinal.raw()))])
                        }
                        crate::lir::LirCorridorElement::FacilityBand(ordinal) => {
                            t.record([field(1, Value::U8(1)), field(2, Value::U32(ordinal.raw()))])
                        }
                    }),
            );
            t.row([
                field(1, Value::U32(record.ordinal.raw())),
                field(2, Value::StableId128(stable_id_bytes(record.stable_id))),
                field(3, Value::U32(record.reference_section.raw())),
                field(4, elements),
            ])
        }),
    );
    t.table(
        2,
        lir.road_sections.iter().map(|record| {
            t.row([
                field(1, Value::U32(record.ordinal.raw())),
                field(2, Value::StableId128(stable_id_bytes(record.stable_id))),
                field(3, Value::U32(record.road_corridor.raw())),
                field(4, t.utf8(&record.kind_id)),
                field(
                    5,
                    t.ordinals_of(&lir.road_section_lanes[record.lanes.as_usize_range()]),
                ),
            ])
        }),
    );
    t.table(
        3,
        lir.authoring_lanes.iter().map(|record| {
            t.row([
                Some(field(1, Value::U32(record.ordinal.raw()))),
                Some(field(
                    2,
                    Value::StableId128(stable_id_bytes(record.stable_id)),
                )),
                Some(field(3, Value::U32(record.road_section.raw()))),
                Some(field(
                    4,
                    t.ordinals_of(&lir.authoring_lane_edges[record.edge_chain.as_usize_range()]),
                )),
                record
                    .lane_group
                    .map(|group| field(5, Value::U32(group.raw()))),
            ])
        }),
    );
    t.table_with(4, || {
        for record in &lir.lane_edges {
            let length_mm = closed_lane_edge_length_mm(record)?;
            let successors = lir.lane_edge_successors[record.successors.as_usize_range()]
                .iter()
                .copied()
                .filter(|successor| {
                    !internal_edges[record.ordinal.index()] && !internal_edges[successor.index()]
                })
                .map(|ordinal| ordinal.raw());
            t.row([
                field(1, Value::U32(record.ordinal.raw())),
                field(2, Value::StableId128(stable_id_bytes(record.stable_id))),
                field(3, Value::U32(length_mm)),
                field(4, Value::U32(record.speed_limit_mm_s)),
                field(5, t.ordinals(successors)),
            ]);
        }
        Ok::<_, PortableEmissionError>(())
    })?;
    t.table(
        5,
        lir.junctions.iter().map(|record| {
            t.row([
                field(1, Value::U32(record.ordinal.raw())),
                field(2, Value::StableId128(stable_id_bytes(record.stable_id))),
                field(
                    3,
                    t.ordinals_of(&lir.junction_movements[record.movements.as_usize_range()]),
                ),
            ])
        }),
    );
    t.table(
        6,
        lir.movements.iter().map(|record| {
            t.row([
                Some(field(1, Value::U32(record.ordinal.raw()))),
                Some(field(
                    2,
                    Value::StableId128(stable_id_bytes(record.stable_id)),
                )),
                Some(field(3, Value::U32(record.junction.raw()))),
                Some(field(4, t.utf8(&record.directed_entry_approach_key))),
                Some(field(5, t.utf8(&record.directed_exit_approach_key))),
                Some(field(
                    6,
                    t.ordinals_of(
                        &lir.movement_maneuver_paths[record.maneuver_paths.as_usize_range()],
                    ),
                )),
                record
                    .turn_direction
                    .map(|direction| field(7, Value::U8(direction.code()))),
            ])
        }),
    );
    t.table(
        7,
        lir.maneuver_paths.iter().map(|record| {
            t.row([
                field(1, Value::U32(record.ordinal.raw())),
                field(2, Value::StableId128(stable_id_bytes(record.stable_id))),
                field(3, Value::U32(record.movement.raw())),
                field(
                    4,
                    t.ordinals_of(&lir.maneuver_path_edges[record.edges.as_usize_range()]),
                ),
                field(
                    5,
                    t.ordinals_of(&lir.maneuver_path_gates[record.maneuver_gates.as_usize_range()]),
                ),
                field(
                    6,
                    t.ordinals_of(
                        &lir.maneuver_path_waiting_zones[record.waiting_zones.as_usize_range()],
                    ),
                ),
            ])
        }),
    );
    t.table(
        8,
        lir.maneuver_gates.iter().map(|record| {
            let (control, group) = match record.signal_control {
                crate::lir::LirSignalControl::None => (0, None),
                crate::lir::LirSignalControl::Group(group) => (1, Some(group.raw())),
            };
            t.row([
                Some(field(1, Value::U32(record.ordinal.raw()))),
                Some(field(
                    2,
                    Value::StableId128(stable_id_bytes(record.stable_id)),
                )),
                Some(field(3, Value::U32(record.maneuver_path.raw()))),
                Some(field(4, Value::U32(record.transition_index))),
                Some(field(5, Value::U32(record.stop_line.raw()))),
                Some(field(6, Value::U8(control))),
                group.map(|group| field(7, Value::U32(group))),
            ])
        }),
    );
    t.table(
        9,
        lir.waiting_zones.iter().map(|record| {
            t.row([
                field(1, Value::U32(record.ordinal.raw())),
                field(2, Value::StableId128(stable_id_bytes(record.stable_id))),
                field(3, Value::U32(record.maneuver_path.raw())),
                field(4, Value::U32(record.entry_gate.raw())),
                field(5, Value::U32(record.release_gate.raw())),
                field(6, Value::U32(record.max_occupancy)),
            ])
        }),
    );
    t.table(
        10,
        lir.stop_lines.iter().map(|record| {
            t.row([
                field(1, Value::U32(record.ordinal.raw())),
                field(2, Value::StableId128(stable_id_bytes(record.stable_id))),
                field(3, Value::U32(record.lane_edge.raw())),
                field(
                    4,
                    t.ordinals_of(
                        &lir.stop_line_maneuver_gates[record.maneuver_gates.as_usize_range()],
                    ),
                ),
            ])
        }),
    );
    t.table(
        11,
        lir.signal_groups.iter().map(|record| {
            t.row([
                field(1, Value::U32(record.ordinal.raw())),
                field(2, Value::StableId128(stable_id_bytes(record.stable_id))),
                field(3, Value::U32(record.controller.raw())),
                field(
                    4,
                    t.ordinals_of(
                        &lir.signal_group_maneuver_gates[record.maneuver_gates.as_usize_range()],
                    ),
                ),
            ])
        }),
    );
    t.table(
        12,
        lir.signal_controllers.iter().map(|record| {
            t.row([
                field(1, Value::U32(record.ordinal.raw())),
                field(2, Value::StableId128(stable_id_bytes(record.stable_id))),
                field(3, Value::U64(record.offset_ms)),
                field(4, Value::U64(record.cycle_duration_ms)),
                field(
                    5,
                    t.ordinals_of(
                        &lir.signal_controller_groups[record.signal_groups.as_usize_range()],
                    ),
                ),
                field(
                    6,
                    t.ordinals_of(&lir.signal_controller_phases[record.phases.as_usize_range()]),
                ),
            ])
        }),
    );
    t.table(
        13,
        lir.signal_phases.iter().map(|record| {
            let states = t.records(
                lir.signal_phase_states[record.states.as_usize_range()]
                    .iter()
                    .map(|state| {
                        let aspect = match state.aspect {
                            laneflow_static_contract::SignalAspect::Red => 0,
                            laneflow_static_contract::SignalAspect::Yellow => 1,
                            laneflow_static_contract::SignalAspect::Green => 2,
                            _ => unreachable!(
                                "validated LIR only stores the closed v1 signal aspects"
                            ),
                        };
                        t.record([
                            field(1, Value::U32(state.signal_group.raw())),
                            field(2, Value::U8(aspect)),
                        ])
                    }),
            );
            t.row([
                field(1, Value::U32(record.ordinal.raw())),
                field(2, Value::StableId128(stable_id_bytes(record.stable_id))),
                field(3, Value::U32(record.controller.raw())),
                field(4, Value::U64(record.duration_ms)),
                field(5, states),
            ])
        }),
    );
    t.table(
        14,
        lir.parking_facilities.iter().map(|record| {
            macro_rules! anchors {
                ($anchors:expr) => {
                    t.records($anchors.iter().map(|anchor| {
                        t.record([
                            field(1, Value::U32(anchor.lane_edge.raw())),
                            field(2, Value::U32(anchor.progress_mm)),
                        ])
                    }))
                };
            }
            let virtual_entries = anchors!(
                lir.parking_facility_virtual_entries[record.virtual_entries.as_usize_range()]
            );
            let virtual_exits =
                anchors!(lir.parking_facility_virtual_exits[record.virtual_exits.as_usize_range()]);
            t.row([
                field(1, Value::U32(record.ordinal.raw())),
                field(2, Value::StableId128(stable_id_bytes(record.stable_id))),
                field(
                    3,
                    t.ordinals_of(
                        &lir.parking_facility_spaces[record.parking_spaces.as_usize_range()],
                    ),
                ),
                field(4, Value::U32(record.virtual_capacity)),
                field(5, virtual_entries),
                field(6, virtual_exits),
            ])
        }),
    );
    t.table_with(15, || {
        for record in &lir.parking_spaces {
            parking_space_row(t, lir, record)?;
        }
        Ok::<_, PortableEmissionError>(())
    })?;
    t.table(
        16,
        lir.lane_groups.iter().map(|record| {
            t.row([
                field(1, Value::U32(record.ordinal.raw())),
                field(2, Value::StableId128(stable_id_bytes(record.stable_id))),
                field(3, Value::U32(record.road_section.raw())),
                field(
                    4,
                    t.ordinals_of(&lir.lane_group_members[record.members.as_usize_range()]),
                ),
            ])
        }),
    );
    t.table(
        17,
        lir.facility_bands.iter().map(|record| {
            t.row([
                field(1, Value::U32(record.ordinal.raw())),
                field(2, Value::StableId128(stable_id_bytes(record.stable_id))),
                field(3, Value::U32(record.road_corridor.raw())),
                field(4, t.utf8(&record.kind_id)),
            ])
        }),
    );
    t.table(
        18,
        lir.participant_classes.iter().map(|record| {
            t.row([
                Some(field(1, Value::U32(record.ordinal.raw()))),
                Some(field(
                    2,
                    Value::StableId128(stable_id_bytes(record.stable_id)),
                )),
                record
                    .parent
                    .map(|parent| field(3, Value::U32(parent.raw()))),
                Some(field(4, Value::U32(record.depth))),
                Some(field(5, Value::U32(record.subtree_enter))),
                Some(field(6, Value::U32(record.subtree_exit))),
            ])
        }),
    );
    t.table(
        19,
        lir.access_rules.iter().map(|record| {
            let (target_kind, target_ordinal) = match record.target {
                crate::lir::LirAccessTarget::LaneEdge(ordinal) => (0, ordinal.raw()),
                crate::lir::LirAccessTarget::LaneGroup(ordinal) => (1, ordinal.raw()),
                crate::lir::LirAccessTarget::RoadSection(ordinal) => (2, ordinal.raw()),
                crate::lir::LirAccessTarget::ManeuverPath(ordinal) => (3, ordinal.raw()),
            };
            let effect = match record.effect {
                laneflow_static_contract::AccessEffect::Deny => 0,
                laneflow_static_contract::AccessEffect::Allow => 1,
                _ => unreachable!("validated LIR only stores the closed v1 access effects"),
            };
            let participant_classes = t.ordinals_of(
                &lir.access_rule_participant_classes[record.participant_classes.as_usize_range()],
            );
            let regulation = record.regulation.as_ref().map(|regulation| {
                field(
                    7,
                    t.records([t.record([
                        Some(field(1, t.utf8(&regulation.jurisdiction))),
                        Some(field(2, t.utf8(&regulation.version))),
                        regulation
                            .source
                            .as_ref()
                            .map(|source| field(3, t.utf8(source))),
                    ])]),
                )
            });
            t.row([
                Some(field(1, Value::U32(record.ordinal.raw()))),
                Some(field(
                    2,
                    Value::StableId128(stable_id_bytes(record.stable_id)),
                )),
                Some(field(3, Value::U8(target_kind))),
                Some(field(4, Value::U32(target_ordinal))),
                Some(field(5, Value::U8(effect))),
                Some(field(6, participant_classes)),
                regulation,
                Some(field(8, Value::I32(record.priority))),
            ])
        }),
    );
    t.table(
        20,
        lir.vehicle_profiles
            .iter()
            .map(|record| vehicle_profile_row(t, record)),
    );
    t.table(
        21,
        lir.conflict_zones.iter().map(|record| {
            t.row([
                field(1, Value::U32(record.ordinal.raw())),
                field(2, Value::StableId128(stable_id_bytes(record.stable_id))),
                field(3, Value::U32(record.junction.raw())),
            ])
        }),
    );
    t.table(
        22,
        lir.canonical_frames.iter().map(|record| {
            t.row([
                field(1, Value::U32(record.ordinal.raw())),
                field(2, Value::StableId128(stable_id_bytes(record.stable_id))),
            ])
        }),
    );
    t.table(
        23,
        lir.participant_streams.iter().map(|record| {
            let passages = t.records(
                lir.conflict_passages[record.passages.as_usize_range()]
                    .iter()
                    .map(|passage| conflict_passage_record(t, passage)),
            );
            t.row([
                field(1, Value::U32(record.ordinal.raw())),
                field(2, Value::StableId128(stable_id_bytes(record.stable_id))),
                field(3, Value::U32(record.junction.raw())),
                field(4, Value::U32(record.maneuver_path.raw())),
                field(5, passages),
            ])
        }),
    );
    policy::declarations(t, lir);
    Ok(())
}

fn conflict_passage_record(
    t: &TreeBuilder,
    passage: &crate::lir::LirConflictPassage,
) -> super::tree::RecordRef {
    let (entry_kind, entry_reference) = conflict_anchor_reference(passage.entry.reference);
    let (exit_kind, exit_reference) = conflict_anchor_reference(passage.exit.reference);
    t.record([
        Some(field(1, Value::U32(passage.conflict_zone.raw()))),
        Some(field(2, Value::U8(entry_kind))),
        Some(field(3, Value::U32(entry_reference))),
        passage
            .entry
            .progress_mm
            .map(|progress_mm| field(4, Value::U32(progress_mm))),
        Some(field(5, Value::U8(exit_kind))),
        Some(field(6, Value::U32(exit_reference))),
        passage
            .exit
            .progress_mm
            .map(|progress_mm| field(7, Value::U32(progress_mm))),
    ])
}

fn conflict_anchor_reference(reference: crate::lir::LirPathAnchorReference) -> (u8, u32) {
    match reference {
        crate::lir::LirPathAnchorReference::Gate(gate) => (0, gate.raw()),
        crate::lir::LirPathAnchorReference::EdgeBoundary(boundary_index) => (1, boundary_index),
        crate::lir::LirPathAnchorReference::Interior { path_edge_index } => (2, path_edge_index),
    }
}

/// 交通边长在 HIR 空间冻结时已提交为写出毫米。
fn closed_lane_edge_length_mm(
    record: &crate::lir::LirLaneEdge,
) -> Result<u32, PortableEmissionError> {
    if record.length_mm < laneflow_static_contract::MIN_LANE_EDGE_LENGTH_MM
        || record.length_mm > laneflow_static_contract::MAX_LANE_EDGE_LENGTH_MM
    {
        return Err(PortableEmissionError::InternalBindingMismatch);
    }
    Ok(record.length_mm)
}

fn parking_anchor_progress_mm(
    lir: &crate::lir::LirUnit,
    progress_mm: u32,
    edge: laneflow_static_contract::LaneEdgeOrdinal,
) -> Result<u32, PortableEmissionError> {
    let length_mm = closed_lane_edge_length_mm(
        lir.lane_edges
            .get(edge.index())
            .ok_or(PortableEmissionError::InternalBindingMismatch)?,
    )?;
    let min_mm = laneflow_static_contract::PARKING_ANCHOR_ENDPOINT_CLEARANCE_MM;
    let max_mm = length_mm.saturating_sub(min_mm);
    if !(min_mm..=max_mm).contains(&progress_mm) {
        return Err(PortableEmissionError::InternalBindingMismatch);
    }
    Ok(progress_mm)
}

fn parking_space_row(
    t: &TreeBuilder,
    lir: &crate::lir::LirUnit,
    record: &crate::lir::LirParkingSpace,
) -> Result<(), PortableEmissionError> {
    let entry_mm =
        parking_anchor_progress_mm(lir, record.entry.progress_mm, record.entry.lane_edge)?;
    let exit_mm = parking_anchor_progress_mm(lir, record.exit.progress_mm, record.exit.lane_edge)?;
    t.row([
        Some(field(1, Value::U32(record.ordinal.raw()))),
        Some(field(
            2,
            Value::StableId128(stable_id_bytes(record.stable_id)),
        )),
        record
            .parking_facility
            .map(|area| field(3, Value::U32(area.raw()))),
        Some(field(4, Value::U32(record.entry.lane_edge.raw()))),
        Some(field(5, Value::U32(entry_mm))),
        Some(field(6, Value::U32(record.exit.lane_edge.raw()))),
        Some(field(7, Value::U32(exit_mm))),
        Some(field(8, Value::I32(record.geometry.lateral_offset_mm))),
        Some(field(9, Value::F32(record.geometry.heading_offset_radians))),
        Some(field(10, Value::U32(record.geometry.length_mm))),
        Some(field(11, Value::U32(record.geometry.width_mm))),
    ]);
    Ok(())
}

fn vehicle_profile_row(
    t: &TreeBuilder,
    record: &crate::lir::LirVehicleProfile,
) -> super::tree::RowRef {
    let g = record.geometry;
    t.row([
        field(1, Value::U32(record.ordinal.raw())),
        field(2, Value::StableId128(stable_id_bytes(record.stable_id))),
        field(3, Value::U32(record.participant_class.raw())),
        field(4, Value::U32(record.length_mm)),
        field(5, Value::U32(record.desired_speed_mm_s)),
        field(6, Value::U32(record.min_gap_mm)),
        field(7, Value::F32(record.time_headway_seconds)),
        field(
            8,
            Value::F32(record.max_acceleration_meters_per_second_squared),
        ),
        field(
            9,
            Value::F32(record.comfortable_deceleration_meters_per_second_squared),
        ),
        field(
            10,
            Value::F32(record.emergency_deceleration_meters_per_second_squared),
        ),
        field(
            11,
            t.records([t.record([
                field(1, Value::U32(g.wheelbase_mm)),
                field(2, Value::U32(g.track_width_mm)),
                field(3, Value::I32(g.reference_height_mm)),
                field(4, Value::U32(g.rear_overhang_mm)),
                field(5, Value::U32(g.width_mm)),
                field(6, Value::I32(g.bottom_height_mm)),
                field(7, Value::I32(g.top_height_mm)),
                field(8, Value::F32(g.max_steering_angle_rad)),
                field(9, Value::F32(g.max_steering_rate_rad_s)),
            ])]),
        ),
    ])
}

fn canonical_relation_tables(t: &TreeBuilder, lir: &crate::lir::LirUnit) -> [TableRef; 5] {
    [
        t.table(
            1,
            lir.junction_internal_edges.iter().map(|record| {
                t.row([
                    field(1, Value::U32(record.edge.raw())),
                    field(2, Value::U32(record.junction.raw())),
                ])
            }),
        ),
        policy::evidence_table(t, lir),
        policy::gap_table(t, lir),
        policy::stream_table(t, lir),
        policy::gate_table(t, lir),
    ]
}

fn point_records(t: &TreeBuilder, points: &[crate::lir::LirCanonicalPoint3F32]) -> Value {
    t.records(points.iter().map(|point| {
        t.record([
            field(1, Value::F32(point.x)),
            field(2, Value::F32(point.y)),
            field(3, Value::F32(point.z)),
        ])
    }))
}

fn lane_edge_geometry_table(
    t: &TreeBuilder,
    lir: &crate::lir::LirUnit,
    inputs: &LfcaInputs<'_>,
) -> TableRef {
    t.table(
        2,
        lir.lane_edge_geometries
            .iter()
            .zip(inputs.lane_geometry_applies.iter())
            .enumerate()
            .map(|(lane_edge, (geometry, applies))| {
                let points =
                    point_records(t, &lir.canonical_points[geometry.points.as_usize_range()]);
                let segments = t.records(
                    lir.spatial_segments[geometry.segments.as_usize_range()]
                        .iter()
                        .map(|segment| {
                            t.record([
                                field(1, Value::F32(segment.length_meters)),
                                field(2, Value::F32(segment.cumulative_end_meters)),
                                field(3, Value::F32(segment.tangent[0])),
                                field(4, Value::F32(segment.tangent[1])),
                                field(5, Value::F32(segment.tangent[2])),
                                field(6, Value::F32(segment.up[0])),
                                field(7, Value::F32(segment.up[1])),
                                field(8, Value::F32(segment.up[2])),
                            ])
                        }),
                );
                t.row([
                    field(
                        1,
                        Value::U32(
                            u32::try_from(lane_edge)
                                .expect("compile limits cap geometry counts at u32"),
                        ),
                    ),
                    field(2, Value::U32(geometry.canonical_frame.raw())),
                    field(3, Value::F32(geometry.arc_length_meters)),
                    field(4, points),
                    field(5, segments),
                    field(6, Value::U8(u8::from(*applies))),
                ])
            }),
    )
}

fn facility_band_geometry_table(
    t: &TreeBuilder,
    lir: &crate::lir::LirUnit,
    inputs: &LfcaInputs<'_>,
) -> TableRef {
    t.table(
        3,
        lir.facility_band_geometries
            .iter()
            .zip(inputs.facility_geometry_applies.iter())
            .map(|(geometry, applies)| {
                let points =
                    point_records(t, &lir.canonical_points[geometry.points.as_usize_range()]);
                t.row([
                    field(1, Value::U32(geometry.facility_band.raw())),
                    field(2, Value::U32(geometry.canonical_frame.raw())),
                    field(3, points),
                    field(4, Value::U8(u8::from(*applies))),
                ])
            }),
    )
}

fn conflict_zone_region_table(t: &TreeBuilder, lir: &crate::lir::LirUnit) -> TableRef {
    t.table(
        4,
        lir.conflict_zone_regions.iter().map(|region| {
            let ring = t.records(
                lir.conflict_region_points[region.ring_xz.as_usize_range()]
                    .iter()
                    .map(|point| {
                        t.record([field(1, Value::F32(point.x)), field(2, Value::F32(point.z))])
                    }),
            );
            t.row([
                field(1, Value::U32(region.conflict_zone.raw())),
                field(2, Value::U32(region.canonical_frame.raw())),
                field(3, Value::F32(region.min_y)),
                field(4, Value::F32(region.max_y)),
                field(5, ring),
            ])
        }),
    )
}
