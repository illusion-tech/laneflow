//! LFCA 冲突静态语义与派生覆盖索引。

use laneflow_format::{RegistryCheckedFieldValue, RegistryCheckedRowView, ValueCheckedObjectView};
use laneflow_static_contract::{
    ConflictZoneOrdinal, EntityKind, JunctionOrdinal, ManeuverGateOrdinal, ManeuverPathOrdinal,
    ParticipantStreamOrdinal,
};

use crate::builder::{allocate_vec, checked_record_vector, checked_u8, checked_u32};
use crate::traffic::logical_bytes;
use crate::{BuildError, BuildStructure, RangeU32, SharedIdentityIndex, SharedTrafficNetwork};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConflictPathAnchor {
    Gate(ManeuverGateOrdinal),
    EdgeBoundary(u32),
    Interior {
        path_edge_index: u32,
        progress_millimetres: u32,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ConflictPassage {
    conflict_zone: ConflictZoneOrdinal,
    entry: ConflictPathAnchor,
    exit: ConflictPathAnchor,
    admission_gate: ManeuverGateOrdinal,
}

impl ConflictPassage {
    #[must_use]
    pub const fn conflict_zone(self) -> ConflictZoneOrdinal {
        self.conflict_zone
    }

    #[must_use]
    pub const fn entry(self) -> ConflictPathAnchor {
        self.entry
    }

    #[must_use]
    pub const fn exit(self) -> ConflictPathAnchor {
        self.exit
    }

    #[must_use]
    pub const fn admission_gate(self) -> ManeuverGateOrdinal {
        self.admission_gate
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ConflictZoneView<'a> {
    junction: JunctionOrdinal,
    participant_streams: &'a [ParticipantStreamOrdinal],
}

impl<'a> ConflictZoneView<'a> {
    #[must_use]
    pub const fn junction(self) -> JunctionOrdinal {
        self.junction
    }

    #[must_use]
    pub const fn participant_streams(self) -> &'a [ParticipantStreamOrdinal] {
        self.participant_streams
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ParticipantStreamView<'a> {
    junction: JunctionOrdinal,
    maneuver_path: ManeuverPathOrdinal,
    passages: &'a [ConflictPassage],
}

impl<'a> ParticipantStreamView<'a> {
    #[must_use]
    pub const fn junction(self) -> JunctionOrdinal {
        self.junction
    }

    #[must_use]
    pub const fn maneuver_path(self) -> ManeuverPathOrdinal {
        self.maneuver_path
    }

    #[must_use]
    pub const fn passages(self) -> &'a [ConflictPassage] {
        self.passages
    }
}

/// 冲突区、参与者流、owner-local passage 与派生 zone-stream CSR。
pub struct SharedConflictNetwork {
    zone_junctions: Box<[JunctionOrdinal]>,
    junction_zone_ranges: Box<[RangeU32]>,
    junction_zones: Box<[ConflictZoneOrdinal]>,
    zone_stream_ranges: Box<[RangeU32]>,
    zone_streams: Box<[ParticipantStreamOrdinal]>,
    stream_junctions: Box<[JunctionOrdinal]>,
    junction_stream_ranges: Box<[RangeU32]>,
    junction_streams: Box<[ParticipantStreamOrdinal]>,
    stream_paths: Box<[ManeuverPathOrdinal]>,
    path_stream_ranges: Box<[RangeU32]>,
    path_streams: Box<[ParticipantStreamOrdinal]>,
    stream_passage_ranges: Box<[RangeU32]>,
    passages: Box<[ConflictPassage]>,
}

impl SharedConflictNetwork {
    #[must_use]
    pub fn conflict_zone(&self, zone: ConflictZoneOrdinal) -> Option<ConflictZoneView<'_>> {
        Some(ConflictZoneView {
            junction: *self.zone_junctions.get(zone.index())?,
            participant_streams: self
                .zone_stream_ranges
                .get(zone.index())?
                .slice(&self.zone_streams),
        })
    }

    #[must_use]
    pub fn junction_conflict_zones(
        &self,
        junction: JunctionOrdinal,
    ) -> Option<&[ConflictZoneOrdinal]> {
        Some(
            self.junction_zone_ranges
                .get(junction.index())?
                .slice(&self.junction_zones),
        )
    }

    #[must_use]
    pub fn junction_participant_streams(
        &self,
        junction: JunctionOrdinal,
    ) -> Option<&[ParticipantStreamOrdinal]> {
        Some(
            self.junction_stream_ranges
                .get(junction.index())?
                .slice(&self.junction_streams),
        )
    }

    #[must_use]
    pub fn maneuver_path_participant_streams(
        &self,
        path: ManeuverPathOrdinal,
    ) -> Option<&[ParticipantStreamOrdinal]> {
        Some(
            self.path_stream_ranges
                .get(path.index())?
                .slice(&self.path_streams),
        )
    }

    #[must_use]
    pub fn participant_stream(
        &self,
        stream: ParticipantStreamOrdinal,
    ) -> Option<ParticipantStreamView<'_>> {
        Some(ParticipantStreamView {
            junction: *self.stream_junctions.get(stream.index())?,
            maneuver_path: *self.stream_paths.get(stream.index())?,
            passages: self
                .stream_passage_ranges
                .get(stream.index())?
                .slice(&self.passages),
        })
    }

    #[must_use]
    pub fn retained_logical_bytes(&self) -> u64 {
        logical_bytes::<JunctionOrdinal>(self.zone_junctions.len())
            + logical_bytes::<RangeU32>(self.junction_zone_ranges.len())
            + logical_bytes::<ConflictZoneOrdinal>(self.junction_zones.len())
            + logical_bytes::<RangeU32>(self.zone_stream_ranges.len())
            + logical_bytes::<ParticipantStreamOrdinal>(self.zone_streams.len())
            + logical_bytes::<JunctionOrdinal>(self.stream_junctions.len())
            + logical_bytes::<RangeU32>(self.junction_stream_ranges.len())
            + logical_bytes::<ParticipantStreamOrdinal>(self.junction_streams.len())
            + logical_bytes::<ManeuverPathOrdinal>(self.stream_paths.len())
            + logical_bytes::<RangeU32>(self.path_stream_ranges.len())
            + logical_bytes::<ParticipantStreamOrdinal>(self.path_streams.len())
            + logical_bytes::<RangeU32>(self.stream_passage_ranges.len())
            + logical_bytes::<ConflictPassage>(self.passages.len())
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct PathPosition {
    edge_index: u32,
    progress_mm: u32,
}

pub(crate) fn build_conflict(
    view: ValueCheckedObjectView<'_>,
    traffic: &SharedTrafficNetwork,
    identity: &SharedIdentityIndex,
    expected_passage_count: u32,
    mut poll: impl FnMut(u32) -> Result<(), BuildError>,
) -> Result<SharedConflictNetwork, BuildError> {
    let structure = BuildStructure::Conflict;
    let entity_section = view
        .registry_view()
        .section(2)
        .ok_or(BuildError::InputInvariant {
            structure: BuildStructure::CanonicalEntityTable,
        })?;
    let zone_table = entity_section
        .table(20)
        .ok_or(BuildError::InputInvariant { structure })?;
    let stream_table = entity_section
        .table(22)
        .ok_or(BuildError::InputInvariant { structure })?;
    let counts = traffic.entity_counts();
    let zone_count = counts.count(EntityKind::ConflictZone);
    let stream_count = counts.count(EntityKind::ParticipantStream);
    let junction_count = counts.count(EntityKind::Junction);
    let path_count = counts.count(EntityKind::ManeuverPath);

    let mut zone_junctions = allocate_vec(zone_count, structure)?;
    let mut stream_ref_counts = allocate_vec(zone_count, structure)?;
    stream_ref_counts.resize(zone_count as usize, 0_u32);
    for (index, row) in zone_table.rows().enumerate() {
        let ordinal =
            u32::try_from(index).map_err(|_| BuildError::ArithmeticOverflow { structure })?;
        poll(ordinal)?;
        expect_ordinal(row, ordinal, structure)?;
        let junction = checked_u32(row, 3, structure)?;
        check_reference(junction, junction_count, structure)?;
        zone_junctions.push(JunctionOrdinal::from_raw(junction));
    }

    let mut stream_junctions = allocate_vec(stream_count, structure)?;
    let mut stream_paths = allocate_vec(stream_count, structure)?;
    let mut stream_passage_ranges = allocate_vec(stream_count, structure)?;
    let mut passages = allocate_vec(expected_passage_count, structure)?;
    let mut zone_seen_by_stream = allocate_vec(zone_count, BuildStructure::BuilderScratch)?;
    zone_seen_by_stream.resize(zone_count as usize, u32::MAX);
    for (index, row) in stream_table.rows().enumerate() {
        let stream_raw =
            u32::try_from(index).map_err(|_| BuildError::ArithmeticOverflow { structure })?;
        poll(stream_raw)?;
        expect_ordinal(row, stream_raw, structure)?;
        let junction_raw = checked_u32(row, 3, structure)?;
        let path_raw = checked_u32(row, 4, structure)?;
        check_reference(junction_raw, junction_count, structure)?;
        check_reference(path_raw, path_count, structure)?;
        let junction = JunctionOrdinal::from_raw(junction_raw);
        let path = ManeuverPathOrdinal::from_raw(path_raw);
        let path_view =
            traffic
                .maneuvers()
                .maneuver_path(path)
                .ok_or(BuildError::ReferenceOutOfBounds {
                    structure,
                    ordinal: path_raw,
                    limit: path_count,
                })?;
        if traffic.relations().movement_junction(path_view.movement()) != Some(junction) {
            return Err(BuildError::InputInvariant { structure });
        }
        stream_junctions.push(junction);
        stream_paths.push(path);
        let passage_start = u32::try_from(passages.len())
            .map_err(|_| BuildError::ArithmeticOverflow { structure })?;
        let passage_rows = checked_record_vector(row, 5, structure)?;
        if passage_rows.is_empty() {
            return Err(BuildError::InputInvariant { structure });
        }
        let mut previous_key = None;
        for passage_row in passage_rows.rows() {
            let zone_raw = checked_u32(passage_row, 1, structure)?;
            check_reference(zone_raw, zone_count, structure)?;
            let zone = ConflictZoneOrdinal::from_raw(zone_raw);
            if zone_junctions[zone.index()] != junction
                || zone_seen_by_stream[zone.index()] == stream_raw
            {
                return Err(BuildError::InputInvariant { structure });
            }
            zone_seen_by_stream[zone.index()] = stream_raw;
            let (entry, entry_position) = parse_anchor(
                passage_row,
                2,
                3,
                4,
                path,
                path_view.edges(),
                traffic,
                structure,
            )?;
            let (exit, exit_position) = parse_anchor(
                passage_row,
                5,
                6,
                7,
                path,
                path_view.edges(),
                traffic,
                structure,
            )?;
            if entry_position >= exit_position {
                return Err(BuildError::InputInvariant { structure });
            }
            let stable_id = identity
                .stable_id(zone)
                .ok_or(BuildError::InputInvariant { structure })?
                .into_untyped()
                .into_bytes();
            let key = (entry_position, exit_position, stable_id);
            if previous_key.is_some_and(|previous| previous >= key) {
                return Err(BuildError::NonCanonicalOrder {
                    structure,
                    previous: stream_raw,
                    actual: stream_raw,
                });
            }
            previous_key = Some(key);
            let (admission_gate, next_gate_position) = derive_admission_gate(
                path,
                path_view.maneuver_gates(),
                entry_position,
                traffic,
                structure,
            )?;
            if next_gate_position.is_some_and(|next| exit_position > next) {
                return Err(BuildError::InputInvariant { structure });
            }
            for waiting in path_view.waiting_zones() {
                let waiting = traffic
                    .relations()
                    .waiting_zone(*waiting)
                    .ok_or(BuildError::InputInvariant { structure })?;
                let waiting_entry = gate_position(waiting.entry_gate(), path, traffic, structure)?;
                let waiting_exit = gate_position(waiting.release_gate(), path, traffic, structure)?;
                if entry_position < waiting_exit && waiting_entry < exit_position {
                    return Err(BuildError::InputInvariant { structure });
                }
            }
            passages.push(ConflictPassage {
                conflict_zone: zone,
                entry,
                exit,
                admission_gate,
            });
            stream_ref_counts[zone.index()] = stream_ref_counts[zone.index()]
                .checked_add(1)
                .ok_or(BuildError::ArithmeticOverflow { structure })?;
        }
        let passage_end = u32::try_from(passages.len())
            .map_err(|_| BuildError::ArithmeticOverflow { structure })?;
        stream_passage_ranges.push(RangeU32::new(
            passage_start,
            passage_end.saturating_sub(passage_start),
        ));
    }
    if passages.len() != expected_passage_count as usize {
        return Err(BuildError::InputInvariant { structure });
    }
    drop(zone_seen_by_stream);

    let mut zone_stream_ranges = allocate_vec(zone_count, structure)?;
    let mut total = 0_u32;
    for count in &stream_ref_counts {
        if *count < 2 {
            return Err(BuildError::InputInvariant { structure });
        }
        zone_stream_ranges.push(RangeU32::new(total, *count));
        total = total
            .checked_add(*count)
            .ok_or(BuildError::ArithmeticOverflow { structure })?;
    }
    let mut zone_streams = allocate_vec(total, structure)?;
    zone_streams.resize(total as usize, ParticipantStreamOrdinal::from_raw(0));
    let mut cursors: Vec<u32> = zone_stream_ranges
        .iter()
        .map(|range| range.start())
        .collect();
    for (stream_index, range) in stream_passage_ranges.iter().enumerate() {
        let stream = ParticipantStreamOrdinal::from_raw(
            u32::try_from(stream_index).expect("format-bounded stream index fits u32"),
        );
        for passage in range.slice(&passages) {
            let cursor = &mut cursors[passage.conflict_zone.index()];
            zone_streams[*cursor as usize] = stream;
            *cursor += 1;
        }
    }

    let (junction_zone_ranges, junction_zones) = build_owner_members(
        &zone_junctions,
        junction_count,
        ConflictZoneOrdinal::from_raw,
        structure,
    )?;
    let (junction_stream_ranges, junction_streams) = build_owner_members(
        &stream_junctions,
        junction_count,
        ParticipantStreamOrdinal::from_raw,
        structure,
    )?;
    let (path_stream_ranges, path_streams) = build_owner_members(
        &stream_paths,
        path_count,
        ParticipantStreamOrdinal::from_raw,
        structure,
    )?;

    Ok(SharedConflictNetwork {
        zone_junctions: zone_junctions.into_boxed_slice(),
        junction_zone_ranges,
        junction_zones,
        zone_stream_ranges: zone_stream_ranges.into_boxed_slice(),
        zone_streams: zone_streams.into_boxed_slice(),
        stream_junctions: stream_junctions.into_boxed_slice(),
        junction_stream_ranges,
        junction_streams,
        stream_paths: stream_paths.into_boxed_slice(),
        path_stream_ranges,
        path_streams,
        stream_passage_ranges: stream_passage_ranges.into_boxed_slice(),
        passages: passages.into_boxed_slice(),
    })
}

type OwnerMembers<M> = (Box<[RangeU32]>, Box<[M]>);

fn build_owner_members<O, M>(
    owners: &[O],
    owner_count: u32,
    member_from_raw: impl Fn(u32) -> M,
    structure: BuildStructure,
) -> Result<OwnerMembers<M>, BuildError>
where
    O: Copy + IntoOwnerIndex,
    M: Copy,
{
    let mut counts = allocate_vec(owner_count, BuildStructure::BuilderScratch)?;
    counts.resize(owner_count as usize, 0_u32);
    for owner in owners {
        let index = owner.owner_index();
        counts[index] = counts[index]
            .checked_add(1)
            .ok_or(BuildError::ArithmeticOverflow { structure })?;
    }

    let mut ranges = allocate_vec(owner_count, structure)?;
    let mut total = 0_u32;
    for count in &counts {
        ranges.push(RangeU32::new(total, *count));
        total = total
            .checked_add(*count)
            .ok_or(BuildError::ArithmeticOverflow { structure })?;
    }
    debug_assert_eq!(total as usize, owners.len());

    let placeholder = member_from_raw(0);
    let mut members = allocate_vec(total, structure)?;
    members.resize(total as usize, placeholder);
    let mut cursors: Vec<u32> = ranges.iter().map(|range| range.start()).collect();
    for (raw, owner) in owners.iter().enumerate() {
        let owner_index = owner.owner_index();
        let cursor = &mut cursors[owner_index];
        members[*cursor as usize] = member_from_raw(
            u32::try_from(raw).map_err(|_| BuildError::ArithmeticOverflow { structure })?,
        );
        *cursor = cursor
            .checked_add(1)
            .ok_or(BuildError::ArithmeticOverflow { structure })?;
    }
    Ok((ranges.into_boxed_slice(), members.into_boxed_slice()))
}

trait IntoOwnerIndex {
    fn owner_index(self) -> usize;
}

impl IntoOwnerIndex for JunctionOrdinal {
    fn owner_index(self) -> usize {
        self.index()
    }
}

impl IntoOwnerIndex for ManeuverPathOrdinal {
    fn owner_index(self) -> usize {
        self.index()
    }
}

fn expect_ordinal(
    row: RegistryCheckedRowView<'_>,
    expected: u32,
    structure: BuildStructure,
) -> Result<(), BuildError> {
    let actual = checked_u32(row, 1, structure)?;
    if actual != expected {
        return Err(BuildError::UnexpectedOrdinal {
            structure,
            expected,
            actual,
        });
    }
    Ok(())
}

fn check_reference(value: u32, limit: u32, structure: BuildStructure) -> Result<(), BuildError> {
    if value >= limit {
        return Err(BuildError::ReferenceOutOfBounds {
            structure,
            ordinal: value,
            limit,
        });
    }
    Ok(())
}

fn optional_u32(
    row: RegistryCheckedRowView<'_>,
    tag: u16,
    structure: BuildStructure,
) -> Result<Option<u32>, BuildError> {
    let Some(field) = row.field_by_tag(tag) else {
        return Ok(None);
    };
    match field
        .value()
        .map_err(|_| BuildError::InputInvariant { structure })?
    {
        RegistryCheckedFieldValue::U32(value) => Ok(Some(value)),
        _ => Err(BuildError::InputInvariant { structure }),
    }
}

#[allow(clippy::too_many_arguments)]
fn parse_anchor(
    row: RegistryCheckedRowView<'_>,
    kind_tag: u16,
    reference_tag: u16,
    progress_tag: u16,
    path: ManeuverPathOrdinal,
    path_edges: &[laneflow_static_contract::LaneEdgeOrdinal],
    traffic: &SharedTrafficNetwork,
    structure: BuildStructure,
) -> Result<(ConflictPathAnchor, PathPosition), BuildError> {
    let kind = checked_u8(row, kind_tag, structure)?;
    let reference = checked_u32(row, reference_tag, structure)?;
    let progress = optional_u32(row, progress_tag, structure)?;
    match (kind, progress) {
        (0, None) => {
            let gate = ManeuverGateOrdinal::from_raw(reference);
            let position = gate_position(gate, path, traffic, structure)?;
            Ok((ConflictPathAnchor::Gate(gate), position))
        }
        (1, None) if reference <= path_edges.len() as u32 => Ok((
            ConflictPathAnchor::EdgeBoundary(reference),
            PathPosition {
                edge_index: reference,
                progress_mm: 0,
            },
        )),
        (2, Some(progress_mm)) if reference < path_edges.len() as u32 => {
            let edge = path_edges[reference as usize];
            let length = traffic
                .lane_lengths_millimetres()
                .get(edge.index())
                .copied()
                .ok_or(BuildError::InputInvariant { structure })?;
            if progress_mm == 0 || progress_mm >= length {
                return Err(BuildError::InputInvariant { structure });
            }
            Ok((
                ConflictPathAnchor::Interior {
                    path_edge_index: reference,
                    progress_millimetres: progress_mm,
                },
                PathPosition {
                    edge_index: reference,
                    progress_mm,
                },
            ))
        }
        _ => Err(BuildError::InputInvariant { structure }),
    }
}

fn gate_position(
    gate: ManeuverGateOrdinal,
    path: ManeuverPathOrdinal,
    traffic: &SharedTrafficNetwork,
    structure: BuildStructure,
) -> Result<PathPosition, BuildError> {
    let gate = traffic
        .relations()
        .maneuver_gate(gate)
        .filter(|gate| gate.path() == path)
        .ok_or(BuildError::InputInvariant { structure })?;
    Ok(PathPosition {
        edge_index: gate.transition_index().saturating_add(1),
        progress_mm: 0,
    })
}

fn derive_admission_gate(
    path: ManeuverPathOrdinal,
    gates: &[ManeuverGateOrdinal],
    entry: PathPosition,
    traffic: &SharedTrafficNetwork,
    structure: BuildStructure,
) -> Result<(ManeuverGateOrdinal, Option<PathPosition>), BuildError> {
    let mut admission = None;
    let mut next = None;
    for gate in gates {
        let position = gate_position(*gate, path, traffic, structure)?;
        if position <= entry {
            admission = Some(*gate);
        } else {
            next = Some(position);
            break;
        }
    }
    admission
        .map(|gate| (gate, next))
        .ok_or(BuildError::InputInvariant { structure })
}
