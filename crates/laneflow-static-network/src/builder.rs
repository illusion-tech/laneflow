use core::mem::size_of;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use laneflow_format::{
    CheckedCanonicalNetworkInputV1, RegistryCheckedFieldValue, RegistryCheckedOrdinalVectorView,
    RegistryCheckedRecordVectorView, RegistryCheckedRowView, ValueCheckedObjectView,
};
use laneflow_static_contract::{
    CanonicalFrameOrdinal, EntityKind, FacilityBandOrdinal, IDENTITY_ENCODING_VERSION,
    IDENTITY_REGISTRY_REVISION, LaneEdgeOrdinal, ManeuverGateOrdinal, ManeuverPathOrdinal,
    MovementOrdinal, NETWORK_REVISION_DERIVATION_VERSION, StableId128, WaitingZoneOrdinal,
};

use crate::{
    BuildError, BuildStructure, CanonicalNetworkOrigin, CanonicalPoint, EntityCounts,
    LanePoseNetwork, ManeuverTransitionCandidate, PartitionPlanningHints, RangeU32,
    SegmentGeometry, SharedIdentityIndex, SharedManeuverNetwork, SharedNetworkRevision,
    SharedSpatialNetwork, SharedTrafficNetwork, StaticContractVersions,
    identity::{
        IdentityReverseEntry, allocate_forward_identity, kind_index, radix_sort_reverse_identity,
        reverse_entry_bytes, seal_forward_identity,
    },
    spatial::FacilityGeometryEntry,
};

const ENTITY_KIND_COUNT: usize = EntityKind::ALL.len();
const CANCELLATION_POLL_MASK: u32 = 1_023;
type PredecessorIndex = (Box<[RangeU32]>, Box<[LaneEdgeOrdinal]>);
type ManeuverCandidateIndex = (Box<[RangeU32]>, Box<[ManeuverTransitionCandidate]>);

/// Spatial retained-data 构建选择；它不进入 LFCA 或持久化配置档。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SpatialBuildOption {
    Omit,
    RetainAvailable,
}

/// 调用方对一次共享静态路网构建施加的资源上限。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SharedNetworkBuildLimits {
    max_retained_bytes: u64,
    max_scratch_bytes: u64,
}

impl SharedNetworkBuildLimits {
    #[must_use]
    pub const fn new(max_retained_bytes: u64, max_scratch_bytes: u64) -> Self {
        Self {
            max_retained_bytes,
            max_scratch_bytes,
        }
    }

    #[must_use]
    pub const fn max_retained_bytes(self) -> u64 {
        self.max_retained_bytes
    }

    #[must_use]
    pub const fn max_scratch_bytes(self) -> u64 {
        self.max_scratch_bytes
    }
}

/// 一次构建的非持久化调用选项。
#[derive(Clone, Copy, Debug)]
pub struct SharedNetworkBuildOptions<'a> {
    spatial: SpatialBuildOption,
    limits: SharedNetworkBuildLimits,
    cancellation: Option<&'a AtomicBool>,
}

impl<'a> SharedNetworkBuildOptions<'a> {
    #[must_use]
    pub const fn new(spatial: SpatialBuildOption, limits: SharedNetworkBuildLimits) -> Self {
        Self {
            spatial,
            limits,
            cancellation: None,
        }
    }

    #[must_use]
    pub const fn with_cancellation(mut self, cancellation: &'a AtomicBool) -> Self {
        self.cancellation = Some(cancellation);
        self
    }

    #[must_use]
    pub const fn spatial(self) -> SpatialBuildOption {
        self.spatial
    }

    #[must_use]
    pub const fn limits(self) -> SharedNetworkBuildLimits {
        self.limits
    }
}

