#![allow(clippy::type_complexity)]

use std::collections::{BTreeMap, BTreeSet};

use laneflow_format::{RegistryCheckedFieldValue, RegistryCheckedRowView, ValueCheckedObjectView};
use laneflow_static_contract::{
    AccessEffect, AccessRuleOrdinal, AuthoringLaneOrdinal, EntityKind, FacilityBandOrdinal,
    JunctionOrdinal, LaneEdgeOrdinal, LaneGroupOrdinal, MAX_ACCEL_METERS_PER_SECOND_SQUARED,
    MAX_MIN_GAP_MM, MAX_PARKING_LATERAL_OFFSET_ABS_MM, MAX_TIME_HEADWAY_SECONDS,
    MAX_VEHICLE_LENGTH_MM, MIN_ACCEL_METERS_PER_SECOND_SQUARED, MIN_PARKING_LATERAL_OFFSET_ABS_MM,
    MIN_SPEED_MM_S, MIN_VEHICLE_LENGTH_MM, ManeuverGateOrdinal, ManeuverPathOrdinal,
    MovementOrdinal, PARKING_ANCHOR_ENDPOINT_CLEARANCE_MM, ParkingFacilityOrdinal,
    ParkingSpaceOrdinal, ParticipantClassOrdinal, RoadCorridorOrdinal, RoadSectionOrdinal,
    SignalAspect, SignalControllerOrdinal, SignalGroupOrdinal, SignalPhaseOrdinal, StableId128,
    StopLineOrdinal, WaitingZoneOrdinal,
};

use crate::builder::{
    SharedNetworkBuildOptions, allocate_vec, checked_f32, checked_field, checked_i32,
    checked_ordinal_vector, checked_record_vector, checked_stable_id, checked_u8, checked_u32,
    heading_f32_stored, poll_cancelled, u32_in_closed_range,
};
use crate::relations::{
    ACCESS_UNCONSTRAINED_ROW, AccessCell, AccessTarget, CorridorElement, FacilityKind,
    ParkingLaneAnchor, RelationPayloads, SharedRelationClosure, assemble, empty_optional,
    get_optional, set_optional,
};
use crate::{BuildError, BuildStructure, EntityCounts, RangeU32, SharedManeuverNetwork};

const STRUCTURE: BuildStructure = BuildStructure::RelationClosure;
const MAX_PORTABLE_SIGNAL_TIME_MS: u64 = 9_007_199_254_740_991;

#[allow(clippy::too_many_arguments)]
pub(crate) fn build_relations(
    view: ValueCheckedObjectView<'_>,
    entity_counts: &EntityCounts,
    lane_lengths: &[u32],
    successor_ranges: &[RangeU32],
    successors: &[LaneEdgeOrdinal],
    maneuvers: &SharedManeuverNetwork,
    options: SharedNetworkBuildOptions<'_>,
) -> Result<SharedRelationClosure, BuildError> {
    let unique_limit = max_entity_count(entity_counts).max(1);
    let mut unique = UniqueCheck::new(unique_limit)?;
    let mut intern = Intern::new(
        options
            .limits()
            .max_scratch_bytes()
            .saturating_sub(unique_stamp_bytes(unique_limit)?),
    );
    let lane_count = entity_counts.count(EntityKind::LaneEdge);
    let mut edge_authoring = empty_optional(lane_count)?;
    let mut edge_junction = empty_optional(lane_count)?;
    let mut edge_stop_line = empty_optional(lane_count)?;

    let (corridor_reference_section, corridor_element_ranges, corridor_elements) =
        build_corridors(view, entity_counts, options)?;
    let (section_corridor, section_kind, section_lane_ranges, section_lanes) =
        build_sections(view, entity_counts, &mut intern, &mut unique, options)?;
    let (authoring_section, authoring_edge_ranges, authoring_edges, authoring_group) =
        build_authoring_lanes(
            view,
            entity_counts,
            lane_count,
            &mut edge_authoring,
            options,
        )?;
    let (lane_group_section, lane_group_member_ranges, lane_group_members) =
        build_lane_groups(view, entity_counts, &mut unique, options)?;
    close_overlay_membership(
        &section_lane_ranges,
        &section_lanes,
        &authoring_section,
        &lane_group_section,
        &lane_group_member_ranges,
        &lane_group_members,
        &authoring_group,
        options,
    )?;
    let (band_corridor, band_kind) =
        build_facility_bands(view, entity_counts, &mut intern, options)?;
    close_corridor_ownership(
        &corridor_reference_section,
        &corridor_element_ranges,
        &corridor_elements,
        &section_corridor,
        &band_corridor,
        options,
    )?;
    let (
        junction_movement_ranges,
        junction_movements,
        movement_junction,
        movement_path_ranges,
        movement_paths,
    ) = build_junctions(view, entity_counts, &mut unique, options)?;
    close_owner_members(
        &junction_movement_ranges,
        &junction_movements,
        &movement_junction,
        MovementOrdinal::index,
        JunctionOrdinal::from_raw,
        entity_counts.count(EntityKind::Junction),
        options,
    )?;
    build_internal_edges(
        view,
        lane_count,
        entity_counts.count(EntityKind::Junction),
        &mut edge_junction,
        options,
    )?;
    close_internal_edges(
        maneuvers,
        &movement_junction,
        &edge_junction,
        lane_count,
        options,
    )?;
    close_authoring_chains(
        &authoring_edge_ranges,
        &authoring_edges,
        successor_ranges,
        successors,
        maneuvers,
        &edge_junction,
        options,
    )?;
    let (
        stop_line_edge,
        stop_line_gate_ranges,
        stop_line_gates,
        gate_path,
        gate_transition_index,
        gate_stop_line,
        gate_signal_group,
        waiting_path,
        waiting_entry_gate,
        waiting_release_gate,
        waiting_max_occupancy,
    ) = build_gates_and_waiting(
        view,
        entity_counts,
        lane_count,
        &mut edge_stop_line,
        &mut unique,
        options,
    )?;
    close_owner_members(
        maneuvers.path_gate_ranges(),
        maneuvers.path_gates(),
        &gate_path,
        ManeuverGateOrdinal::index,
        ManeuverPathOrdinal::from_raw,
        entity_counts.count(EntityKind::ManeuverPath),
        options,
    )?;
    close_owner_members(
        maneuvers.path_waiting_ranges(),
        maneuvers.path_waiting_zones(),
        &waiting_path,
        WaitingZoneOrdinal::index,
        ManeuverPathOrdinal::from_raw,
        entity_counts.count(EntityKind::ManeuverPath),
        options,
    )?;
    let signals = build_signals(view, entity_counts, &mut unique, options)?;
    close_owner_members(
        &signals.controller_group_ranges,
        &signals.controller_groups,
        &signals.group_controller,
        SignalGroupOrdinal::index,
        SignalControllerOrdinal::from_raw,
        entity_counts.count(EntityKind::SignalController),
        options,
    )?;
    close_optional_owner_members(
        &signals.group_gate_ranges,
        &signals.group_gates,
        &gate_signal_group,
        ManeuverGateOrdinal::raw,
        SignalGroupOrdinal::from_raw,
        entity_counts.count(EntityKind::ManeuverGate),
        options,
    )?;
    let parking = build_parking(
        view,
        entity_counts,
        lane_count,
        lane_lengths,
        &mut unique,
        options,
    )?;
    close_optional_owner_members(
        &parking.parking_space_ranges,
        &parking.parking_spaces,
        &parking.space_area,
        ParkingSpaceOrdinal::raw,
        ParkingFacilityOrdinal::from_raw,
        entity_counts.count(EntityKind::ParkingSpace),
        options,
    )?;
    let mut classes = build_classes(view, entity_counts, options)?;
    close_class_forest(&mut classes, options)?;
    let rules = build_access_rules(view, entity_counts, &mut unique, options)?;
    let profiles = build_profiles(view, entity_counts, options)?;
    let (edge_row_starts, edge_cells, path_row_starts, path_cells, access_class_count) =
        resolve_access_planes(
            entity_counts,
            &edge_authoring,
            &authoring_section,
            &authoring_group,
            &lane_group_members,
            &lane_group_member_ranges,
            &lane_group_section,
            &section_lanes,
            &section_lane_ranges,
            &authoring_edges,
            &authoring_edge_ranges,
            &classes,
            &rules,
            options,
        )?;

    let closure = assemble(
        intern.seal(),
        corridor_reference_section,
        corridor_element_ranges,
        corridor_elements,
        section_corridor,
        section_kind,
        section_lane_ranges,
        section_lanes,
        authoring_section,
        authoring_edge_ranges,
        authoring_edges,
        authoring_group,
        edge_authoring,
        edge_junction,
        edge_stop_line,
        junction_movement_ranges,
        junction_movements,
        movement_junction,
        movement_path_ranges,
        movement_paths,
        stop_line_edge,
        stop_line_gate_ranges,
        stop_line_gates,
        gate_path,
        gate_transition_index,
        gate_stop_line,
        gate_signal_group,
        waiting_path,
        waiting_entry_gate,
        waiting_release_gate,
        waiting_max_occupancy,
        signals.group_controller,
        signals.group_gate_ranges,
        signals.group_gates,
        signals.controller_offset_ms,
        signals.controller_cycle_ms,
        signals.controller_group_ranges,
        signals.controller_groups,
        signals.controller_phase_ranges,
        signals.controller_phases,
        signals.phase_controller,
        signals.phase_duration_ms,
        signals.phase_end_offset_ms,
        signals.phase_state_ranges,
        signals.phase_state_groups,
        signals.phase_state_aspects,
        parking.parking_space_ranges,
        parking.parking_spaces,
        parking.virtual_capacity,
        parking.virtual_entry_ranges,
        parking.virtual_entries,
        parking.virtual_exit_ranges,
        parking.virtual_exits,
        parking.space_area,
        parking.space_entry_edge,
        parking.space_entry_progress,
        parking.space_exit_edge,
        parking.space_exit_progress,
        parking.space_lateral,
        parking.space_heading,
        parking.space_length,
        parking.space_width,
        lane_group_section,
        lane_group_member_ranges,
        lane_group_members,
        band_corridor,
        band_kind,
        classes.parent,
        classes.depth,
        classes.subtree_enter,
        classes.subtree_exit,
        rules.target,
        rules.effect,
        rules.class_ranges,
        rules.classes,
        rules.priority,
        profiles.class,
        profiles.length,
        profiles.desired_speed,
        profiles.min_gap,
        profiles.time_headway,
        profiles.max_accel,
        profiles.comfort_decel,
        profiles.emergency_decel,
        access_class_count,
        edge_row_starts,
        edge_cells,
        path_row_starts,
        path_cells,
    );
    if closure.retained_logical_bytes() > options.limits().max_retained_bytes() {
        return Err(BuildError::BudgetExceeded {
            structure: BuildStructure::RetainedOutput,
            required: closure.retained_logical_bytes(),
            limit: options.limits().max_retained_bytes(),
        });
    }
    Ok(closure)
}

struct Intern {
    by_token: BTreeMap<Box<str>, u32>,
    used: u64,
    limit: u64,
}

impl Intern {
    fn new(limit: u64) -> Self {
        Self {
            by_token: BTreeMap::new(),
            used: 0,
            limit,
        }
    }

    fn intern(&mut self, token: &str) -> Result<u32, BuildError> {
        if let Some(&index) = self.by_token.get(token) {
            return Ok(index);
        }
        let extra = intern_entry_bytes(token.len())?;
        let used = self
            .used
            .checked_add(extra)
            .ok_or(BuildError::ArithmeticOverflow {
                structure: BuildStructure::BuilderScratch,
            })?;
        if used > self.limit {
            return Err(BuildError::BudgetExceeded {
                structure: BuildStructure::BuilderScratch,
                required: used,
                limit: self.limit,
            });
        }
        let index =
            u32::try_from(self.by_token.len()).map_err(|_| BuildError::ArithmeticOverflow {
                structure: STRUCTURE,
            })?;
        self.by_token.insert(token.into(), index);
        self.used = used;
        Ok(index)
    }

    fn seal(self) -> Box<[Box<str>]> {
        let mut tokens = vec![None; self.by_token.len()];
        for (token, index) in self.by_token {
            tokens[usize::try_from(index).expect("u32 fits")] = Some(token);
        }
        tokens
            .into_iter()
            .map(|token| token.expect("dense intern"))
            .collect::<Vec<_>>()
            .into_boxed_slice()
    }
}

fn intern_entry_bytes(token_len: usize) -> Result<u64, BuildError> {
    let node =
        u64::try_from(core::mem::size_of::<(Box<str>, u32)>() + 3 * core::mem::size_of::<usize>())
            .map_err(|_| BuildError::ArithmeticOverflow {
                structure: BuildStructure::BuilderScratch,
            })?;
    let payload = u64::try_from(token_len).map_err(|_| BuildError::ArithmeticOverflow {
        structure: BuildStructure::BuilderScratch,
    })?;
    node.checked_add(payload)
        .ok_or(BuildError::ArithmeticOverflow {
            structure: BuildStructure::BuilderScratch,
        })
}

fn unique_stamp_bytes(limit: u32) -> Result<u64, BuildError> {
    u64::from(limit)
        .checked_mul(u64::try_from(core::mem::size_of::<u32>()).map_err(|_| {
            BuildError::ArithmeticOverflow {
                structure: BuildStructure::BuilderScratch,
            }
        })?)
        .ok_or(BuildError::ArithmeticOverflow {
            structure: BuildStructure::BuilderScratch,
        })
}

fn entity_table<'a>(
    view: ValueCheckedObjectView<'a>,
    kind: EntityKind,
) -> Result<laneflow_format::RegistryCheckedTableView<'a>, BuildError> {
    let want = kind.code();
    let section = view
        .registry_view()
        .section(2)
        .ok_or(BuildError::InputInvariant {
            structure: STRUCTURE,
        })?;
    for (index, _) in section.tables().enumerate() {
        let table = section
            .table(u32::try_from(index).expect("table index fits u32"))
            .ok_or(BuildError::InputInvariant {
                structure: STRUCTURE,
            })?;
        if table.kind() == want {
            return Ok(table);
        }
    }
    Err(BuildError::InputInvariant {
        structure: STRUCTURE,
    })
}

fn relation_table<'a>(
    view: ValueCheckedObjectView<'a>,
    index: usize,
) -> Result<laneflow_format::RegistryCheckedTableView<'a>, BuildError> {
    view.registry_view()
        .section(3)
        .and_then(|section| section.table(u32::try_from(index).expect("table index fits u32")))
        .ok_or(BuildError::InputInvariant {
            structure: STRUCTURE,
        })
}

fn dest_len(len: usize) -> Result<u32, BuildError> {
    u32::try_from(len).map_err(|_| BuildError::ArithmeticOverflow {
        structure: STRUCTURE,
    })
}

