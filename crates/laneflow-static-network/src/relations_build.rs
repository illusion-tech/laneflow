#![allow(clippy::type_complexity)]

use std::collections::{BTreeMap, BTreeSet};

use laneflow_format::{RegistryCheckedFieldValue, RegistryCheckedRowView, ValueCheckedObjectView};
use laneflow_static_contract::{
    AccessEffect, AccessRuleOrdinal, AuthoringLaneOrdinal, EntityKind, FacilityBandOrdinal,
    JunctionOrdinal, LaneEdgeOrdinal, LaneGroupOrdinal, ManeuverGateOrdinal, ManeuverPathOrdinal,
    MovementOrdinal, PARKING_ANCHOR_ENDPOINT_CLEARANCE_METERS, ParkingAreaOrdinal,
    ParkingSpaceOrdinal, ParticipantClassOrdinal, RoadCorridorOrdinal, RoadSectionOrdinal,
    SignalAspect, SignalControllerOrdinal, SignalGroupOrdinal, SignalPhaseOrdinal,
    StaticRouteOrdinal, StopLineOrdinal, WaitingZoneOrdinal,
};

use crate::builder::{
    SharedNetworkBuildOptions, allocate_vec, checked_f64, checked_field, checked_ordinal_vector,
    checked_record_vector, checked_u8, checked_u32, poll_cancelled,
};
use crate::relations::{
    ACCESS_UNCONSTRAINED_ROW, AccessCell, AccessTarget, BoundedDistance, CorridorElement,
    FacilityKind, RelationPayloads, SharedRelationClosure, assemble, empty_optional, get_optional,
    set_optional,
};
use crate::{BuildError, BuildStructure, EntityCounts, RangeU32, SharedManeuverNetwork};

const STRUCTURE: BuildStructure = BuildStructure::RelationClosure;
const MAX_PORTABLE_SIGNAL_TIME_MS: u64 = 9_007_199_254_740_991;
const ROUTE_DISTANCE_SEGMENT_LIMIT: f64 = f64::MAX / 16.0;