#[derive(Clone, Copy)]
struct BuildCounts {
    contracts: StaticContractVersions,
    entity_counts: EntityCounts,
    entity_count_total: u32,
    identity_count: u32,
    topology_pair_capacity: u32,
    maneuver_path_edge_count: u32,
    maneuver_path_gate_count: u32,
    maneuver_path_waiting_zone_count: u32,
    maneuver_transition_count: u32,
    spatial_present: bool,
    direction_profile: u8,
    lane_geometry_count: u32,
    lane_point_count: u32,
    lane_segment_count: u32,
    facility_geometry_count: u32,
    facility_point_count: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct TransitionPair {
    predecessor: u32,
    successor: u32,
}

#[derive(Clone, Copy)]
struct CandidateBuildEntry {
    predecessor: u32,
    candidate: ManeuverTransitionCandidate,
}

#[derive(Clone, Copy)]
struct GateTopology {
    maneuver_path: u32,
    transition_index: u32,
    stop_line: u32,
}

#[derive(Clone, Copy)]
struct WaitingZoneTopology {
    maneuver_path: u32,
    entry_transition_index: u32,
    release_transition_index: u32,
}

#[derive(Clone, Copy)]
struct StopLineTopology {
    lane_edge: u32,
}

#[derive(Clone, Copy)]
struct ManeuverPathBuildEntry {
    movement: MovementOrdinal,
    transition_range: RangeU32,
    waiting_zone_range: RangeU32,
}

struct TopologyPlan {
    pairs: Vec<TransitionPair>,
    candidates: Vec<CandidateBuildEntry>,
    maneuver_paths: Vec<ManeuverPathBuildEntry>,
    waiting_zones: Vec<WaitingZoneOrdinal>,
}

impl TopologyPlan {
    fn successor_count(&self) -> Result<u32, BuildError> {
        u32::try_from(self.pairs.len()).map_err(|_| BuildError::ArithmeticOverflow {
            structure: BuildStructure::LaneSuccessors,
        })
    }
}

/// 从受检 LFCA 构建完全拥有、不可变的共享静态路网根。
pub fn build_shared_network_revision(
    input: CheckedCanonicalNetworkInputV1<'_>,
    options: SharedNetworkBuildOptions<'_>,
) -> Result<Arc<SharedNetworkRevision>, BuildError> {
    check_cancelled(options)?;
    let counts = count_and_preflight(input.value_checked_view(), options)?;
    check_scratch_budget(counts, options)?;
    let topology = build_topology_plan(input.value_checked_view(), &counts, options)?;
    check_retained_budget(counts, topology.successor_count()?, options)?;
    check_cancelled(options)?;

    let mut forward_identity = allocate_forward_identity(&counts.entity_counts)?;
    let mut reverse_identity =
        allocate_vec(counts.identity_count, BuildStructure::CanonicalIdentity)?;
    fill_identity(
        input.value_checked_view(),
        &counts,
        &mut forward_identity,
        &mut reverse_identity,
        options,
    )?;
    let reverse_identity = radix_sort_reverse_identity(reverse_identity)?;

    check_cancelled(options)?;
    let traffic = build_traffic(
        input.value_checked_view(),
        &counts,
        &forward_identity,
        topology,
        options,
    )?;
    let identity = SharedIdentityIndex::from_parts(
        seal_forward_identity(forward_identity),
        reverse_identity.into_boxed_slice(),
    );
    let planning_hints = PartitionPlanningHints::from_traffic(&traffic)?;
    let spatial = build_spatial(input.value_checked_view(), &counts, &traffic, options)?;

    check_cancelled(options)?;
    let origin = CanonicalNetworkOrigin::new(
        input.canonical_artifact_digest(),
        input.canonical_artifact_byte_length(),
        input.network_revision(),
        counts.contracts,
    );
    Ok(Arc::new(SharedNetworkRevision {
        origin,
        traffic,
        identity,
        planning_hints,
        spatial,
    }))
}

fn count_and_preflight(
    view: ValueCheckedObjectView<'_>,
    options: SharedNetworkBuildOptions<'_>,
) -> Result<BuildCounts, BuildError> {
    let registry = view.registry_view();
    let contract_row = singleton_row(view, 0, BuildStructure::ContractVersions)?;
    let contracts = StaticContractVersions::new(
        checked_u16(contract_row, 1, BuildStructure::ContractVersions)?,
        checked_u16(contract_row, 2, BuildStructure::ContractVersions)?,
        checked_u16(contract_row, 3, BuildStructure::ContractVersions)?,
        checked_u16(contract_row, 4, BuildStructure::ContractVersions)?,
        checked_u16(contract_row, 5, BuildStructure::ContractVersions)?,
        checked_u16(contract_row, 6, BuildStructure::ContractVersions)?,
    );
    if contracts.identity_encoding_version() != IDENTITY_ENCODING_VERSION
        || contracts.identity_registry_revision() != IDENTITY_REGISTRY_REVISION
        || contracts.network_revision_derivation_version() != NETWORK_REVISION_DERIVATION_VERSION
    {
        return Err(BuildError::ContractMismatch {
            structure: BuildStructure::ContractVersions,
        });
    }

    let entity_section = registry.section(2).ok_or(BuildError::InputInvariant {
        structure: BuildStructure::CanonicalEntityTable,
    })?;
    let mut entity_counts = [0_u32; ENTITY_KIND_COUNT];
    let mut entity_count_total = 0_u32;
    for (index, table) in entity_section.tables().enumerate() {
        let Some(slot) = entity_counts.get_mut(index) else {
            return Err(BuildError::InputInvariant {
                structure: BuildStructure::CanonicalEntityTable,
            });
        };
        *slot = table.row_count();
        entity_count_total = checked_add_count(
            entity_count_total,
            table.row_count(),
            BuildStructure::CanonicalEntityTable,
        )?;
    }
    let entity_counts = EntityCounts::new(entity_counts);

    let identity_count = registry
        .section(1)
        .and_then(|section| section.table(0))
        .map(|table| table.row_count())
        .ok_or(BuildError::InputInvariant {
            structure: BuildStructure::CanonicalIdentity,
        })?;

    let lane_table = entity_section.table(3).ok_or(BuildError::InputInvariant {
        structure: BuildStructure::LaneEdge,
    })?;
    let mut canonical_successor_count = 0_u32;
    for (index, row) in lane_table.rows().enumerate() {
        poll_cancelled(options, u32::try_from(index).unwrap_or(u32::MAX))?;
        canonical_successor_count = canonical_successor_count
            .checked_add(checked_ordinal_vector(row, 5, BuildStructure::LaneSuccessors)?.len())
            .ok_or(BuildError::ArithmeticOverflow {
                structure: BuildStructure::LaneSuccessors,
            })?;
    }

    let maneuver_path_table = entity_section.table(6).ok_or(BuildError::InputInvariant {
        structure: BuildStructure::ManeuverPath,
    })?;
    let mut maneuver_path_edge_count = 0_u32;
    let mut maneuver_path_gate_count = 0_u32;
    let mut maneuver_path_waiting_zone_count = 0_u32;
    let mut maneuver_transition_count = 0_u32;
    for (index, row) in maneuver_path_table.rows().enumerate() {
        poll_cancelled(options, u32::try_from(index).unwrap_or(u32::MAX))?;
        let edges = checked_ordinal_vector(row, 4, BuildStructure::ManeuverPath)?;
        if edges.len() < 2 {
            return Err(BuildError::InputInvariant {
                structure: BuildStructure::ManeuverPath,
            });
        }
        maneuver_path_edge_count = checked_add_count(
            maneuver_path_edge_count,
            edges.len(),
            BuildStructure::ManeuverPath,
        )?;
        maneuver_transition_count = checked_add_count(
            maneuver_transition_count,
            edges
                .len()
                .checked_sub(1)
                .ok_or(BuildError::InputInvariant {
                    structure: BuildStructure::ManeuverPath,
                })?,
            BuildStructure::ManeuverCandidates,
        )?;
        maneuver_path_gate_count = checked_add_count(
            maneuver_path_gate_count,
            checked_ordinal_vector(row, 5, BuildStructure::ManeuverPath)?.len(),
            BuildStructure::ManeuverPath,
        )?;
        maneuver_path_waiting_zone_count = checked_add_count(
            maneuver_path_waiting_zone_count,
            checked_ordinal_vector(row, 6, BuildStructure::ManeuverPath)?.len(),
            BuildStructure::ManeuverPath,
        )?;
    }
    let topology_pair_capacity = checked_add_count(
        canonical_successor_count,
        maneuver_transition_count,
        BuildStructure::LaneSuccessors,
    )?;

    let spatial_section = registry.section(4).ok_or(BuildError::InputInvariant {
        structure: BuildStructure::SpatialPresence,
    })?;
    let presence = spatial_section
        .table(0)
        .and_then(|table| table.row(0))
        .ok_or(BuildError::InputInvariant {
            structure: BuildStructure::SpatialPresence,
        })?;
    let spatial_present = checked_u8(presence, 1, BuildStructure::SpatialPresence)? != 0;
    let direction_profile = checked_u8(presence, 2, BuildStructure::SpatialPresence)?;

    let lane_geometry_table = spatial_section.table(1).ok_or(BuildError::InputInvariant {
        structure: BuildStructure::LaneEdgeGeometry,
    })?;
    let lane_geometry_count = lane_geometry_table.row_count();
    let mut lane_point_count = 0_u32;
    let mut lane_segment_count = 0_u32;
    for (index, row) in lane_geometry_table.rows().enumerate() {
        poll_cancelled(options, u32::try_from(index).unwrap_or(u32::MAX))?;
        lane_point_count = checked_add_count(
            lane_point_count,
            checked_record_vector(row, 4, BuildStructure::LaneEdgeGeometry)?.len(),
            BuildStructure::LaneEdgeGeometry,
        )?;
        lane_segment_count = checked_add_count(
            lane_segment_count,
            checked_record_vector(row, 5, BuildStructure::LaneEdgeGeometry)?.len(),
            BuildStructure::LaneEdgeGeometry,
        )?;
    }

    let facility_geometry_table = spatial_section.table(2).ok_or(BuildError::InputInvariant {
        structure: BuildStructure::FacilityBandGeometry,
    })?;
    let facility_geometry_count = facility_geometry_table.row_count();
    let mut facility_point_count = 0_u32;
    for (index, row) in facility_geometry_table.rows().enumerate() {
        poll_cancelled(options, u32::try_from(index).unwrap_or(u32::MAX))?;
        facility_point_count = checked_add_count(
            facility_point_count,
            checked_record_vector(row, 3, BuildStructure::FacilityBandGeometry)?.len(),
            BuildStructure::FacilityBandGeometry,
        )?;
    }

    let has_spatial_payload = direction_profile != 0
        || entity_counts.count(EntityKind::CanonicalFrame) != 0
        || lane_geometry_count != 0
        || facility_geometry_count != 0;
    if spatial_present != has_spatial_payload {
        return Err(BuildError::SpatialPresenceMismatch);
    }

    let execution_row = singleton_row(view, 5, BuildStructure::ExecutionContract)?;
    if checked_u16(execution_row, 1, BuildStructure::ExecutionContract)?
        != contracts.static_execution_contract_version()
        || checked_u16(execution_row, 2, BuildStructure::ExecutionContract)?
            != contracts.constraint_contract_version()
    {
        return Err(BuildError::ContractMismatch {
            structure: BuildStructure::ExecutionContract,
        });
    }

    Ok(BuildCounts {
        contracts,
        entity_counts,
        entity_count_total,
        identity_count,
        topology_pair_capacity,
        maneuver_path_edge_count,
        maneuver_path_gate_count,
        maneuver_path_waiting_zone_count,
        maneuver_transition_count,
        spatial_present,
        direction_profile,
        lane_geometry_count,
        lane_point_count,
        lane_segment_count,
        facility_geometry_count,
        facility_point_count,
    })
}

fn check_retained_budget(
    counts: BuildCounts,
    runtime_successor_count: u32,
    options: SharedNetworkBuildOptions<'_>,
) -> Result<(), BuildError> {
    let lane_count = counts.entity_counts.count(EntityKind::LaneEdge);
    let maneuver_path_count = counts.entity_counts.count(EntityKind::ManeuverPath);
    let mut retained = u64::try_from(size_of::<SharedNetworkRevision>()).map_err(|_| {
        BuildError::ArithmeticOverflow {
            structure: BuildStructure::RetainedOutput,
        }
    })?;
    retained = add_retained::<StableId128>(retained, counts.entity_count_total)?;
    retained = add_retained_bytes(
        retained,
        counts
            .identity_count
            .checked_mul(u32::try_from(reverse_entry_bytes()).map_err(|_| {
                BuildError::ArithmeticOverflow {
                    structure: BuildStructure::RetainedOutput,
                }
            })?)
            .ok_or(BuildError::ArithmeticOverflow {
                structure: BuildStructure::RetainedOutput,
            })?,
    )?;
    retained = add_retained::<f64>(
        retained,
        lane_count
            .checked_mul(2)
            .ok_or(BuildError::ArithmeticOverflow {
                structure: BuildStructure::RetainedOutput,
            })?,
    )?;
    retained = add_retained::<RangeU32>(
        retained,
        lane_count
            .checked_mul(2)
            .ok_or(BuildError::ArithmeticOverflow {
                structure: BuildStructure::RetainedOutput,
            })?,
    )?;
    retained = add_retained::<LaneEdgeOrdinal>(
        retained,
        runtime_successor_count
            .checked_mul(2)
            .ok_or(BuildError::ArithmeticOverflow {
                structure: BuildStructure::RetainedOutput,
            })?,
    )?;
    retained = add_retained::<u32>(retained, lane_count)?;
    retained = add_retained::<MovementOrdinal>(retained, maneuver_path_count)?;
    retained = add_retained::<RangeU32>(
        retained,
        maneuver_path_count
            .checked_mul(3)
            .ok_or(BuildError::ArithmeticOverflow {
                structure: BuildStructure::RetainedOutput,
            })?,
    )?;
    retained = add_retained::<LaneEdgeOrdinal>(retained, counts.maneuver_path_edge_count)?;
    retained = add_retained::<ManeuverGateOrdinal>(retained, counts.maneuver_path_gate_count)?;
    retained =
        add_retained::<WaitingZoneOrdinal>(retained, counts.maneuver_path_waiting_zone_count)?;
    retained = add_retained::<RangeU32>(retained, lane_count)?;
    retained =
        add_retained::<ManeuverTransitionCandidate>(retained, counts.maneuver_transition_count)?;

    if options.spatial == SpatialBuildOption::RetainAvailable && counts.spatial_present {
        if counts.lane_geometry_count != 0 {
            retained = add_retained::<CanonicalFrameOrdinal>(retained, lane_count)?;
            retained = add_retained::<f32>(retained, lane_count)?;
            retained = add_retained::<RangeU32>(
                retained,
                lane_count
                    .checked_mul(2)
                    .ok_or(BuildError::ArithmeticOverflow {
                        structure: BuildStructure::RetainedOutput,
                    })?,
            )?;
            retained = add_retained::<CanonicalPoint>(retained, counts.lane_point_count)?;
            retained = add_retained::<SegmentGeometry>(retained, counts.lane_segment_count)?;
        }
        retained = add_retained::<FacilityGeometryEntry>(retained, counts.facility_geometry_count)?;
        retained = add_retained::<CanonicalPoint>(retained, counts.facility_point_count)?;
    }

    if retained > options.limits.max_retained_bytes {
        return Err(BuildError::BudgetExceeded {
            structure: BuildStructure::RetainedOutput,
            required: retained,
            limit: options.limits.max_retained_bytes,
        });
    }

    Ok(())
}

fn check_scratch_budget(
    counts: BuildCounts,
    options: SharedNetworkBuildOptions<'_>,
) -> Result<(), BuildError> {
    let lane_count = counts.entity_counts.count(EntityKind::LaneEdge);
    let topology_pairs = structure_bytes::<TransitionPair>(
        counts.topology_pair_capacity,
        BuildStructure::BuilderScratch,
    )?;
    let candidate_entries = structure_bytes::<CandidateBuildEntry>(
        counts.maneuver_transition_count,
        BuildStructure::BuilderScratch,
    )?;
    let maneuver_path_entries = structure_bytes::<ManeuverPathBuildEntry>(
        counts.entity_counts.count(EntityKind::ManeuverPath),
        BuildStructure::BuilderScratch,
    )?;
    let waiting_zone_entries = structure_bytes::<WaitingZoneOrdinal>(
        counts.maneuver_path_waiting_zone_count,
        BuildStructure::BuilderScratch,
    )?;
    let topology_base = topology_pairs
        .checked_add(candidate_entries)
        .and_then(|bytes| bytes.checked_add(maneuver_path_entries))
        .and_then(|bytes| bytes.checked_add(waiting_zone_entries))
        .ok_or(BuildError::ArithmeticOverflow {
            structure: BuildStructure::BuilderScratch,
        })?;
    let gate_lookup = structure_bytes::<GateTopology>(
        counts.entity_counts.count(EntityKind::ManeuverGate),
        BuildStructure::BuilderScratch,
    )?;
    let waiting_zone_lookup = structure_bytes::<WaitingZoneTopology>(
        counts.entity_counts.count(EntityKind::WaitingZone),
        BuildStructure::BuilderScratch,
    )?;
    let stop_line_lookup = structure_bytes::<StopLineTopology>(
        counts.entity_counts.count(EntityKind::StopLine),
        BuildStructure::BuilderScratch,
    )?;
    let maneuver_lookup = gate_lookup
        .checked_add(waiting_zone_lookup)
        .and_then(|bytes| bytes.checked_add(stop_line_lookup))
        .ok_or(BuildError::ArithmeticOverflow {
            structure: BuildStructure::BuilderScratch,
        })?;
    let radix_pairs = structure_bytes::<TransitionPair>(
        counts.topology_pair_capacity,
        BuildStructure::BuilderScratch,
    )?;
    let dense_cursors = structure_bytes::<u32>(lane_count, BuildStructure::BuilderScratch)?;
    let radix_sort =
        radix_pairs
            .checked_add(dense_cursors)
            .ok_or(BuildError::ArithmeticOverflow {
                structure: BuildStructure::BuilderScratch,
            })?;
    let identity_scratch = structure_bytes::<IdentityReverseEntry>(
        counts.identity_count,
        BuildStructure::BuilderScratch,
    )?;
    let predecessor_scratch = structure_bytes::<u32>(lane_count, BuildStructure::BuilderScratch)?;
    let scratch = topology_base
        .checked_add(
            maneuver_lookup
                .max(radix_sort)
                .max(identity_scratch)
                .max(predecessor_scratch),
        )
        .ok_or(BuildError::ArithmeticOverflow {
            structure: BuildStructure::BuilderScratch,
        })?;
    if scratch > options.limits.max_scratch_bytes {
        return Err(BuildError::BudgetExceeded {
            structure: BuildStructure::BuilderScratch,
            required: scratch,
            limit: options.limits.max_scratch_bytes,
        });
    }
    Ok(())
}

fn build_topology_plan(
    view: ValueCheckedObjectView<'_>,
    counts: &BuildCounts,
    options: SharedNetworkBuildOptions<'_>,
) -> Result<TopologyPlan, BuildError> {
    let entity_section = view
        .registry_view()
        .section(2)
        .ok_or(BuildError::InputInvariant {
            structure: BuildStructure::CanonicalEntityTable,
        })?;
    let lane_count = counts.entity_counts.count(EntityKind::LaneEdge);
    let maneuver_path_count = counts.entity_counts.count(EntityKind::ManeuverPath);
    let maneuver_gate_count = counts.entity_counts.count(EntityKind::ManeuverGate);
    let waiting_zone_count = counts.entity_counts.count(EntityKind::WaitingZone);
    let stop_line_count = counts.entity_counts.count(EntityKind::StopLine);
    if counts.maneuver_path_gate_count != maneuver_gate_count
        || counts.maneuver_path_waiting_zone_count != waiting_zone_count
    {
        return Err(BuildError::InputInvariant {
            structure: BuildStructure::ManeuverPath,
        });
    }

    let mut pairs = allocate_vec(
        counts.topology_pair_capacity,
        BuildStructure::BuilderScratch,
    )?;
    let lane_table = entity_section.table(3).ok_or(BuildError::InputInvariant {
        structure: BuildStructure::LaneEdge,
    })?;
    for (index, row) in lane_table.rows().enumerate() {
        let predecessor = u32::try_from(index).map_err(|_| BuildError::ArithmeticOverflow {
            structure: BuildStructure::LaneSuccessors,
        })?;
        poll_cancelled(options, predecessor)?;
        let actual = checked_u32(row, 1, BuildStructure::LaneEdge)?;
        if actual != predecessor {
            return Err(BuildError::UnexpectedOrdinal {
                structure: BuildStructure::LaneEdge,
                expected: predecessor,
                actual,
            });
        }
        let successors = checked_ordinal_vector(row, 5, BuildStructure::LaneSuccessors)?;
        let mut previous = None;
        for vector_index in 0..successors.len() {
            let successor = successors
                .get(vector_index)
                .ok_or(BuildError::InputInvariant {
                    structure: BuildStructure::LaneSuccessors,
                })?;
            if successor >= lane_count {
                return Err(BuildError::ReferenceOutOfBounds {
                    structure: BuildStructure::LaneSuccessors,
                    ordinal: successor,
                    limit: lane_count,
                });
            }
            if let Some(previous) = previous
                && successor <= previous
            {
                return Err(BuildError::NonCanonicalOrder {
                    structure: BuildStructure::LaneSuccessors,
                    previous,
                    actual: successor,
                });
            }
            previous = Some(successor);
            pairs.push(TransitionPair {
                predecessor,
                successor,
            });
        }
    }

    let stop_line_table = entity_section.table(9).ok_or(BuildError::InputInvariant {
        structure: BuildStructure::ManeuverCandidates,
    })?;
    let mut stop_line_topology = allocate_vec(stop_line_count, BuildStructure::BuilderScratch)?;
    for (index, row) in stop_line_table.rows().enumerate() {
        let expected = u32::try_from(index).map_err(|_| BuildError::ArithmeticOverflow {
            structure: BuildStructure::ManeuverCandidates,
        })?;
        let actual = checked_u32(row, 1, BuildStructure::ManeuverCandidates)?;
        if actual != expected {
            return Err(BuildError::UnexpectedOrdinal {
                structure: BuildStructure::ManeuverCandidates,
                expected,
                actual,
            });
        }
        let lane_edge = checked_u32(row, 3, BuildStructure::ManeuverCandidates)?;
        if lane_edge >= lane_count {
            return Err(BuildError::ReferenceOutOfBounds {
                structure: BuildStructure::ManeuverCandidates,
                ordinal: lane_edge,
                limit: lane_count,
            });
        }
        stop_line_topology.push(StopLineTopology { lane_edge });
    }

    let gate_table = entity_section.table(7).ok_or(BuildError::InputInvariant {
        structure: BuildStructure::ManeuverCandidates,
    })?;
    let mut gate_topology = allocate_vec(maneuver_gate_count, BuildStructure::BuilderScratch)?;
    for (index, row) in gate_table.rows().enumerate() {
        let expected = u32::try_from(index).map_err(|_| BuildError::ArithmeticOverflow {
            structure: BuildStructure::ManeuverCandidates,
        })?;
        let actual = checked_u32(row, 1, BuildStructure::ManeuverCandidates)?;
        if actual != expected {
            return Err(BuildError::UnexpectedOrdinal {
                structure: BuildStructure::ManeuverCandidates,
                expected,
                actual,
            });
        }
        let maneuver_path = checked_u32(row, 3, BuildStructure::ManeuverCandidates)?;
        if maneuver_path >= maneuver_path_count {
            return Err(BuildError::ReferenceOutOfBounds {
                structure: BuildStructure::ManeuverCandidates,
                ordinal: maneuver_path,
                limit: maneuver_path_count,
            });
        }
        let stop_line = checked_u32(row, 5, BuildStructure::ManeuverCandidates)?;
        if stop_line >= stop_line_count {
            return Err(BuildError::ReferenceOutOfBounds {
                structure: BuildStructure::ManeuverCandidates,
                ordinal: stop_line,
                limit: stop_line_count,
            });
        }
        gate_topology.push(GateTopology {
            maneuver_path,
            transition_index: checked_u32(row, 4, BuildStructure::ManeuverCandidates)?,
            stop_line,
        });
    }

    let waiting_zone_table = entity_section.table(8).ok_or(BuildError::InputInvariant {
        structure: BuildStructure::ManeuverPath,
    })?;
    let mut waiting_zone_topology =
        allocate_vec(waiting_zone_count, BuildStructure::BuilderScratch)?;
    for (index, row) in waiting_zone_table.rows().enumerate() {
        let expected = u32::try_from(index).map_err(|_| BuildError::ArithmeticOverflow {
            structure: BuildStructure::ManeuverPath,
        })?;
        let actual = checked_u32(row, 1, BuildStructure::ManeuverPath)?;
        if actual != expected {
            return Err(BuildError::UnexpectedOrdinal {
                structure: BuildStructure::ManeuverPath,
                expected,
                actual,
            });
        }
        let maneuver_path = checked_u32(row, 3, BuildStructure::ManeuverPath)?;
        if maneuver_path >= maneuver_path_count {
            return Err(BuildError::ReferenceOutOfBounds {
                structure: BuildStructure::ManeuverPath,
                ordinal: maneuver_path,
                limit: maneuver_path_count,
            });
        }
        let entry_gate = checked_u32(row, 4, BuildStructure::ManeuverPath)?;
        let release_gate = checked_u32(row, 5, BuildStructure::ManeuverPath)?;
        let entry = *gate_topology
            .get(
                usize::try_from(entry_gate).map_err(|_| BuildError::ReferenceOutOfBounds {
                    structure: BuildStructure::ManeuverPath,
                    ordinal: entry_gate,
                    limit: maneuver_gate_count,
                })?,
            )
            .ok_or(BuildError::ReferenceOutOfBounds {
                structure: BuildStructure::ManeuverPath,
                ordinal: entry_gate,
                limit: maneuver_gate_count,
            })?;
        let release = *gate_topology
            .get(
                usize::try_from(release_gate).map_err(|_| BuildError::ReferenceOutOfBounds {
                    structure: BuildStructure::ManeuverPath,
                    ordinal: release_gate,
                    limit: maneuver_gate_count,
                })?,
            )
            .ok_or(BuildError::ReferenceOutOfBounds {
                structure: BuildStructure::ManeuverPath,
                ordinal: release_gate,
                limit: maneuver_gate_count,
            })?;
        if entry.maneuver_path != maneuver_path
            || release.maneuver_path != maneuver_path
            || entry.transition_index >= release.transition_index
        {
            return Err(BuildError::InputInvariant {
                structure: BuildStructure::ManeuverPath,
            });
        }
        waiting_zone_topology.push(WaitingZoneTopology {
            maneuver_path,
            entry_transition_index: entry.transition_index,
            release_transition_index: release.transition_index,
        });
    }

    let path_table = entity_section.table(6).ok_or(BuildError::InputInvariant {
        structure: BuildStructure::ManeuverPath,
    })?;
    let mut candidates = allocate_vec(
        counts.maneuver_transition_count,
        BuildStructure::BuilderScratch,
    )?;
    let mut maneuver_paths = allocate_vec(maneuver_path_count, BuildStructure::BuilderScratch)?;
    let mut path_waiting_zones = allocate_vec(
        counts.maneuver_path_waiting_zone_count,
        BuildStructure::BuilderScratch,
    )?;
    let movement_count = counts.entity_counts.count(EntityKind::Movement);
    for (index, row) in path_table.rows().enumerate() {
        let path = u32::try_from(index).map_err(|_| BuildError::ArithmeticOverflow {
            structure: BuildStructure::ManeuverPath,
        })?;
        poll_cancelled(options, path)?;
        let actual = checked_u32(row, 1, BuildStructure::ManeuverPath)?;
        if actual != path {
            return Err(BuildError::UnexpectedOrdinal {
                structure: BuildStructure::ManeuverPath,
                expected: path,
                actual,
            });
        }
        let movement = checked_u32(row, 3, BuildStructure::ManeuverPath)?;
        if movement >= movement_count {
            return Err(BuildError::ReferenceOutOfBounds {
                structure: BuildStructure::ManeuverPath,
                ordinal: movement,
                limit: movement_count,
            });
        }

        let edges = checked_ordinal_vector(row, 4, BuildStructure::ManeuverPath)?;
        let transition_count = edges
            .len()
            .checked_sub(1)
            .ok_or(BuildError::InputInvariant {
                structure: BuildStructure::ManeuverPath,
            })?;
        for edge_index in 0..edges.len() {
            let edge = edges.get(edge_index).ok_or(BuildError::InputInvariant {
                structure: BuildStructure::ManeuverPath,
            })?;
            if edge >= lane_count {
                return Err(BuildError::ReferenceOutOfBounds {
                    structure: BuildStructure::ManeuverPath,
                    ordinal: edge,
                    limit: lane_count,
                });
            }
        }

        let gates = checked_ordinal_vector(row, 5, BuildStructure::ManeuverPath)?;
        let mut previous_gate_transition = None;
        for gate_index in 0..gates.len() {
            let gate = gates.get(gate_index).ok_or(BuildError::InputInvariant {
                structure: BuildStructure::ManeuverCandidates,
            })?;
            let topology = *gate_topology
                .get(
                    usize::try_from(gate).map_err(|_| BuildError::ReferenceOutOfBounds {
                        structure: BuildStructure::ManeuverCandidates,
                        ordinal: gate,
                        limit: maneuver_gate_count,
                    })?,
                )
                .ok_or(BuildError::ReferenceOutOfBounds {
                    structure: BuildStructure::ManeuverCandidates,
                    ordinal: gate,
                    limit: maneuver_gate_count,
                })?;
            if topology.maneuver_path != path || topology.transition_index >= transition_count {
                return Err(BuildError::InputInvariant {
                    structure: BuildStructure::ManeuverCandidates,
                });
            }
            let expected_lane_edge =
                edges
                    .get(topology.transition_index)
                    .ok_or(BuildError::InputInvariant {
                        structure: BuildStructure::ManeuverCandidates,
                    })?;
            validate_gate_stop_line_edge(
                topology,
                &stop_line_topology,
                expected_lane_edge,
                stop_line_count,
            )?;
            if let Some(previous) = previous_gate_transition
                && topology.transition_index <= previous
            {
                return Err(BuildError::NonCanonicalOrder {
                    structure: BuildStructure::ManeuverCandidates,
                    previous,
                    actual: topology.transition_index,
                });
            }
            previous_gate_transition = Some(topology.transition_index);
        }

        let waiting_zones = checked_ordinal_vector(row, 6, BuildStructure::ManeuverPath)?;
        let waiting_zone_start = u32::try_from(path_waiting_zones.len()).map_err(|_| {
            BuildError::ArithmeticOverflow {
                structure: BuildStructure::ManeuverPath,
            }
        })?;
        let mut previous_waiting_transitions = None;
        for waiting_index in 0..waiting_zones.len() {
            let waiting_zone =
                waiting_zones
                    .get(waiting_index)
                    .ok_or(BuildError::InputInvariant {
                        structure: BuildStructure::ManeuverPath,
                    })?;
            if waiting_zone >= waiting_zone_count {
                return Err(BuildError::ReferenceOutOfBounds {
                    structure: BuildStructure::ManeuverPath,
                    ordinal: waiting_zone,
                    limit: waiting_zone_count,
                });
            }
            let topology = waiting_zone_topology[usize::try_from(waiting_zone)
                .expect("format-bounded waiting-zone ordinal fits usize")];
            if topology.maneuver_path != path {
                return Err(BuildError::InputInvariant {
                    structure: BuildStructure::ManeuverPath,
                });
            }
            let transitions = (
                topology.entry_transition_index,
                topology.release_transition_index,
            );
            if previous_waiting_transitions.is_some_and(|previous| transitions <= previous) {
                return Err(BuildError::InputInvariant {
                    structure: BuildStructure::ManeuverPath,
                });
            }
            previous_waiting_transitions = Some(transitions);
            path_waiting_zones.push(WaitingZoneOrdinal::from_raw(waiting_zone));
        }

        let transition_start =
            u32::try_from(candidates.len()).map_err(|_| BuildError::ArithmeticOverflow {
                structure: BuildStructure::ManeuverCandidates,
            })?;
        let mut gate_index = 0_u32;
        for transition_index in 0..transition_count {
            let predecessor = edges
                .get(transition_index)
                .ok_or(BuildError::InputInvariant {
                    structure: BuildStructure::ManeuverPath,
                })?;
            let successor = edges
                .get(transition_index + 1)
                .ok_or(BuildError::InputInvariant {
                    structure: BuildStructure::ManeuverPath,
                })?;
            let maneuver_gate = if gate_index < gates.len() {
                let gate = gates.get(gate_index).ok_or(BuildError::InputInvariant {
                    structure: BuildStructure::ManeuverCandidates,
                })?;
                let topology = gate_topology
                    [usize::try_from(gate).expect("format-bounded gate ordinal fits usize")];
                if topology.transition_index == transition_index {
                    gate_index += 1;
                    Some(ManeuverGateOrdinal::from_raw(gate))
                } else {
                    None
                }
            } else {
                None
            };
            pairs.push(TransitionPair {
                predecessor,
                successor,
            });
            candidates.push(CandidateBuildEntry {
                predecessor,
                candidate: ManeuverTransitionCandidate::new(
                    LaneEdgeOrdinal::from_raw(successor),
                    ManeuverPathOrdinal::from_raw(path),
                    transition_index,
                    maneuver_gate,
                ),
            });
        }
        if gate_index != gates.len() {
            return Err(BuildError::InputInvariant {
                structure: BuildStructure::ManeuverCandidates,
            });
        }
        maneuver_paths.push(ManeuverPathBuildEntry {
            movement: MovementOrdinal::from_raw(movement),
            transition_range: RangeU32::new(transition_start, transition_count),
            waiting_zone_range: RangeU32::new(waiting_zone_start, waiting_zones.len()),
        });
    }

    drop(gate_topology);
    drop(waiting_zone_topology);
    drop(stop_line_topology);
    let pairs = radix_sort_transition_pairs(pairs, lane_count, options)?;
    Ok(TopologyPlan {
        pairs,
        candidates,
        maneuver_paths,
        waiting_zones: path_waiting_zones,
    })
}

fn validate_gate_stop_line_edge(
    gate: GateTopology,
    stop_lines: &[StopLineTopology],
    expected_lane_edge: u32,
    stop_line_count: u32,
) -> Result<(), BuildError> {
    let stop_line = stop_lines
        .get(
            usize::try_from(gate.stop_line).map_err(|_| BuildError::ReferenceOutOfBounds {
                structure: BuildStructure::ManeuverCandidates,
                ordinal: gate.stop_line,
                limit: stop_line_count,
            })?,
        )
        .ok_or(BuildError::ReferenceOutOfBounds {
            structure: BuildStructure::ManeuverCandidates,
            ordinal: gate.stop_line,
            limit: stop_line_count,
        })?;
    if stop_line.lane_edge != expected_lane_edge {
        return Err(BuildError::InputInvariant {
            structure: BuildStructure::ManeuverCandidates,
        });
    }
    Ok(())
}

fn radix_sort_transition_pairs(
    mut pairs: Vec<TransitionPair>,
    lane_count: u32,
    options: SharedNetworkBuildOptions<'_>,
) -> Result<Vec<TransitionPair>, BuildError> {
    if pairs.len() < 2 {
        return Ok(pairs);
    }
    let pair_count = u32::try_from(pairs.len()).map_err(|_| BuildError::ArithmeticOverflow {
        structure: BuildStructure::LaneSuccessors,
    })?;
    let mut scratch = allocate_vec(pair_count, BuildStructure::BuilderScratch)?;
    scratch.resize(
        pairs.len(),
        TransitionPair {
            predecessor: 0,
            successor: 0,
        },
    );
    let mut cursors = allocate_vec(lane_count, BuildStructure::BuilderScratch)?;
    cursors.resize(
        usize::try_from(lane_count).expect("u32 lane count fits usize"),
        0_u32,
    );

    stable_count_transition_pairs(
        &pairs,
        &mut scratch,
        &mut cursors,
        |pair| pair.successor,
        options,
    )?;
    cursors.fill(0);
    stable_count_transition_pairs(
        &scratch,
        &mut pairs,
        &mut cursors,
        |pair| pair.predecessor,
        options,
    )?;
    check_cancelled(options)?;
    pairs.dedup();
    Ok(pairs)
}

fn stable_count_transition_pairs(
    input: &[TransitionPair],
    output: &mut [TransitionPair],
    cursors: &mut [u32],
    key: impl Fn(TransitionPair) -> u32,
    options: SharedNetworkBuildOptions<'_>,
) -> Result<(), BuildError> {
    if input.len() != output.len() {
        return Err(BuildError::InputInvariant {
            structure: BuildStructure::LaneSuccessors,
        });
    }
    for (index, pair) in input.iter().copied().enumerate() {
        poll_cancelled(options, u32::try_from(index).unwrap_or(u32::MAX))?;
        let raw = key(pair);
        let count = cursors
            .get_mut(usize::try_from(raw).expect("checked lane ordinal fits usize"))
            .ok_or(BuildError::InputInvariant {
                structure: BuildStructure::LaneSuccessors,
            })?;
        *count = count.checked_add(1).ok_or(BuildError::ArithmeticOverflow {
            structure: BuildStructure::LaneSuccessors,
        })?;
    }

    let mut start = 0_u32;
    for count in cursors.iter_mut() {
        let len = *count;
        *count = start;
        start = start
            .checked_add(len)
            .ok_or(BuildError::ArithmeticOverflow {
                structure: BuildStructure::LaneSuccessors,
            })?;
    }
    if usize::try_from(start).ok() != Some(input.len()) {
        return Err(BuildError::InputInvariant {
            structure: BuildStructure::LaneSuccessors,
        });
    }

    for (index, pair) in input.iter().copied().enumerate() {
        poll_cancelled(options, u32::try_from(index).unwrap_or(u32::MAX))?;
        let raw = key(pair);
        let cursor = cursors
            .get_mut(usize::try_from(raw).expect("checked lane ordinal fits usize"))
            .ok_or(BuildError::InputInvariant {
                structure: BuildStructure::LaneSuccessors,
            })?;
        let slot = output
            .get_mut(usize::try_from(*cursor).expect("u32 cursor fits usize"))
            .ok_or(BuildError::InputInvariant {
                structure: BuildStructure::LaneSuccessors,
            })?;
        *slot = pair;
        *cursor = cursor
            .checked_add(1)
            .ok_or(BuildError::ArithmeticOverflow {
                structure: BuildStructure::LaneSuccessors,
            })?;
    }
    Ok(())
}

fn fill_identity(
    view: ValueCheckedObjectView<'_>,
    counts: &BuildCounts,
    forward: &mut [Vec<StableId128>; ENTITY_KIND_COUNT],
    reverse: &mut Vec<IdentityReverseEntry>,
    options: SharedNetworkBuildOptions<'_>,
) -> Result<(), BuildError> {
    let table = view
        .registry_view()
        .section(1)
        .and_then(|section| section.table(0))
        .ok_or(BuildError::InputInvariant {
            structure: BuildStructure::CanonicalIdentity,
        })?;
    let mut previous_kind = None;
    for (index, row) in table.rows().enumerate() {
        poll_cancelled(options, u32::try_from(index).unwrap_or(u32::MAX))?;
        let entity_kind =
            EntityKind::from_code(checked_u16(row, 1, BuildStructure::CanonicalIdentity)?).ok_or(
                BuildError::InputInvariant {
                    structure: BuildStructure::CanonicalIdentity,
                },
            )?;
        if let Some(previous) = previous_kind
            && entity_kind < previous
        {
            return Err(BuildError::EntityKindOrder {
                previous,
                actual: entity_kind,
            });
        }
        previous_kind = Some(entity_kind);

        let kind_index = kind_index(entity_kind);
        let expected = u32::try_from(forward[kind_index].len()).map_err(|_| {
            BuildError::ArithmeticOverflow {
                structure: BuildStructure::CanonicalIdentity,
            }
        })?;
        let actual = checked_u32(row, 2, BuildStructure::CanonicalIdentity)?;
        if actual != expected {
            return Err(BuildError::UnexpectedOrdinal {
                structure: BuildStructure::CanonicalIdentity,
                expected,
                actual,
            });
        }
        if actual >= counts.entity_counts.count(entity_kind) {
            return Err(BuildError::ReferenceOutOfBounds {
                structure: BuildStructure::CanonicalIdentity,
                ordinal: actual,
                limit: counts.entity_counts.count(entity_kind),
            });
        }
        // StableId 的前像派生由 compiler 拥有；这里闭合声明 ID 与 typed ordinal 的双射。
        let stable_id = checked_stable_id(row, 3, BuildStructure::CanonicalIdentity)?;
        forward[kind_index].push(stable_id);
        reverse.push(IdentityReverseEntry {
            entity_kind,
            stable_id,
            ordinal: actual,
        });
    }

    for entity_kind in EntityKind::ALL {
        let identity_count =
            u32::try_from(forward[kind_index(entity_kind)].len()).map_err(|_| {
                BuildError::ArithmeticOverflow {
                    structure: BuildStructure::CanonicalIdentity,
                }
            })?;
        let entity_count = counts.entity_counts.count(entity_kind);
        if identity_count != entity_count {
            return Err(BuildError::EntityCountMismatch {
                entity_kind,
                identity_count,
                entity_count,
            });
        }
    }
    Ok(())
}

fn build_traffic(
    view: ValueCheckedObjectView<'_>,
    counts: &BuildCounts,
    forward_identity: &[Vec<StableId128>; ENTITY_KIND_COUNT],
    topology: TopologyPlan,
    options: SharedNetworkBuildOptions<'_>,
) -> Result<SharedTrafficNetwork, BuildError> {
    let lane_count = counts.entity_counts.count(EntityKind::LaneEdge);
    let mut lane_lengths = allocate_vec(lane_count, BuildStructure::LaneEdge)?;
    let mut lane_speed_limits = allocate_vec(lane_count, BuildStructure::LaneEdge)?;

    let entity_section = view
        .registry_view()
        .section(2)
        .ok_or(BuildError::InputInvariant {
            structure: BuildStructure::CanonicalEntityTable,
        })?;
    for (table_index, table) in entity_section.tables().enumerate() {
        let entity_kind = *EntityKind::ALL
            .get(table_index)
            .ok_or(BuildError::InputInvariant {
                structure: BuildStructure::CanonicalEntityTable,
            })?;
        for (row_index, row) in table.rows().enumerate() {
            let expected =
                u32::try_from(row_index).map_err(|_| BuildError::ArithmeticOverflow {
                    structure: BuildStructure::CanonicalEntityTable,
                })?;
            poll_cancelled(options, expected)?;
            let actual = checked_u32(row, 1, BuildStructure::CanonicalEntityTable)?;
            if actual != expected {
                return Err(BuildError::UnexpectedOrdinal {
                    structure: BuildStructure::CanonicalEntityTable,
                    expected,
                    actual,
                });
            }
            let stable_id = checked_stable_id(row, 2, BuildStructure::CanonicalEntityTable)?;
            if forward_identity[kind_index(entity_kind)]
                .get(row_index)
                .copied()
                != Some(stable_id)
            {
                return Err(BuildError::StableIdMismatch {
                    entity_kind,
                    ordinal: actual,
                });
            }

            if entity_kind == EntityKind::LaneEdge {
                lane_lengths.push(checked_f64(row, 3, BuildStructure::LaneEdge)?);
                lane_speed_limits.push(checked_f64(row, 4, BuildStructure::LaneEdge)?);
            }
        }
    }

    let TopologyPlan {
        pairs,
        candidates,
        maneuver_paths,
        waiting_zones,
    } = topology;
    let (successor_ranges, successors) = build_successors(lane_count, pairs)?;
    let (predecessor_ranges, predecessors) =
        build_predecessors(lane_count, &successor_ranges, &successors)?;
    let maneuvers =
        build_maneuver_network(counts, maneuver_paths, waiting_zones, candidates, options)?;
    Ok(SharedTrafficNetwork::new(
        counts.entity_counts,
        lane_lengths.into_boxed_slice(),
        lane_speed_limits.into_boxed_slice(),
        successor_ranges.into_boxed_slice(),
        successors.into_boxed_slice(),
        predecessor_ranges,
        predecessors,
        maneuvers,
    ))
}

fn build_successors(
    lane_count: u32,
    pairs: Vec<TransitionPair>,
) -> Result<(Vec<RangeU32>, Vec<LaneEdgeOrdinal>), BuildError> {
    let successor_count =
        u32::try_from(pairs.len()).map_err(|_| BuildError::ArithmeticOverflow {
            structure: BuildStructure::LaneSuccessors,
        })?;
    let mut successor_ranges = allocate_vec(lane_count, BuildStructure::LaneSuccessors)?;
    let mut successors = allocate_vec(successor_count, BuildStructure::LaneSuccessors)?;
    let mut pairs = pairs.into_iter().peekable();
    for predecessor in 0..lane_count {
        let start =
            u32::try_from(successors.len()).map_err(|_| BuildError::ArithmeticOverflow {
                structure: BuildStructure::LaneSuccessors,
            })?;
        while pairs
            .peek()
            .is_some_and(|pair| pair.predecessor == predecessor)
        {
            let pair = pairs.next().expect("peeked transition pair");
            successors.push(LaneEdgeOrdinal::from_raw(pair.successor));
        }
        let end = u32::try_from(successors.len()).map_err(|_| BuildError::ArithmeticOverflow {
            structure: BuildStructure::LaneSuccessors,
        })?;
        successor_ranges.push(RangeU32::new(start, end - start));
    }
    if pairs.next().is_some() {
        return Err(BuildError::InputInvariant {
            structure: BuildStructure::LaneSuccessors,
        });
    }
    Ok((successor_ranges, successors))
}

fn build_maneuver_network(
    counts: &BuildCounts,
    path_entries: Vec<ManeuverPathBuildEntry>,
    path_waiting_zones: Vec<WaitingZoneOrdinal>,
    candidate_entries: Vec<CandidateBuildEntry>,
    options: SharedNetworkBuildOptions<'_>,
) -> Result<SharedManeuverNetwork, BuildError> {
    let path_count = counts.entity_counts.count(EntityKind::ManeuverPath);
    let lane_count = counts.entity_counts.count(EntityKind::LaneEdge);
    if path_entries.len() != usize::try_from(path_count).expect("u32 path count fits usize")
        || path_waiting_zones.len()
            != usize::try_from(counts.maneuver_path_waiting_zone_count)
                .expect("u32 waiting-zone count fits usize")
        || candidate_entries.len()
            != usize::try_from(counts.maneuver_transition_count)
                .expect("u32 transition count fits usize")
    {
        return Err(BuildError::InputInvariant {
            structure: BuildStructure::ManeuverPath,
        });
    }
    let mut movements = allocate_vec(path_count, BuildStructure::ManeuverPath)?;
    let mut edge_ranges = allocate_vec(path_count, BuildStructure::ManeuverPath)?;
    let mut edges = allocate_vec(
        counts.maneuver_path_edge_count,
        BuildStructure::ManeuverPath,
    )?;
    let mut gate_ranges = allocate_vec(path_count, BuildStructure::ManeuverPath)?;
    let mut gates = allocate_vec(
        counts.maneuver_path_gate_count,
        BuildStructure::ManeuverPath,
    )?;
    let mut waiting_ranges = allocate_vec(path_count, BuildStructure::ManeuverPath)?;

    for (path_index, path) in path_entries.iter().copied().enumerate() {
        poll_cancelled(options, u32::try_from(path_index).unwrap_or(u32::MAX))?;
        movements.push(path.movement);

        let transition_entries = path.transition_range.slice(&candidate_entries);
        let first = transition_entries
            .first()
            .ok_or(BuildError::InputInvariant {
                structure: BuildStructure::ManeuverPath,
            })?;
        let expected_path =
            u32::try_from(path_index).map_err(|_| BuildError::ArithmeticOverflow {
                structure: BuildStructure::ManeuverPath,
            })?;
        let edge_start =
            u32::try_from(edges.len()).map_err(|_| BuildError::ArithmeticOverflow {
                structure: BuildStructure::ManeuverPath,
            })?;
        edges.push(LaneEdgeOrdinal::from_raw(first.predecessor));
        let mut previous_successor = None;
        for (transition_index, entry) in transition_entries.iter().copied().enumerate() {
            if entry.candidate.maneuver_path().raw() != expected_path
                || entry.candidate.transition_index()
                    != u32::try_from(transition_index).map_err(|_| {
                        BuildError::ArithmeticOverflow {
                            structure: BuildStructure::ManeuverCandidates,
                        }
                    })?
                || previous_successor.is_some_and(|edge| edge != entry.predecessor)
            {
                return Err(BuildError::InputInvariant {
                    structure: BuildStructure::ManeuverCandidates,
                });
            }
            let successor = entry.candidate.successor();
            edges.push(successor);
            previous_successor = Some(successor.raw());
        }
        edge_ranges.push(RangeU32::new(
            edge_start,
            path.transition_range
                .len()
                .checked_add(1)
                .ok_or(BuildError::ArithmeticOverflow {
                    structure: BuildStructure::ManeuverPath,
                })?,
        ));

        let gate_start =
            u32::try_from(gates.len()).map_err(|_| BuildError::ArithmeticOverflow {
                structure: BuildStructure::ManeuverPath,
            })?;
        for entry in transition_entries {
            if let Some(gate) = entry.candidate.maneuver_gate() {
                gates.push(gate);
            }
        }
        let gate_end = u32::try_from(gates.len()).map_err(|_| BuildError::ArithmeticOverflow {
            structure: BuildStructure::ManeuverPath,
        })?;
        gate_ranges.push(RangeU32::new(gate_start, gate_end - gate_start));

        let _ = path.waiting_zone_range.slice(&path_waiting_zones);
        waiting_ranges.push(path.waiting_zone_range);
    }

    if edges.len()
        != usize::try_from(counts.maneuver_path_edge_count)
            .expect("u32 maneuver path edge count fits usize")
        || gates.len()
            != usize::try_from(counts.maneuver_path_gate_count)
                .expect("u32 maneuver path gate count fits usize")
    {
        return Err(BuildError::InputInvariant {
            structure: BuildStructure::ManeuverPath,
        });
    }

    let (candidate_ranges, candidates) =
        build_maneuver_candidates(lane_count, candidate_entries, options)?;

    Ok(SharedManeuverNetwork::new(
        movements.into_boxed_slice(),
        edge_ranges.into_boxed_slice(),
        edges.into_boxed_slice(),
        gate_ranges.into_boxed_slice(),
        gates.into_boxed_slice(),
        waiting_ranges.into_boxed_slice(),
        path_waiting_zones.into_boxed_slice(),
        candidate_ranges,
        candidates,
    ))
}

fn build_maneuver_candidates(
    lane_count: u32,
    entries: Vec<CandidateBuildEntry>,
    options: SharedNetworkBuildOptions<'_>,
) -> Result<ManeuverCandidateIndex, BuildError> {
    let candidate_count =
        u32::try_from(entries.len()).map_err(|_| BuildError::ArithmeticOverflow {
            structure: BuildStructure::ManeuverCandidates,
        })?;
    let mut cursors = allocate_vec(lane_count, BuildStructure::BuilderScratch)?;
    cursors.resize(
        usize::try_from(lane_count).expect("u32 lane count fits usize"),
        0_u32,
    );
    for (index, entry) in entries.iter().copied().enumerate() {
        poll_cancelled(options, u32::try_from(index).unwrap_or(u32::MAX))?;
        let count = cursors
            .get_mut(usize::try_from(entry.predecessor).expect("checked lane ordinal fits usize"))
            .ok_or(BuildError::InputInvariant {
                structure: BuildStructure::ManeuverCandidates,
            })?;
        *count = count.checked_add(1).ok_or(BuildError::ArithmeticOverflow {
            structure: BuildStructure::ManeuverCandidates,
        })?;
    }

    let mut ranges = allocate_vec(lane_count, BuildStructure::ManeuverCandidates)?;
    let mut start = 0_u32;
    for count in &mut cursors {
        ranges.push(RangeU32::new(start, *count));
        let next = start
            .checked_add(*count)
            .ok_or(BuildError::ArithmeticOverflow {
                structure: BuildStructure::ManeuverCandidates,
            })?;
        *count = start;
        start = next;
    }
    if start != candidate_count {
        return Err(BuildError::InputInvariant {
            structure: BuildStructure::ManeuverCandidates,
        });
    }

    let mut candidates = allocate_vec(candidate_count, BuildStructure::ManeuverCandidates)?;
    candidates.resize(
        entries.len(),
        ManeuverTransitionCandidate::new(
            LaneEdgeOrdinal::from_raw(0),
            ManeuverPathOrdinal::from_raw(0),
            0,
            None,
        ),
    );
    for (index, entry) in entries.into_iter().enumerate() {
        poll_cancelled(options, u32::try_from(index).unwrap_or(u32::MAX))?;
        let cursor = cursors
            .get_mut(usize::try_from(entry.predecessor).expect("checked lane ordinal fits usize"))
            .ok_or(BuildError::InputInvariant {
                structure: BuildStructure::ManeuverCandidates,
            })?;
        let slot = candidates
            .get_mut(usize::try_from(*cursor).expect("u32 cursor fits usize"))
            .ok_or(BuildError::InputInvariant {
                structure: BuildStructure::ManeuverCandidates,
            })?;
        *slot = entry.candidate;
        *cursor = cursor
            .checked_add(1)
            .ok_or(BuildError::ArithmeticOverflow {
                structure: BuildStructure::ManeuverCandidates,
            })?;
    }
    Ok((ranges.into_boxed_slice(), candidates.into_boxed_slice()))
}

fn build_predecessors(
    lane_count: u32,
    successor_ranges: &[RangeU32],
    successors: &[LaneEdgeOrdinal],
) -> Result<PredecessorIndex, BuildError> {
    let mut cursors = allocate_vec(lane_count, BuildStructure::BuilderScratch)?;
    cursors.resize(
        usize::try_from(lane_count).expect("u32 lane count fits usize"),
        0_u32,
    );
    for successor in successors {
        let count = cursors
            .get_mut(successor.index())
            .ok_or(BuildError::InputInvariant {
                structure: BuildStructure::LanePredecessors,
            })?;
        *count = count.checked_add(1).ok_or(BuildError::ArithmeticOverflow {
            structure: BuildStructure::LanePredecessors,
        })?;
    }

    let mut predecessor_ranges = allocate_vec(lane_count, BuildStructure::LanePredecessors)?;
    let mut start = 0_u32;
    for count in &mut cursors {
        predecessor_ranges.push(RangeU32::new(start, *count));
        let next = start
            .checked_add(*count)
            .ok_or(BuildError::ArithmeticOverflow {
                structure: BuildStructure::LanePredecessors,
            })?;
        *count = start;
        start = next;
    }
    if usize::try_from(start).ok() != Some(successors.len()) {
        return Err(BuildError::InputInvariant {
            structure: BuildStructure::LanePredecessors,
        });
    }

    let mut predecessors = allocate_vec(start, BuildStructure::LanePredecessors)?;
    predecessors.resize(
        usize::try_from(start).expect("u32 predecessor count fits usize"),
        LaneEdgeOrdinal::from_raw(0),
    );
    for (source_raw, range) in successor_ranges.iter().copied().enumerate() {
        let source = LaneEdgeOrdinal::try_from_usize(source_raw).map_err(|_| {
            BuildError::ArithmeticOverflow {
                structure: BuildStructure::LanePredecessors,
            }
        })?;
        for target in range.slice(successors) {
            let cursor = cursors
                .get_mut(target.index())
                .ok_or(BuildError::InputInvariant {
                    structure: BuildStructure::LanePredecessors,
                })?;
            let slot = predecessors
                .get_mut(usize::try_from(*cursor).expect("u32 cursor fits usize"))
                .ok_or(BuildError::InputInvariant {
                    structure: BuildStructure::LanePredecessors,
                })?;
            *slot = source;
            *cursor = cursor
                .checked_add(1)
                .ok_or(BuildError::ArithmeticOverflow {
                    structure: BuildStructure::LanePredecessors,
                })?;
        }
    }

    Ok((
        predecessor_ranges.into_boxed_slice(),
        predecessors.into_boxed_slice(),
    ))
}

fn build_spatial(
    view: ValueCheckedObjectView<'_>,
    counts: &BuildCounts,
    traffic: &SharedTrafficNetwork,
    options: SharedNetworkBuildOptions<'_>,
) -> Result<Option<SharedSpatialNetwork>, BuildError> {
    let retain = options.spatial == SpatialBuildOption::RetainAvailable;
    let lane_count = counts.entity_counts.count(EntityKind::LaneEdge);
    let frame_count = counts.entity_counts.count(EntityKind::CanonicalFrame);
    let facility_count = counts.entity_counts.count(EntityKind::FacilityBand);
    if counts.lane_geometry_count != 0 && counts.lane_geometry_count != lane_count {
        return Err(BuildError::SpatialCoverageMismatch {
            lane_edges: lane_count,
            geometries: counts.lane_geometry_count,
        });
    }

    let mut lane_frames = allocate_if_retained(
        retain,
        counts.lane_geometry_count,
        BuildStructure::LaneEdgeGeometry,
    )?;
    let mut lane_arc_lengths = allocate_if_retained(
        retain,
        counts.lane_geometry_count,
        BuildStructure::LaneEdgeGeometry,
    )?;
    let mut lane_point_ranges = allocate_if_retained(
        retain,
        counts.lane_geometry_count,
        BuildStructure::LaneEdgeGeometry,
    )?;
    let mut lane_points = allocate_if_retained(
        retain,
        counts.lane_point_count,
        BuildStructure::LaneEdgeGeometry,
    )?;
    let mut lane_segment_ranges = allocate_if_retained(
        retain,
        counts.lane_geometry_count,
        BuildStructure::LaneEdgeGeometry,
    )?;
    let mut lane_segments = allocate_if_retained(
        retain,
        counts.lane_segment_count,
        BuildStructure::LaneEdgeGeometry,
    )?;

    let spatial_section = view
        .registry_view()
        .section(4)
        .ok_or(BuildError::InputInvariant {
            structure: BuildStructure::SpatialPresence,
        })?;
    let lane_table = spatial_section.table(1).ok_or(BuildError::InputInvariant {
        structure: BuildStructure::LaneEdgeGeometry,
    })?;
    for (row_index, row) in lane_table.rows().enumerate() {
        let expected = u32::try_from(row_index).map_err(|_| BuildError::ArithmeticOverflow {
            structure: BuildStructure::LaneEdgeGeometry,
        })?;
        poll_cancelled(options, expected)?;
        let lane_edge = checked_u32(row, 1, BuildStructure::LaneEdgeGeometry)?;
        if lane_edge != expected {
            return Err(BuildError::UnexpectedOrdinal {
                structure: BuildStructure::LaneEdgeGeometry,
                expected,
                actual: lane_edge,
            });
        }
        if lane_edge >= lane_count {
            return Err(BuildError::ReferenceOutOfBounds {
                structure: BuildStructure::LaneEdgeGeometry,
                ordinal: lane_edge,
                limit: lane_count,
            });
        }
        let frame = checked_u32(row, 2, BuildStructure::LaneEdgeGeometry)?;
        if frame >= frame_count {
            return Err(BuildError::ReferenceOutOfBounds {
                structure: BuildStructure::LaneEdgeGeometry,
                ordinal: frame,
                limit: frame_count,
            });
        }
        let arc_length = checked_f32(row, 3, BuildStructure::LaneEdgeGeometry)?;
        let traffic_length = *traffic
            .lane_lengths_meters()
            .get(usize::try_from(lane_edge).expect("u32 lane ordinal fits usize"))
            .ok_or(BuildError::InputInvariant {
                structure: BuildStructure::LaneEdgeGeometry,
            })?;
        if !spatial_length_matches(traffic_length, arc_length) {
            return Err(BuildError::SpatialLengthMismatch {
                lane_edge,
                traffic_length_meters: traffic_length,
                spatial_length_meters: arc_length,
            });
        }

        let points = checked_record_vector(row, 4, BuildStructure::LaneEdgeGeometry)?;
        let segments = checked_record_vector(row, 5, BuildStructure::LaneEdgeGeometry)?;
        if retain {
            lane_frames.push(CanonicalFrameOrdinal::from_raw(frame));
            lane_arc_lengths.push(arc_length);
            let point_start =
                u32::try_from(lane_points.len()).map_err(|_| BuildError::ArithmeticOverflow {
                    structure: BuildStructure::LaneEdgeGeometry,
                })?;
            fill_points(points, &mut lane_points, BuildStructure::LaneEdgeGeometry)?;
            lane_point_ranges.push(RangeU32::new(point_start, points.len()));

            let segment_start =
                u32::try_from(lane_segments.len()).map_err(|_| BuildError::ArithmeticOverflow {
                    structure: BuildStructure::LaneEdgeGeometry,
                })?;
            fill_segments(segments, &mut lane_segments)?;
            lane_segment_ranges.push(RangeU32::new(segment_start, segments.len()));
        }
    }

    let mut facility_entries = allocate_if_retained(
        retain,
        counts.facility_geometry_count,
        BuildStructure::FacilityBandGeometry,
    )?;
    let mut facility_points = allocate_if_retained(
        retain,
        counts.facility_point_count,
        BuildStructure::FacilityBandGeometry,
    )?;
    let facility_table = spatial_section.table(2).ok_or(BuildError::InputInvariant {
        structure: BuildStructure::FacilityBandGeometry,
    })?;
    let mut previous_facility = None;
    for (row_index, row) in facility_table.rows().enumerate() {
        poll_cancelled(options, u32::try_from(row_index).unwrap_or(u32::MAX))?;
        let facility = checked_u32(row, 1, BuildStructure::FacilityBandGeometry)?;
        if facility >= facility_count {
            return Err(BuildError::ReferenceOutOfBounds {
                structure: BuildStructure::FacilityBandGeometry,
                ordinal: facility,
                limit: facility_count,
            });
        }
        if let Some(previous) = previous_facility
            && facility <= previous
        {
            return Err(BuildError::NonCanonicalOrder {
                structure: BuildStructure::FacilityBandGeometry,
                previous,
                actual: facility,
            });
        }
        previous_facility = Some(facility);
        let frame = checked_u32(row, 2, BuildStructure::FacilityBandGeometry)?;
        if frame >= frame_count {
            return Err(BuildError::ReferenceOutOfBounds {
                structure: BuildStructure::FacilityBandGeometry,
                ordinal: frame,
                limit: frame_count,
            });
        }
        let points = checked_record_vector(row, 3, BuildStructure::FacilityBandGeometry)?;
        if retain {
            let start = u32::try_from(facility_points.len()).map_err(|_| {
                BuildError::ArithmeticOverflow {
                    structure: BuildStructure::FacilityBandGeometry,
                }
            })?;
            fill_points(
                points,
                &mut facility_points,
                BuildStructure::FacilityBandGeometry,
            )?;
            facility_entries.push(FacilityGeometryEntry {
                facility_band: FacilityBandOrdinal::from_raw(facility),
                canonical_frame: CanonicalFrameOrdinal::from_raw(frame),
                point_range: RangeU32::new(start, points.len()),
            });
        }
    }

    if !retain || !counts.spatial_present {
        return Ok(None);
    }
    let lane_pose = if counts.lane_geometry_count == 0 {
        None
    } else {
        Some(LanePoseNetwork::new(
            lane_frames.into_boxed_slice(),
            lane_arc_lengths.into_boxed_slice(),
            lane_point_ranges.into_boxed_slice(),
            lane_points.into_boxed_slice(),
            lane_segment_ranges.into_boxed_slice(),
            lane_segments.into_boxed_slice(),
        ))
    };
    Ok(Some(SharedSpatialNetwork::new(
        counts.direction_profile,
        lane_pose,
        facility_entries.into_boxed_slice(),
        facility_points.into_boxed_slice(),
    )))
}

fn fill_points(
    records: RegistryCheckedRecordVectorView<'_>,
    output: &mut Vec<CanonicalPoint>,
    structure: BuildStructure,
) -> Result<(), BuildError> {
    for row in records.rows() {
        output.push(CanonicalPoint {
            x: checked_f32(row, 1, structure)?,
            y: checked_f32(row, 2, structure)?,
            z: checked_f32(row, 3, structure)?,
        });
    }
    Ok(())
}

fn fill_segments(
    records: RegistryCheckedRecordVectorView<'_>,
    output: &mut Vec<SegmentGeometry>,
) -> Result<(), BuildError> {
    for row in records.rows() {
        output.push(SegmentGeometry {
            length_meters: checked_f32(row, 1, BuildStructure::LaneEdgeGeometry)?,
            cumulative_end_meters: checked_f32(row, 2, BuildStructure::LaneEdgeGeometry)?,
            tangent: [
                checked_f32(row, 3, BuildStructure::LaneEdgeGeometry)?,
                checked_f32(row, 4, BuildStructure::LaneEdgeGeometry)?,
                checked_f32(row, 5, BuildStructure::LaneEdgeGeometry)?,
            ],
            up: [
                checked_f32(row, 6, BuildStructure::LaneEdgeGeometry)?,
                checked_f32(row, 7, BuildStructure::LaneEdgeGeometry)?,
                checked_f32(row, 8, BuildStructure::LaneEdgeGeometry)?,
            ],
        });
    }
    Ok(())
}

fn spatial_length_matches(traffic_length_meters: f64, spatial_length_meters: f32) -> bool {
    let spatial = f64::from(spatial_length_meters);
    let tolerance = laneflow_static_contract::SPATIAL_LENGTH_ABS_TOLERANCE_METERS.max(
        laneflow_static_contract::SPATIAL_LENGTH_REL_TOLERANCE * traffic_length_meters.max(spatial),
    );
    (traffic_length_meters - spatial).abs() <= tolerance
}

fn singleton_row(
    view: ValueCheckedObjectView<'_>,
    section_ordinal: u32,
    structure: BuildStructure,
) -> Result<RegistryCheckedRowView<'_>, BuildError> {
    view.registry_view()
        .section(section_ordinal)
        .and_then(|section| section.table(0))
        .and_then(|table| table.row(0))
        .ok_or(BuildError::InputInvariant { structure })
}