pub(crate) fn count_relation_payloads(
    view: ValueCheckedObjectView<'_>,
    entity_counts: &EntityCounts,
    options: SharedNetworkBuildOptions<'_>,
) -> Result<RelationPayloads, BuildError> {
    let (intern_keys, intern_utf8) = count_intern_payloads(view, options)?;
    let corridor_elements = sum_record_field(view, EntityKind::RoadCorridor, 4, options)?;
    let section_lanes = sum_ordinal_field(view, EntityKind::RoadSection, 5, options)?;
    let authoring_edges = sum_ordinal_field(view, EntityKind::AuthoringLane, 4, options)?;
    let junction_movements = sum_ordinal_field(view, EntityKind::Junction, 3, options)?;
    let movement_paths = sum_ordinal_field(view, EntityKind::Movement, 6, options)?;
    let stop_line_gates = sum_ordinal_field(view, EntityKind::StopLine, 4, options)?;
    let group_gates = sum_ordinal_field(view, EntityKind::SignalGroup, 4, options)?;
    let controller_groups = sum_ordinal_field(view, EntityKind::SignalController, 5, options)?;
    let controller_phases = sum_ordinal_field(view, EntityKind::SignalController, 6, options)?;
    let phase_states = sum_record_field(view, EntityKind::SignalPhase, 5, options)?;
    let parking_spaces = sum_ordinal_field(view, EntityKind::ParkingFacility, 3, options)?;
    let parking_virtual_entries = sum_record_field(view, EntityKind::ParkingFacility, 5, options)?;
    let parking_virtual_exits = sum_record_field(view, EntityKind::ParkingFacility, 6, options)?;
    let lane_group_members = sum_ordinal_field(view, EntityKind::LaneGroup, 4, options)?;
    let rule_classes = sum_ordinal_field(view, EntityKind::AccessRule, 6, options)?;
    let pass_a_scratch = relation_pass_a_scratch(entity_counts, intern_keys, intern_utf8)?;
    Ok(RelationPayloads {
        corridor_elements,
        section_lanes,
        authoring_edges,
        junction_movements,
        movement_paths,
        stop_line_gates,
        group_gates,
        controller_groups,
        controller_phases,
        phase_states,
        parking_spaces,
        parking_virtual_entries,
        parking_virtual_exits,
        lane_group_members,
        rule_classes,
        intern_keys,
        intern_utf8,
        edge_cells: 0,
        path_cells: 0,
        pass_a_scratch,
    })
}

pub(crate) fn finish_relation_payloads(
    view: ValueCheckedObjectView<'_>,
    entity_counts: &EntityCounts,
    mut payloads: RelationPayloads,
    options: SharedNetworkBuildOptions<'_>,
) -> Result<RelationPayloads, BuildError> {
    let (edge_cells, path_cells) = count_access_cells(view, entity_counts, options)?;
    payloads.edge_cells = edge_cells;
    payloads.path_cells = path_cells;
    Ok(payloads)
}

fn pass_a_count_bytes<T>(count: u32) -> Result<u64, BuildError> {
    u64::from(count)
        .checked_mul(u64::try_from(core::mem::size_of::<T>()).map_err(|_| {
            BuildError::ArithmeticOverflow {
                structure: BuildStructure::BuilderScratch,
            }
        })?)
        .ok_or(BuildError::ArithmeticOverflow {
            structure: BuildStructure::BuilderScratch,
        })
}

fn relation_pass_a_scratch(
    entity_counts: &EntityCounts,
    intern_keys: u32,
    intern_utf8: u32,
) -> Result<u64, BuildError> {
    let intern = intern_entry_bytes(0)?
        .checked_mul(u64::from(intern_keys))
        .and_then(|bytes| bytes.checked_add(u64::from(intern_utf8)))
        .ok_or(BuildError::ArithmeticOverflow {
            structure: BuildStructure::BuilderScratch,
        })?;
    let lane = entity_counts.count(EntityKind::LaneEdge);
    let group = entity_counts.count(EntityKind::LaneGroup);
    let section = entity_counts.count(EntityKind::RoadSection);
    let path = entity_counts.count(EntityKind::ManeuverPath);
    let authoring = entity_counts.count(EntityKind::AuthoringLane);
    let flags = lane
        .checked_add(group)
        .and_then(|count| count.checked_add(section))
        .and_then(|count| count.checked_add(path))
        .and_then(|count| count.checked_add(section))
        .and_then(|count| count.checked_add(group))
        .ok_or(BuildError::ArithmeticOverflow {
            structure: BuildStructure::BuilderScratch,
        })?;
    let access = pass_a_count_bytes::<bool>(flags)?;
    let access = access
        .checked_add(pass_a_count_bytes::<u32>(authoring)?)
        .ok_or(BuildError::ArithmeticOverflow {
            structure: BuildStructure::BuilderScratch,
        })?;
    let access = access
        .checked_add(pass_a_count_bytes::<Option<u32>>(authoring)?)
        .ok_or(BuildError::ArithmeticOverflow {
            structure: BuildStructure::BuilderScratch,
        })?;
    let access = access
        .checked_add(pass_a_count_bytes::<Option<u32>>(lane)?)
        .ok_or(BuildError::ArithmeticOverflow {
            structure: BuildStructure::BuilderScratch,
        })?;
    let parking = pass_a_count_bytes::<StableId128>(lane)?;
    Ok(intern.max(access).max(parking))
}

fn interned_facility_token(token: &str, lane_bearing: bool) -> Option<&str> {
    if lane_bearing {
        (token.starts_with("x-lane-") && token.len() > "x-lane-".len()).then_some(token)
    } else if token.starts_with("x-") && token.len() > 2 && !token.starts_with("x-lane-") {
        Some(token)
    } else {
        None
    }
}

fn count_intern_payloads(
    view: ValueCheckedObjectView<'_>,
    options: SharedNetworkBuildOptions<'_>,
) -> Result<(u32, u32), BuildError> {
    let mut seen = BTreeSet::<&str>::new();
    let mut utf8 = 0_u32;
    for (kind, lane_bearing) in [
        (EntityKind::RoadSection, true),
        (EntityKind::FacilityBand, false),
    ] {
        let table = entity_table(view, kind)?;
        for (index, row) in table.rows().enumerate() {
            poll_cancelled(options, u32::try_from(index).unwrap_or(u32::MAX))?;
            let Some(token) = interned_facility_token(checked_utf8(row, 4)?, lane_bearing) else {
                continue;
            };
            if !seen.insert(token) {
                continue;
            }
            utf8 = utf8
                .checked_add(u32::try_from(token.len()).map_err(|_| {
                    BuildError::ArithmeticOverflow {
                        structure: STRUCTURE,
                    }
                })?)
                .ok_or(BuildError::ArithmeticOverflow {
                    structure: STRUCTURE,
                })?;
        }
    }
    let keys = u32::try_from(seen.len()).map_err(|_| BuildError::ArithmeticOverflow {
        structure: STRUCTURE,
    })?;
    Ok((keys, utf8))
}

fn mark_constrained(flags: &mut [bool], ordinal: u32, limit: u32) -> Result<(), BuildError> {
    if ordinal >= limit {
        return Err(BuildError::ReferenceOutOfBounds {
            structure: BuildStructure::AccessPlane,
            ordinal,
            limit,
        });
    }
    flags[usize::try_from(ordinal).expect("u32 fits")] = true;
    Ok(())
}

fn count_access_cells(
    view: ValueCheckedObjectView<'_>,
    entity_counts: &EntityCounts,
    options: SharedNetworkBuildOptions<'_>,
) -> Result<(u32, u32), BuildError> {
    let class_count = entity_counts.count(EntityKind::ParticipantClass);
    let lane_count = entity_counts.count(EntityKind::LaneEdge);
    let group_count = entity_counts.count(EntityKind::LaneGroup);
    let section_count = entity_counts.count(EntityKind::RoadSection);
    let path_count = entity_counts.count(EntityKind::ManeuverPath);
    let mut edge_direct = vec![false; usize::try_from(lane_count).expect("u32 fits")];
    let mut group_rules = vec![false; usize::try_from(group_count).expect("u32 fits")];
    let mut section_rules = vec![false; usize::try_from(section_count).expect("u32 fits")];
    let mut path_rules = vec![false; usize::try_from(path_count).expect("u32 fits")];
    let rule_table = entity_table(view, EntityKind::AccessRule)?;
    for (index, row) in rule_table.rows().enumerate() {
        poll_cancelled(options, u32::try_from(index).unwrap_or(u32::MAX))?;
        let kind = checked_u8(row, 3, STRUCTURE)?;
        let ordinal = checked_u32(row, 4, STRUCTURE)?;
        match kind {
            0 => mark_constrained(&mut edge_direct, ordinal, lane_count)?,
            1 => mark_constrained(&mut group_rules, ordinal, group_count)?,
            2 => mark_constrained(&mut section_rules, ordinal, section_count)?,
            3 => mark_constrained(&mut path_rules, ordinal, path_count)?,
            _ => {
                return Err(BuildError::InputInvariant {
                    structure: BuildStructure::AccessPlane,
                });
            }
        }
    }
    if class_count == 0 {
        return Ok((0, 0));
    }
    let authoring_count = entity_counts.count(EntityKind::AuthoringLane);
    let mut authoring_section = vec![0_u32; usize::try_from(authoring_count).expect("u32 fits")];
    let mut authoring_group = vec![None; usize::try_from(authoring_count).expect("u32 fits")];
    let mut edge_authoring = vec![None; usize::try_from(lane_count).expect("u32 fits")];
    let authoring_table = entity_table(view, EntityKind::AuthoringLane)?;
    for (index, row) in authoring_table.rows().enumerate() {
        let expected = u32::try_from(index).map_err(|_| BuildError::ArithmeticOverflow {
            structure: STRUCTURE,
        })?;
        poll_cancelled(options, expected)?;
        let section = checked_u32(row, 3, STRUCTURE)?;
        if section >= section_count {
            return Err(BuildError::ReferenceOutOfBounds {
                structure: STRUCTURE,
                ordinal: section,
                limit: section_count,
            });
        }
        authoring_section[index] = section;
        if let Some(group) = optional_u32(row, 5)? {
            if group >= group_count {
                return Err(BuildError::ReferenceOutOfBounds {
                    structure: STRUCTURE,
                    ordinal: group,
                    limit: group_count,
                });
            }
            authoring_group[index] = Some(group);
        }
        let chain = checked_ordinal_vector(row, 4, STRUCTURE)?;
        for chain_index in 0..chain.len() {
            poll_cancelled(options, chain_index)?;
            let edge = chain.get(chain_index).ok_or(BuildError::InputInvariant {
                structure: STRUCTURE,
            })?;
            if edge >= lane_count {
                return Err(BuildError::ReferenceOutOfBounds {
                    structure: STRUCTURE,
                    ordinal: edge,
                    limit: lane_count,
                });
            }
            edge_authoring[usize::try_from(edge).expect("u32 fits")] = Some(expected);
        }
    }
    let mut section_used = vec![false; usize::try_from(section_count).expect("u32 fits")];
    let mut group_used = vec![false; usize::try_from(group_count).expect("u32 fits")];
    let mut direct_rows = 0_u32;
    for edge in 0..edge_direct.len() {
        if edge_direct[edge] {
            direct_rows = direct_rows
                .checked_add(1)
                .ok_or(BuildError::ArithmeticOverflow {
                    structure: BuildStructure::AccessPlane,
                })?;
            continue;
        }
        let Some(lane) = edge_authoring[edge] else {
            continue;
        };
        let lane_index = usize::try_from(lane).expect("u32 fits");
        match authoring_group[lane_index] {
            Some(group) if group_rules[usize::try_from(group).expect("u32 fits")] => {
                group_used[usize::try_from(group).expect("u32 fits")] = true;
            }
            _ => {
                let section = authoring_section[lane_index];
                if section_rules[usize::try_from(section).expect("u32 fits")] {
                    section_used[usize::try_from(section).expect("u32 fits")] = true;
                }
            }
        }
    }
    let context_rows = u32::try_from(
        section_used.iter().filter(|used| **used).count()
            + group_used.iter().filter(|used| **used).count(),
    )
    .map_err(|_| BuildError::ArithmeticOverflow {
        structure: BuildStructure::AccessPlane,
    })?;
    let edge_rows =
        direct_rows
            .checked_add(context_rows)
            .ok_or(BuildError::ArithmeticOverflow {
                structure: BuildStructure::AccessPlane,
            })?;
    let path_rows =
        u32::try_from(path_rules.iter().filter(|used| **used).count()).map_err(|_| {
            BuildError::ArithmeticOverflow {
                structure: BuildStructure::AccessPlane,
            }
        })?;
    let edge_cells = edge_rows
        .checked_mul(class_count)
        .ok_or(BuildError::ArithmeticOverflow {
            structure: BuildStructure::AccessPlane,
        })?;
    let path_cells = path_rows
        .checked_mul(class_count)
        .ok_or(BuildError::ArithmeticOverflow {
            structure: BuildStructure::AccessPlane,
        })?;
    Ok((edge_cells, path_cells))
}

fn sum_ordinal_field(
    view: ValueCheckedObjectView<'_>,
    kind: EntityKind,
    tag: u16,
    options: SharedNetworkBuildOptions<'_>,
) -> Result<u32, BuildError> {
    let table = entity_table(view, kind)?;
    let mut total = 0_u32;
    for (index, row) in table.rows().enumerate() {
        poll_cancelled(options, u32::try_from(index).unwrap_or(u32::MAX))?;
        total = total
            .checked_add(checked_ordinal_vector(row, tag, STRUCTURE)?.len())
            .ok_or(BuildError::ArithmeticOverflow {
                structure: STRUCTURE,
            })?;
    }
    Ok(total)
}

fn sum_record_field(
    view: ValueCheckedObjectView<'_>,
    kind: EntityKind,
    tag: u16,
    options: SharedNetworkBuildOptions<'_>,
) -> Result<u32, BuildError> {
    let table = entity_table(view, kind)?;
    let mut total = 0_u32;
    for (index, row) in table.rows().enumerate() {
        poll_cancelled(options, u32::try_from(index).unwrap_or(u32::MAX))?;
        total = total
            .checked_add(checked_record_vector(row, tag, STRUCTURE)?.len())
            .ok_or(BuildError::ArithmeticOverflow {
                structure: STRUCTURE,
            })?;
    }
    Ok(total)
}

fn expect_row_ordinal(row: RegistryCheckedRowView<'_>, expected: u32) -> Result<(), BuildError> {
    let actual = checked_u32(row, 1, STRUCTURE)?;
    if actual != expected {
        return Err(BuildError::UnexpectedOrdinal {
            structure: STRUCTURE,
            expected,
            actual,
        });
    }
    Ok(())
}

fn checked_u64(row: RegistryCheckedRowView<'_>, tag: u16) -> Result<u64, BuildError> {
    match checked_field(row, tag, STRUCTURE)? {
        RegistryCheckedFieldValue::U64(value) => Ok(value),
        _ => Err(BuildError::InputInvariant {
            structure: STRUCTURE,
        }),
    }
}

fn checked_utf8<'a>(row: RegistryCheckedRowView<'a>, tag: u16) -> Result<&'a str, BuildError> {
    match checked_field(row, tag, STRUCTURE)? {
        RegistryCheckedFieldValue::Utf8(value) => Ok(value),
        _ => Err(BuildError::InputInvariant {
            structure: STRUCTURE,
        }),
    }
}

fn optional_u32(row: RegistryCheckedRowView<'_>, tag: u16) -> Result<Option<u32>, BuildError> {
    match row.field_by_tag(tag) {
        None => Ok(None),
        Some(field) => match field.value().map_err(|_| BuildError::InputInvariant {
            structure: STRUCTURE,
        })? {
            RegistryCheckedFieldValue::U32(value) => Ok(Some(value)),
            _ => Err(BuildError::InputInvariant {
                structure: STRUCTURE,
            }),
        },
    }
}

#[derive(Clone, Copy)]
enum MemberOrder {
    /// 规范集合：严格递增，兼作唯一性。
    CanonicalSet,
    /// 领域序：保留 LFCA 向量顺序，只拒绝越界与重复。
    Sequence,
}

fn max_entity_count(entity_counts: &EntityCounts) -> u32 {
    EntityKind::ALL
        .iter()
        .map(|kind| entity_counts.count(*kind))
        .max()
        .unwrap_or(0)
}