#[allow(clippy::too_many_arguments)]
pub(crate) fn build_relations(
    view: ValueCheckedObjectView<'_>,
    entity_counts: &EntityCounts,
    lane_lengths: &[f64],
    lane_speeds: &[f64],
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
        ParkingAreaOrdinal::from_raw,
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
    let routes = build_routes(
        view,
        entity_counts,
        lane_lengths,
        lane_speeds,
        &gate_signal_group,
        &edge_junction,
        successor_ranges,
        successors,
        maneuvers,
        &gate_path,
        &gate_transition_index,
        &waiting_path,
        &waiting_entry_gate,
        &waiting_release_gate,
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
        routes.edge_ranges,
        routes.edges,
        routes.gate_ranges,
        routes.transition_gates,
        routes.maneuver_ranges,
        routes.maneuver_paths,
        routes.maneuver_entry,
        routes.maneuver_exit,
        routes.maneuver_gate_occ_start,
        routes.maneuver_gate_occ_count,
        routes.maneuver_waiting_occ_start,
        routes.maneuver_waiting_occ_count,
        routes.gate_occ_ranges,
        routes.gate_occ_gates,
        routes.gate_occ_maneuver,
        routes.gate_occ_from,
        routes.gate_occ_next,
        routes.gate_occ_next_boundary,
        routes.gate_occ_waiting,
        routes.waiting_occ_ranges,
        routes.waiting_occ_zones,
        routes.waiting_occ_maneuver,
        routes.waiting_occ_entry_gate,
        routes.waiting_occ_release_gate,
        routes.waiting_occ_entry_edge,
        routes.waiting_occ_release_edge,
        routes.reverse_kind,
        routes.reverse_ordinal,
        routes.reverse_route,
        routes.reverse_occurrence,
        routes.distance_to_end,
        routes.distance_ranges,
        routes.distance_segments,
        routes.distance_offsets,
        routes.segment_totals,
        routes.segment_ranges,
        routes.next_controlled_gate,
        routes.next_controlled_from,
        routes.next_controlled_distance,
        routes.speed_limit_from,
        routes.speed_limit_to_edge,
        routes.speed_limit_target,
        routes.speed_limit_ranges,
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
    view.registry_view()
        .section(2)
        .and_then(|section| section.table(u32::from(kind.code() - 1)))
        .ok_or(BuildError::InputInvariant {
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

pub(crate) fn count_relation_payloads(
    view: ValueCheckedObjectView<'_>,
    entity_counts: &EntityCounts,
    options: SharedNetworkBuildOptions<'_>,
) -> Result<RelationPayloads, BuildError> {
    let (intern_keys, intern_utf8) = count_intern_payloads(view, options)?;
    let (edge_cells, path_cells) = count_access_cells(view, entity_counts, options)?;
    let (route_segment_totals, speed_limit_transitions) =
        count_route_derived(view, entity_counts, options)?;
    Ok(RelationPayloads {
        corridor_elements: sum_record_field(view, EntityKind::RoadCorridor, 4, options)?,
        section_lanes: sum_ordinal_field(view, EntityKind::RoadSection, 5, options)?,
        authoring_edges: sum_ordinal_field(view, EntityKind::AuthoringLane, 4, options)?,
        junction_movements: sum_ordinal_field(view, EntityKind::Junction, 3, options)?,
        movement_paths: sum_ordinal_field(view, EntityKind::Movement, 6, options)?,
        stop_line_gates: sum_ordinal_field(view, EntityKind::StopLine, 4, options)?,
        group_gates: sum_ordinal_field(view, EntityKind::SignalGroup, 4, options)?,
        controller_groups: sum_ordinal_field(view, EntityKind::SignalController, 5, options)?,
        controller_phases: sum_ordinal_field(view, EntityKind::SignalController, 6, options)?,
        phase_states: sum_record_field(view, EntityKind::SignalPhase, 5, options)?,
        parking_spaces: sum_ordinal_field(view, EntityKind::ParkingArea, 3, options)?,
        lane_group_members: sum_ordinal_field(view, EntityKind::LaneGroup, 4, options)?,
        rule_classes: sum_ordinal_field(view, EntityKind::AccessRule, 6, options)?,
        route_edges: sum_ordinal_field(view, EntityKind::StaticRoute, 3, options)?,
        route_transitions: sum_record_field(view, EntityKind::StaticRoute, 4, options)?,
        route_maneuvers: relation_table(view, 1)?.row_count(),
        route_gate_occurrences: relation_table(view, 2)?.row_count(),
        route_waiting_occurrences: relation_table(view, 3)?.row_count(),
        route_reverse: relation_table(view, 4)?.row_count(),
        intern_keys,
        intern_utf8,
        edge_cells,
        path_cells,
        route_segment_totals,
        speed_limit_transitions,
    })
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

fn count_route_derived(
    view: ValueCheckedObjectView<'_>,
    entity_counts: &EntityCounts,
    options: SharedNetworkBuildOptions<'_>,
) -> Result<(u32, u32), BuildError> {
    let lane_count = entity_counts.count(EntityKind::LaneEdge);
    let lane_table = entity_table(view, EntityKind::LaneEdge)?;
    let mut lengths = Vec::with_capacity(usize::try_from(lane_count).expect("u32 fits"));
    let mut speeds = Vec::with_capacity(usize::try_from(lane_count).expect("u32 fits"));
    for (index, row) in lane_table.rows().enumerate() {
        poll_cancelled(options, u32::try_from(index).unwrap_or(u32::MAX))?;
        lengths.push(checked_f64(row, 3, STRUCTURE)?);
        speeds.push(checked_f64(row, 4, STRUCTURE)?);
    }
    if lengths.len() != usize::try_from(lane_count).expect("u32 fits") {
        return Err(BuildError::InputInvariant {
            structure: STRUCTURE,
        });
    }
    let mut totals = 0_u32;
    let mut drops = 0_u32;
    let route_table = entity_table(view, EntityKind::StaticRoute)?;
    for (index, row) in route_table.rows().enumerate() {
        poll_cancelled(options, u32::try_from(index).unwrap_or(u32::MAX))?;
        let route_edges = checked_ordinal_vector(row, 3, STRUCTURE)?;
        let mut route_lengths =
            Vec::with_capacity(usize::try_from(route_edges.len()).expect("u32"));
        let mut previous_speed = None;
        for edge_index in 0..route_edges.len() {
            poll_cancelled(options, edge_index)?;
            let edge = route_edges
                .get(edge_index)
                .ok_or(BuildError::InputInvariant {
                    structure: STRUCTURE,
                })?;
            if edge >= lane_count {
                return Err(BuildError::ReferenceOutOfBounds {
                    structure: STRUCTURE,
                    ordinal: edge,
                    limit: lane_count,
                });
            }
            let slot = usize::try_from(edge).expect("u32 fits");
            route_lengths.push(lengths[slot]);
            let speed = speeds[slot];
            if let Some(from_speed) = previous_speed
                && speed < from_speed
            {
                drops = drops.checked_add(1).ok_or(BuildError::ArithmeticOverflow {
                    structure: STRUCTURE,
                })?;
            }
            previous_speed = Some(speed);
        }
        let segment_count = u32::try_from(segmented_route_coordinates(&route_lengths).2.len())
            .map_err(|_| BuildError::ArithmeticOverflow {
                structure: STRUCTURE,
            })?;
        totals = totals
            .checked_add(segment_count)
            .ok_or(BuildError::ArithmeticOverflow {
                structure: STRUCTURE,
            })?;
    }
    Ok((totals, drops))
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

fn checked_i32(row: RegistryCheckedRowView<'_>, tag: u16) -> Result<i32, BuildError> {
    match checked_field(row, tag, STRUCTURE)? {
        RegistryCheckedFieldValue::I32(value) => Ok(value),
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

fn close_parking_progress(
    edge: u32,
    progress: f64,
    lane_lengths: &[f64],
) -> Result<(), BuildError> {
    let length = lane_lengths
        .get(usize::try_from(edge).expect("u32 fits"))
        .copied()
        .ok_or(BuildError::ReferenceOutOfBounds {
            structure: STRUCTURE,
            ordinal: edge,
            limit: u32::try_from(lane_lengths.len()).unwrap_or(u32::MAX),
        })?;
    if !progress.is_finite()
        || !length.is_finite()
        || progress <= PARKING_ANCHOR_ENDPOINT_CLEARANCE_METERS
        || progress >= length - PARKING_ANCHOR_ENDPOINT_CLEARANCE_METERS
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
    space_area: crate::relations::OptionalColumn<ParkingAreaOrdinal>,
    space_entry_edge: Box<[LaneEdgeOrdinal]>,
    space_entry_progress: Box<[f64]>,
    space_exit_edge: Box<[LaneEdgeOrdinal]>,
    space_exit_progress: Box<[f64]>,
    space_lateral: Box<[f64]>,
    space_heading: Box<[f64]>,
    space_length: Box<[f64]>,
    space_width: Box<[f64]>,
}

fn build_parking(
    view: ValueCheckedObjectView<'_>,
    entity_counts: &EntityCounts,
    lane_count: u32,
    lane_lengths: &[f64],
    unique: &mut UniqueCheck,
    options: SharedNetworkBuildOptions<'_>,
) -> Result<Parking, BuildError> {
    let area_count = entity_counts.count(EntityKind::ParkingArea);
    let space_count = entity_counts.count(EntityKind::ParkingSpace);
    let area_table = entity_table(view, EntityKind::ParkingArea)?;
    let mut ranges = allocate_vec(area_count, STRUCTURE)?;
    let mut spaces = Vec::new();
    for (index, row) in area_table.rows().enumerate() {
        let expected = u32::try_from(index).map_err(|_| BuildError::ArithmeticOverflow {
            structure: STRUCTURE,
        })?;
        poll_cancelled(options, expected)?;
        expect_row_ordinal(row, expected)?;
        ranges.push(push_members(
            checked_ordinal_vector(row, 3, STRUCTURE)?,
            &mut spaces,
            space_count,
            MemberOrder::CanonicalSet,
            options,
            unique,
        )?);
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
                ParkingAreaOrdinal::from_raw(area_ordinal),
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
        let entry_at = checked_f64(row, 5, STRUCTURE)?;
        let exit_at = checked_f64(row, 7, STRUCTURE)?;
        close_parking_progress(entry, entry_at, lane_lengths)?;
        close_parking_progress(exit, exit_at, lane_lengths)?;
        entry_edge.push(LaneEdgeOrdinal::from_raw(entry));
        entry_progress.push(entry_at);
        exit_edge.push(LaneEdgeOrdinal::from_raw(exit));
        exit_progress.push(exit_at);
        lateral.push(checked_f64(row, 8, STRUCTURE)?);
        heading.push(checked_f64(row, 9, STRUCTURE)?);
        length.push(checked_f64(row, 10, STRUCTURE)?);
        width.push(checked_f64(row, 11, STRUCTURE)?);
    }
    Ok(Parking {
        parking_space_ranges: ranges.into_boxed_slice(),
        parking_spaces: spaces
            .into_iter()
            .map(ParkingSpaceOrdinal::from_raw)
            .collect::<Vec<_>>()
            .into_boxed_slice(),
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
        let _ = row.field_by_tag(7);
        priority.push(checked_i32(row, 8)?);
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
    length: Box<[f64]>,
    desired_speed: Box<[f64]>,
    min_gap: Box<[f64]>,
    time_headway: Box<[f64]>,
    max_accel: Box<[f64]>,
    comfort_decel: Box<[f64]>,
    emergency_decel: Box<[f64]>,
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
        length.push(checked_f64(row, 4, STRUCTURE)?);
        desired.push(checked_f64(row, 5, STRUCTURE)?);
        gap.push(checked_f64(row, 6, STRUCTURE)?);
        headway.push(checked_f64(row, 7, STRUCTURE)?);
        accel.push(checked_f64(row, 8, STRUCTURE)?);
        comfort.push(checked_f64(row, 9, STRUCTURE)?);
        emergency.push(checked_f64(row, 10, STRUCTURE)?);
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

struct Routes {
    edge_ranges: Box<[RangeU32]>,
    edges: Box<[LaneEdgeOrdinal]>,
    gate_ranges: Box<[RangeU32]>,
    transition_gates: Box<[Option<ManeuverGateOrdinal>]>,
    maneuver_ranges: Box<[RangeU32]>,
    maneuver_paths: Box<[ManeuverPathOrdinal]>,
    maneuver_entry: Box<[u32]>,
    maneuver_exit: Box<[u32]>,
    maneuver_gate_occ_start: Box<[u32]>,
    maneuver_gate_occ_count: Box<[u32]>,
    maneuver_waiting_occ_start: Box<[u32]>,
    maneuver_waiting_occ_count: Box<[u32]>,
    gate_occ_ranges: Box<[RangeU32]>,
    gate_occ_gates: Box<[ManeuverGateOrdinal]>,
    gate_occ_maneuver: Box<[u32]>,
    gate_occ_from: Box<[u32]>,
    gate_occ_next: Box<[Option<u32>]>,
    gate_occ_next_boundary: Box<[u32]>,
    gate_occ_waiting: Box<[Option<u32>]>,
    waiting_occ_ranges: Box<[RangeU32]>,
    waiting_occ_zones: Box<[WaitingZoneOrdinal]>,
    waiting_occ_maneuver: Box<[u32]>,
    waiting_occ_entry_gate: Box<[u32]>,
    waiting_occ_release_gate: Box<[u32]>,
    waiting_occ_entry_edge: Box<[u32]>,
    waiting_occ_release_edge: Box<[u32]>,
    reverse_kind: Box<[u16]>,
    reverse_ordinal: Box<[u32]>,
    reverse_route: Box<[StaticRouteOrdinal]>,
    reverse_occurrence: Box<[u32]>,
    distance_to_end: Box<[BoundedDistance]>,
    distance_ranges: Box<[RangeU32]>,
    distance_segments: Box<[u32]>,
    distance_offsets: Box<[f64]>,
    segment_totals: Box<[f64]>,
    segment_ranges: Box<[RangeU32]>,
    next_controlled_gate: Box<[Option<ManeuverGateOrdinal>]>,
    next_controlled_from: Box<[u32]>,
    next_controlled_distance: Box<[BoundedDistance]>,
    speed_limit_from: Box<[u32]>,
    speed_limit_to_edge: Box<[LaneEdgeOrdinal]>,
    speed_limit_target: Box<[f64]>,
    speed_limit_ranges: Box<[RangeU32]>,
}

struct OccurrenceCursor {
    current_route: Option<u32>,
    start: u32,
    expected_index: u32,
}

impl OccurrenceCursor {
    fn new() -> Self {
        Self {
            current_route: None,
            start: 0,
            expected_index: 0,
        }
    }

    fn observe(
        &mut self,
        route: u32,
        occurrence_index: u32,
        dest_len: u32,
        ranges: &mut [RangeU32],
        route_count: u32,
    ) -> Result<(), BuildError> {
        if route >= route_count {
            return Err(BuildError::ReferenceOutOfBounds {
                structure: STRUCTURE,
                ordinal: route,
                limit: route_count,
            });
        }
        match self.current_route {
            None => {
                if occurrence_index != 0 {
                    return Err(BuildError::InputInvariant {
                        structure: STRUCTURE,
                    });
                }
                self.current_route = Some(route);
                self.start = dest_len;
                self.expected_index = 1;
            }
            Some(previous) if previous == route => {
                if occurrence_index != self.expected_index {
                    return Err(BuildError::InputInvariant {
                        structure: STRUCTURE,
                    });
                }
                self.expected_index =
                    self.expected_index
                        .checked_add(1)
                        .ok_or(BuildError::ArithmeticOverflow {
                            structure: STRUCTURE,
                        })?;
            }
            Some(previous) if previous < route => {
                ranges[previous as usize] = RangeU32::new(self.start, dest_len - self.start);
                if occurrence_index != 0 {
                    return Err(BuildError::InputInvariant {
                        structure: STRUCTURE,
                    });
                }
                self.current_route = Some(route);
                self.start = dest_len;
                self.expected_index = 1;
            }
            Some(previous) => {
                return Err(BuildError::NonCanonicalOrder {
                    structure: STRUCTURE,
                    previous,
                    actual: route,
                });
            }
        }
        Ok(())
    }

    fn finish(self, dest_len: u32, ranges: &mut [RangeU32]) {
        if let Some(previous) = self.current_route {
            ranges[previous as usize] = RangeU32::new(self.start, dest_len - self.start);
        }
    }
}

fn dest_len(len: usize) -> Result<u32, BuildError> {
    u32::try_from(len).map_err(|_| BuildError::ArithmeticOverflow {
        structure: STRUCTURE,
    })
}

fn route_pair_connected(
    from: LaneEdgeOrdinal,
    to: LaneEdgeOrdinal,
    successor_ranges: &[RangeU32],
    successors: &[LaneEdgeOrdinal],
    maneuvers: &SharedManeuverNetwork,
) -> bool {
    if successor_ranges
        .get(from.index())
        .is_some_and(|range| range.slice(successors).contains(&to))
    {
        return true;
    }
    maneuvers
        .transition_candidates(from)
        .is_some_and(|candidates| {
            candidates
                .iter()
                .any(|candidate| candidate.successor() == to)
        })
}

struct ReconstructedRoute {
    transition_gates: Vec<Option<ManeuverGateOrdinal>>,
    maneuver_paths: Vec<ManeuverPathOrdinal>,
    maneuver_entry: Vec<u32>,
    maneuver_exit: Vec<u32>,
    maneuver_gate_occ_start: Vec<u32>,
    maneuver_gate_occ_count: Vec<u32>,
    maneuver_waiting_occ_start: Vec<u32>,
    maneuver_waiting_occ_count: Vec<u32>,
    gate_occ_gates: Vec<ManeuverGateOrdinal>,
    gate_occ_maneuver: Vec<u32>,
    gate_occ_from: Vec<u32>,
    gate_occ_next: Vec<Option<u32>>,
    gate_occ_next_boundary: Vec<u32>,
    gate_occ_waiting: Vec<Option<u32>>,
    waiting_occ_zones: Vec<WaitingZoneOrdinal>,
    waiting_occ_maneuver: Vec<u32>,
    waiting_occ_entry_gate: Vec<u32>,
    waiting_occ_release_gate: Vec<u32>,
    waiting_occ_entry_edge: Vec<u32>,
    waiting_occ_release_edge: Vec<u32>,
}

fn unique_entry_path_match(
    from: LaneEdgeOrdinal,
    to: LaneEdgeOrdinal,
    remaining: &[LaneEdgeOrdinal],
    maneuvers: &SharedManeuverNetwork,
) -> Result<Option<ManeuverPathOrdinal>, BuildError> {
    let Some(candidates) = maneuvers.transition_candidates(from) else {
        return Ok(None);
    };
    let mut matched = None;
    let mut saw_entry = false;
    for candidate in candidates {
        if candidate.successor() != to || candidate.transition_index() != 0 {
            continue;
        }
        saw_entry = true;
        let path = maneuvers.maneuver_path(candidate.maneuver_path()).ok_or(
            BuildError::InputInvariant {
                structure: STRUCTURE,
            },
        )?;
        if remaining.starts_with(path.edges()) {
            match matched {
                None => matched = Some(candidate.maneuver_path()),
                Some(first) if first != candidate.maneuver_path() => {
                    return Err(BuildError::InputInvariant {
                        structure: STRUCTURE,
                    });
                }
                Some(_) => {}
            }
        }
    }
    if !saw_entry {
        return Ok(None);
    }
    matched
        .ok_or(BuildError::InputInvariant {
            structure: STRUCTURE,
        })
        .map(Some)
}

fn reconstruct_route_occurrences(
    route_edges: &[LaneEdgeOrdinal],
    edge_junction: &crate::relations::OptionalColumn<JunctionOrdinal>,
    maneuvers: &SharedManeuverNetwork,
    gate_transition_index: &[u32],
    waiting_entry_gate: &[ManeuverGateOrdinal],
    waiting_release_gate: &[ManeuverGateOrdinal],
) -> Result<ReconstructedRoute, BuildError> {
    if let (Some(first), Some(last)) = (route_edges.first(), route_edges.last())
        && (get_optional(edge_junction, first.raw()).is_some()
            || get_optional(edge_junction, last.raw()).is_some())
    {
        return Err(BuildError::InputInvariant {
            structure: STRUCTURE,
        });
    }
    let transition_len = route_edges.len().saturating_sub(1);
    let mut expected = ReconstructedRoute {
        transition_gates: vec![None; transition_len],
        maneuver_paths: Vec::new(),
        maneuver_entry: Vec::new(),
        maneuver_exit: Vec::new(),
        maneuver_gate_occ_start: Vec::new(),
        maneuver_gate_occ_count: Vec::new(),
        maneuver_waiting_occ_start: Vec::new(),
        maneuver_waiting_occ_count: Vec::new(),
        gate_occ_gates: Vec::new(),
        gate_occ_maneuver: Vec::new(),
        gate_occ_from: Vec::new(),
        gate_occ_next: Vec::new(),
        gate_occ_next_boundary: Vec::new(),
        gate_occ_waiting: Vec::new(),
        waiting_occ_zones: Vec::new(),
        waiting_occ_maneuver: Vec::new(),
        waiting_occ_entry_gate: Vec::new(),
        waiting_occ_release_gate: Vec::new(),
        waiting_occ_entry_edge: Vec::new(),
        waiting_occ_release_edge: Vec::new(),
    };
    let mut internal_coverage = vec![None; route_edges.len()];
    for entry_index in 0..transition_len {
        let from = route_edges[entry_index];
        let to = route_edges[entry_index + 1];
        let Some(path_ordinal) =
            unique_entry_path_match(from, to, &route_edges[entry_index..], maneuvers)?
        else {
            continue;
        };
        let path = maneuvers
            .maneuver_path(path_ordinal)
            .ok_or(BuildError::InputInvariant {
                structure: STRUCTURE,
            })?;
        let exit_index = entry_index
            .checked_add(path.edges().len())
            .and_then(|value| value.checked_sub(1))
            .ok_or(BuildError::ArithmeticOverflow {
                structure: STRUCTURE,
            })?;
        if exit_index >= route_edges.len() {
            return Err(BuildError::InputInvariant {
                structure: STRUCTURE,
            });
        }
        for coverage in internal_coverage
            .iter_mut()
            .take(exit_index)
            .skip(entry_index + 1)
        {
            if coverage.is_some() {
                return Err(BuildError::InputInvariant {
                    structure: STRUCTURE,
                });
            }
            *coverage = Some(path_ordinal);
        }
        let maneuver_index = dest_len(expected.maneuver_paths.len())?;
        let gate_start = dest_len(expected.gate_occ_gates.len())?;
        let entry_u32 = dest_len(entry_index)?;
        let exit_u32 = dest_len(exit_index)?;
        for &gate in path.maneuver_gates() {
            let transition_index =
                *gate_transition_index
                    .get(gate.index())
                    .ok_or(BuildError::InputInvariant {
                        structure: STRUCTURE,
                    })?;
            let from_route =
                entry_u32
                    .checked_add(transition_index)
                    .ok_or(BuildError::ArithmeticOverflow {
                        structure: STRUCTURE,
                    })?;
            let from_slot = usize::try_from(from_route).expect("u32");
            if from_slot >= expected.transition_gates.len() {
                return Err(BuildError::InputInvariant {
                    structure: STRUCTURE,
                });
            }
            expected.transition_gates[from_slot] = Some(gate);
            expected.gate_occ_gates.push(gate);
            expected.gate_occ_maneuver.push(maneuver_index);
            expected.gate_occ_from.push(from_route);
            expected.gate_occ_next.push(None);
            expected.gate_occ_next_boundary.push(exit_u32);
            expected.gate_occ_waiting.push(None);
        }
        let gate_end = dest_len(expected.gate_occ_gates.len())?;
        for local in gate_start..gate_end {
            let slot = usize::try_from(local).expect("u32");
            let last = local + 1 == gate_end;
            expected.gate_occ_next[slot] = if last { None } else { Some(local + 1) };
            expected.gate_occ_next_boundary[slot] = if last {
                exit_u32
            } else {
                expected.gate_occ_from[slot + 1]
            };
        }
        let waiting_start = dest_len(expected.waiting_occ_zones.len())?;
        for &zone in path.waiting_zones() {
            let entry_gate =
                *waiting_entry_gate
                    .get(zone.index())
                    .ok_or(BuildError::InputInvariant {
                        structure: STRUCTURE,
                    })?;
            let release_gate =
                *waiting_release_gate
                    .get(zone.index())
                    .ok_or(BuildError::InputInvariant {
                        structure: STRUCTURE,
                    })?;
            let entry_local = (gate_start..gate_end)
                .find(|local| {
                    expected.gate_occ_gates[usize::try_from(*local).expect("u32")] == entry_gate
                })
                .ok_or(BuildError::InputInvariant {
                    structure: STRUCTURE,
                })?;
            let release_local = (gate_start..gate_end)
                .find(|local| {
                    expected.gate_occ_gates[usize::try_from(*local).expect("u32")] == release_gate
                })
                .ok_or(BuildError::InputInvariant {
                    structure: STRUCTURE,
                })?;
            let waiting_local = dest_len(expected.waiting_occ_zones.len())?;
            let entry_slot = usize::try_from(entry_local).expect("u32");
            expected.gate_occ_waiting[entry_slot] = Some(waiting_local);
            expected.waiting_occ_zones.push(zone);
            expected.waiting_occ_maneuver.push(maneuver_index);
            expected.waiting_occ_entry_gate.push(entry_local);
            expected.waiting_occ_release_gate.push(release_local);
            expected
                .waiting_occ_entry_edge
                .push(expected.gate_occ_from[entry_slot]);
            expected
                .waiting_occ_release_edge
                .push(expected.gate_occ_from[usize::try_from(release_local).expect("u32")]);
        }
        let waiting_end = dest_len(expected.waiting_occ_zones.len())?;
        expected.maneuver_paths.push(path_ordinal);
        expected.maneuver_entry.push(entry_u32);
        expected.maneuver_exit.push(exit_u32);
        expected.maneuver_gate_occ_start.push(gate_start);
        expected
            .maneuver_gate_occ_count
            .push(
                gate_end
                    .checked_sub(gate_start)
                    .ok_or(BuildError::ArithmeticOverflow {
                        structure: STRUCTURE,
                    })?,
            );
        expected.maneuver_waiting_occ_start.push(waiting_start);
        expected
            .maneuver_waiting_occ_count
            .push(waiting_end.checked_sub(waiting_start).ok_or(
                BuildError::ArithmeticOverflow {
                    structure: STRUCTURE,
                },
            )?);
    }
    for (edge_index, edge) in route_edges.iter().enumerate() {
        if get_optional(edge_junction, edge.raw()).is_some()
            && internal_coverage[edge_index].is_none()
        {
            return Err(BuildError::InputInvariant {
                structure: STRUCTURE,
            });
        }
    }
    Ok(expected)
}

fn segmented_route_coordinates(edge_lengths: &[f64]) -> (Vec<u32>, Vec<f64>, Vec<f64>) {
    let mut segments = Vec::with_capacity(edge_lengths.len());
    let mut offsets = Vec::with_capacity(edge_lengths.len());
    let mut totals = Vec::new();
    let mut current_total = 0.0;
    let mut current_has_occurrence = false;
    for edge_length in edge_lengths.iter().copied() {
        let combined = current_total + edge_length;
        let must_start_segment = current_has_occurrence
            && (edge_length > ROUTE_DISTANCE_SEGMENT_LIMIT
                || current_total > ROUTE_DISTANCE_SEGMENT_LIMIT - edge_length
                || combined == current_total
                || combined == edge_length);
        if must_start_segment {
            totals.push(current_total);
            current_total = 0.0;
        }
        segments.push(u32::try_from(totals.len()).expect("segment index fits"));
        offsets.push(current_total);
        current_total += edge_length;
        current_has_occurrence = true;
        if edge_length > ROUTE_DISTANCE_SEGMENT_LIMIT {
            totals.push(current_total);
            current_total = 0.0;
            current_has_occurrence = false;
        }
    }
    if current_has_occurrence {
        totals.push(current_total);
    }
    (segments, offsets, totals)
}

#[allow(clippy::too_many_arguments)]
fn build_routes(
    view: ValueCheckedObjectView<'_>,
    entity_counts: &EntityCounts,
    lane_lengths: &[f64],
    lane_speeds: &[f64],
    gate_signal_group: &crate::relations::OptionalColumn<SignalGroupOrdinal>,
    edge_junction: &crate::relations::OptionalColumn<JunctionOrdinal>,
    successor_ranges: &[RangeU32],
    successors: &[LaneEdgeOrdinal],
    maneuvers: &SharedManeuverNetwork,
    _gate_path: &[ManeuverPathOrdinal],
    gate_transition_index: &[u32],
    _waiting_path: &[ManeuverPathOrdinal],
    waiting_entry_gate: &[ManeuverGateOrdinal],
    waiting_release_gate: &[ManeuverGateOrdinal],
    options: SharedNetworkBuildOptions<'_>,
) -> Result<Routes, BuildError> {
    let route_count = entity_counts.count(EntityKind::StaticRoute);
    let lane_count = entity_counts.count(EntityKind::LaneEdge);
    let gate_limit = entity_counts.count(EntityKind::ManeuverGate);
    let path_limit = entity_counts.count(EntityKind::ManeuverPath);
    let waiting_limit = entity_counts.count(EntityKind::WaitingZone);
    let table = entity_table(view, EntityKind::StaticRoute)?;
    let mut edge_ranges = allocate_vec(route_count, STRUCTURE)?;
    let mut edges = Vec::new();
    let mut gate_ranges = allocate_vec(route_count, STRUCTURE)?;
    let mut transition_gates = Vec::new();
    let mut distance_ranges = allocate_vec(route_count, STRUCTURE)?;
    let mut distance_to_end = Vec::new();
    let mut distance_segments = Vec::new();
    let mut distance_offsets = Vec::new();
    let mut segment_totals = Vec::new();
    let mut segment_ranges = allocate_vec(route_count, STRUCTURE)?;
    let mut next_gate = Vec::new();
    let mut next_from = Vec::new();
    let mut next_distance = Vec::new();
    let mut speed_from = Vec::new();
    let mut speed_to = Vec::new();
    let mut speed_target = Vec::new();
    let mut speed_ranges = allocate_vec(route_count, STRUCTURE)?;
    for (index, row) in table.rows().enumerate() {
        let expected = u32::try_from(index).map_err(|_| BuildError::ArithmeticOverflow {
            structure: STRUCTURE,
        })?;
        poll_cancelled(options, expected)?;
        expect_row_ordinal(row, expected)?;
        let route_edges = checked_ordinal_vector(row, 3, STRUCTURE)?;
        let start = u32::try_from(edges.len()).map_err(|_| BuildError::ArithmeticOverflow {
            structure: STRUCTURE,
        })?;
        for edge_index in 0..route_edges.len() {
            poll_cancelled(options, edge_index)?;
            let edge = route_edges
                .get(edge_index)
                .ok_or(BuildError::InputInvariant {
                    structure: STRUCTURE,
                })?;
            if edge >= lane_count {
                return Err(BuildError::ReferenceOutOfBounds {
                    structure: STRUCTURE,
                    ordinal: edge,
                    limit: lane_count,
                });
            }
            edges.push(LaneEdgeOrdinal::from_raw(edge));
        }
        edge_ranges.push(RangeU32::new(start, route_edges.len()));
        let route_edge_slice = &edges[usize::try_from(start).expect("u32")..];
        for pair in route_edge_slice.windows(2) {
            if !route_pair_connected(pair[0], pair[1], successor_ranges, successors, maneuvers) {
                return Err(BuildError::InputInvariant {
                    structure: STRUCTURE,
                });
            }
        }
        let gates = checked_record_vector(row, 4, STRUCTURE)?;
        let expected_transitions = route_edges.len().saturating_sub(1);
        if gates.len() != expected_transitions {
            return Err(BuildError::InputInvariant {
                structure: STRUCTURE,
            });
        }
        let gate_start =
            u32::try_from(transition_gates.len()).map_err(|_| BuildError::ArithmeticOverflow {
                structure: STRUCTURE,
            })?;
        for (gate_index, gate_row) in gates.rows().enumerate() {
            poll_cancelled(options, u32::try_from(gate_index).unwrap_or(u32::MAX))?;
            let gate = match optional_u32(gate_row, 1)? {
                None => None,
                Some(gate) if gate < gate_limit => Some(ManeuverGateOrdinal::from_raw(gate)),
                Some(gate) => {
                    return Err(BuildError::ReferenceOutOfBounds {
                        structure: STRUCTURE,
                        ordinal: gate,
                        limit: gate_limit,
                    });
                }
            };
            transition_gates.push(gate);
        }
        gate_ranges.push(RangeU32::new(gate_start, expected_transitions));

        let route_edge_slice = &edges[usize::try_from(start).expect("u32")..];
        let mut suffix = BoundedDistance::finite(0.0);
        let mut suffix_list = vec![BoundedDistance::finite(0.0); route_edge_slice.len()];
        for (index, edge) in route_edge_slice.iter().enumerate().rev() {
            suffix = suffix.add(lane_lengths.get(edge.index()).copied().unwrap_or(0.0));
            suffix_list[index] = suffix;
        }
        let dist_start =
            u32::try_from(distance_to_end.len()).map_err(|_| BuildError::ArithmeticOverflow {
                structure: STRUCTURE,
            })?;
        distance_to_end.extend_from_slice(&suffix_list);
        distance_ranges.push(RangeU32::new(
            dist_start,
            u32::try_from(suffix_list.len()).expect("fits"),
        ));
        let lengths: Vec<f64> = route_edge_slice
            .iter()
            .map(|edge| lane_lengths.get(edge.index()).copied().unwrap_or(0.0))
            .collect();
        let (segments, offsets, totals) = segmented_route_coordinates(&lengths);
        let segment_start =
            u32::try_from(segment_totals.len()).map_err(|_| BuildError::ArithmeticOverflow {
                structure: STRUCTURE,
            })?;
        distance_segments.extend_from_slice(&segments);
        distance_offsets.extend_from_slice(&offsets);
        segment_totals.extend_from_slice(&totals);
        segment_ranges.push(RangeU32::new(
            segment_start,
            u32::try_from(totals.len()).expect("fits"),
        ));

        let mut next = None;
        let mut next_from_edge = 0_u32;
        let mut next_dist = BoundedDistance::finite(0.0);
        let mut next_gates = vec![None; route_edge_slice.len()];
        let mut next_froms = vec![0_u32; route_edge_slice.len()];
        let mut next_dists = vec![BoundedDistance::finite(0.0); route_edge_slice.len()];
        let transition_start = usize::try_from(gate_start).expect("u32");
        let transition_count = usize::try_from(expected_transitions).expect("u32");
        let route_transitions =
            &transition_gates[transition_start..transition_start + transition_count];
        for route_edge_index in (0..route_edge_slice.len()).rev() {
            let length = lane_lengths
                .get(route_edge_slice[route_edge_index].index())
                .copied()
                .unwrap_or(0.0);
            if let Some(gate) = route_transitions
                .get(route_edge_index)
                .copied()
                .flatten()
                .filter(|gate| get_optional(gate_signal_group, gate.raw()).is_some())
            {
                next = Some(gate);
                next_from_edge = u32::try_from(route_edge_index).expect("fits");
                next_dist = BoundedDistance::finite(0.0).add(length);
            } else if next.is_some() {
                next_dist = next_dist.add(length);
            }
            next_gates[route_edge_index] = next;
            next_froms[route_edge_index] = next_from_edge;
            next_dists[route_edge_index] = next_dist;
        }
        next_gate.extend_from_slice(&next_gates);
        next_from.extend_from_slice(&next_froms);
        next_distance.extend_from_slice(&next_dists);

        let speed_start =
            u32::try_from(speed_from.len()).map_err(|_| BuildError::ArithmeticOverflow {
                structure: STRUCTURE,
            })?;
        for (from_index, pair) in route_edge_slice.windows(2).enumerate() {
            let from_speed = lane_speeds.get(pair[0].index()).copied().unwrap_or(0.0);
            let to_speed = lane_speeds.get(pair[1].index()).copied().unwrap_or(0.0);
            if to_speed < from_speed {
                speed_from.push(u32::try_from(from_index).expect("fits"));
                speed_to.push(pair[1]);
                speed_target.push(to_speed);
            }
        }
        speed_ranges.push(RangeU32::new(
            speed_start,
            u32::try_from(speed_from.len()).expect("fits") - speed_start,
        ));
    }

    let maneuver_table = relation_table(view, 1)?;
    let gate_occ_table = relation_table(view, 2)?;
    let waiting_occ_table = relation_table(view, 3)?;
    let mut maneuver_ranges = vec![RangeU32::new(0, 0); usize::try_from(route_count).expect("u32")];
    let mut maneuver_paths = Vec::new();
    let mut maneuver_entry = Vec::new();
    let mut maneuver_exit = Vec::new();
    let mut maneuver_gate_occ_start = Vec::new();
    let mut maneuver_gate_occ_count = Vec::new();
    let mut maneuver_waiting_occ_start = Vec::new();
    let mut maneuver_waiting_occ_count = Vec::new();
    let mut cursor = OccurrenceCursor::new();
    for (index, row) in maneuver_table.rows().enumerate() {
        poll_cancelled(options, u32::try_from(index).unwrap_or(u32::MAX))?;
        let route = checked_u32(row, 1, STRUCTURE)?;
        let occurrence_index = checked_u32(row, 2, STRUCTURE)?;
        cursor.observe(
            route,
            occurrence_index,
            dest_len(maneuver_paths.len())?,
            &mut maneuver_ranges,
            route_count,
        )?;
        let path = checked_u32(row, 3, STRUCTURE)?;
        if path >= path_limit {
            return Err(BuildError::ReferenceOutOfBounds {
                structure: STRUCTURE,
                ordinal: path,
                limit: path_limit,
            });
        }
        maneuver_paths.push(ManeuverPathOrdinal::from_raw(path));
        maneuver_entry.push(checked_u32(row, 4, STRUCTURE)?);
        maneuver_exit.push(checked_u32(row, 5, STRUCTURE)?);
        maneuver_gate_occ_start.push(checked_u32(row, 6, STRUCTURE)?);
        maneuver_gate_occ_count.push(checked_u32(row, 7, STRUCTURE)?);
        maneuver_waiting_occ_start.push(checked_u32(row, 8, STRUCTURE)?);
        maneuver_waiting_occ_count.push(checked_u32(row, 9, STRUCTURE)?);
    }
    cursor.finish(dest_len(maneuver_paths.len())?, &mut maneuver_ranges);

    let mut gate_occ_ranges = vec![RangeU32::new(0, 0); usize::try_from(route_count).expect("u32")];
    let mut gate_occ_gates = Vec::new();
    let mut gate_occ_maneuver = Vec::new();
    let mut gate_occ_from = Vec::new();
    let mut gate_occ_next = Vec::new();
    let mut gate_occ_next_boundary = Vec::new();
    let mut gate_occ_waiting = Vec::new();
    cursor = OccurrenceCursor::new();
    for (index, row) in gate_occ_table.rows().enumerate() {
        poll_cancelled(options, u32::try_from(index).unwrap_or(u32::MAX))?;
        let route = checked_u32(row, 1, STRUCTURE)?;
        let occurrence_index = checked_u32(row, 2, STRUCTURE)?;
        cursor.observe(
            route,
            occurrence_index,
            dest_len(gate_occ_gates.len())?,
            &mut gate_occ_ranges,
            route_count,
        )?;
        let gate = checked_u32(row, 3, STRUCTURE)?;
        if gate >= gate_limit {
            return Err(BuildError::ReferenceOutOfBounds {
                structure: STRUCTURE,
                ordinal: gate,
                limit: gate_limit,
            });
        }
        gate_occ_gates.push(ManeuverGateOrdinal::from_raw(gate));
        gate_occ_maneuver.push(checked_u32(row, 4, STRUCTURE)?);
        gate_occ_from.push(checked_u32(row, 5, STRUCTURE)?);
        gate_occ_next.push(optional_u32(row, 6)?);
        gate_occ_next_boundary.push(checked_u32(row, 7, STRUCTURE)?);
        gate_occ_waiting.push(optional_u32(row, 8)?);
    }
    cursor.finish(dest_len(gate_occ_gates.len())?, &mut gate_occ_ranges);

    let mut waiting_occ_ranges =
        vec![RangeU32::new(0, 0); usize::try_from(route_count).expect("u32")];
    let mut waiting_occ_zones = Vec::new();
    let mut waiting_occ_maneuver = Vec::new();
    let mut waiting_occ_entry_gate = Vec::new();
    let mut waiting_occ_release_gate = Vec::new();
    let mut waiting_occ_entry_edge = Vec::new();
    let mut waiting_occ_release_edge = Vec::new();
    cursor = OccurrenceCursor::new();
    for (index, row) in waiting_occ_table.rows().enumerate() {
        poll_cancelled(options, u32::try_from(index).unwrap_or(u32::MAX))?;
        let route = checked_u32(row, 1, STRUCTURE)?;
        let occurrence_index = checked_u32(row, 2, STRUCTURE)?;
        cursor.observe(
            route,
            occurrence_index,
            dest_len(waiting_occ_zones.len())?,
            &mut waiting_occ_ranges,
            route_count,
        )?;
        let zone = checked_u32(row, 3, STRUCTURE)?;
        if zone >= waiting_limit {
            return Err(BuildError::ReferenceOutOfBounds {
                structure: STRUCTURE,
                ordinal: zone,
                limit: waiting_limit,
            });
        }
        waiting_occ_zones.push(WaitingZoneOrdinal::from_raw(zone));
        waiting_occ_maneuver.push(checked_u32(row, 4, STRUCTURE)?);
        waiting_occ_entry_gate.push(checked_u32(row, 5, STRUCTURE)?);
        waiting_occ_release_gate.push(checked_u32(row, 6, STRUCTURE)?);
        waiting_occ_entry_edge.push(checked_u32(row, 7, STRUCTURE)?);
        waiting_occ_release_edge.push(checked_u32(row, 8, STRUCTURE)?);
    }
    cursor.finish(dest_len(waiting_occ_zones.len())?, &mut waiting_occ_ranges);

    close_route_occurrences(
        route_count,
        &edge_ranges,
        &edges,
        &gate_ranges,
        &transition_gates,
        &maneuver_ranges,
        &maneuver_paths,
        &maneuver_entry,
        &maneuver_exit,
        &maneuver_gate_occ_start,
        &maneuver_gate_occ_count,
        &maneuver_waiting_occ_start,
        &maneuver_waiting_occ_count,
        &gate_occ_ranges,
        &gate_occ_gates,
        &gate_occ_maneuver,
        &gate_occ_from,
        &gate_occ_next,
        &gate_occ_next_boundary,
        &gate_occ_waiting,
        &waiting_occ_ranges,
        &waiting_occ_zones,
        &waiting_occ_maneuver,
        &waiting_occ_entry_gate,
        &waiting_occ_release_gate,
        &waiting_occ_entry_edge,
        &waiting_occ_release_edge,
        waiting_entry_gate,
        waiting_release_gate,
        edge_junction,
        maneuvers,
        gate_transition_index,
        options,
    )?;

    let (reverse_kind, reverse_ordinal, reverse_route, reverse_occurrence) = build_route_reverse(
        view,
        entity_counts,
        route_count,
        &edge_ranges,
        &edges,
        &maneuver_ranges,
        &maneuver_paths,
        &gate_occ_ranges,
        &gate_occ_gates,
        &waiting_occ_ranges,
        &waiting_occ_zones,
        options,
    )?;

    Ok(Routes {
        edge_ranges: edge_ranges.into_boxed_slice(),
        edges: edges.into_boxed_slice(),
        gate_ranges: gate_ranges.into_boxed_slice(),
        transition_gates: transition_gates.into_boxed_slice(),
        maneuver_ranges: maneuver_ranges.into_boxed_slice(),
        maneuver_paths: maneuver_paths.into_boxed_slice(),
        maneuver_entry: maneuver_entry.into_boxed_slice(),
        maneuver_exit: maneuver_exit.into_boxed_slice(),
        maneuver_gate_occ_start: maneuver_gate_occ_start.into_boxed_slice(),
        maneuver_gate_occ_count: maneuver_gate_occ_count.into_boxed_slice(),
        maneuver_waiting_occ_start: maneuver_waiting_occ_start.into_boxed_slice(),
        maneuver_waiting_occ_count: maneuver_waiting_occ_count.into_boxed_slice(),
        gate_occ_ranges: gate_occ_ranges.into_boxed_slice(),
        gate_occ_gates: gate_occ_gates.into_boxed_slice(),
        gate_occ_maneuver: gate_occ_maneuver.into_boxed_slice(),
        gate_occ_from: gate_occ_from.into_boxed_slice(),
        gate_occ_next: gate_occ_next.into_boxed_slice(),
        gate_occ_next_boundary: gate_occ_next_boundary.into_boxed_slice(),
        gate_occ_waiting: gate_occ_waiting.into_boxed_slice(),
        waiting_occ_ranges: waiting_occ_ranges.into_boxed_slice(),
        waiting_occ_zones: waiting_occ_zones.into_boxed_slice(),
        waiting_occ_maneuver: waiting_occ_maneuver.into_boxed_slice(),
        waiting_occ_entry_gate: waiting_occ_entry_gate.into_boxed_slice(),
        waiting_occ_release_gate: waiting_occ_release_gate.into_boxed_slice(),
        waiting_occ_entry_edge: waiting_occ_entry_edge.into_boxed_slice(),
        waiting_occ_release_edge: waiting_occ_release_edge.into_boxed_slice(),
        reverse_kind,
        reverse_ordinal,
        reverse_route,
        reverse_occurrence,
        distance_to_end: distance_to_end.into_boxed_slice(),
        distance_ranges: distance_ranges.into_boxed_slice(),
        distance_segments: distance_segments.into_boxed_slice(),
        distance_offsets: distance_offsets.into_boxed_slice(),
        segment_totals: segment_totals.into_boxed_slice(),
        segment_ranges: segment_ranges.into_boxed_slice(),
        next_controlled_gate: next_gate.into_boxed_slice(),
        next_controlled_from: next_from.into_boxed_slice(),
        next_controlled_distance: next_distance.into_boxed_slice(),
        speed_limit_from: speed_from.into_boxed_slice(),
        speed_limit_to_edge: speed_to.into_boxed_slice(),
        speed_limit_target: speed_target.into_boxed_slice(),
        speed_limit_ranges: speed_ranges.into_boxed_slice(),
    })
}

#[allow(clippy::too_many_arguments)]
fn close_route_occurrences(
    route_count: u32,
    edge_ranges: &[RangeU32],
    edges: &[LaneEdgeOrdinal],
    gate_ranges: &[RangeU32],
    transition_gates: &[Option<ManeuverGateOrdinal>],
    maneuver_ranges: &[RangeU32],
    maneuver_paths: &[ManeuverPathOrdinal],
    maneuver_entry: &[u32],
    maneuver_exit: &[u32],
    maneuver_gate_occ_start: &[u32],
    maneuver_gate_occ_count: &[u32],
    maneuver_waiting_occ_start: &[u32],
    maneuver_waiting_occ_count: &[u32],
    gate_occ_ranges: &[RangeU32],
    gate_occ_gates: &[ManeuverGateOrdinal],
    gate_occ_maneuver: &[u32],
    gate_occ_from: &[u32],
    gate_occ_next: &[Option<u32>],
    gate_occ_next_boundary: &[u32],
    gate_occ_waiting: &[Option<u32>],
    waiting_occ_ranges: &[RangeU32],
    waiting_occ_zones: &[WaitingZoneOrdinal],
    waiting_occ_maneuver: &[u32],
    waiting_occ_entry_gate: &[u32],
    waiting_occ_release_gate: &[u32],
    waiting_occ_entry_edge: &[u32],
    waiting_occ_release_edge: &[u32],
    waiting_entry_gate: &[ManeuverGateOrdinal],
    waiting_release_gate: &[ManeuverGateOrdinal],
    edge_junction: &crate::relations::OptionalColumn<JunctionOrdinal>,
    maneuvers: &SharedManeuverNetwork,
    gate_transition_index: &[u32],
    options: SharedNetworkBuildOptions<'_>,
) -> Result<(), BuildError> {
    for route in 0..route_count {
        poll_cancelled(options, route)?;
        let route_index = usize::try_from(route).expect("u32 fits");
        let route_edges = edge_ranges[route_index].slice(edges);
        let expected = reconstruct_route_occurrences(
            route_edges,
            edge_junction,
            maneuvers,
            gate_transition_index,
            waiting_entry_gate,
            waiting_release_gate,
        )?;
        if gate_ranges[route_index].slice(transition_gates) != expected.transition_gates.as_slice()
            || maneuver_ranges[route_index].slice(maneuver_paths)
                != expected.maneuver_paths.as_slice()
            || maneuver_ranges[route_index].slice(maneuver_entry)
                != expected.maneuver_entry.as_slice()
            || maneuver_ranges[route_index].slice(maneuver_exit)
                != expected.maneuver_exit.as_slice()
            || maneuver_ranges[route_index].slice(maneuver_gate_occ_start)
                != expected.maneuver_gate_occ_start.as_slice()
            || maneuver_ranges[route_index].slice(maneuver_gate_occ_count)
                != expected.maneuver_gate_occ_count.as_slice()
            || maneuver_ranges[route_index].slice(maneuver_waiting_occ_start)
                != expected.maneuver_waiting_occ_start.as_slice()
            || maneuver_ranges[route_index].slice(maneuver_waiting_occ_count)
                != expected.maneuver_waiting_occ_count.as_slice()
            || gate_occ_ranges[route_index].slice(gate_occ_gates)
                != expected.gate_occ_gates.as_slice()
            || gate_occ_ranges[route_index].slice(gate_occ_maneuver)
                != expected.gate_occ_maneuver.as_slice()
            || gate_occ_ranges[route_index].slice(gate_occ_from)
                != expected.gate_occ_from.as_slice()
            || gate_occ_ranges[route_index].slice(gate_occ_next)
                != expected.gate_occ_next.as_slice()
            || gate_occ_ranges[route_index].slice(gate_occ_next_boundary)
                != expected.gate_occ_next_boundary.as_slice()
            || gate_occ_ranges[route_index].slice(gate_occ_waiting)
                != expected.gate_occ_waiting.as_slice()
            || waiting_occ_ranges[route_index].slice(waiting_occ_zones)
                != expected.waiting_occ_zones.as_slice()
            || waiting_occ_ranges[route_index].slice(waiting_occ_maneuver)
                != expected.waiting_occ_maneuver.as_slice()
            || waiting_occ_ranges[route_index].slice(waiting_occ_entry_gate)
                != expected.waiting_occ_entry_gate.as_slice()
            || waiting_occ_ranges[route_index].slice(waiting_occ_release_gate)
                != expected.waiting_occ_release_gate.as_slice()
            || waiting_occ_ranges[route_index].slice(waiting_occ_entry_edge)
                != expected.waiting_occ_entry_edge.as_slice()
            || waiting_occ_ranges[route_index].slice(waiting_occ_release_edge)
                != expected.waiting_occ_release_edge.as_slice()
        {
            return Err(BuildError::InputInvariant {
                structure: STRUCTURE,
            });
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn build_route_reverse(
    view: ValueCheckedObjectView<'_>,
    entity_counts: &EntityCounts,
    route_count: u32,
    edge_ranges: &[RangeU32],
    edges: &[LaneEdgeOrdinal],
    maneuver_ranges: &[RangeU32],
    maneuver_paths: &[ManeuverPathOrdinal],
    gate_occ_ranges: &[RangeU32],
    gate_occ_gates: &[ManeuverGateOrdinal],
    waiting_occ_ranges: &[RangeU32],
    waiting_occ_zones: &[WaitingZoneOrdinal],
    options: SharedNetworkBuildOptions<'_>,
) -> Result<
    (
        Box<[u16]>,
        Box<[u32]>,
        Box<[StaticRouteOrdinal]>,
        Box<[u32]>,
    ),
    BuildError,
> {
    let table = relation_table(view, 4)?;
    let mut kinds = Vec::new();
    let mut ordinals = Vec::new();
    let mut routes = Vec::new();
    let mut occurrences = Vec::new();
    let mut seen_edge = vec![false; edges.len()];
    let mut seen_maneuver = vec![false; maneuver_paths.len()];
    let mut seen_gate = vec![false; gate_occ_gates.len()];
    let mut seen_waiting = vec![false; waiting_occ_zones.len()];
    let mut previous_key = None;
    for (index, row) in table.rows().enumerate() {
        poll_cancelled(options, u32::try_from(index).unwrap_or(u32::MAX))?;
        let kind_code = match checked_field(row, 1, STRUCTURE)? {
            RegistryCheckedFieldValue::U16(value) => value,
            _ => {
                return Err(BuildError::InputInvariant {
                    structure: STRUCTURE,
                });
            }
        };
        let kind = EntityKind::from_code(kind_code).ok_or(BuildError::InputInvariant {
            structure: STRUCTURE,
        })?;
        let ordinal = checked_u32(row, 2, STRUCTURE)?;
        let limit = entity_counts.count(kind);
        if ordinal >= limit {
            return Err(BuildError::ReferenceOutOfBounds {
                structure: STRUCTURE,
                ordinal,
                limit,
            });
        }
        let route = checked_u32(row, 3, STRUCTURE)?;
        if route >= route_count {
            return Err(BuildError::ReferenceOutOfBounds {
                structure: STRUCTURE,
                ordinal: route,
                limit: route_count,
            });
        }
        let occurrence = checked_u32(row, 4, STRUCTURE)?;
        let key = (kind_code, ordinal, route, occurrence);
        if let Some(previous) = previous_key
            && key <= previous
        {
            let (previous_part, actual_part) = if key.0 != previous.0 {
                (u32::from(previous.0), u32::from(key.0))
            } else if key.1 != previous.1 {
                (previous.1, key.1)
            } else if key.2 != previous.2 {
                (previous.2, key.2)
            } else {
                (previous.3, key.3)
            };
            return Err(BuildError::NonCanonicalOrder {
                structure: STRUCTURE,
                previous: previous_part,
                actual: actual_part,
            });
        }
        previous_key = Some(key);
        let route_index = usize::try_from(route).expect("u32 fits");
        match kind {
            EntityKind::LaneEdge => close_reverse_payload(
                edge_ranges[route_index],
                occurrence,
                edges,
                ordinal,
                LaneEdgeOrdinal::raw,
                &mut seen_edge,
            )?,
            EntityKind::ManeuverPath => close_reverse_payload(
                maneuver_ranges[route_index],
                occurrence,
                maneuver_paths,
                ordinal,
                ManeuverPathOrdinal::raw,
                &mut seen_maneuver,
            )?,
            EntityKind::ManeuverGate => close_reverse_payload(
                gate_occ_ranges[route_index],
                occurrence,
                gate_occ_gates,
                ordinal,
                ManeuverGateOrdinal::raw,
                &mut seen_gate,
            )?,
            EntityKind::WaitingZone => close_reverse_payload(
                waiting_occ_ranges[route_index],
                occurrence,
                waiting_occ_zones,
                ordinal,
                WaitingZoneOrdinal::raw,
                &mut seen_waiting,
            )?,
            _ => {
                return Err(BuildError::InputInvariant {
                    structure: STRUCTURE,
                });
            }
        }
        kinds.push(kind_code);
        ordinals.push(ordinal);
        routes.push(StaticRouteOrdinal::from_raw(route));
        occurrences.push(occurrence);
    }
    if seen_edge.iter().any(|seen| !*seen)
        || seen_maneuver.iter().any(|seen| !*seen)
        || seen_gate.iter().any(|seen| !*seen)
        || seen_waiting.iter().any(|seen| !*seen)
    {
        return Err(BuildError::InputInvariant {
            structure: STRUCTURE,
        });
    }
    bucket_reverse_rows(kinds, ordinals, routes, occurrences, entity_counts, options)
}

fn bucket_reverse_rows(
    kinds: Vec<u16>,
    ordinals: Vec<u32>,
    routes: Vec<StaticRouteOrdinal>,
    occurrences: Vec<u32>,
    entity_counts: &EntityCounts,
    options: SharedNetworkBuildOptions<'_>,
) -> Result<
    (
        Box<[u16]>,
        Box<[u32]>,
        Box<[StaticRouteOrdinal]>,
        Box<[u32]>,
    ),
    BuildError,
> {
    let len = kinds.len();
    let len_u32 = dest_len(len)?;
    let mut out_kinds = allocate_vec(len_u32, STRUCTURE)?;
    out_kinds.resize(len, 0);
    let mut out_ordinals = allocate_vec(len_u32, STRUCTURE)?;
    out_ordinals.resize(len, 0);
    let mut out_routes = allocate_vec(len_u32, STRUCTURE)?;
    out_routes.resize(len, StaticRouteOrdinal::from_raw(0));
    let mut out_occurrences = allocate_vec(len_u32, STRUCTURE)?;
    out_occurrences.resize(len, 0);
    let mut cursor = 0_usize;
    for kind in [
        EntityKind::LaneEdge,
        EntityKind::ManeuverPath,
        EntityKind::ManeuverGate,
        EntityKind::WaitingZone,
    ] {
        poll_cancelled(options, u32::from(kind.code()))?;
        let code = kind.code();
        let limit = usize::try_from(entity_counts.count(kind)).expect("u32 fits");
        let mut counts: Vec<usize> = allocate_vec(entity_counts.count(kind), STRUCTURE)?;
        counts.resize(limit, 0);
        for index in 0..len {
            poll_cancelled(options, u32::try_from(index).unwrap_or(u32::MAX))?;
            if kinds[index] == code {
                let ordinal = usize::try_from(ordinals[index]).expect("u32 fits");
                counts[ordinal] =
                    counts[ordinal]
                        .checked_add(1)
                        .ok_or(BuildError::ArithmeticOverflow {
                            structure: STRUCTURE,
                        })?;
            }
        }
        let mut pos: Vec<usize> = allocate_vec(entity_counts.count(kind), STRUCTURE)?;
        pos.resize(limit, 0);
        let mut running = cursor;
        for ordinal in 0..limit {
            pos[ordinal] = running;
            running += counts[ordinal];
        }
        for index in 0..len {
            poll_cancelled(options, u32::try_from(index).unwrap_or(u32::MAX))?;
            if kinds[index] != code {
                continue;
            }
            let ordinal = usize::try_from(ordinals[index]).expect("u32 fits");
            let slot = pos[ordinal];
            pos[ordinal] += 1;
            out_kinds[slot] = kinds[index];
            out_ordinals[slot] = ordinals[index];
            out_routes[slot] = routes[index];
            out_occurrences[slot] = occurrences[index];
        }
        cursor = running;
    }
    Ok((
        out_kinds.into_boxed_slice(),
        out_ordinals.into_boxed_slice(),
        out_routes.into_boxed_slice(),
        out_occurrences.into_boxed_slice(),
    ))
}

fn close_reverse_payload<T: Copy>(
    range: RangeU32,
    occurrence: u32,
    column: &[T],
    ordinal: u32,
    raw: impl Fn(T) -> u32,
    seen: &mut [bool],
) -> Result<(), BuildError> {
    if occurrence >= range.len() {
        return Err(BuildError::ReferenceOutOfBounds {
            structure: STRUCTURE,
            ordinal: occurrence,
            limit: range.len(),
        });
    }
    let slot = usize::try_from(range.start().checked_add(occurrence).ok_or(
        BuildError::ArithmeticOverflow {
            structure: STRUCTURE,
        },
    )?)
    .expect("u32 fits");
    let actual = column
        .get(slot)
        .copied()
        .ok_or(BuildError::InputInvariant {
            structure: STRUCTURE,
        })?;
    if raw(actual) != ordinal || seen.get(slot).copied().unwrap_or(true) {
        return Err(BuildError::InputInvariant {
            structure: STRUCTURE,
        });
    }
    seen[slot] = true;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{OccurrenceCursor, UniqueCheck, segmented_route_coordinates};
    use crate::relations::{RouteDistanceIndexView, RouteDistanceQuery};
    use crate::{BuildError, RangeU32};

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
    fn segmented_coordinates_preserve_finite_windows_around_huge_edges() {
        let lengths = [f64::MAX, 1.0, 2.0, f64::MAX];
        let (segments, offsets, totals) = segmented_route_coordinates(&lengths);
        let suffix = [
            crate::BoundedDistance::finite(0.0)
                .add(f64::MAX)
                .add(2.0)
                .add(1.0)
                .add(f64::MAX),
            crate::BoundedDistance::finite(0.0)
                .add(f64::MAX)
                .add(2.0)
                .add(1.0),
            crate::BoundedDistance::finite(0.0).add(f64::MAX).add(2.0),
            crate::BoundedDistance::finite(0.0).add(f64::MAX),
        ];
        let view = RouteDistanceIndexView::from_parts(&segments, &offsets, &totals, &suffix);
        assert_eq!(
            view.distance_within(1, 0.0, 2, 2.0, 3.0),
            RouteDistanceQuery::Within(3.0)
        );
        assert_eq!(
            view.distance_within(1, 0.0, 3, f64::MAX, f64::MAX),
            RouteDistanceQuery::BeyondHorizon
        );
    }

    #[test]
    fn occurrence_cursor_rejects_gap_overlap_and_backward_owner() {
        let mut ranges = vec![RangeU32::new(0, 0); 3];
        let mut cursor = OccurrenceCursor::new();
        cursor
            .observe(0, 0, 0, &mut ranges, 3)
            .expect("first occurrence");
        cursor
            .observe(0, 1, 1, &mut ranges, 3)
            .expect("dense successor");
        assert!(
            cursor.observe(0, 3, 2, &mut ranges, 3).is_err(),
            "gap in occurrenceIndex"
        );

        let mut ranges = vec![RangeU32::new(0, 0); 3];
        let mut cursor = OccurrenceCursor::new();
        cursor
            .observe(1, 0, 0, &mut ranges, 3)
            .expect("later owner");
        assert!(
            cursor.observe(0, 0, 1, &mut ranges, 3).is_err(),
            "backward owner"
        );
    }
}