fn checked_u8(
    row: RegistryCheckedRowView<'_>,
    tag: u16,
    structure: BuildStructure,
) -> Result<u8, BuildError> {
    match checked_field(row, tag, structure)? {
        RegistryCheckedFieldValue::U8(value) => Ok(value),
        _ => Err(BuildError::InputInvariant { structure }),
    }
}

fn checked_u16(
    row: RegistryCheckedRowView<'_>,
    tag: u16,
    structure: BuildStructure,
) -> Result<u16, BuildError> {
    match checked_field(row, tag, structure)? {
        RegistryCheckedFieldValue::U16(value) => Ok(value),
        _ => Err(BuildError::InputInvariant { structure }),
    }
}

fn checked_u32(
    row: RegistryCheckedRowView<'_>,
    tag: u16,
    structure: BuildStructure,
) -> Result<u32, BuildError> {
    match checked_field(row, tag, structure)? {
        RegistryCheckedFieldValue::U32(value) => Ok(value),
        _ => Err(BuildError::InputInvariant { structure }),
    }
}

fn checked_f32(
    row: RegistryCheckedRowView<'_>,
    tag: u16,
    structure: BuildStructure,
) -> Result<f32, BuildError> {
    match checked_field(row, tag, structure)? {
        RegistryCheckedFieldValue::F32(value) => Ok(value),
        _ => Err(BuildError::InputInvariant { structure }),
    }
}