struct UniqueCheck {
    generation: u32,
    stamps: Vec<u32>,
}

impl UniqueCheck {
    fn new(limit: u32) -> Result<Self, BuildError> {
        let capacity = limit.max(1);
        let mut stamps = allocate_vec(capacity, STRUCTURE)?;
        stamps.resize(usize::try_from(capacity).expect("u32 fits"), 0);
        Ok(Self {
            generation: 1,
            stamps,
        })
    }

    fn ensure_unique(&mut self, members: &[u32]) -> Result<(), BuildError> {
        if members.len() < 2 {
            return Ok(());
        }
        if self.generation == u32::MAX {
            self.stamps.fill(0);
            self.generation = 1;
        }
        let generation = self.generation;
        self.generation += 1;
        let stamp_limit = u32::try_from(self.stamps.len()).unwrap_or(u32::MAX);
        for &member in members {
            let slot = usize::try_from(member).expect("u32 fits");
            let stamp = self
                .stamps
                .get_mut(slot)
                .ok_or(BuildError::ReferenceOutOfBounds {
                    structure: STRUCTURE,
                    ordinal: member,
                    limit: stamp_limit,
                })?;
            if *stamp == generation {
                return Err(BuildError::InputInvariant {
                    structure: STRUCTURE,
                });
            }
            *stamp = generation;
        }
        Ok(())
    }
}

fn push_members(
    members: laneflow_format::RegistryCheckedOrdinalVectorView<'_>,
    dest: &mut Vec<u32>,
    limit: u32,
    order: MemberOrder,
    options: SharedNetworkBuildOptions<'_>,
    unique: &mut UniqueCheck,
) -> Result<RangeU32, BuildError> {
    let start = u32::try_from(dest.len()).map_err(|_| BuildError::ArithmeticOverflow {
        structure: STRUCTURE,
    })?;
    let mut previous = None;
    for index in 0..members.len() {
        poll_cancelled(options, index)?;
        let member = members.get(index).ok_or(BuildError::InputInvariant {
            structure: STRUCTURE,
        })?;
        if member >= limit {
            return Err(BuildError::ReferenceOutOfBounds {
                structure: STRUCTURE,
                ordinal: member,
                limit,
            });
        }
        if let (MemberOrder::CanonicalSet, Some(previous)) = (order, previous)
            && member <= previous
        {
            return Err(BuildError::NonCanonicalOrder {
                structure: STRUCTURE,
                previous,
                actual: member,
            });
        }
        previous = Some(member);
        dest.push(member);
    }
    if matches!(order, MemberOrder::Sequence) {
        unique.ensure_unique(&dest[usize::try_from(start).expect("u32 fits")..])?;
    }
    Ok(RangeU32::new(start, members.len()))
}

#[allow(clippy::too_many_arguments)]
fn close_overlay_membership(
    section_lane_ranges: &[RangeU32],
    section_lanes: &[AuthoringLaneOrdinal],
    authoring_section: &[RoadSectionOrdinal],
    lane_group_section: &[RoadSectionOrdinal],
    lane_group_member_ranges: &[RangeU32],
    lane_group_members: &[AuthoringLaneOrdinal],
    authoring_group: &crate::relations::OptionalColumn<LaneGroupOrdinal>,
    options: SharedNetworkBuildOptions<'_>,
) -> Result<(), BuildError> {
    let authoring_count = authoring_section.len();
    let mut seen_section = vec![false; authoring_count];
    for (section_index, range) in section_lane_ranges.iter().enumerate() {
        poll_cancelled(options, u32::try_from(section_index).unwrap_or(u32::MAX))?;
        let section = RoadSectionOrdinal::from_raw(u32::try_from(section_index).expect("fits"));
        for lane in range.slice(section_lanes) {
            if authoring_section.get(lane.index()) != Some(&section) {
                return Err(BuildError::InputInvariant {
                    structure: STRUCTURE,
                });
            }
            let slot = lane.index();
            if seen_section.get(slot).copied().unwrap_or(true) {
                return Err(BuildError::InputInvariant {
                    structure: STRUCTURE,
                });
            }
            seen_section[slot] = true;
        }
    }
    if seen_section.iter().any(|seen| !*seen) {
        return Err(BuildError::InputInvariant {
            structure: STRUCTURE,
        });
    }

    let mut section_position = vec![0_u32; authoring_count];
    for range in section_lane_ranges {
        for (local, lane) in range.slice(section_lanes).iter().enumerate() {
            section_position[lane.index()] = dest_len(local)?;
        }
    }

    let mut seen_group: Vec<Option<LaneGroupOrdinal>> = vec![None; authoring_count];
    for (group_index, range) in lane_group_member_ranges.iter().enumerate() {
        poll_cancelled(options, u32::try_from(group_index).unwrap_or(u32::MAX))?;
        let group = LaneGroupOrdinal::from_raw(u32::try_from(group_index).expect("fits"));
        let section = *lane_group_section
            .get(group_index)
            .ok_or(BuildError::InputInvariant {
                structure: STRUCTURE,
            })?;
        let mut previous_position = None;
        for lane in range.slice(lane_group_members) {
            if authoring_section.get(lane.index()) != Some(&section) {
                return Err(BuildError::InputInvariant {
                    structure: STRUCTURE,
                });
            }
            if get_optional(authoring_group, lane.raw()) != Some(group) {
                return Err(BuildError::InputInvariant {
                    structure: STRUCTURE,
                });
            }
            let position =
                *section_position
                    .get(lane.index())
                    .ok_or(BuildError::InputInvariant {
                        structure: STRUCTURE,
                    })?;
            if let Some(previous) = previous_position
                && position <= previous
            {
                return Err(BuildError::NonCanonicalOrder {
                    structure: STRUCTURE,
                    previous,
                    actual: position,
                });
            }
            previous_position = Some(position);
            let slot = lane.index();
            if seen_group.get(slot).copied().flatten().is_some() {
                return Err(BuildError::InputInvariant {
                    structure: STRUCTURE,
                });
            }
            seen_group[slot] = Some(group);
        }
    }
    for (index, expected) in seen_group.iter().enumerate() {
        let actual = get_optional(authoring_group, u32::try_from(index).expect("fits"));
        if actual != *expected {
            return Err(BuildError::InputInvariant {
                structure: STRUCTURE,
            });
        }
    }
    Ok(())
}

fn accel_in_range(value: f32) -> bool {
    (MIN_ACCEL_METERS_PER_SECOND_SQUARED..=MAX_ACCEL_METERS_PER_SECOND_SQUARED).contains(&value)
}

fn close_parking_progress(
    edge: u32,
    progress: u32,
    lane_lengths: &[u32],
) -> Result<(), BuildError> {
    let length = lane_lengths
        .get(usize::try_from(edge).expect("u32 fits"))
        .copied()
        .ok_or(BuildError::ReferenceOutOfBounds {
            structure: STRUCTURE,
            ordinal: edge,
            limit: u32::try_from(lane_lengths.len()).unwrap_or(u32::MAX),
        })?;
    if progress < PARKING_ANCHOR_ENDPOINT_CLEARANCE_MM
        || progress > length.saturating_sub(PARKING_ANCHOR_ENDPOINT_CLEARANCE_MM)
    {
        return Err(BuildError::InputInvariant {
            structure: STRUCTURE,
        });
    }
    Ok(())
}

fn close_corridor_ownership(
    corridor_reference_section: &[RoadSectionOrdinal],
    corridor_element_ranges: &[RangeU32],
    corridor_elements: &[CorridorElement],
    section_corridor: &[RoadCorridorOrdinal],
    band_corridor: &[RoadCorridorOrdinal],
    options: SharedNetworkBuildOptions<'_>,
) -> Result<(), BuildError> {
    let mut section_owner = vec![None; section_corridor.len()];
    let mut band_owner = vec![None; band_corridor.len()];
    for (corridor_index, range) in corridor_element_ranges.iter().enumerate() {
        poll_cancelled(options, u32::try_from(corridor_index).unwrap_or(u32::MAX))?;
        let corridor = RoadCorridorOrdinal::from_raw(u32::try_from(corridor_index).expect("fits"));
        let mut saw_reference = false;
        let reference =
            *corridor_reference_section
                .get(corridor_index)
                .ok_or(BuildError::InputInvariant {
                    structure: STRUCTURE,
                })?;
        for element in range.slice(corridor_elements) {
            match *element {
                CorridorElement::RoadSection(section) => {
                    if section_corridor.get(section.index()) != Some(&corridor) {
                        return Err(BuildError::InputInvariant {
                            structure: STRUCTURE,
                        });
                    }
                    if section_owner
                        .get(section.index())
                        .copied()
                        .flatten()
                        .is_some()
                    {
                        return Err(BuildError::InputInvariant {
                            structure: STRUCTURE,
                        });
                    }
                    section_owner[section.index()] = Some(corridor);
                    if section == reference {
                        saw_reference = true;
                    }
                }
                CorridorElement::FacilityBand(band) => {
                    if band_corridor.get(band.index()) != Some(&corridor) {
                        return Err(BuildError::InputInvariant {
                            structure: STRUCTURE,
                        });
                    }
                    if band_owner.get(band.index()).copied().flatten().is_some() {
                        return Err(BuildError::InputInvariant {
                            structure: STRUCTURE,
                        });
                    }
                    band_owner[band.index()] = Some(corridor);
                }
            }
        }
        if !saw_reference {
            return Err(BuildError::InputInvariant {
                structure: STRUCTURE,
            });
        }
    }
    for (index, owner) in section_corridor.iter().enumerate() {
        if section_owner.get(index).copied().flatten() != Some(*owner) {
            return Err(BuildError::InputInvariant {
                structure: STRUCTURE,
            });
        }
    }
    for (index, owner) in band_corridor.iter().enumerate() {
        if band_owner.get(index).copied().flatten() != Some(*owner) {
            return Err(BuildError::InputInvariant {
                structure: STRUCTURE,
            });
        }
    }
    Ok(())
}

fn close_owner_members<M, O>(
    ranges: &[RangeU32],
    members: &[M],
    scalar: &[O],
    member_index: impl Fn(M) -> usize,
    make_owner: impl Fn(u32) -> O,
    owner_count: u32,
    options: SharedNetworkBuildOptions<'_>,
) -> Result<(), BuildError>
where
    M: Copy,
    O: Copy + PartialEq,
{
    if ranges.len() != usize::try_from(owner_count).expect("u32") {
        return Err(BuildError::InputInvariant {
            structure: STRUCTURE,
        });
    }
    let mut seen = vec![None; scalar.len()];
    for (owner_index, range) in ranges.iter().enumerate() {
        poll_cancelled(options, u32::try_from(owner_index).unwrap_or(u32::MAX))?;
        let owner = make_owner(u32::try_from(owner_index).expect("fits"));
        for member in range.slice(members) {
            let slot = member_index(*member);
            if scalar.get(slot) != Some(&owner) {
                return Err(BuildError::InputInvariant {
                    structure: STRUCTURE,
                });
            }
            if seen.get(slot).copied().flatten().is_some() {
                return Err(BuildError::InputInvariant {
                    structure: STRUCTURE,
                });
            }
            seen[slot] = Some(owner);
        }
    }
    for (index, owner) in scalar.iter().enumerate() {
        if seen.get(index).copied().flatten() != Some(*owner) {
            return Err(BuildError::InputInvariant {
                structure: STRUCTURE,
            });
        }
    }
    Ok(())
}

fn close_optional_owner_members<M, O>(
    ranges: &[RangeU32],
    members: &[M],
    scalar: &crate::relations::OptionalColumn<O>,
    member_raw: impl Fn(M) -> u32,
    make_owner: impl Fn(u32) -> O,
    member_limit: u32,
    options: SharedNetworkBuildOptions<'_>,
) -> Result<(), BuildError>
where
    M: Copy,
    O: Copy + PartialEq,
{
    let mut seen = vec![None; usize::try_from(member_limit).expect("u32")];
    for (owner_index, range) in ranges.iter().enumerate() {
        poll_cancelled(options, u32::try_from(owner_index).unwrap_or(u32::MAX))?;
        let owner = make_owner(u32::try_from(owner_index).expect("fits"));
        for member in range.slice(members) {
            let raw = member_raw(*member);
            if get_optional(scalar, raw) != Some(owner) {
                return Err(BuildError::InputInvariant {
                    structure: STRUCTURE,
                });
            }
            let slot = usize::try_from(raw).expect("u32");
            if seen.get(slot).copied().flatten().is_some() {
                return Err(BuildError::InputInvariant {
                    structure: STRUCTURE,
                });
            }
            seen[slot] = Some(owner);
        }
    }
    for (index, expected) in seen.iter().enumerate() {
        let actual = get_optional(scalar, u32::try_from(index).expect("fits"));
        if actual != *expected {
            return Err(BuildError::InputInvariant {
                structure: STRUCTURE,
            });
        }
    }
    Ok(())
}

fn close_class_forest(
    classes: &mut Classes,
    options: SharedNetworkBuildOptions<'_>,
) -> Result<(), BuildError> {
    let count = classes.depth.len();
    let count_u32 = u32::try_from(count).expect("fits");
    for index in 0..count {
        poll_cancelled(options, u32::try_from(index).unwrap_or(u32::MAX))?;
        let raw = u32::try_from(index).expect("fits");
        if let Some(parent) = get_optional(&classes.parent, raw)
            && (parent.index() == index || parent.index() >= count)
        {
            return Err(BuildError::InputInvariant {
                structure: STRUCTURE,
            });
        }
    }
    let (depth, enter, exit) = reconstruct_class_intervals(&classes.parent, count_u32)?;
    if depth.as_slice() != classes.depth.as_ref()
        || enter.as_slice() != classes.subtree_enter.as_ref()
        || exit.as_slice() != classes.subtree_exit.as_ref()
    {
        return Err(BuildError::InputInvariant {
            structure: STRUCTURE,
        });
    }
    let mut by_enter = vec![0_u32; count];
    for (index, &enter) in classes.subtree_enter.iter().enumerate() {
        let slot = usize::try_from(enter).expect("u32 fits");
        if slot >= count {
            return Err(BuildError::InputInvariant {
                structure: STRUCTURE,
            });
        }
        by_enter[slot] = u32::try_from(index).expect("fits");
    }
    classes.by_enter = by_enter.into_boxed_slice();
    Ok(())
}

