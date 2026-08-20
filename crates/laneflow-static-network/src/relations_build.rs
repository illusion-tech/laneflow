#![allow(clippy::type_complexity)]

use std::collections::BTreeMap;

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
    FacilityKind, SharedRelationClosure, assemble, empty_optional, get_optional, set_optional,
};
use crate::{BuildError, BuildStructure, EntityCounts, RangeU32};

const STRUCTURE: BuildStructure = BuildStructure::RelationClosure;

pub(crate) fn build_relations(
    view: ValueCheckedObjectView<'_>,
    entity_counts: &EntityCounts,
    lane_lengths: &[f64],
    lane_speeds: &[f64],
    options: SharedNetworkBuildOptions<'_>,
) -> Result<SharedRelationClosure, BuildError> {
    let mut intern = Intern::default();
    let lane_count = entity_counts.count(EntityKind::LaneEdge);
    let mut edge_authoring = empty_optional(lane_count)?;
    let mut edge_junction = empty_optional(lane_count)?;
    let mut edge_stop_line = empty_optional(lane_count)?;

    let (corridor_reference_section, corridor_element_ranges, corridor_elements) =
        build_corridors(view, entity_counts, options)?;
    let (section_corridor, section_kind, section_lane_ranges, section_lanes) =
        build_sections(view, entity_counts, &mut intern, options)?;
    let (authoring_section, authoring_edge_ranges, authoring_edges, authoring_group) =
        build_authoring_lanes(
            view,
            entity_counts,
            lane_count,
            &mut edge_authoring,
            options,
        )?;
    let (lane_group_section, lane_group_member_ranges, lane_group_members) =
        build_lane_groups(view, entity_counts, options)?;
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
    let (
        junction_movement_ranges,
        junction_movements,
        movement_junction,
        movement_path_ranges,
        movement_paths,
    ) = build_junctions(view, entity_counts, options)?;
    build_internal_edges(
        view,
        lane_count,
        entity_counts.count(EntityKind::Junction),
        &mut edge_junction,
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
        options,
    )?;
    let signals = build_signals(view, entity_counts, options)?;
    let parking = build_parking(view, entity_counts, lane_count, lane_lengths, options)?;
    let classes = build_classes(view, entity_counts, options)?;
    let rules = build_access_rules(view, entity_counts, options)?;
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

#[derive(Default)]
struct Intern {
    tokens: Vec<Box<str>>,
    by_token: BTreeMap<Box<str>, u32>,
}

impl Intern {
    fn intern(&mut self, token: &str) -> Result<u32, BuildError> {
        if let Some(&index) = self.by_token.get(token) {
            return Ok(index);
        }
        let index =
            u32::try_from(self.tokens.len()).map_err(|_| BuildError::ArithmeticOverflow {
                structure: STRUCTURE,
            })?;
        let owned: Box<str> = token.into();
        self.by_token.insert(owned.clone(), index);
        self.tokens.push(owned);
        Ok(index)
    }

    fn seal(self) -> Box<[Box<str>]> {
        self.tokens.into_boxed_slice()
    }
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

fn push_members(
    members: laneflow_format::RegistryCheckedOrdinalVectorView<'_>,
    dest: &mut Vec<u32>,
    limit: u32,
    order: MemberOrder,
    options: SharedNetworkBuildOptions<'_>,
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
        ensure_unique(&dest[usize::try_from(start).expect("u32 fits")..])?;
    }
    Ok(RangeU32::new(start, members.len()))
}

fn ensure_unique(members: &[u32]) -> Result<(), BuildError> {
    if members.len() < 2 {
        return Ok(());
    }
    let mut sorted = members.to_vec();
    sorted.sort_unstable();
    if sorted.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(BuildError::InputInvariant {
            structure: STRUCTURE,
        });
    }
    Ok(())
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

    let mut seen_group: Vec<Option<LaneGroupOrdinal>> = vec![None; authoring_count];
    for (group_index, range) in lane_group_member_ranges.iter().enumerate() {
        poll_cancelled(options, u32::try_from(group_index).unwrap_or(u32::MAX))?;
        let group = LaneGroupOrdinal::from_raw(u32::try_from(group_index).expect("fits"));
        let section = *lane_group_section
            .get(group_index)
            .ok_or(BuildError::InputInvariant {
                structure: STRUCTURE,
            })?;
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

#[allow(clippy::type_complexity)]
fn build_gates_and_waiting(
    view: ValueCheckedObjectView<'_>,
    entity_counts: &EntityCounts,
    lane_count: u32,
    edge_stop_line: &mut crate::relations::OptionalColumn<StopLineOrdinal>,
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
        offsets.push(checked_u64(row, 3)?);
        cycles.push(checked_u64(row, 4)?);
        controller_group_ranges.push(push_members(
            checked_ordinal_vector(row, 5, STRUCTURE)?,
            &mut controller_groups,
            group_count,
            MemberOrder::CanonicalSet,
            options,
        )?);
        controller_phase_ranges.push(push_members(
            checked_ordinal_vector(row, 6, STRUCTURE)?,
            &mut controller_phases,
            phase_count,
            MemberOrder::Sequence,
            options,
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
        durations.push(checked_u64(row, 4)?);
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
        ranges.push(push_members(
            checked_ordinal_vector(row, 6, STRUCTURE)?,
            &mut classes,
            class_limit,
            MemberOrder::CanonicalSet,
            options,
        )?);
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

fn is_descendant_or_self(classes: &Classes, profile: u32, ancestor: u32) -> bool {
    let Some(enter) = classes.subtree_enter.get(profile as usize).copied() else {
        return false;
    };
    let Some(exit) = classes.subtree_exit.get(profile as usize).copied() else {
        return false;
    };
    let Some(ancestor_enter) = classes.subtree_enter.get(ancestor as usize).copied() else {
        return false;
    };
    let Some(ancestor_exit) = classes.subtree_exit.get(ancestor as usize).copied() else {
        return false;
    };
    enter >= ancestor_enter && exit <= ancestor_exit
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
    for (profile, slot) in verdicts.iter_mut().enumerate() {
        let mut depth = None;
        for class in rule_classes {
            if is_descendant_or_self(classes, profile as u32, class.raw()) {
                let class_depth = classes.depth[class.index()];
                depth = Some(depth.map_or(class_depth, |best: u32| best.max(class_depth)));
            }
        }
        let Some(depth) = depth else {
            continue;
        };
        let key = (
            depth,
            target_specificity(rules.target[rule_index as usize]),
            rules.priority[rule_index as usize],
        );
        let verdict = ClassVerdict {
            key,
            min_allow: matches!(rules.effect[rule_index as usize], AccessEffect::Allow)
                .then_some(rule_index),
            min_deny: matches!(rules.effect[rule_index as usize], AccessEffect::Deny)
                .then_some(rule_index),
        };
        *slot = Some(match *slot {
            Some(existing) => existing.merge(verdict),
            None => verdict,
        });
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
        lane_group_members,
        lane_group_member_ranges,
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
        authoring_group,
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
    lane_group_members: &[AuthoringLaneOrdinal],
    lane_group_member_ranges: &[RangeU32],
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

    let mut lane_groups: BTreeMap<(u32, u32), Vec<u32>> = BTreeMap::new();
    for (group_index, verdicts) in group_verdicts.iter().enumerate() {
        if verdicts.is_none() {
            continue;
        }
        let Some(section) = lane_group_section.get(group_index).copied() else {
            continue;
        };
        let Some(range) = lane_group_member_ranges.get(group_index) else {
            continue;
        };
        let group = u32::try_from(group_index).expect("usize group fits u32");
        for lane in range.slice(lane_group_members) {
            if authoring_section
                .get(lane.index())
                .is_none_or(|owned| *owned != section)
            {
                return Err(BuildError::InputInvariant {
                    structure: BuildStructure::AccessPlane,
                });
            }
            lane_groups
                .entry((section.raw(), lane.raw()))
                .or_default()
                .push(group);
        }
    }

    let mut lane_contexts: BTreeMap<u32, Option<u32>> = BTreeMap::new();
    let mut context_keys: BTreeMap<(u32, Box<[u32]>), u32> = BTreeMap::new();
    let mut context_verdicts: Vec<Vec<Option<ClassVerdict>>> = Vec::new();
    let mut signature_rows: BTreeMap<(Option<u32>, Box<[u32]>), u32> = BTreeMap::new();
    let mut starts = allocate_vec(lane_count, BuildStructure::AccessPlane)?;
    let mut cells = Vec::new();
    for edge in 0..lane_count {
        poll_cancelled(options, edge)?;
        let context_id = match get_optional(edge_authoring, edge) {
            Some(lane) => {
                let section = authoring_section.get(lane.index()).copied().ok_or(
                    BuildError::InputInvariant {
                        structure: BuildStructure::AccessPlane,
                    },
                )?;
                *lane_contexts.entry(lane.raw()).or_insert_with(|| {
                    let groups = lane_groups.get(&(section.raw(), lane.raw()));
                    if section_verdicts
                        .get(section.index())
                        .is_none_or(Option::is_none)
                        && groups.is_none_or(|list| list.is_empty())
                    {
                        return None;
                    }
                    let key = (
                        section.raw(),
                        groups.map_or_else(Box::default, |list| list.as_slice().into()),
                    );
                    if let Some(&id) = context_keys.get(&key) {
                        return Some(id);
                    }
                    let mut merged = match section_verdicts.get(section.index()) {
                        Some(Some(verdicts)) => verdicts.clone(),
                        _ => vec![None; class_len],
                    };
                    if let Some(groups) = groups {
                        for &group_index in groups {
                            if let Some(Some(delta)) = group_verdicts.get(group_index as usize) {
                                merge_verdicts(&mut merged, delta);
                            }
                        }
                    }
                    let id = u32::try_from(context_verdicts.len()).expect("context fits u32");
                    context_verdicts.push(merged);
                    context_keys.insert(key, id);
                    Some(id)
                })
            }
            None => None,
        };
        let direct = &edge_direct[edge as usize];
        if context_id.is_none() && direct.is_empty() {
            starts.push(ACCESS_UNCONSTRAINED_ROW);
            continue;
        }
        let signature = (context_id, direct.as_slice().into());
        if let Some(&row_start) = signature_rows.get(&signature) {
            starts.push(row_start);
            continue;
        }
        if class_count == 0 {
            starts.push(ACCESS_UNCONSTRAINED_ROW);
            signature_rows.insert(signature, ACCESS_UNCONSTRAINED_ROW);
            continue;
        }
        let row_start = cells.len();
        check_access_cell_capacity((signature_rows.len() + 1).checked_mul(class_len).ok_or(
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
        let start = u32::try_from(row_start).map_err(|_| BuildError::ArithmeticOverflow {
            structure: BuildStructure::AccessPlane,
        })?;
        signature_rows.insert(signature, start);
        starts.push(start);
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

fn build_routes(
    view: ValueCheckedObjectView<'_>,
    entity_counts: &EntityCounts,
    lane_lengths: &[f64],
    lane_speeds: &[f64],
    gate_signal_group: &crate::relations::OptionalColumn<SignalGroupOrdinal>,
    edge_junction: &crate::relations::OptionalColumn<JunctionOrdinal>,
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
        &maneuver_ranges,
        &maneuver_entry,
        &maneuver_exit,
        &maneuver_gate_occ_start,
        &maneuver_gate_occ_count,
        &maneuver_waiting_occ_start,
        &maneuver_waiting_occ_count,
        &gate_occ_ranges,
        &gate_occ_maneuver,
        &gate_occ_from,
        &gate_occ_next,
        &gate_occ_next_boundary,
        &gate_occ_waiting,
        &waiting_occ_ranges,
        &waiting_occ_maneuver,
        &waiting_occ_entry_gate,
        &waiting_occ_release_gate,
        &waiting_occ_entry_edge,
        &waiting_occ_release_edge,
        edge_junction,
        options,
    )?;

    let (reverse_kind, reverse_ordinal, reverse_route, reverse_occurrence) = build_route_reverse(
        view,
        entity_counts,
        route_count,
        &edge_ranges,
        &maneuver_ranges,
        &gate_occ_ranges,
        &waiting_occ_ranges,
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
    maneuver_ranges: &[RangeU32],
    maneuver_entry: &[u32],
    maneuver_exit: &[u32],
    maneuver_gate_occ_start: &[u32],
    maneuver_gate_occ_count: &[u32],
    maneuver_waiting_occ_start: &[u32],
    maneuver_waiting_occ_count: &[u32],
    gate_occ_ranges: &[RangeU32],
    gate_occ_maneuver: &[u32],
    gate_occ_from: &[u32],
    gate_occ_next: &[Option<u32>],
    gate_occ_next_boundary: &[u32],
    gate_occ_waiting: &[Option<u32>],
    waiting_occ_ranges: &[RangeU32],
    waiting_occ_maneuver: &[u32],
    waiting_occ_entry_gate: &[u32],
    waiting_occ_release_gate: &[u32],
    waiting_occ_entry_edge: &[u32],
    waiting_occ_release_edge: &[u32],
    edge_junction: &crate::relations::OptionalColumn<JunctionOrdinal>,
    options: SharedNetworkBuildOptions<'_>,
) -> Result<(), BuildError> {
    for route in 0..route_count {
        poll_cancelled(options, route)?;
        let route_index = usize::try_from(route).expect("u32 fits");
        let edge_count = edge_ranges[route_index].len();
        let man_range = maneuver_ranges[route_index];
        let gate_range = gate_occ_ranges[route_index];
        let wait_range = waiting_occ_ranges[route_index];
        let man_len = man_range.len();
        let mut gate_cursor = 0_u32;
        let mut wait_cursor = 0_u32;
        for local in 0..man_len {
            let index = man_range.start() + local;
            let slot = usize::try_from(index).expect("u32 fits");
            if maneuver_gate_occ_start[slot] != gate_cursor
                || maneuver_waiting_occ_start[slot] != wait_cursor
            {
                return Err(BuildError::InputInvariant {
                    structure: STRUCTURE,
                });
            }
            let gate_end = maneuver_gate_occ_start[slot]
                .checked_add(maneuver_gate_occ_count[slot])
                .ok_or(BuildError::ArithmeticOverflow {
                    structure: STRUCTURE,
                })?;
            let wait_end = maneuver_waiting_occ_start[slot]
                .checked_add(maneuver_waiting_occ_count[slot])
                .ok_or(BuildError::ArithmeticOverflow {
                    structure: STRUCTURE,
                })?;
            if gate_end > gate_range.len() || wait_end > wait_range.len() {
                return Err(BuildError::InputInvariant {
                    structure: STRUCTURE,
                });
            }
            gate_cursor = gate_end;
            wait_cursor = wait_end;
            let entry = maneuver_entry[slot];
            let exit = maneuver_exit[slot];
            if entry > exit || exit >= edge_count {
                return Err(BuildError::InputInvariant {
                    structure: STRUCTURE,
                });
            }
        }
        if gate_cursor != gate_range.len() || wait_cursor != wait_range.len() {
            return Err(BuildError::InputInvariant {
                structure: STRUCTURE,
            });
        }

        for local in 0..gate_range.len() {
            let index = gate_range.start() + local;
            let slot = usize::try_from(index).expect("u32 fits");
            let man_local = gate_occ_maneuver[slot];
            if man_local >= man_len {
                return Err(BuildError::InputInvariant {
                    structure: STRUCTURE,
                });
            }
            let man_slot = usize::try_from(man_range.start() + man_local).expect("u32 fits");
            let start = maneuver_gate_occ_start[man_slot];
            let end = start.checked_add(maneuver_gate_occ_count[man_slot]).ok_or(
                BuildError::ArithmeticOverflow {
                    structure: STRUCTURE,
                },
            )?;
            if local < start || local >= end || gate_occ_from[slot] >= edge_count {
                return Err(BuildError::InputInvariant {
                    structure: STRUCTURE,
                });
            }
            let last_in_maneuver = local + 1 == end;
            match (gate_occ_next[slot], last_in_maneuver) {
                (None, true) => {}
                (Some(next), false) if next == local + 1 => {}
                _ => {
                    return Err(BuildError::InputInvariant {
                        structure: STRUCTURE,
                    });
                }
            }
            let expected_boundary = if last_in_maneuver {
                maneuver_exit[man_slot]
            } else {
                let next_slot = usize::try_from(gate_range.start() + local + 1).expect("u32 fits");
                gate_occ_from[next_slot]
            };
            if gate_occ_next_boundary[slot] != expected_boundary {
                return Err(BuildError::InputInvariant {
                    structure: STRUCTURE,
                });
            }
            if let Some(waiting_local) = gate_occ_waiting[slot]
                && waiting_local >= wait_range.len()
            {
                return Err(BuildError::InputInvariant {
                    structure: STRUCTURE,
                });
            }
        }

        for local in 0..wait_range.len() {
            let index = wait_range.start() + local;
            let slot = usize::try_from(index).expect("u32 fits");
            let man_local = waiting_occ_maneuver[slot];
            if man_local >= man_len {
                return Err(BuildError::InputInvariant {
                    structure: STRUCTURE,
                });
            }
            let man_slot = usize::try_from(man_range.start() + man_local).expect("u32 fits");
            let start = maneuver_waiting_occ_start[man_slot];
            let end = start
                .checked_add(maneuver_waiting_occ_count[man_slot])
                .ok_or(BuildError::ArithmeticOverflow {
                    structure: STRUCTURE,
                })?;
            if local < start || local >= end {
                return Err(BuildError::InputInvariant {
                    structure: STRUCTURE,
                });
            }
            let gate_start = maneuver_gate_occ_start[man_slot];
            let gate_end = gate_start
                .checked_add(maneuver_gate_occ_count[man_slot])
                .ok_or(BuildError::ArithmeticOverflow {
                    structure: STRUCTURE,
                })?;
            let entry_gate = waiting_occ_entry_gate[slot];
            let release_gate = waiting_occ_release_gate[slot];
            if entry_gate < gate_start
                || entry_gate >= gate_end
                || release_gate < gate_start
                || release_gate >= gate_end
            {
                return Err(BuildError::InputInvariant {
                    structure: STRUCTURE,
                });
            }
            let entry_slot = usize::try_from(gate_range.start() + entry_gate).expect("u32 fits");
            let release_slot =
                usize::try_from(gate_range.start() + release_gate).expect("u32 fits");
            if waiting_occ_entry_edge[slot] != gate_occ_from[entry_slot]
                || waiting_occ_release_edge[slot] != gate_occ_from[release_slot]
            {
                return Err(BuildError::InputInvariant {
                    structure: STRUCTURE,
                });
            }
        }

        let route_edges = edge_ranges[route_index].slice(edges);
        let mut covered = vec![false; route_edges.len()];
        for local in 0..man_len {
            let slot = usize::try_from(man_range.start() + local).expect("u32 fits");
            let entry = usize::try_from(maneuver_entry[slot]).expect("u32 fits");
            let exit = usize::try_from(maneuver_exit[slot]).expect("u32 fits");
            for covered_edge in covered.iter_mut().take(exit).skip(entry + 1) {
                if *covered_edge {
                    return Err(BuildError::InputInvariant {
                        structure: STRUCTURE,
                    });
                }
                *covered_edge = true;
            }
        }
        for (edge_index, edge) in route_edges.iter().enumerate() {
            if get_optional(edge_junction, edge.raw()).is_some() && !covered[edge_index] {
                return Err(BuildError::InputInvariant {
                    structure: STRUCTURE,
                });
            }
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
    maneuver_ranges: &[RangeU32],
    gate_occ_ranges: &[RangeU32],
    waiting_occ_ranges: &[RangeU32],
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
        let occ_limit = match kind {
            EntityKind::ManeuverPath => maneuver_ranges[route as usize].len(),
            EntityKind::ManeuverGate => gate_occ_ranges[route as usize].len(),
            EntityKind::WaitingZone => waiting_occ_ranges[route as usize].len(),
            EntityKind::LaneEdge => edge_ranges[route as usize].len(),
            _ => {
                return Err(BuildError::InputInvariant {
                    structure: STRUCTURE,
                });
            }
        };
        if occurrence >= occ_limit {
            return Err(BuildError::ReferenceOutOfBounds {
                structure: STRUCTURE,
                ordinal: occurrence,
                limit: occ_limit,
            });
        }
        kinds.push(kind_code);
        ordinals.push(ordinal);
        routes.push(StaticRouteOrdinal::from_raw(route));
        occurrences.push(occurrence);
    }
    Ok((
        kinds.into_boxed_slice(),
        ordinals.into_boxed_slice(),
        routes.into_boxed_slice(),
        occurrences.into_boxed_slice(),
    ))
}

#[cfg(test)]
mod tests {
    use super::{OccurrenceCursor, ensure_unique};
    use crate::{BuildError, RangeU32};

    #[test]
    fn sequence_members_reject_duplicates_but_allow_non_monotonic_order() {
        assert!(ensure_unique(&[2, 0, 1]).is_ok());
        assert!(matches!(
            ensure_unique(&[1, 0, 1]),
            Err(BuildError::InputInvariant { .. })
        ));
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