fn checked_f64(
    row: RegistryCheckedRowView<'_>,
    tag: u16,
    structure: BuildStructure,
) -> Result<f64, BuildError> {
    match checked_field(row, tag, structure)? {
        RegistryCheckedFieldValue::F64(value) => Ok(value),
        _ => Err(BuildError::InputInvariant { structure }),
    }
}

fn checked_stable_id(
    row: RegistryCheckedRowView<'_>,
    tag: u16,
    structure: BuildStructure,
) -> Result<StableId128, BuildError> {
    match checked_field(row, tag, structure)? {
        RegistryCheckedFieldValue::StableId128(value) => Ok(value),
        _ => Err(BuildError::InputInvariant { structure }),
    }
}

fn checked_ordinal_vector(
    row: RegistryCheckedRowView<'_>,
    tag: u16,
    structure: BuildStructure,
) -> Result<RegistryCheckedOrdinalVectorView<'_>, BuildError> {
    match checked_field(row, tag, structure)? {
        RegistryCheckedFieldValue::OrdinalVectorU32(value) => Ok(value),
        _ => Err(BuildError::InputInvariant { structure }),
    }
}

fn checked_record_vector(
    row: RegistryCheckedRowView<'_>,
    tag: u16,
    structure: BuildStructure,
) -> Result<RegistryCheckedRecordVectorView<'_>, BuildError> {
    match checked_field(row, tag, structure)? {
        RegistryCheckedFieldValue::RecordVector(value) => Ok(value),
        _ => Err(BuildError::InputInvariant { structure }),
    }
}