fn reconstruct_class_intervals(
    parent: &crate::relations::OptionalColumn<ParticipantClassOrdinal>,
    count: u32,
) -> Result<(Vec<u32>, Vec<u32>, Vec<u32>), BuildError> {
    let len = usize::try_from(count).expect("u32 fits");
    let mut depths = vec![None::<u32>; len];
    for start in 0..len {
        let mut chain = Vec::new();
        let mut current = start;
        let mut hops = 0_u32;
        let mut next_depth = loop {
            hops = hops.checked_add(1).ok_or(BuildError::ArithmeticOverflow {
                structure: STRUCTURE,
            })?;
            if hops > count.saturating_add(1) {
                return Err(BuildError::InputInvariant {
                    structure: STRUCTURE,
                });
            }
            if let Some(depth) = depths[current] {
                break depth.checked_add(1).ok_or(BuildError::ArithmeticOverflow {
                    structure: STRUCTURE,
                })?;
            }
            chain.push(current);
            match get_optional(parent, u32::try_from(current).expect("fits")) {
                Some(node) => current = node.index(),
                None => break 0,
            }
        };
        for node in chain.into_iter().rev() {
            depths[node] = Some(next_depth);
            next_depth = next_depth
                .checked_add(1)
                .ok_or(BuildError::ArithmeticOverflow {
                    structure: STRUCTURE,
                })?;
        }
    }
    let depth = depths
        .into_iter()
        .map(|value| {
            value.ok_or(BuildError::InputInvariant {
                structure: STRUCTURE,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;

    let mut children: Vec<Vec<u32>> = vec![Vec::new(); len];
    for child in 0..count {
        if let Some(owner) = get_optional(parent, child) {
            children[owner.index()].push(child);
        }
    }
    let mut enter = vec![0_u32; len];
    let mut exit = vec![0_u32; len];
    let mut next_enter = 0_u32;
    for root in 0..count {
        if get_optional(parent, root).is_some() {
            continue;
        }
        let mut stack = vec![(root, 0_usize)];
        while let Some(&(node, cursor)) = stack.last() {
            if cursor == 0 {
                enter[node as usize] = next_enter;
                next_enter = next_enter
                    .checked_add(1)
                    .ok_or(BuildError::ArithmeticOverflow {
                        structure: STRUCTURE,
                    })?;
            }
            if cursor < children[node as usize].len() {
                let child = children[node as usize][cursor];
                stack.last_mut().expect("stack").1 += 1;
                stack.push((child, 0));
            } else {
                exit[node as usize] = next_enter;
                stack.pop();
            }
        }
    }
    Ok((depth, enter, exit))
}

fn parse_facility_kind(
    token: &str,
    intern: &mut Intern,
    lane_bearing: bool,
) -> Result<FacilityKind, BuildError> {
    let kind = match token {
        "motorLane" => FacilityKind::MotorLane,
        "nonMotorLane" => FacilityKind::NonMotorLane,
        "sidewalk" => FacilityKind::Sidewalk,
        "median" => FacilityKind::Median,
        "plantingStrip" => FacilityKind::PlantingStrip,
        "facilityStrip" => FacilityKind::FacilityStrip,
        "shoulder" => FacilityKind::Shoulder,
        other if other.starts_with("x-lane-") && other.len() > "x-lane-".len() => {
            FacilityKind::Custom {
                intern: intern.intern(other)?,
                lane_bearing: true,
            }
        }
        other if other.starts_with("x-") && other.len() > 2 && !other.starts_with("x-lane-") => {
            FacilityKind::Custom {
                intern: intern.intern(other)?,
                lane_bearing: false,
            }
        }
        _ => {
            return Err(BuildError::InputInvariant {
                structure: STRUCTURE,
            });
        }
    };
    if kind.is_lane_bearing() != lane_bearing {
        return Err(BuildError::InputInvariant {
            structure: STRUCTURE,
        });
    }
    Ok(kind)
}

fn build_corridors(
    view: ValueCheckedObjectView<'_>,
    entity_counts: &EntityCounts,
    options: SharedNetworkBuildOptions<'_>,
) -> Result<
    (
        Box<[RoadSectionOrdinal]>,
        Box<[RangeU32]>,
        Box<[CorridorElement]>,
    ),
    BuildError,
> {
    let count = entity_counts.count(EntityKind::RoadCorridor);
    let table = entity_table(view, EntityKind::RoadCorridor)?;
    let section_limit = entity_counts.count(EntityKind::RoadSection);
    let band_limit = entity_counts.count(EntityKind::FacilityBand);
    let mut refs = allocate_vec(count, STRUCTURE)?;
    let mut ranges = allocate_vec(count, STRUCTURE)?;
    let mut elements = Vec::new();
    for (index, row) in table.rows().enumerate() {
        let expected = u32::try_from(index).map_err(|_| BuildError::ArithmeticOverflow {
            structure: STRUCTURE,
        })?;
        poll_cancelled(options, expected)?;
        expect_row_ordinal(row, expected)?;
        let reference = checked_u32(row, 3, STRUCTURE)?;
        if reference >= section_limit {
            return Err(BuildError::ReferenceOutOfBounds {
                structure: STRUCTURE,
                ordinal: reference,
                limit: section_limit,
            });
        }
        refs.push(RoadSectionOrdinal::from_raw(reference));
        let records = checked_record_vector(row, 4, STRUCTURE)?;
        let start = u32::try_from(elements.len()).map_err(|_| BuildError::ArithmeticOverflow {
            structure: STRUCTURE,
        })?;
        for (element_index, element) in records.rows().enumerate() {
            poll_cancelled(options, u32::try_from(element_index).unwrap_or(u32::MAX))?;
            let kind = checked_u8(element, 1, STRUCTURE)?;
            let ordinal = checked_u32(element, 2, STRUCTURE)?;
            let parsed = match kind {
                0 => {
                    if ordinal >= section_limit {
                        return Err(BuildError::ReferenceOutOfBounds {
                            structure: STRUCTURE,
                            ordinal,
                            limit: section_limit,
                        });
                    }
                    CorridorElement::RoadSection(RoadSectionOrdinal::from_raw(ordinal))
                }
                1 => {
                    if ordinal >= band_limit {
                        return Err(BuildError::ReferenceOutOfBounds {
                            structure: STRUCTURE,
                            ordinal,
                            limit: band_limit,
                        });
                    }
                    CorridorElement::FacilityBand(FacilityBandOrdinal::from_raw(ordinal))
                }
                _ => {
                    return Err(BuildError::InputInvariant {
                        structure: STRUCTURE,
                    });
                }
            };
            elements.push(parsed);
        }
        let len = u32::try_from(elements.len()).map_err(|_| BuildError::ArithmeticOverflow {
            structure: STRUCTURE,
        })? - start;
        ranges.push(RangeU32::new(start, len));
    }
    Ok((
        refs.into_boxed_slice(),
        ranges.into_boxed_slice(),
        elements.into_boxed_slice(),
    ))
}

fn build_sections(
    view: ValueCheckedObjectView<'_>,
    entity_counts: &EntityCounts,
    intern: &mut Intern,
    unique: &mut UniqueCheck,
    options: SharedNetworkBuildOptions<'_>,
) -> Result<
    (
        Box<[RoadCorridorOrdinal]>,
        Box<[FacilityKind]>,
        Box<[RangeU32]>,
        Box<[AuthoringLaneOrdinal]>,
    ),
    BuildError,
> {
    let count = entity_counts.count(EntityKind::RoadSection);
    let table = entity_table(view, EntityKind::RoadSection)?;
    let corridor_limit = entity_counts.count(EntityKind::RoadCorridor);
    let lane_limit = entity_counts.count(EntityKind::AuthoringLane);
    let mut corridors = allocate_vec(count, STRUCTURE)?;
    let mut kinds = allocate_vec(count, STRUCTURE)?;
    let mut ranges = allocate_vec(count, STRUCTURE)?;
    let mut lanes = Vec::new();
    for (index, row) in table.rows().enumerate() {
        let expected = u32::try_from(index).map_err(|_| BuildError::ArithmeticOverflow {
            structure: STRUCTURE,
        })?;
        poll_cancelled(options, expected)?;
        expect_row_ordinal(row, expected)?;
        let corridor = checked_u32(row, 3, STRUCTURE)?;
        if corridor >= corridor_limit {
            return Err(BuildError::ReferenceOutOfBounds {
                structure: STRUCTURE,
                ordinal: corridor,
                limit: corridor_limit,
            });
        }
        corridors.push(RoadCorridorOrdinal::from_raw(corridor));
        kinds.push(parse_facility_kind(checked_utf8(row, 4)?, intern, true)?);
        let members = checked_ordinal_vector(row, 5, STRUCTURE)?;
        let range = push_members(
            members,
            &mut lanes,
            lane_limit,
            MemberOrder::Sequence,
            options,
            unique,
        )?;
        ranges.push(range);
    }
    Ok((
        corridors.into_boxed_slice(),
        kinds.into_boxed_slice(),
        ranges.into_boxed_slice(),
        lanes
            .into_iter()
            .map(AuthoringLaneOrdinal::from_raw)
            .collect::<Vec<_>>()
            .into_boxed_slice(),
    ))
}

fn build_authoring_lanes(
    view: ValueCheckedObjectView<'_>,
    entity_counts: &EntityCounts,
    lane_count: u32,
    edge_authoring: &mut crate::relations::OptionalColumn<AuthoringLaneOrdinal>,
    options: SharedNetworkBuildOptions<'_>,
) -> Result<
    (
        Box<[RoadSectionOrdinal]>,
        Box<[RangeU32]>,
        Box<[LaneEdgeOrdinal]>,
        crate::relations::OptionalColumn<LaneGroupOrdinal>,
    ),
    BuildError,
> {
    let count = entity_counts.count(EntityKind::AuthoringLane);
    let table = entity_table(view, EntityKind::AuthoringLane)?;
    let section_limit = entity_counts.count(EntityKind::RoadSection);
    let group_limit = entity_counts.count(EntityKind::LaneGroup);
    let mut sections = allocate_vec(count, STRUCTURE)?;
    let mut ranges = allocate_vec(count, STRUCTURE)?;
    let mut edges = Vec::new();
    let mut groups = empty_optional(count)?;
    for (index, row) in table.rows().enumerate() {
        let expected = u32::try_from(index).map_err(|_| BuildError::ArithmeticOverflow {
            structure: STRUCTURE,
        })?;
        poll_cancelled(options, expected)?;
        expect_row_ordinal(row, expected)?;
        let section = checked_u32(row, 3, STRUCTURE)?;
        if section >= section_limit {
            return Err(BuildError::ReferenceOutOfBounds {
                structure: STRUCTURE,
                ordinal: section,
                limit: section_limit,
            });
        }
        sections.push(RoadSectionOrdinal::from_raw(section));
        let chain = checked_ordinal_vector(row, 4, STRUCTURE)?;
        let start = u32::try_from(edges.len()).map_err(|_| BuildError::ArithmeticOverflow {
            structure: STRUCTURE,
        })?;
        for chain_index in 0..chain.len() {
            poll_cancelled(options, chain_index)?;
            let edge = chain.get(chain_index).ok_or(BuildError::InputInvariant {
                structure: STRUCTURE,
            })?;
            if edge >= lane_count {
                return Err(BuildError::ReferenceOutOfBounds {
                    structure: STRUCTURE,
                    ordinal: edge,
                    limit: lane_count,
                });
            }
            set_optional(
                edge_authoring,
                edge,
                AuthoringLaneOrdinal::from_raw(expected),
            )?;
            edges.push(LaneEdgeOrdinal::from_raw(edge));
        }
        ranges.push(RangeU32::new(start, chain.len()));
        if let Some(group) = optional_u32(row, 5)? {
            if group >= group_limit {
                return Err(BuildError::ReferenceOutOfBounds {
                    structure: STRUCTURE,
                    ordinal: group,
                    limit: group_limit,
                });
            }
            set_optional(&mut groups, expected, LaneGroupOrdinal::from_raw(group))?;
        }
    }
    Ok((
        sections.into_boxed_slice(),
        ranges.into_boxed_slice(),
        edges.into_boxed_slice(),
        groups,
    ))
}

fn build_lane_groups(
    view: ValueCheckedObjectView<'_>,
    entity_counts: &EntityCounts,
    unique: &mut UniqueCheck,
    options: SharedNetworkBuildOptions<'_>,
) -> Result<
    (
        Box<[RoadSectionOrdinal]>,
        Box<[RangeU32]>,
        Box<[AuthoringLaneOrdinal]>,
    ),
    BuildError,
> {
    let count = entity_counts.count(EntityKind::LaneGroup);
    let table = entity_table(view, EntityKind::LaneGroup)?;
    let section_limit = entity_counts.count(EntityKind::RoadSection);
    let lane_limit = entity_counts.count(EntityKind::AuthoringLane);
    let mut sections = allocate_vec(count, STRUCTURE)?;
    let mut ranges = allocate_vec(count, STRUCTURE)?;
    let mut members = Vec::new();
    for (index, row) in table.rows().enumerate() {
        let expected = u32::try_from(index).map_err(|_| BuildError::ArithmeticOverflow {
            structure: STRUCTURE,
        })?;
        poll_cancelled(options, expected)?;
        expect_row_ordinal(row, expected)?;
        let section = checked_u32(row, 3, STRUCTURE)?;
        if section >= section_limit {
            return Err(BuildError::ReferenceOutOfBounds {
                structure: STRUCTURE,
                ordinal: section,
                limit: section_limit,
            });
        }
        sections.push(RoadSectionOrdinal::from_raw(section));
        let range = push_members(
            checked_ordinal_vector(row, 4, STRUCTURE)?,
            &mut members,
            lane_limit,
            MemberOrder::Sequence,
            options,
            unique,
        )?;
        ranges.push(range);
    }
    Ok((
        sections.into_boxed_slice(),
        ranges.into_boxed_slice(),
        members
            .into_iter()
            .map(AuthoringLaneOrdinal::from_raw)
            .collect::<Vec<_>>()
            .into_boxed_slice(),
    ))
}

fn build_facility_bands(
    view: ValueCheckedObjectView<'_>,
    entity_counts: &EntityCounts,
    intern: &mut Intern,
    options: SharedNetworkBuildOptions<'_>,
) -> Result<(Box<[RoadCorridorOrdinal]>, Box<[FacilityKind]>), BuildError> {
    let count = entity_counts.count(EntityKind::FacilityBand);
    let table = entity_table(view, EntityKind::FacilityBand)?;
    let corridor_limit = entity_counts.count(EntityKind::RoadCorridor);
    let mut corridors = allocate_vec(count, STRUCTURE)?;
    let mut kinds = allocate_vec(count, STRUCTURE)?;
    for (index, row) in table.rows().enumerate() {
        let expected = u32::try_from(index).map_err(|_| BuildError::ArithmeticOverflow {
            structure: STRUCTURE,
        })?;
        poll_cancelled(options, expected)?;
        expect_row_ordinal(row, expected)?;
        let corridor = checked_u32(row, 3, STRUCTURE)?;
        if corridor >= corridor_limit {
            return Err(BuildError::ReferenceOutOfBounds {
                structure: STRUCTURE,
                ordinal: corridor,
                limit: corridor_limit,
            });
        }
        corridors.push(RoadCorridorOrdinal::from_raw(corridor));
        kinds.push(parse_facility_kind(checked_utf8(row, 4)?, intern, false)?);
    }
    Ok((corridors.into_boxed_slice(), kinds.into_boxed_slice()))
}

fn build_junctions(
    view: ValueCheckedObjectView<'_>,
    entity_counts: &EntityCounts,
    unique: &mut UniqueCheck,
    options: SharedNetworkBuildOptions<'_>,
) -> Result<
    (
        Box<[RangeU32]>,
        Box<[MovementOrdinal]>,
        Box<[JunctionOrdinal]>,
        Box<[RangeU32]>,
        Box<[ManeuverPathOrdinal]>,
    ),
    BuildError,
> {
    let junction_count = entity_counts.count(EntityKind::Junction);
    let movement_count = entity_counts.count(EntityKind::Movement);
    let path_limit = entity_counts.count(EntityKind::ManeuverPath);
    let junction_table = entity_table(view, EntityKind::Junction)?;
    let mut junction_ranges = allocate_vec(junction_count, STRUCTURE)?;
    let mut junction_movements = Vec::new();
    for (index, row) in junction_table.rows().enumerate() {
        let expected = u32::try_from(index).map_err(|_| BuildError::ArithmeticOverflow {
            structure: STRUCTURE,
        })?;
        poll_cancelled(options, expected)?;
        expect_row_ordinal(row, expected)?;
        let range = push_members(
            checked_ordinal_vector(row, 3, STRUCTURE)?,
            &mut junction_movements,
            movement_count,
            MemberOrder::CanonicalSet,
            options,
            unique,
        )?;
        junction_ranges.push(range);
    }
    let movement_table = entity_table(view, EntityKind::Movement)?;
    let mut movement_junction = allocate_vec(movement_count, STRUCTURE)?;
    let mut path_ranges = allocate_vec(movement_count, STRUCTURE)?;
    let mut paths = Vec::new();
    for (index, row) in movement_table.rows().enumerate() {
        let expected = u32::try_from(index).map_err(|_| BuildError::ArithmeticOverflow {
            structure: STRUCTURE,
        })?;
        poll_cancelled(options, expected)?;
        expect_row_ordinal(row, expected)?;
        let junction = checked_u32(row, 3, STRUCTURE)?;
        if junction >= junction_count {
            return Err(BuildError::ReferenceOutOfBounds {
                structure: STRUCTURE,
                ordinal: junction,
                limit: junction_count,
            });
        }
        movement_junction.push(JunctionOrdinal::from_raw(junction));
        let range = push_members(
            checked_ordinal_vector(row, 6, STRUCTURE)?,
            &mut paths,
            path_limit,
            MemberOrder::CanonicalSet,
            options,
            unique,
        )?;
        path_ranges.push(range);
    }
    Ok((
        junction_ranges.into_boxed_slice(),
        junction_movements
            .into_iter()
            .map(MovementOrdinal::from_raw)
            .collect::<Vec<_>>()
            .into_boxed_slice(),
        movement_junction.into_boxed_slice(),
        path_ranges.into_boxed_slice(),
        paths
            .into_iter()
            .map(ManeuverPathOrdinal::from_raw)
            .collect::<Vec<_>>()
            .into_boxed_slice(),
    ))
}

fn build_internal_edges(
    view: ValueCheckedObjectView<'_>,
    lane_count: u32,
    junction_count: u32,
    edge_junction: &mut crate::relations::OptionalColumn<JunctionOrdinal>,
    options: SharedNetworkBuildOptions<'_>,
) -> Result<(), BuildError> {
    let table = relation_table(view, 0)?;
    for (index, row) in table.rows().enumerate() {
        poll_cancelled(options, u32::try_from(index).unwrap_or(u32::MAX))?;
        let edge = checked_u32(row, 1, STRUCTURE)?;
        let junction = checked_u32(row, 2, STRUCTURE)?;
        if edge >= lane_count {
            return Err(BuildError::ReferenceOutOfBounds {
                structure: STRUCTURE,
                ordinal: edge,
                limit: lane_count,
            });
        }
        if junction >= junction_count {
            return Err(BuildError::ReferenceOutOfBounds {
                structure: STRUCTURE,
                ordinal: junction,
                limit: junction_count,
            });
        }
        set_optional(edge_junction, edge, JunctionOrdinal::from_raw(junction))?;
    }
    Ok(())
}

/// 路口内部边必须等于每条机动路径 `edges[1..len-1]` 的排他属主，且与 LFCA `JunctionInternalEdge` 列一致。
fn close_internal_edges(
    maneuvers: &SharedManeuverNetwork,
    movement_junction: &[JunctionOrdinal],
    edge_junction: &crate::relations::OptionalColumn<JunctionOrdinal>,
    lane_count: u32,
    options: SharedNetworkBuildOptions<'_>,
) -> Result<(), BuildError> {
    let lane_len = usize::try_from(lane_count).expect("u32 fits");
    let mut derived = vec![None; lane_len];
    let mut boundary = vec![false; lane_len];
    let path_count = maneuvers.maneuver_path_count();
    for path_index in 0..path_count {
        poll_cancelled(options, path_index)?;
        let path_ordinal = ManeuverPathOrdinal::from_raw(path_index);
        let path = maneuvers
            .maneuver_path(path_ordinal)
            .ok_or(BuildError::InputInvariant {
                structure: STRUCTURE,
            })?;
        let edges = path.edges();
        if edges.len() < 2 {
            return Err(BuildError::InputInvariant {
                structure: STRUCTURE,
            });
        }
        let junction =
            *movement_junction
                .get(path.movement().index())
                .ok_or(BuildError::InputInvariant {
                    structure: STRUCTURE,
                })?;
        let first = edges[0];
        let last = *edges.last().expect("path has exit");
        for edge in [first, last] {
            if derived[edge.index()].is_some() {
                return Err(BuildError::InputInvariant {
                    structure: STRUCTURE,
                });
            }
            boundary[edge.index()] = true;
        }
        for edge in &edges[1..edges.len() - 1] {
            if boundary[edge.index()] {
                return Err(BuildError::InputInvariant {
                    structure: STRUCTURE,
                });
            }
            match derived[edge.index()] {
                Some(existing) if existing != junction => {
                    return Err(BuildError::InputInvariant {
                        structure: STRUCTURE,
                    });
                }
                Some(_) => {}
                None => derived[edge.index()] = Some(junction),
            }
        }
    }
    for edge in 0..lane_count {
        poll_cancelled(options, edge)?;
        if derived[usize::try_from(edge).expect("u32 fits")] != get_optional(edge_junction, edge) {
            return Err(BuildError::InputInvariant {
                structure: STRUCTURE,
            });
        }
    }
    Ok(())
}

/// 编制链相邻边：两端都不在内部边集合时必须落在规范后继；任一端在内部边集合时必须是某条机动路径的相邻 occurrence。
fn close_authoring_chains(
    authoring_edge_ranges: &[RangeU32],
    authoring_edges: &[LaneEdgeOrdinal],
    successor_ranges: &[RangeU32],
    successors: &[LaneEdgeOrdinal],
    maneuvers: &SharedManeuverNetwork,
    edge_junction: &crate::relations::OptionalColumn<JunctionOrdinal>,
    options: SharedNetworkBuildOptions<'_>,
) -> Result<(), BuildError> {
    for (lane_index, range) in authoring_edge_ranges.iter().enumerate() {
        poll_cancelled(options, u32::try_from(lane_index).unwrap_or(u32::MAX))?;
        if range.is_empty() {
            return Err(BuildError::InputInvariant {
                structure: STRUCTURE,
            });
        }
        let chain = range.slice(authoring_edges);
        for pair in chain.windows(2) {
            let from = pair[0];
            let to = pair[1];
            let touches_internal = get_optional(edge_junction, from.raw()).is_some()
                || get_optional(edge_junction, to.raw()).is_some();
            let connected = if touches_internal {
                maneuvers
                    .transition_candidates(from)
                    .is_some_and(|candidates| {
                        candidates
                            .iter()
                            .any(|candidate| candidate.successor() == to)
                    })
            } else {
                successor_ranges
                    .get(from.index())
                    .is_some_and(|successors_range| {
                        successors_range.slice(successors).contains(&to)
                    })
            };
            if !connected {
                return Err(BuildError::InputInvariant {
                    structure: STRUCTURE,
                });
            }
        }
    }
    Ok(())
}

#[allow(clippy::type_complexity)]
fn build_gates_and_waiting(
    view: ValueCheckedObjectView<'_>,
    entity_counts: &EntityCounts,
    lane_count: u32,
    edge_stop_line: &mut crate::relations::OptionalColumn<StopLineOrdinal>,
    unique: &mut UniqueCheck,
    options: SharedNetworkBuildOptions<'_>,
) -> Result<
    (
        Box<[LaneEdgeOrdinal]>,
        Box<[RangeU32]>,
        Box<[ManeuverGateOrdinal]>,
        Box<[ManeuverPathOrdinal]>,
        Box<[u32]>,
        Box<[StopLineOrdinal]>,
        crate::relations::OptionalColumn<SignalGroupOrdinal>,
        Box<[ManeuverPathOrdinal]>,
        Box<[ManeuverGateOrdinal]>,
        Box<[ManeuverGateOrdinal]>,
        Box<[u32]>,
    ),
    BuildError,
> {
    let gate_count = entity_counts.count(EntityKind::ManeuverGate);
    let waiting_count = entity_counts.count(EntityKind::WaitingZone);
    let stop_count = entity_counts.count(EntityKind::StopLine);
    let path_limit = entity_counts.count(EntityKind::ManeuverPath);
    let group_limit = entity_counts.count(EntityKind::SignalGroup);
    let gate_table = entity_table(view, EntityKind::ManeuverGate)?;
    let mut gate_path = allocate_vec(gate_count, STRUCTURE)?;
    let mut gate_transition = allocate_vec(gate_count, STRUCTURE)?;
    let mut gate_stop = allocate_vec(gate_count, STRUCTURE)?;
    let mut gate_group = empty_optional(gate_count)?;
    for (index, row) in gate_table.rows().enumerate() {
        let expected = u32::try_from(index).map_err(|_| BuildError::ArithmeticOverflow {
            structure: STRUCTURE,
        })?;
        poll_cancelled(options, expected)?;
        expect_row_ordinal(row, expected)?;
        let path = checked_u32(row, 3, STRUCTURE)?;
        if path >= path_limit {
            return Err(BuildError::ReferenceOutOfBounds {
                structure: STRUCTURE,
                ordinal: path,
                limit: path_limit,
            });
        }
        gate_path.push(ManeuverPathOrdinal::from_raw(path));
        gate_transition.push(checked_u32(row, 4, STRUCTURE)?);
        let stop = checked_u32(row, 5, STRUCTURE)?;
        if stop >= stop_count {
            return Err(BuildError::ReferenceOutOfBounds {
                structure: STRUCTURE,
                ordinal: stop,
                limit: stop_count,
            });
        }
        gate_stop.push(StopLineOrdinal::from_raw(stop));
        let control = checked_u8(row, 6, STRUCTURE)?;
        let group = optional_u32(row, 7)?;
        match (control, group) {
            (0, None) => {}
            (1, Some(group)) if group < group_limit => {
                set_optional(
                    &mut gate_group,
                    expected,
                    SignalGroupOrdinal::from_raw(group),
                )?;
            }
            _ => {
                return Err(BuildError::InputInvariant {
                    structure: STRUCTURE,
                });
            }
        }
    }

    let stop_table = entity_table(view, EntityKind::StopLine)?;
    let mut stop_edges = allocate_vec(stop_count, STRUCTURE)?;
    let mut stop_ranges = allocate_vec(stop_count, STRUCTURE)?;
    let mut stop_gates = Vec::new();
    for (index, row) in stop_table.rows().enumerate() {
        let expected = u32::try_from(index).map_err(|_| BuildError::ArithmeticOverflow {
            structure: STRUCTURE,
        })?;
        poll_cancelled(options, expected)?;
        expect_row_ordinal(row, expected)?;
        let edge = checked_u32(row, 3, STRUCTURE)?;
        if edge >= lane_count {
            return Err(BuildError::ReferenceOutOfBounds {
                structure: STRUCTURE,
                ordinal: edge,
                limit: lane_count,
            });
        }
        set_optional(edge_stop_line, edge, StopLineOrdinal::from_raw(expected))?;
        stop_edges.push(LaneEdgeOrdinal::from_raw(edge));
        let range = push_members(
            checked_ordinal_vector(row, 4, STRUCTURE)?,
            &mut stop_gates,
            gate_count,
            MemberOrder::CanonicalSet,
            options,
            unique,
        )?;
        stop_ranges.push(range);
    }

    let waiting_table = entity_table(view, EntityKind::WaitingZone)?;
    let mut waiting_path = allocate_vec(waiting_count, STRUCTURE)?;
    let mut waiting_entry = allocate_vec(waiting_count, STRUCTURE)?;
    let mut waiting_release = allocate_vec(waiting_count, STRUCTURE)?;
    let mut waiting_occ = allocate_vec(waiting_count, STRUCTURE)?;
    for (index, row) in waiting_table.rows().enumerate() {
        let expected = u32::try_from(index).map_err(|_| BuildError::ArithmeticOverflow {
            structure: STRUCTURE,
        })?;
        poll_cancelled(options, expected)?;
        expect_row_ordinal(row, expected)?;
        let path = checked_u32(row, 3, STRUCTURE)?;
        let entry = checked_u32(row, 4, STRUCTURE)?;
        let release = checked_u32(row, 5, STRUCTURE)?;
        if path >= path_limit || entry >= gate_count || release >= gate_count {
            return Err(BuildError::ReferenceOutOfBounds {
                structure: STRUCTURE,
                ordinal: path.max(entry).max(release),
                limit: path_limit.max(gate_count),
            });
        }
        waiting_path.push(ManeuverPathOrdinal::from_raw(path));
        waiting_entry.push(ManeuverGateOrdinal::from_raw(entry));
        waiting_release.push(ManeuverGateOrdinal::from_raw(release));
        waiting_occ.push(checked_u32(row, 6, STRUCTURE)?);
    }

    Ok((
        stop_edges.into_boxed_slice(),
        stop_ranges.into_boxed_slice(),
        stop_gates
            .into_iter()
            .map(ManeuverGateOrdinal::from_raw)
            .collect::<Vec<_>>()
            .into_boxed_slice(),
        gate_path.into_boxed_slice(),
        gate_transition.into_boxed_slice(),
        gate_stop.into_boxed_slice(),
        gate_group,
        waiting_path.into_boxed_slice(),
        waiting_entry.into_boxed_slice(),
        waiting_release.into_boxed_slice(),
        waiting_occ.into_boxed_slice(),
    ))
}

struct Signals {
    group_controller: Box<[SignalControllerOrdinal]>,
    group_gate_ranges: Box<[RangeU32]>,
    group_gates: Box<[ManeuverGateOrdinal]>,
    controller_offset_ms: Box<[u64]>,
    controller_cycle_ms: Box<[u64]>,
    controller_group_ranges: Box<[RangeU32]>,
    controller_groups: Box<[SignalGroupOrdinal]>,
    controller_phase_ranges: Box<[RangeU32]>,
    controller_phases: Box<[SignalPhaseOrdinal]>,
    phase_controller: Box<[SignalControllerOrdinal]>,
    phase_duration_ms: Box<[u64]>,
    phase_end_offset_ms: Box<[u64]>,
    phase_state_ranges: Box<[RangeU32]>,
    phase_state_groups: Box<[SignalGroupOrdinal]>,
    phase_state_aspects: Box<[SignalAspect]>,
}

fn build_signals(
    view: ValueCheckedObjectView<'_>,
    entity_counts: &EntityCounts,
    unique: &mut UniqueCheck,
    options: SharedNetworkBuildOptions<'_>,
) -> Result<Signals, BuildError> {
    let group_count = entity_counts.count(EntityKind::SignalGroup);
    let controller_count = entity_counts.count(EntityKind::SignalController);
    let phase_count = entity_counts.count(EntityKind::SignalPhase);
    let gate_limit = entity_counts.count(EntityKind::ManeuverGate);
    let group_table = entity_table(view, EntityKind::SignalGroup)?;
    let mut group_controller = allocate_vec(group_count, STRUCTURE)?;
    let mut group_ranges = allocate_vec(group_count, STRUCTURE)?;
    let mut group_gates = Vec::new();
    for (index, row) in group_table.rows().enumerate() {
        let expected = u32::try_from(index).map_err(|_| BuildError::ArithmeticOverflow {
            structure: STRUCTURE,
        })?;
        poll_cancelled(options, expected)?;
        expect_row_ordinal(row, expected)?;
        let controller = checked_u32(row, 3, STRUCTURE)?;
        if controller >= controller_count {
            return Err(BuildError::ReferenceOutOfBounds {
                structure: STRUCTURE,
                ordinal: controller,
                limit: controller_count,
            });
        }
        group_controller.push(SignalControllerOrdinal::from_raw(controller));
        let range = push_members(
            checked_ordinal_vector(row, 4, STRUCTURE)?,
            &mut group_gates,
            gate_limit,
            MemberOrder::CanonicalSet,
            options,
            unique,
        )?;
        group_ranges.push(range);
    }

    let controller_table = entity_table(view, EntityKind::SignalController)?;
    let mut offsets = allocate_vec(controller_count, STRUCTURE)?;
    let mut cycles = allocate_vec(controller_count, STRUCTURE)?;
    let mut controller_group_ranges = allocate_vec(controller_count, STRUCTURE)?;
    let mut controller_groups = Vec::new();
    let mut controller_phase_ranges = allocate_vec(controller_count, STRUCTURE)?;
    let mut controller_phases = Vec::new();
    for (index, row) in controller_table.rows().enumerate() {
        let expected = u32::try_from(index).map_err(|_| BuildError::ArithmeticOverflow {
            structure: STRUCTURE,
        })?;
        poll_cancelled(options, expected)?;
        expect_row_ordinal(row, expected)?;
        let offset = checked_u64(row, 3)?;
        let cycle = checked_u64(row, 4)?;
        if offset > MAX_PORTABLE_SIGNAL_TIME_MS
            || cycle == 0
            || cycle > MAX_PORTABLE_SIGNAL_TIME_MS
            || offset >= cycle
        {
            return Err(BuildError::InputInvariant {
                structure: STRUCTURE,
            });
        }
        offsets.push(offset);
        cycles.push(cycle);
        controller_group_ranges.push(push_members(
            checked_ordinal_vector(row, 5, STRUCTURE)?,
            &mut controller_groups,
            group_count,
            MemberOrder::CanonicalSet,
            options,
            unique,
        )?);
        controller_phase_ranges.push(push_members(
            checked_ordinal_vector(row, 6, STRUCTURE)?,
            &mut controller_phases,
            phase_count,
            MemberOrder::Sequence,
            options,
            unique,
        )?);
    }

    let phase_table = entity_table(view, EntityKind::SignalPhase)?;
    let mut phase_controller = allocate_vec(phase_count, STRUCTURE)?;
    let mut durations = allocate_vec(phase_count, STRUCTURE)?;
    let mut state_ranges = allocate_vec(phase_count, STRUCTURE)?;
    let mut state_groups = Vec::new();
    let mut state_aspects = Vec::new();
    for (index, row) in phase_table.rows().enumerate() {
        let expected = u32::try_from(index).map_err(|_| BuildError::ArithmeticOverflow {
            structure: STRUCTURE,
        })?;
        poll_cancelled(options, expected)?;
        expect_row_ordinal(row, expected)?;
        let controller = checked_u32(row, 3, STRUCTURE)?;
        if controller >= controller_count {
            return Err(BuildError::ReferenceOutOfBounds {
                structure: STRUCTURE,
                ordinal: controller,
                limit: controller_count,
            });
        }
        phase_controller.push(SignalControllerOrdinal::from_raw(controller));
        let duration = checked_u64(row, 4)?;
        if !(1..=MAX_PORTABLE_SIGNAL_TIME_MS).contains(&duration) {
            return Err(BuildError::InputInvariant {
                structure: STRUCTURE,
            });
        }
        durations.push(duration);
        let states = checked_record_vector(row, 5, STRUCTURE)?;
        let start =
            u32::try_from(state_groups.len()).map_err(|_| BuildError::ArithmeticOverflow {
                structure: STRUCTURE,
            })?;
        let mut previous_group = None;
        for (state_index, state) in states.rows().enumerate() {
            poll_cancelled(options, u32::try_from(state_index).unwrap_or(u32::MAX))?;
            let group = checked_u32(state, 1, STRUCTURE)?;
            if group >= group_count {
                return Err(BuildError::ReferenceOutOfBounds {
                    structure: STRUCTURE,
                    ordinal: group,
                    limit: group_count,
                });
            }
            if let Some(previous) = previous_group
                && group <= previous
            {
                return Err(BuildError::NonCanonicalOrder {
                    structure: STRUCTURE,
                    previous,
                    actual: group,
                });
            }
            previous_group = Some(group);
            let aspect = match checked_u8(state, 2, STRUCTURE)? {
                0 => SignalAspect::Red,
                1 => SignalAspect::Yellow,
                2 => SignalAspect::Green,
                _ => {
                    return Err(BuildError::InputInvariant {
                        structure: STRUCTURE,
                    });
                }
            };
            state_groups.push(SignalGroupOrdinal::from_raw(group));
            state_aspects.push(aspect);
        }
        let len =
            u32::try_from(state_groups.len()).map_err(|_| BuildError::ArithmeticOverflow {
                structure: STRUCTURE,
            })? - start;
        state_ranges.push(RangeU32::new(start, len));
    }

    let phase_len = usize::try_from(phase_count).expect("u32 fits");
    let mut ends = allocate_vec(phase_count, STRUCTURE)?;
    ends.resize(phase_len, 0);
    let mut assigned = allocate_vec(phase_count, STRUCTURE)?;
    assigned.resize(phase_len, false);
    for (controller, cycle) in cycles.iter().copied().enumerate() {
        poll_cancelled(options, u32::try_from(controller).unwrap_or(u32::MAX))?;
        let mut cursor = 0_u64;
        let range = controller_phase_ranges[controller];
        for phase in range.slice(controller_phases.as_slice()) {
            let index = usize::try_from(*phase).expect("u32 fits");
            if assigned[index]
                || phase_controller[index].raw() != u32::try_from(controller).expect("fits")
            {
                return Err(BuildError::InputInvariant {
                    structure: STRUCTURE,
                });
            }
            cursor =
                cursor
                    .checked_add(durations[index])
                    .ok_or(BuildError::ArithmeticOverflow {
                        structure: STRUCTURE,
                    })?;
            ends[index] = cursor;
            assigned[index] = true;
        }
        if cursor != cycle {
            return Err(BuildError::InputInvariant {
                structure: STRUCTURE,
            });
        }
    }
    if assigned.iter().any(|seen| !*seen) {
        return Err(BuildError::InputInvariant {
            structure: STRUCTURE,
        });
    }
    for (controller, range) in controller_phase_ranges.iter().enumerate() {
        let owned = controller_group_ranges[controller].slice(controller_groups.as_slice());
        for phase in range.slice(controller_phases.as_slice()) {
            let phase_index = usize::try_from(*phase).expect("u32");
            if phase_controller[phase_index].raw() != u32::try_from(controller).expect("fits") {
                return Err(BuildError::InputInvariant {
                    structure: STRUCTURE,
                });
            }
            let states = state_ranges[phase_index].slice(state_groups.as_slice());
            if states.len() != owned.len() {
                return Err(BuildError::InputInvariant {
                    structure: STRUCTURE,
                });
            }
            for (state_group, &owned_group) in states.iter().zip(owned.iter()) {
                if state_group.raw() != owned_group
                    || group_controller[state_group.index()].raw()
                        != u32::try_from(controller).expect("fits")
                {
                    return Err(BuildError::InputInvariant {
                        structure: STRUCTURE,
                    });
                }
            }
        }
    }

    Ok(Signals {
        group_controller: group_controller.into_boxed_slice(),
        group_gate_ranges: group_ranges.into_boxed_slice(),
        group_gates: group_gates
            .into_iter()
            .map(ManeuverGateOrdinal::from_raw)
            .collect::<Vec<_>>()
            .into_boxed_slice(),
        controller_offset_ms: offsets.into_boxed_slice(),
        controller_cycle_ms: cycles.into_boxed_slice(),
        controller_group_ranges: controller_group_ranges.into_boxed_slice(),
        controller_groups: controller_groups
            .into_iter()
            .map(SignalGroupOrdinal::from_raw)
            .collect::<Vec<_>>()
            .into_boxed_slice(),
        controller_phase_ranges: controller_phase_ranges.into_boxed_slice(),
        controller_phases: controller_phases
            .into_iter()
            .map(SignalPhaseOrdinal::from_raw)
            .collect::<Vec<_>>()
            .into_boxed_slice(),
        phase_controller: phase_controller.into_boxed_slice(),
        phase_duration_ms: durations.into_boxed_slice(),
        phase_end_offset_ms: ends.into_boxed_slice(),
        phase_state_ranges: state_ranges.into_boxed_slice(),
        phase_state_groups: state_groups.into_boxed_slice(),
        phase_state_aspects: state_aspects.into_boxed_slice(),
    })
}

struct Parking {
    parking_space_ranges: Box<[RangeU32]>,
    parking_spaces: Box<[ParkingSpaceOrdinal]>,
    virtual_capacity: Box<[u32]>,
    virtual_entry_ranges: Box<[RangeU32]>,
    virtual_entries: Box<[ParkingLaneAnchor]>,
    virtual_exit_ranges: Box<[RangeU32]>,
    virtual_exits: Box<[ParkingLaneAnchor]>,
    space_area: crate::relations::OptionalColumn<ParkingFacilityOrdinal>,
    space_entry_edge: Box<[LaneEdgeOrdinal]>,
    space_entry_progress: Box<[u32]>,
    space_exit_edge: Box<[LaneEdgeOrdinal]>,
    space_exit_progress: Box<[u32]>,
    space_lateral: Box<[i32]>,
    space_heading: Box<[f32]>,
    space_length: Box<[u32]>,
    space_width: Box<[u32]>,
}

fn build_parking(
    view: ValueCheckedObjectView<'_>,
    entity_counts: &EntityCounts,
    lane_count: u32,
    lane_lengths: &[u32],
    unique: &mut UniqueCheck,
    options: SharedNetworkBuildOptions<'_>,
) -> Result<Parking, BuildError> {
    let area_count = entity_counts.count(EntityKind::ParkingFacility);
    let space_count = entity_counts.count(EntityKind::ParkingSpace);
    let area_table = entity_table(view, EntityKind::ParkingFacility)?;
    let lane_ids = lane_edge_stable_ids(view, lane_count)?;
    let mut ranges = allocate_vec(area_count, STRUCTURE)?;
    let mut spaces = Vec::new();
    let mut virtual_capacity = allocate_vec(area_count, STRUCTURE)?;
    let mut virtual_entry_ranges = allocate_vec(area_count, STRUCTURE)?;
    let mut virtual_entries = Vec::new();
    let mut virtual_exit_ranges = allocate_vec(area_count, STRUCTURE)?;
    let mut virtual_exits = Vec::new();
    for (index, row) in area_table.rows().enumerate() {
        let expected = u32::try_from(index).map_err(|_| BuildError::ArithmeticOverflow {
            structure: STRUCTURE,
        })?;
        poll_cancelled(options, expected)?;
        expect_row_ordinal(row, expected)?;
        let space_range = push_members(
            checked_ordinal_vector(row, 3, STRUCTURE)?,
            &mut spaces,
            space_count,
            MemberOrder::CanonicalSet,
            options,
            unique,
        )?;
        let capacity = checked_u32(row, 4, STRUCTURE)?;
        let entry_range = push_parking_lane_anchors(
            checked_record_vector(row, 5, STRUCTURE)?,
            &lane_ids,
            lane_lengths,
            &mut virtual_entries,
        )?;
        let exit_range = push_parking_lane_anchors(
            checked_record_vector(row, 6, STRUCTURE)?,
            &lane_ids,
            lane_lengths,
            &mut virtual_exits,
        )?;
        if space_range.is_empty() && capacity == 0 {
            return Err(BuildError::InputInvariant {
                structure: STRUCTURE,
            });
        }
        if (capacity == 0 && (!entry_range.is_empty() || !exit_range.is_empty()))
            || (capacity > 0 && (entry_range.is_empty() || exit_range.is_empty()))
        {
            return Err(BuildError::InputInvariant {
                structure: STRUCTURE,
            });
        }
        ranges.push(space_range);
        virtual_capacity.push(capacity);
        virtual_entry_ranges.push(entry_range);
        virtual_exit_ranges.push(exit_range);
    }
    let space_table = entity_table(view, EntityKind::ParkingSpace)?;
    let mut area = empty_optional(space_count)?;
    let mut entry_edge = allocate_vec(space_count, STRUCTURE)?;
    let mut entry_progress = allocate_vec(space_count, STRUCTURE)?;
    let mut exit_edge = allocate_vec(space_count, STRUCTURE)?;
    let mut exit_progress = allocate_vec(space_count, STRUCTURE)?;
    let mut lateral = allocate_vec(space_count, STRUCTURE)?;
    let mut heading = allocate_vec(space_count, STRUCTURE)?;
    let mut length = allocate_vec(space_count, STRUCTURE)?;
    let mut width = allocate_vec(space_count, STRUCTURE)?;
    for (index, row) in space_table.rows().enumerate() {
        let expected = u32::try_from(index).map_err(|_| BuildError::ArithmeticOverflow {
            structure: STRUCTURE,
        })?;
        poll_cancelled(options, expected)?;
        expect_row_ordinal(row, expected)?;
        if let Some(area_ordinal) = optional_u32(row, 3)? {
            if area_ordinal >= area_count {
                return Err(BuildError::ReferenceOutOfBounds {
                    structure: STRUCTURE,
                    ordinal: area_ordinal,
                    limit: area_count,
                });
            }
            set_optional(
                &mut area,
                expected,
                ParkingFacilityOrdinal::from_raw(area_ordinal),
            )?;
        }
        let entry = checked_u32(row, 4, STRUCTURE)?;
        let exit = checked_u32(row, 6, STRUCTURE)?;
        if entry >= lane_count || exit >= lane_count {
            return Err(BuildError::ReferenceOutOfBounds {
                structure: STRUCTURE,
                ordinal: entry.max(exit),
                limit: lane_count,
            });
        }
        let entry_at = checked_u32(row, 5, STRUCTURE)?;
        let exit_at = checked_u32(row, 7, STRUCTURE)?;
        close_parking_progress(entry, entry_at, lane_lengths)?;
        close_parking_progress(exit, exit_at, lane_lengths)?;
        entry_edge.push(LaneEdgeOrdinal::from_raw(entry));
        entry_progress.push(entry_at);
        exit_edge.push(LaneEdgeOrdinal::from_raw(exit));
        exit_progress.push(exit_at);
        let lateral_mm = checked_i32(row, 8, STRUCTURE)?;
        let lateral_abs = lateral_mm.unsigned_abs();
        if !(MIN_PARKING_LATERAL_OFFSET_ABS_MM..=MAX_PARKING_LATERAL_OFFSET_ABS_MM)
            .contains(&lateral_abs)
        {
            return Err(BuildError::InputInvariant {
                structure: STRUCTURE,
            });
        }
        lateral.push(lateral_mm);
        heading.push(heading_f32_stored(
            checked_f32(row, 9, STRUCTURE)?,
            STRUCTURE,
        )?);
        length.push(u32_in_closed_range(
            checked_u32(row, 10, STRUCTURE)?,
            MIN_VEHICLE_LENGTH_MM,
            MAX_VEHICLE_LENGTH_MM,
            STRUCTURE,
        )?);
        width.push(u32_in_closed_range(
            checked_u32(row, 11, STRUCTURE)?,
            MIN_VEHICLE_LENGTH_MM,
            MAX_VEHICLE_LENGTH_MM,
            STRUCTURE,
        )?);
    }
    Ok(Parking {
        parking_space_ranges: ranges.into_boxed_slice(),
        parking_spaces: spaces
            .into_iter()
            .map(ParkingSpaceOrdinal::from_raw)
            .collect::<Vec<_>>()
            .into_boxed_slice(),
        virtual_capacity: virtual_capacity.into_boxed_slice(),
        virtual_entry_ranges: virtual_entry_ranges.into_boxed_slice(),
        virtual_entries: virtual_entries.into_boxed_slice(),
        virtual_exit_ranges: virtual_exit_ranges.into_boxed_slice(),
        virtual_exits: virtual_exits.into_boxed_slice(),
        space_area: area,
        space_entry_edge: entry_edge.into_boxed_slice(),
        space_entry_progress: entry_progress.into_boxed_slice(),
        space_exit_edge: exit_edge.into_boxed_slice(),
        space_exit_progress: exit_progress.into_boxed_slice(),
        space_lateral: lateral.into_boxed_slice(),
        space_heading: heading.into_boxed_slice(),
        space_length: length.into_boxed_slice(),
        space_width: width.into_boxed_slice(),
    })
}

fn lane_edge_stable_ids(
    view: ValueCheckedObjectView<'_>,
    lane_count: u32,
) -> Result<Box<[StableId128]>, BuildError> {
    let table = entity_table(view, EntityKind::LaneEdge)?;
    let mut stable_ids = allocate_vec(lane_count, STRUCTURE)?;
    for (index, row) in table.rows().enumerate() {
        let expected = u32::try_from(index).map_err(|_| BuildError::ArithmeticOverflow {
            structure: STRUCTURE,
        })?;
        expect_row_ordinal(row, expected)?;
        stable_ids.push(checked_stable_id(row, 2, STRUCTURE)?);
    }
    if stable_ids.len() != usize::try_from(lane_count).expect("u32 fits usize") {
        return Err(BuildError::InputInvariant {
            structure: STRUCTURE,
        });
    }
    Ok(stable_ids.into_boxed_slice())
}

fn push_parking_lane_anchors(
    records: laneflow_format::RegistryCheckedRecordVectorView<'_>,
    lane_ids: &[StableId128],
    lane_lengths: &[u32],
    output: &mut Vec<ParkingLaneAnchor>,
) -> Result<RangeU32, BuildError> {
    let start = u32::try_from(output.len()).map_err(|_| BuildError::ArithmeticOverflow {
        structure: STRUCTURE,
    })?;
    let mut previous = None;
    for row in records.rows() {
        let lane = checked_u32(row, 1, STRUCTURE)?;
        let lane_index = usize::try_from(lane).expect("u32 fits usize");
        let stable_id = *lane_ids
            .get(lane_index)
            .ok_or(BuildError::ReferenceOutOfBounds {
                structure: STRUCTURE,
                ordinal: lane,
                limit: u32::try_from(lane_ids.len()).unwrap_or(u32::MAX),
            })?;
        let progress_mm = checked_u32(row, 2, STRUCTURE)?;
        close_parking_progress(lane, progress_mm, lane_lengths)?;
        let key = (stable_id, progress_mm);
        if previous.is_some_and(|previous| previous >= key) {
            return Err(BuildError::InputInvariant {
                structure: STRUCTURE,
            });
        }
        previous = Some(key);
        output.push(ParkingLaneAnchor {
            lane_edge: LaneEdgeOrdinal::from_raw(lane),
            progress_mm,
        });
    }
    let len = u32::try_from(output.len())
        .map_err(|_| BuildError::ArithmeticOverflow {
            structure: STRUCTURE,
        })?
        .checked_sub(start)
        .ok_or(BuildError::ArithmeticOverflow {
            structure: STRUCTURE,
        })?;
    Ok(RangeU32::new(start, len))
}

struct Classes {
    parent: crate::relations::OptionalColumn<ParticipantClassOrdinal>,
    depth: Box<[u32]>,
    subtree_enter: Box<[u32]>,
    subtree_exit: Box<[u32]>,
    by_enter: Box<[u32]>,
}

fn build_classes(
    view: ValueCheckedObjectView<'_>,
    entity_counts: &EntityCounts,
    options: SharedNetworkBuildOptions<'_>,
) -> Result<Classes, BuildError> {
    let count = entity_counts.count(EntityKind::ParticipantClass);
    let table = entity_table(view, EntityKind::ParticipantClass)?;
    let mut parent = empty_optional(count)?;
    let mut depth = allocate_vec(count, STRUCTURE)?;
    let mut enter = allocate_vec(count, STRUCTURE)?;
    let mut exit = allocate_vec(count, STRUCTURE)?;
    for (index, row) in table.rows().enumerate() {
        let expected = u32::try_from(index).map_err(|_| BuildError::ArithmeticOverflow {
            structure: STRUCTURE,
        })?;
        poll_cancelled(options, expected)?;
        expect_row_ordinal(row, expected)?;
        if let Some(parent_ordinal) = optional_u32(row, 3)? {
            if parent_ordinal >= count {
                return Err(BuildError::ReferenceOutOfBounds {
                    structure: STRUCTURE,
                    ordinal: parent_ordinal,
                    limit: count,
                });
            }
            set_optional(
                &mut parent,
                expected,
                ParticipantClassOrdinal::from_raw(parent_ordinal),
            )?;
        }
        depth.push(checked_u32(row, 4, STRUCTURE)?);
        enter.push(checked_u32(row, 5, STRUCTURE)?);
        exit.push(checked_u32(row, 6, STRUCTURE)?);
    }
    Ok(Classes {
        parent,
        depth: depth.into_boxed_slice(),
        subtree_enter: enter.into_boxed_slice(),
        subtree_exit: exit.into_boxed_slice(),
        by_enter: Box::new([]),
    })
}

struct Rules {
    target: Box<[AccessTarget]>,
    effect: Box<[AccessEffect]>,
    class_ranges: Box<[RangeU32]>,
    classes: Box<[ParticipantClassOrdinal]>,
    priority: Box<[i32]>,
}

fn build_access_rules(
    view: ValueCheckedObjectView<'_>,
    entity_counts: &EntityCounts,
    unique: &mut UniqueCheck,
    options: SharedNetworkBuildOptions<'_>,
) -> Result<Rules, BuildError> {
    let count = entity_counts.count(EntityKind::AccessRule);
    let table = entity_table(view, EntityKind::AccessRule)?;
    let class_limit = entity_counts.count(EntityKind::ParticipantClass);
    let mut target = allocate_vec(count, STRUCTURE)?;
    let mut effect = allocate_vec(count, STRUCTURE)?;
    let mut ranges = allocate_vec(count, STRUCTURE)?;
    let mut classes = Vec::new();
    let mut priority = allocate_vec(count, STRUCTURE)?;
    let mut regulation = None;
    for (index, row) in table.rows().enumerate() {
        let expected = u32::try_from(index).map_err(|_| BuildError::ArithmeticOverflow {
            structure: STRUCTURE,
        })?;
        poll_cancelled(options, expected)?;
        expect_row_ordinal(row, expected)?;
        let kind = checked_u8(row, 3, STRUCTURE)?;
        let ordinal = checked_u32(row, 4, STRUCTURE)?;
        target.push(parse_access_target(kind, ordinal, entity_counts)?);
        effect.push(match checked_u8(row, 5, STRUCTURE)? {
            0 => AccessEffect::Deny,
            1 => AccessEffect::Allow,
            _ => {
                return Err(BuildError::InputInvariant {
                    structure: STRUCTURE,
                });
            }
        });
        let range = push_members(
            checked_ordinal_vector(row, 6, STRUCTURE)?,
            &mut classes,
            class_limit,
            MemberOrder::CanonicalSet,
            options,
            unique,
        )?;
        if range.is_empty() {
            return Err(BuildError::InputInvariant {
                structure: STRUCTURE,
            });
        }
        ranges.push(range);
        close_regulation_authority(row, &mut regulation)?;
        priority.push(checked_i32(row, 8, STRUCTURE)?);
    }
    Ok(Rules {
        target: target.into_boxed_slice(),
        effect: effect.into_boxed_slice(),
        class_ranges: ranges.into_boxed_slice(),
        classes: classes
            .into_iter()
            .map(ParticipantClassOrdinal::from_raw)
            .collect::<Vec<_>>()
            .into_boxed_slice(),
        priority: priority.into_boxed_slice(),
    })
}

/// Core AccessRegistry phase 9.6：已声明 `regulation` 必须共享同一
/// `(jurisdiction, version)`（`source` 不参与）；未声明者跳过。UTF-8 只用于比对，
/// 不 intern、不进入 Traffic。
fn close_regulation_authority<'a>(
    row: RegistryCheckedRowView<'a>,
    canonical: &mut Option<(&'a str, &'a str)>,
) -> Result<(), BuildError> {
    let Some(field) = row.field_by_tag(7) else {
        return Ok(());
    };
    let RegistryCheckedFieldValue::RecordVector(records) =
        field.value().map_err(|_| BuildError::InputInvariant {
            structure: STRUCTURE,
        })?
    else {
        return Err(BuildError::InputInvariant {
            structure: STRUCTURE,
        });
    };
    if records.len() != 1 {
        return Err(BuildError::InputInvariant {
            structure: STRUCTURE,
        });
    }
    let nested = records.rows().next().ok_or(BuildError::InputInvariant {
        structure: STRUCTURE,
    })?;
    bind_regulation_authority(
        canonical,
        (checked_utf8(nested, 1)?, checked_utf8(nested, 2)?),
    )
}

fn bind_regulation_authority<'a>(
    canonical: &mut Option<(&'a str, &'a str)>,
    authority: (&'a str, &'a str),
) -> Result<(), BuildError> {
    if let Some(first) = *canonical {
        if first != authority {
            return Err(BuildError::InputInvariant {
                structure: STRUCTURE,
            });
        }
        return Ok(());
    }
    *canonical = Some(authority);
    Ok(())
}

fn parse_access_target(
    kind: u8,
    ordinal: u32,
    entity_counts: &EntityCounts,
) -> Result<AccessTarget, BuildError> {
    let (limit, target) = match kind {
        0 => (
            entity_counts.count(EntityKind::LaneEdge),
            AccessTarget::LaneEdge(LaneEdgeOrdinal::from_raw(ordinal)),
        ),
        1 => (
            entity_counts.count(EntityKind::LaneGroup),
            AccessTarget::LaneGroup(LaneGroupOrdinal::from_raw(ordinal)),
        ),
        2 => (
            entity_counts.count(EntityKind::RoadSection),
            AccessTarget::RoadSection(RoadSectionOrdinal::from_raw(ordinal)),
        ),
        3 => (
            entity_counts.count(EntityKind::ManeuverPath),
            AccessTarget::ManeuverPath(ManeuverPathOrdinal::from_raw(ordinal)),
        ),
        _ => {
            return Err(BuildError::InputInvariant {
                structure: BuildStructure::AccessPlane,
            });
        }
    };
    if ordinal >= limit {
        return Err(BuildError::ReferenceOutOfBounds {
            structure: BuildStructure::AccessPlane,
            ordinal,
            limit,
        });
    }
    Ok(target)
}

struct Profiles {
    class: Box<[ParticipantClassOrdinal]>,
    length: Box<[u32]>,
    desired_speed: Box<[u32]>,
    min_gap: Box<[u32]>,
    time_headway: Box<[f32]>,
    max_accel: Box<[f32]>,
    comfort_decel: Box<[f32]>,
    emergency_decel: Box<[f32]>,
}

fn build_profiles(
    view: ValueCheckedObjectView<'_>,
    entity_counts: &EntityCounts,
    options: SharedNetworkBuildOptions<'_>,
) -> Result<Profiles, BuildError> {
    let count = entity_counts.count(EntityKind::VehicleProfile);
    let table = entity_table(view, EntityKind::VehicleProfile)?;
    let class_limit = entity_counts.count(EntityKind::ParticipantClass);
    let mut class = allocate_vec(count, STRUCTURE)?;
    let mut length = allocate_vec(count, STRUCTURE)?;
    let mut desired = allocate_vec(count, STRUCTURE)?;
    let mut gap = allocate_vec(count, STRUCTURE)?;
    let mut headway = allocate_vec(count, STRUCTURE)?;
    let mut accel = allocate_vec(count, STRUCTURE)?;
    let mut comfort = allocate_vec(count, STRUCTURE)?;
    let mut emergency = allocate_vec(count, STRUCTURE)?;
    for (index, row) in table.rows().enumerate() {
        let expected = u32::try_from(index).map_err(|_| BuildError::ArithmeticOverflow {
            structure: STRUCTURE,
        })?;
        poll_cancelled(options, expected)?;
        expect_row_ordinal(row, expected)?;
        let class_ordinal = checked_u32(row, 3, STRUCTURE)?;
        if class_ordinal >= class_limit {
            return Err(BuildError::ReferenceOutOfBounds {
                structure: STRUCTURE,
                ordinal: class_ordinal,
                limit: class_limit,
            });
        }
        class.push(ParticipantClassOrdinal::from_raw(class_ordinal));
        length.push(u32_in_closed_range(
            checked_u32(row, 4, STRUCTURE)?,
            MIN_VEHICLE_LENGTH_MM,
            MAX_VEHICLE_LENGTH_MM,
            STRUCTURE,
        )?);
        desired.push(u32_in_closed_range(
            checked_u32(row, 5, STRUCTURE)?,
            MIN_SPEED_MM_S,
            laneflow_static_contract::MAX_SPEED_MM_S,
            STRUCTURE,
        )?);
        let min_gap = checked_u32(row, 6, STRUCTURE)?;
        if min_gap > MAX_MIN_GAP_MM {
            return Err(BuildError::InputInvariant {
                structure: STRUCTURE,
            });
        }
        gap.push(min_gap);
        let time_headway = checked_f32(row, 7, STRUCTURE)?;
        if !(time_headway > 0.0 && time_headway <= MAX_TIME_HEADWAY_SECONDS) {
            return Err(BuildError::InputInvariant {
                structure: STRUCTURE,
            });
        }
        headway.push(time_headway);
        let max_accel = checked_f32(row, 8, STRUCTURE)?;
        let comfort_decel = checked_f32(row, 9, STRUCTURE)?;
        let emergency_decel = checked_f32(row, 10, STRUCTURE)?;
        if !accel_in_range(max_accel)
            || !accel_in_range(comfort_decel)
            || !accel_in_range(emergency_decel)
            || emergency_decel < comfort_decel
        {
            return Err(BuildError::InputInvariant {
                structure: STRUCTURE,
            });
        }
        accel.push(max_accel);
        comfort.push(comfort_decel);
        emergency.push(emergency_decel);
    }
    Ok(Profiles {
        class: class.into_boxed_slice(),
        length: length.into_boxed_slice(),
        desired_speed: desired.into_boxed_slice(),
        min_gap: gap.into_boxed_slice(),
        time_headway: headway.into_boxed_slice(),
        max_accel: accel.into_boxed_slice(),
        comfort_decel: comfort.into_boxed_slice(),
        emergency_decel: emergency.into_boxed_slice(),
    })
}

#[derive(Clone, Copy)]
struct ClassVerdict {
    key: (u32, u8, i32),
    min_allow: Option<u32>,
    min_deny: Option<u32>,
}

impl ClassVerdict {
    fn merge(self, other: Self) -> Self {
        if other.key > self.key {
            return other;
        }
        if other.key < self.key {
            return self;
        }
        Self {
            key: self.key,
            min_allow: opt_min(self.min_allow, other.min_allow),
            min_deny: opt_min(self.min_deny, other.min_deny),
        }
    }
}

fn opt_min(a: Option<u32>, b: Option<u32>) -> Option<u32> {
    match (a, b) {
        (Some(x), Some(y)) => Some(x.min(y)),
        (x, y) => x.or(y),
    }
}

fn target_specificity(target: AccessTarget) -> u8 {
    match target {
        AccessTarget::LaneEdge(_) => 2,
        AccessTarget::LaneGroup(_) => 1,
        AccessTarget::RoadSection(_) => 0,
        AccessTarget::ManeuverPath(_) => 0,
    }
}

fn fold_rule(
    verdicts: &mut [Option<ClassVerdict>],
    rule_index: u32,
    rules: &Rules,
    classes: &Classes,
) {
    let class_range = rules.class_ranges[rule_index as usize];
    let rule_classes = class_range.slice(&rules.classes);
    let specificity = target_specificity(rules.target[rule_index as usize]);
    let priority = rules.priority[rule_index as usize];
    let allow = matches!(rules.effect[rule_index as usize], AccessEffect::Allow);
    for class in rule_classes {
        let enter = classes.subtree_enter[class.index()];
        let exit = classes.subtree_exit[class.index()];
        let class_depth = classes.depth[class.index()];
        let key = (class_depth, specificity, priority);
        let verdict = ClassVerdict {
            key,
            min_allow: allow.then_some(rule_index),
            min_deny: (!allow).then_some(rule_index),
        };
        for time in enter..exit {
            let profile = usize::try_from(classes.by_enter[time as usize]).expect("u32");
            let slot = &mut verdicts[profile];
            *slot = Some(match *slot {
                Some(existing) => existing.merge(verdict),
                None => verdict,
            });
        }
    }
}

fn cells_from_verdicts(
    verdicts: &[Option<ClassVerdict>],
    plane: &'static str,
    unit: u32,
) -> Result<Vec<AccessCell>, BuildError> {
    let mut cells = Vec::with_capacity(verdicts.len());
    write_cells_from_verdicts(verdicts, plane, unit, &mut cells)?;
    Ok(cells)
}

fn write_cells_from_verdicts(
    verdicts: &[Option<ClassVerdict>],
    plane: &'static str,
    unit: u32,
    cells: &mut Vec<AccessCell>,
) -> Result<(), BuildError> {
    for (class, verdict) in verdicts.iter().enumerate() {
        let Some(verdict) = *verdict else {
            cells.push(AccessCell::Unconstrained);
            continue;
        };
        if let (Some(allow), Some(deny)) = (verdict.min_allow, verdict.min_deny) {
            let (first, second) = if allow < deny {
                (allow, deny)
            } else {
                (deny, allow)
            };
            return Err(BuildError::AccessAmbiguity {
                plane,
                unit,
                class: u32::try_from(class).unwrap_or(u32::MAX),
                first_rule: first,
                second_rule: second,
            });
        }
        let winner = verdict.min_allow.or(verdict.min_deny).expect("verdict");
        cells.push(AccessCell::Decided {
            rule: AccessRuleOrdinal::from_raw(winner),
            effect: if verdict.min_allow == Some(winner) {
                AccessEffect::Allow
            } else {
                AccessEffect::Deny
            },
        });
    }
    Ok(())
}

fn merge_verdicts(base: &mut [Option<ClassVerdict>], delta: &[Option<ClassVerdict>]) {
    for (slot, &other) in base.iter_mut().zip(delta) {
        *slot = match (*slot, other) {
            (Some(left), Some(right)) => Some(left.merge(right)),
            (None, right) => right,
            (left, None) => left,
        };
    }
}

fn allocate_rule_buckets(count: u32) -> Result<Vec<Vec<u32>>, BuildError> {
    let mut buckets = allocate_vec(count, BuildStructure::AccessPlane)?;
    buckets.resize(usize::try_from(count).expect("u32"), Vec::new());
    Ok(buckets)
}

fn fold_rule_list(
    rule_indices: &[u32],
    rules: &Rules,
    classes: &Classes,
    class_len: usize,
) -> Result<Vec<Option<ClassVerdict>>, BuildError> {
    let mut verdicts = allocate_vec(
        u32::try_from(class_len).unwrap_or(u32::MAX),
        BuildStructure::AccessPlane,
    )?;
    verdicts.resize(class_len, None);
    for &rule in rule_indices {
        fold_rule(&mut verdicts, rule, rules, classes);
    }
    Ok(verdicts)
}

fn check_access_cell_capacity(cell_count: usize) -> Result<(), BuildError> {
    u32::try_from(cell_count).map_err(|_| BuildError::ArithmeticOverflow {
        structure: BuildStructure::AccessPlane,
    })?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn resolve_access_planes(
    entity_counts: &EntityCounts,
    edge_authoring: &crate::relations::OptionalColumn<AuthoringLaneOrdinal>,
    authoring_section: &[RoadSectionOrdinal],
    authoring_group: &crate::relations::OptionalColumn<LaneGroupOrdinal>,
    lane_group_members: &[AuthoringLaneOrdinal],
    lane_group_member_ranges: &[RangeU32],
    lane_group_section: &[RoadSectionOrdinal],
    section_lanes: &[AuthoringLaneOrdinal],
    section_lane_ranges: &[RangeU32],
    authoring_edges: &[LaneEdgeOrdinal],
    authoring_edge_ranges: &[RangeU32],
    classes: &Classes,
    rules: &Rules,
    options: SharedNetworkBuildOptions<'_>,
) -> Result<
    (
        Box<[u32]>,
        Box<[AccessCell]>,
        Box<[u32]>,
        Box<[AccessCell]>,
        u32,
    ),
    BuildError,
> {
    let class_count = entity_counts.count(EntityKind::ParticipantClass);
    let lane_count = entity_counts.count(EntityKind::LaneEdge);
    let path_count = entity_counts.count(EntityKind::ManeuverPath);
    let group_count = entity_counts.count(EntityKind::LaneGroup);
    let section_count = entity_counts.count(EntityKind::RoadSection);
    let mut edge_direct = allocate_rule_buckets(lane_count)?;
    let mut group_rules = allocate_rule_buckets(group_count)?;
    let mut section_rules = allocate_rule_buckets(section_count)?;
    let mut path_rules = allocate_rule_buckets(path_count)?;
    for (rule_index, target) in rules.target.iter().copied().enumerate() {
        let rule_index = u32::try_from(rule_index).expect("rule fits");
        match target {
            AccessTarget::LaneEdge(edge) => edge_direct[edge.index()].push(rule_index),
            AccessTarget::LaneGroup(group) => group_rules[group.index()].push(rule_index),
            AccessTarget::RoadSection(section) => section_rules[section.index()].push(rule_index),
            AccessTarget::ManeuverPath(path) => path_rules[path.index()].push(rule_index),
        }
    }

    let (edge_starts, edge_cells) = resolve_edge_plane(
        class_count,
        lane_count,
        edge_authoring,
        authoring_section,
        authoring_group,
        lane_group_section,
        &edge_direct,
        &group_rules,
        &section_rules,
        classes,
        rules,
        options,
    )?;
    let (path_starts, path_cells) = materialize_plane(
        path_count,
        class_count,
        "path",
        |path| path_rules[path as usize].clone(),
        classes,
        rules,
        options,
    )?;
    let _ = (
        lane_group_members,
        lane_group_member_ranges,
        section_lanes,
        section_lane_ranges,
        authoring_edges,
        authoring_edge_ranges,
    );
    Ok((
        edge_starts,
        edge_cells,
        path_starts,
        path_cells,
        class_count,
    ))
}

#[allow(clippy::too_many_arguments)]
fn resolve_edge_plane(
    class_count: u32,
    lane_count: u32,
    edge_authoring: &crate::relations::OptionalColumn<AuthoringLaneOrdinal>,
    authoring_section: &[RoadSectionOrdinal],
    authoring_group: &crate::relations::OptionalColumn<LaneGroupOrdinal>,
    lane_group_section: &[RoadSectionOrdinal],
    edge_direct: &[Vec<u32>],
    group_rules: &[Vec<u32>],
    section_rules: &[Vec<u32>],
    classes: &Classes,
    rules: &Rules,
    options: SharedNetworkBuildOptions<'_>,
) -> Result<(Box<[u32]>, Box<[AccessCell]>), BuildError> {
    let class_len = usize::try_from(class_count).expect("u32");
    let section_verdicts: Vec<Option<Vec<Option<ClassVerdict>>>> = section_rules
        .iter()
        .map(|indices| {
            if indices.is_empty() {
                Ok(None)
            } else {
                fold_rule_list(indices, rules, classes, class_len).map(Some)
            }
        })
        .collect::<Result<_, _>>()?;
    let group_verdicts: Vec<Option<Vec<Option<ClassVerdict>>>> = group_rules
        .iter()
        .map(|indices| {
            if indices.is_empty() {
                Ok(None)
            } else {
                fold_rule_list(indices, rules, classes, class_len).map(Some)
            }
        })
        .collect::<Result<_, _>>()?;

    let mut context_verdicts: Vec<Vec<Option<ClassVerdict>>> = Vec::new();
    let mut section_context = vec![None; section_verdicts.len()];
    for (section_index, verdicts) in section_verdicts.iter().enumerate() {
        let Some(verdicts) = verdicts else {
            continue;
        };
        let id = u32::try_from(context_verdicts.len()).expect("context fits u32");
        context_verdicts.push(verdicts.clone());
        section_context[section_index] = Some(id);
    }
    let mut group_context = vec![None; group_verdicts.len()];
    for (group_index, group_delta) in group_verdicts.iter().enumerate() {
        let Some(delta) = group_delta else {
            continue;
        };
        let section = *lane_group_section
            .get(group_index)
            .ok_or(BuildError::InputInvariant {
                structure: BuildStructure::AccessPlane,
            })?;
        let mut merged = match section_verdicts.get(section.index()) {
            Some(Some(verdicts)) => verdicts.clone(),
            _ => vec![None; class_len],
        };
        merge_verdicts(&mut merged, delta);
        let id = u32::try_from(context_verdicts.len()).expect("context fits u32");
        context_verdicts.push(merged);
        group_context[group_index] = Some(id);
    }

    let mut empty_direct_row = vec![None; context_verdicts.len()];
    let mut starts = allocate_vec(lane_count, BuildStructure::AccessPlane)?;
    let mut cells = Vec::new();
    for edge in 0..lane_count {
        poll_cancelled(options, edge)?;
        let context_id = match get_optional(edge_authoring, edge) {
            Some(lane) => {
                let section =
                    *authoring_section
                        .get(lane.index())
                        .ok_or(BuildError::InputInvariant {
                            structure: BuildStructure::AccessPlane,
                        })?;
                match get_optional(authoring_group, lane.raw()) {
                    Some(group) => group_context
                        .get(group.index())
                        .copied()
                        .flatten()
                        .or(section_context.get(section.index()).copied().flatten()),
                    None => section_context.get(section.index()).copied().flatten(),
                }
            }
            None => None,
        };
        let direct = &edge_direct[edge as usize];
        if context_id.is_none() && direct.is_empty() {
            starts.push(ACCESS_UNCONSTRAINED_ROW);
            continue;
        }
        if class_count == 0 {
            starts.push(ACCESS_UNCONSTRAINED_ROW);
            continue;
        }
        if direct.is_empty() {
            let context = context_id.expect("constrained context");
            if let Some(row) = empty_direct_row[context as usize] {
                starts.push(row);
                continue;
            }
            let row_start = dest_len(cells.len())?;
            check_access_cell_capacity(cells.len().checked_add(class_len).ok_or(
                BuildError::ArithmeticOverflow {
                    structure: BuildStructure::AccessPlane,
                },
            )?)?;
            write_cells_from_verdicts(
                &context_verdicts[context as usize],
                "edge",
                edge,
                &mut cells,
            )?;
            empty_direct_row[context as usize] = Some(row_start);
            starts.push(row_start);
            continue;
        }
        let row_start = dest_len(cells.len())?;
        check_access_cell_capacity(cells.len().checked_add(class_len).ok_or(
            BuildError::ArithmeticOverflow {
                structure: BuildStructure::AccessPlane,
            },
        )?)?;
        let mut merged = match context_id {
            Some(context) => context_verdicts[context as usize].clone(),
            None => vec![None; class_len],
        };
        for &rule in direct {
            fold_rule(&mut merged, rule, rules, classes);
        }
        write_cells_from_verdicts(&merged, "edge", edge, &mut cells)?;
        starts.push(row_start);
    }
    Ok((starts.into_boxed_slice(), cells.into_boxed_slice()))
}

fn materialize_plane(
    unit_count: u32,
    class_count: u32,
    plane: &'static str,
    mut candidates_for: impl FnMut(u32) -> Vec<u32>,
    classes: &Classes,
    rules: &Rules,
    options: SharedNetworkBuildOptions<'_>,
) -> Result<(Box<[u32]>, Box<[AccessCell]>), BuildError> {
    let mut starts = allocate_vec(unit_count, BuildStructure::AccessPlane)?;
    let mut cells = Vec::new();
    let class_len = usize::try_from(class_count).expect("u32");
    for unit in 0..unit_count {
        poll_cancelled(options, unit)?;
        let candidates = candidates_for(unit);
        if candidates.is_empty() || class_count == 0 {
            starts.push(ACCESS_UNCONSTRAINED_ROW);
            continue;
        }
        let mut verdicts = vec![None; class_len];
        for rule in candidates {
            fold_rule(&mut verdicts, rule, rules, classes);
        }
        if verdicts.iter().all(Option::is_none) {
            starts.push(ACCESS_UNCONSTRAINED_ROW);
            continue;
        }
        check_access_cell_capacity(cells.len().checked_add(class_len).ok_or(
            BuildError::ArithmeticOverflow {
                structure: BuildStructure::AccessPlane,
            },
        )?)?;
        let start = u32::try_from(cells.len()).map_err(|_| BuildError::ArithmeticOverflow {
            structure: BuildStructure::AccessPlane,
        })?;
        cells.extend(cells_from_verdicts(&verdicts, plane, unit)?);
        starts.push(start);
    }
    Ok((starts.into_boxed_slice(), cells.into_boxed_slice()))
}

#[cfg(test)]
mod tests {
    use super::UniqueCheck;
    use crate::BuildError;

    #[test]
    fn reconstructed_class_intervals_give_strict_child_ranges() {
        let mut parent = crate::relations::empty_optional(2).expect("parent column");
        crate::relations::set_optional(
            &mut parent,
            1,
            laneflow_static_contract::ParticipantClassOrdinal::from_raw(0),
        )
        .expect("child parent");
        let (depth, enter, exit) = super::reconstruct_class_intervals(&parent, 2).expect("forest");
        assert_eq!(depth, vec![0, 1]);
        assert_eq!(enter[0], 0);
        assert_eq!(exit[0], 2);
        assert_eq!(enter[1], 1);
        assert_eq!(exit[1], 2);
        assert!(enter[1] > enter[0]);
        assert!(exit[1] <= exit[0]);
        assert_ne!((enter[0], exit[0]), (enter[1], exit[1]));
    }

    #[test]
    fn sequence_members_reject_duplicates_but_allow_non_monotonic_order() {
        let mut unique = UniqueCheck::new(4).expect("unique table");
        assert!(unique.ensure_unique(&[2, 0, 1]).is_ok());
        let mut unique = UniqueCheck::new(4).expect("unique table");
        assert!(matches!(
            unique.ensure_unique(&[1, 0, 1]),
            Err(BuildError::InputInvariant { .. })
        ));
    }

    #[test]
    fn regulation_authority_requires_one_jurisdiction_version() {
        let mut canonical = None;
        super::bind_regulation_authority(&mut canonical, ("CN-test", "2026-01"))
            .expect("first declared regulation");
        super::bind_regulation_authority(&mut canonical, ("CN-test", "2026-01"))
            .expect("matching provenance");
        assert!(matches!(
            super::bind_regulation_authority(&mut canonical, ("CN-other", "2026-01")),
            Err(BuildError::InputInvariant { .. })
        ));
        let mut canonical = None;
        super::bind_regulation_authority(&mut canonical, ("CN-test", "2026-01"))
            .expect("first declared regulation");
        assert!(matches!(
            super::bind_regulation_authority(&mut canonical, ("CN-test", "2026-02")),
            Err(BuildError::InputInvariant { .. })
        ));
    }
}