fn checked_field<'a>(
    row: RegistryCheckedRowView<'a>,
    tag: u16,
    structure: BuildStructure,
) -> Result<RegistryCheckedFieldValue<'a>, BuildError> {
    row.field_by_tag(tag)
        .ok_or(BuildError::InputInvariant { structure })?
        .value()
        .map_err(|_| BuildError::InputInvariant { structure })
}

fn allocate_vec<T>(count: u32, structure: BuildStructure) -> Result<Vec<T>, BuildError> {
    let capacity = usize::try_from(count).expect("u32 format count fits usize");
    let mut values = Vec::new();
    values
        .try_reserve_exact(capacity)
        .map_err(|_| BuildError::AllocationFailure { structure })?;
    Ok(values)
}

fn allocate_if_retained<T>(
    retain: bool,
    count: u32,
    structure: BuildStructure,
) -> Result<Vec<T>, BuildError> {
    if retain {
        allocate_vec(count, structure)
    } else {
        Ok(Vec::new())
    }
}

fn checked_add_count(left: u32, right: u32, structure: BuildStructure) -> Result<u32, BuildError> {
    left.checked_add(right)
        .ok_or(BuildError::ArithmeticOverflow { structure })
}

fn retained_bytes<T>(count: u32) -> Result<u64, BuildError> {
    structure_bytes::<T>(count, BuildStructure::RetainedOutput)
}

fn structure_bytes<T>(count: u32, structure: BuildStructure) -> Result<u64, BuildError> {
    u64::from(count)
        .checked_mul(
            u64::try_from(size_of::<T>())
                .map_err(|_| BuildError::ArithmeticOverflow { structure })?,
        )
        .ok_or(BuildError::ArithmeticOverflow { structure })
}

fn add_retained<T>(total: u64, count: u32) -> Result<u64, BuildError> {
    total
        .checked_add(retained_bytes::<T>(count)?)
        .ok_or(BuildError::ArithmeticOverflow {
            structure: BuildStructure::RetainedOutput,
        })
}

fn add_retained_bytes(total: u64, bytes: u32) -> Result<u64, BuildError> {
    total
        .checked_add(u64::from(bytes))
        .ok_or(BuildError::ArithmeticOverflow {
            structure: BuildStructure::RetainedOutput,
        })
}

fn check_cancelled(options: SharedNetworkBuildOptions<'_>) -> Result<(), BuildError> {
    if options
        .cancellation
        .is_some_and(|flag| flag.load(Ordering::Relaxed))
    {
        Err(BuildError::Cancelled)
    } else {
        Ok(())
    }
}

fn poll_cancelled(options: SharedNetworkBuildOptions<'_>, ordinal: u32) -> Result<(), BuildError> {
    if ordinal & CANCELLATION_POLL_MASK == 0 {
        check_cancelled(options)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_OPTIONS: SharedNetworkBuildOptions<'static> = SharedNetworkBuildOptions::new(
        SpatialBuildOption::Omit,
        SharedNetworkBuildLimits::new(u64::MAX, u64::MAX),
    );

    #[test]
    fn gate_stop_line_must_cover_transition_predecessor() {
        let gate = GateTopology {
            maneuver_path: 0,
            transition_index: 0,
            stop_line: 0,
        };
        let stop_lines = [StopLineTopology { lane_edge: 4 }];

        assert!(validate_gate_stop_line_edge(gate, &stop_lines, 4, 1).is_ok());
        assert!(matches!(
            validate_gate_stop_line_edge(gate, &stop_lines, 3, 1),
            Err(BuildError::InputInvariant {
                structure: BuildStructure::ManeuverCandidates,
            })
        ));
    }

    #[test]
    fn dense_pair_sort_is_lexicographic_and_deduplicates() {
        let pairs = vec![
            TransitionPair {
                predecessor: 2,
                successor: 1,
            },
            TransitionPair {
                predecessor: 0,
                successor: 2,
            },
            TransitionPair {
                predecessor: 2,
                successor: 0,
            },
            TransitionPair {
                predecessor: 0,
                successor: 2,
            },
        ];

        let actual = radix_sort_transition_pairs(pairs, 3, TEST_OPTIONS).expect("dense pair sort");
        assert_eq!(
            actual,
            [
                TransitionPair {
                    predecessor: 0,
                    successor: 2,
                },
                TransitionPair {
                    predecessor: 2,
                    successor: 0,
                },
                TransitionPair {
                    predecessor: 2,
                    successor: 1,
                },
            ]
        );
    }

    #[test]
    fn candidate_csr_groups_predecessors_and_preserves_path_order() {
        let candidate = |predecessor, successor, path, transition_index| CandidateBuildEntry {
            predecessor,
            candidate: ManeuverTransitionCandidate::new(
                LaneEdgeOrdinal::from_raw(successor),
                ManeuverPathOrdinal::from_raw(path),
                transition_index,
                None,
            ),
        };
        let entries = vec![
            candidate(2, 1, 0, 0),
            candidate(0, 2, 0, 1),
            candidate(2, 0, 1, 0),
        ];

        let (ranges, candidates) =
            build_maneuver_candidates(3, entries, TEST_OPTIONS).expect("candidate CSR");
        assert_eq!(ranges[0].slice(&candidates).len(), 1);
        assert!(ranges[1].slice(&candidates).is_empty());
        let predecessor_two = ranges[2].slice(&candidates);
        assert_eq!(predecessor_two.len(), 2);
        assert_eq!(predecessor_two[0].maneuver_path().raw(), 0);
        assert_eq!(predecessor_two[1].maneuver_path().raw(), 1);
    }

    #[test]
    fn retained_budget_counts_forward_identity_by_entity_rows() {
        let mut entity_counts = [0; ENTITY_KIND_COUNT];
        entity_counts[kind_index(EntityKind::Junction)] = 2;
        let base = u64::try_from(size_of::<SharedNetworkRevision>()).expect("root size");
        let one_id = u64::try_from(size_of::<StableId128>()).expect("stable ID size");
        let counts = BuildCounts {
            contracts: StaticContractVersions::new(0, 0, 0, 0, 0, 0),
            entity_counts: EntityCounts::new(entity_counts),
            entity_count_total: 2,
            identity_count: 0,
            topology_pair_capacity: 0,
            maneuver_path_edge_count: 0,
            maneuver_path_gate_count: 0,
            maneuver_path_waiting_zone_count: 0,
            maneuver_transition_count: 0,
            spatial_present: false,
            direction_profile: 0,
            lane_geometry_count: 0,
            lane_point_count: 0,
            lane_segment_count: 0,
            facility_geometry_count: 0,
            facility_point_count: 0,
        };
        let options = SharedNetworkBuildOptions::new(
            SpatialBuildOption::Omit,
            SharedNetworkBuildLimits::new(base + one_id, u64::MAX),
        );

        assert!(matches!(
            check_retained_budget(counts, 0, options),
            Err(BuildError::BudgetExceeded {
                structure: BuildStructure::RetainedOutput,
                ..
            })
        ));
    }
}
