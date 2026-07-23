//! #207 P2 partitioned occupancy / leader halo 研究原型。
//!
//! 本模块只在测试构建中存在。它以 production `CoreWorld` 为 oracle，模拟 physical edge
//! ownership、按 selected route occurrence + dynamic horizon 生成的只读 halo，以及按完整
//! logical dependency component 求解的 projection。partition ID、遍历顺序和 completion
//! order 都不进入 production state 或 public identity。

use std::collections::{HashMap, HashSet};

use super::*;
use crate::{
    EdgeLength, IidmProfileSpec, LaneEdge, MovementGate, ParkingRegistry, ParkingSpace,
    ParkingSpaceGeometry, SignalAspect, SignalControlInput, SignalController, SignalGroup,
    SignalGroupState, SignalPhase, SignalRegistry, SpeedLimit, StopLine, StopLineLocation,
    VehicleProfile, VehicleProfileHandle, VehicleProfileRegistry,
    longitudinal::{LongitudinalMotion, LongitudinalScratch},
    occupancy::{LeaderObservation, OccupancyScratch, Occupant},
};

const PARTITION_COUNTS: [usize; 4] = [1, 2, 4, 7];
const ASSIGNMENT_SEEDS: [u64; 3] = [0, 1, 0xa5a5_5a5a];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct TestPartitionId(usize);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CompletionPermutation {
    Forward,
    Reverse,
    Rotated,
}

const COMPLETION_PERMUTATIONS: [CompletionPermutation; 3] = [
    CompletionPermutation::Forward,
    CompletionPermutation::Reverse,
    CompletionPermutation::Rotated,
];

#[derive(Clone, Debug, PartialEq, Eq)]
struct TestPartitionMap {
    owners: Vec<TestPartitionId>,
    partition_count: usize,
}

impl TestPartitionMap {
    fn fixture(edge_count: usize, partition_count: usize, seed: u64) -> Self {
        assert!(
            partition_count > 0,
            "research partition count must be positive"
        );
        assert!(
            edge_count > 0,
            "research fixture must contain physical edges"
        );

        let owners = (0..edge_count)
            .map(|edge_index| {
                let owner = match seed {
                    // 连续 edge cluster，保留 locality 对照。
                    0 => edge_index * partition_count / edge_count,
                    // 交错 ownership，主动制造边界依赖和迁移。
                    1 => edge_index % partition_count,
                    // 稳定 hash assignment，避免只验证规则拓扑。
                    _ => {
                        let mixed = (edge_index as u64)
                            .wrapping_mul(0x9e37_79b9_7f4a_7c15)
                            .rotate_left(17)
                            ^ seed;
                        usize::try_from(mixed % partition_count as u64)
                            .expect("partition index must fit usize")
                    }
                };
                TestPartitionId(owner)
            })
            .collect();

        Self {
            owners,
            partition_count,
        }
    }

    fn owner(&self, edge: EdgeHandle) -> TestPartitionId {
        self.owners[edge.index()]
    }

    fn vehicle_owner(&self, world: &CoreWorld, vehicle: &VehicleState) -> TestPartitionId {
        self.owner(world.vehicle_edge(vehicle))
    }
}

#[derive(Clone, Copy, Debug)]
struct PartitionEntry {
    occupant: Occupant,
    is_halo: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct OccupantBits {
    vehicle: VehicleHandle,
    front_progress: u64,
    vehicle_length: u64,
    update_sequence: u64,
}

impl From<Occupant> for OccupantBits {
    fn from(occupant: Occupant) -> Self {
        Self {
            vehicle: occupant.vehicle,
            front_progress: occupant.front_progress.to_bits(),
            vehicle_length: occupant.vehicle_length.to_bits(),
            update_sequence: occupant.update_sequence,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct LeaderBits {
    leader: VehicleHandle,
    bumper_gap: u64,
}

impl From<LeaderObservation> for LeaderBits {
    fn from(observation: LeaderObservation) -> Self {
        Self {
            leader: observation.leader,
            bumper_gap: observation.bumper_gap.to_bits(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct DependencySummary {
    component_count: usize,
    cross_partition_components: usize,
    cycle_count: usize,
    max_depth: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ResearchMetrics {
    owned_min: usize,
    owned_max: usize,
    remote_slices: usize,
    halo_unique_vehicles: usize,
    halo_total_copies: usize,
    cross_partition_dependencies: usize,
    dependency: DependencySummary,
    oracle_occupancy_bytes: usize,
    retained_occupancy_bytes: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct OccupancyOracle {
    edges: Vec<Vec<OccupantBits>>,
    leaders: Vec<(VehicleHandle, Option<LeaderBits>)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PartitionedObservation {
    edges: Vec<Vec<OccupantBits>>,
    leaders: Vec<(VehicleHandle, Option<LeaderBits>)>,
    metrics: ResearchMetrics,
}

#[derive(Clone, Debug, PartialEq)]
struct MotionFingerprint {
    motion: LongitudinalMotion,
    float_bits: [u64; 13],
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct ScenarioEvidence {
    saw_halo: bool,
    saw_multi_edge_halo: bool,
    saw_cross_partition_dependency: bool,
    saw_cross_partition_component: bool,
    saw_cycle: bool,
    saw_boundary_migration: bool,
}

impl ScenarioEvidence {
    fn merge(&mut self, other: Self) {
        self.saw_halo |= other.saw_halo;
        self.saw_multi_edge_halo |= other.saw_multi_edge_halo;
        self.saw_cross_partition_dependency |= other.saw_cross_partition_dependency;
        self.saw_cross_partition_component |= other.saw_cross_partition_component;
        self.saw_cycle |= other.saw_cycle;
        self.saw_boundary_migration |= other.saw_boundary_migration;
    }
}

fn partition_order(
    partition_count: usize,
    permutation: CompletionPermutation,
) -> Vec<TestPartitionId> {
    let mut order = (0..partition_count)
        .map(TestPartitionId)
        .collect::<Vec<_>>();
    match permutation {
        CompletionPermutation::Forward => {}
        CompletionPermutation::Reverse => order.reverse(),
        CompletionPermutation::Rotated => {
            let split = partition_count / 2;
            order.rotate_left(split);
        }
    }
    order
}

fn stable_handles(world: &CoreWorld) -> Vec<VehicleHandle> {
    world.vehicle_update_order.iter().collect()
}

fn active_or_stopped(vehicle: &VehicleState) -> bool {
    matches!(
        vehicle.status,
        VehicleStatus::Active | VehicleStatus::Stopped
    )
}

fn add_entry(
    entries: &mut [Vec<Vec<PartitionEntry>>],
    partition: TestPartitionId,
    edge: EdgeHandle,
    occupant: Occupant,
    is_halo: bool,
) {
    let edge_entries = &mut entries[partition.0][edge.index()];
    if let Some(existing) = edge_entries
        .iter_mut()
        .find(|entry| entry.occupant.vehicle == occupant.vehicle)
    {
        existing.is_halo &= is_halo;
        return;
    }
    edge_entries.push(PartitionEntry { occupant, is_halo });
}

fn add_route_halo(
    world: &CoreWorld,
    oracle: &OccupancyScratch,
    mapping: &TestPartitionMap,
    follower: &VehicleState,
    bumper_gap_horizon: f64,
    entries: &mut [Vec<Vec<PartitionEntry>>],
) -> Result<(), CoreError> {
    let partition = mapping.vehicle_owner(world, follower);
    let front_horizon = bumper_gap_horizon + oracle.max_vehicle_length();
    CoreWorld::finite_leader_value(follower.handle, "front_horizon", front_horizon)?;

    let route = world
        .route_slot(follower.route)
        .expect("live vehicle route must exist");
    let current_edge = route.edge_handles[follower.route_edge_index];
    let current_edge_length = world
        .lane_graph
        .edge_length(current_edge)
        .expect("route edge must exist")
        .value();
    let mut distance_to_edge_start = current_edge_length - follower.edge_progress.value();

    for edge in route
        .edge_handles
        .iter()
        .copied()
        .skip(follower.route_edge_index + 1)
    {
        CoreWorld::finite_leader_value(
            follower.handle,
            "distance_to_edge_start",
            distance_to_edge_start,
        )?;
        if distance_to_edge_start > front_horizon {
            break;
        }

        if mapping.owner(edge) != partition {
            let remaining = front_horizon - distance_to_edge_start;
            for occupant in oracle.edge(edge).iter().copied() {
                if occupant.front_progress > remaining {
                    break;
                }
                add_entry(entries, partition, edge, occupant, true);
            }
        }

        let edge_length = world
            .lane_graph
            .edge_length(edge)
            .expect("route edge must exist")
            .value();
        if edge_length > front_horizon - distance_to_edge_start {
            break;
        }
        distance_to_edge_start += edge_length;
    }

    Ok(())
}

fn build_partition_views(
    world: &CoreWorld,
    oracle: &OccupancyScratch,
    mapping: &TestPartitionMap,
    permutation: CompletionPermutation,
) -> Result<(Vec<OccupancyScratch>, Vec<usize>), CoreError> {
    let edge_count = world.lane_graph.edges().len();
    let mut entries = vec![vec![Vec::<PartitionEntry>::new(); edge_count]; mapping.partition_count];
    let mut owned_counts = vec![0; mapping.partition_count];

    for edge_index in 0..edge_count {
        let edge = EdgeHandle::new(edge_index);
        let owner = mapping.owner(edge);
        for occupant in oracle.edge(edge).iter().copied() {
            add_entry(&mut entries, owner, edge, occupant, false);
            owned_counts[owner.0] += 1;
        }
    }

    if oracle.occupant_count() > 1 {
        // halo demand 与错误选择都按 logical vehicle update order 产生，不按 partition completion。
        let mut errors = Vec::new();
        for (stable_position, handle) in stable_handles(world).into_iter().enumerate() {
            let Some(vehicle) = world
                .vehicle(handle)
                .filter(|vehicle| active_or_stopped(vehicle))
            else {
                continue;
            };
            let result = world.leader_horizon(vehicle).and_then(|horizon| {
                add_route_halo(world, oracle, mapping, vehicle, horizon, &mut entries)
            });
            if let Err(error) = result {
                errors.push((stable_position, error));
            }
        }
        if let Some((_, error)) = errors.into_iter().min_by_key(|(position, _)| *position) {
            return Err(error);
        }
    }

    let mut views = (0..mapping.partition_count)
        .map(|_| OccupancyScratch::default())
        .collect::<Vec<_>>();
    for partition in partition_order(mapping.partition_count, permutation) {
        let view = &mut views[partition.0];
        view.begin(edge_count, world.vehicles.len());

        let mut edge_order = (0..edge_count).collect::<Vec<_>>();
        if matches!(permutation, CompletionPermutation::Reverse) {
            edge_order.reverse();
        } else if matches!(permutation, CompletionPermutation::Rotated) {
            edge_order.rotate_left(edge_count / 2);
        }
        for edge_index in edge_order.iter().copied() {
            let edge = EdgeHandle::new(edge_index);
            for entry in &entries[partition.0][edge_index] {
                view.count(edge, entry.occupant.vehicle_length);
            }
        }
        view.allocate_occupants();
        for edge_index in edge_order {
            let edge = EdgeHandle::new(edge_index);
            let edge_entries = &entries[partition.0][edge_index];
            if matches!(permutation, CompletionPermutation::Forward) {
                for entry in edge_entries.iter().copied() {
                    view.insert(edge, entry.occupant);
                }
            } else {
                for entry in edge_entries.iter().rev().copied() {
                    view.insert(edge, entry.occupant);
                }
            }
        }
        // `find_leader` 的 front horizon 使用 whole-world max length；它是只读语义元数据，
        // 不能因当前 partition 恰好没有最长车辆而缩短。
        view.set_max_vehicle_length_for_research(oracle.max_vehicle_length());
        view.sort_edges();
    }

    Ok((views, owned_counts))
}

fn occupancy_edges(scratch: &OccupancyScratch, edge_count: usize) -> Vec<Vec<OccupantBits>> {
    (0..edge_count)
        .map(|edge_index| {
            scratch
                .edge(EdgeHandle::new(edge_index))
                .iter()
                .copied()
                .map(OccupantBits::from)
                .collect()
        })
        .collect()
}

fn oracle_occupancy(world: &CoreWorld) -> Result<OccupancyOracle, CoreError> {
    let mut scratch = OccupancyScratch::default();
    world.build_occupancy(&mut scratch);
    let mut leaders = Vec::new();

    for handle in stable_handles(world) {
        let observation = if scratch.occupant_count() <= 1 {
            None
        } else if let Some(vehicle) = world
            .vehicle(handle)
            .filter(|vehicle| active_or_stopped(vehicle))
        {
            let horizon = world.leader_horizon(vehicle)?;
            world.find_leader(&scratch, vehicle, horizon)?
        } else {
            None
        };
        leaders.push((handle, observation.map(LeaderBits::from)));
    }

    Ok(OccupancyOracle {
        edges: occupancy_edges(&scratch, world.lane_graph.edges().len()),
        leaders,
    })
}

fn motion_handles(world: &CoreWorld, scratch: &LongitudinalScratch) -> Vec<VehicleHandle> {
    stable_handles(world)
        .into_iter()
        .filter(|handle| scratch.motion(*handle).is_some())
        .collect()
}

fn logical_components(
    handles: &[VehicleHandle],
    scratch: &LongitudinalScratch,
) -> Vec<Vec<VehicleHandle>> {
    let positions = handles
        .iter()
        .copied()
        .enumerate()
        .map(|(position, handle)| (handle, position))
        .collect::<HashMap<_, _>>();
    let mut parent = (0..handles.len()).collect::<Vec<_>>();

    fn root(parent: &mut [usize], mut index: usize) -> usize {
        while parent[index] != index {
            parent[index] = parent[parent[index]];
            index = parent[index];
        }
        index
    }

    for (position, handle) in handles.iter().copied().enumerate() {
        let Some(leader) = scratch
            .motion(handle)
            .and_then(LongitudinalMotion::leader_for_research)
            .and_then(|leader| positions.get(&leader.leader).copied())
        else {
            continue;
        };
        let left = root(&mut parent, position);
        let right = root(&mut parent, leader);
        if left != right {
            parent[right] = left;
        }
    }

    let mut groups = HashMap::<usize, Vec<VehicleHandle>>::new();
    for (position, handle) in handles.iter().copied().enumerate() {
        let root = root(&mut parent, position);
        groups.entry(root).or_default().push(handle);
    }

    let stable_position = positions;
    let mut components = groups.into_values().collect::<Vec<_>>();
    for component in &mut components {
        component.sort_by_key(|handle| stable_position[handle]);
    }
    components.sort_by_key(|component| stable_position[&component[0]]);
    components
}

fn dependency_summary(
    world: &CoreWorld,
    mapping: &TestPartitionMap,
    scratch: &LongitudinalScratch,
    components: &[Vec<VehicleHandle>],
) -> DependencySummary {
    let positions = motion_handles(world, scratch)
        .into_iter()
        .enumerate()
        .map(|(position, handle)| (handle, position))
        .collect::<HashMap<_, _>>();
    let mut cycle_anchors = HashSet::new();
    let mut max_depth = 0;

    for start in positions.keys().copied() {
        let mut path = Vec::new();
        let mut seen = HashMap::new();
        let mut current = Some(start);
        while let Some(handle) = current {
            if let Some(cycle_start) = seen.get(&handle).copied() {
                let anchor = path[cycle_start..]
                    .iter()
                    .map(|candidate| positions[candidate])
                    .min()
                    .expect("cycle path must be non-empty");
                cycle_anchors.insert(anchor);
                break;
            }
            seen.insert(handle, path.len());
            path.push(handle);
            current = scratch
                .motion(handle)
                .and_then(LongitudinalMotion::leader_for_research)
                .map(|leader| leader.leader)
                .filter(|leader| positions.contains_key(leader));
        }
        max_depth = max_depth.max(path.len());
    }

    let cross_partition_components = components
        .iter()
        .filter(|component| {
            component
                .iter()
                .filter_map(|handle| world.vehicle(*handle))
                .map(|vehicle| mapping.vehicle_owner(world, vehicle))
                .collect::<HashSet<_>>()
                .len()
                > 1
        })
        .count();

    DependencySummary {
        component_count: components.len(),
        cross_partition_components,
        cycle_count: cycle_anchors.len(),
        max_depth,
    }
}

fn motion_fingerprints(
    world: &CoreWorld,
    scratch: &LongitudinalScratch,
) -> Vec<(VehicleHandle, Option<MotionFingerprint>)> {
    stable_handles(world)
        .into_iter()
        .map(|handle| {
            (
                handle,
                scratch.motion(handle).map(|motion| MotionFingerprint {
                    motion,
                    float_bits: motion.float_bits_for_research(),
                }),
            )
        })
        .collect()
}

fn assert_component_projection_exact(
    world: &CoreWorld,
    mapping: &TestPartitionMap,
    permutation: CompletionPermutation,
) -> DependencySummary {
    let mut oracle = world.clone();
    oracle
        .rebuild_occupancy_and_leaders()
        .expect("scenario occupancy must be valid");
    oracle
        .rebuild_longitudinal_motions()
        .expect("scenario motions must be valid");
    let global = oracle.longitudinal_scratch.clone();
    let mut component_projection = global.clone();
    component_projection.reset_geometry_projection_for_research();
    let handles = motion_handles(&oracle, &component_projection);
    let components = logical_components(&handles, &component_projection);
    let dependency = dependency_summary(&oracle, mapping, &component_projection, &components);
    let delta_time = oracle.fixed_delta_time_ms as f64 / 1_000.0;

    let mut component_order = (0..components.len()).collect::<Vec<_>>();
    match permutation {
        CompletionPermutation::Forward => {}
        CompletionPermutation::Reverse => component_order.reverse(),
        CompletionPermutation::Rotated => {
            let split = component_order.len() / 2;
            component_order.rotate_left(split);
        }
    }
    for component_index in component_order {
        component_projection
            .project(components[component_index].iter(), delta_time)
            .expect("logical dependency component projection must succeed");
    }

    assert_eq!(
        motion_fingerprints(&oracle, &component_projection),
        motion_fingerprints(&oracle, &global),
        "component projection must preserve exact motion fields and float bits"
    );
    dependency
}

fn partitioned_observation(
    world: &CoreWorld,
    mapping: &TestPartitionMap,
    permutation: CompletionPermutation,
) -> Result<PartitionedObservation, CoreError> {
    let edge_count = world.lane_graph.edges().len();
    let mut oracle = OccupancyScratch::default();
    world.build_occupancy(&mut oracle);
    let (views, owned_counts) = build_partition_views(world, &oracle, mapping, permutation)?;

    let mut edges = Vec::with_capacity(edge_count);
    for edge_index in 0..edge_count {
        let edge = EdgeHandle::new(edge_index);
        edges.push(
            views[mapping.owner(edge).0]
                .edge(edge)
                .iter()
                .copied()
                .map(OccupantBits::from)
                .collect(),
        );
    }

    let mut horizons = vec![None; world.vehicles.len()];
    if oracle.occupant_count() > 1 {
        for handle in stable_handles(world) {
            let Some(vehicle) = world
                .vehicle(handle)
                .filter(|vehicle| active_or_stopped(vehicle))
            else {
                continue;
            };
            horizons[handle.index()] = Some(world.leader_horizon(vehicle)?);
        }
    }

    let stable_position = stable_handles(world)
        .into_iter()
        .enumerate()
        .map(|(position, handle)| (handle, position))
        .collect::<HashMap<_, _>>();
    let mut resolved = HashMap::new();
    let mut errors = Vec::new();
    for partition in partition_order(mapping.partition_count, permutation) {
        let mut handles = stable_handles(world)
            .into_iter()
            .filter(|handle| {
                world
                    .vehicle(*handle)
                    .filter(|vehicle| active_or_stopped(vehicle))
                    .is_some_and(|vehicle| mapping.vehicle_owner(world, vehicle) == partition)
            })
            .collect::<Vec<_>>();
        if !matches!(permutation, CompletionPermutation::Forward) {
            handles.reverse();
        }
        for handle in handles {
            let vehicle = world.vehicle(handle).expect("owned vehicle must exist");
            let result = if let Some(horizon) = horizons[handle.index()] {
                world.find_leader(&views[partition.0], vehicle, horizon)
            } else {
                Ok(None)
            };
            match result {
                Ok(observation) => {
                    resolved.insert(handle, observation.map(LeaderBits::from));
                }
                Err(error) => errors.push((stable_position[&handle], error)),
            }
        }
    }
    if let Some((_, error)) = errors.into_iter().min_by_key(|(position, _)| *position) {
        return Err(error);
    }

    let leaders = stable_handles(world)
        .into_iter()
        .map(|handle| (handle, resolved.get(&handle).copied().flatten()))
        .collect::<Vec<_>>();
    let mut halo_vehicles = HashSet::new();
    let mut halo_copies = HashSet::new();
    let mut remote_slices = HashSet::new();
    for (partition, view) in views.iter().enumerate() {
        for edge_index in 0..edge_count {
            let edge = EdgeHandle::new(edge_index);
            if mapping.owner(edge).0 == partition {
                continue;
            }
            for occupant in view.edge(edge) {
                halo_vehicles.insert(occupant.vehicle);
                halo_copies.insert((partition, occupant.vehicle));
                remote_slices.insert((partition, edge_index));
            }
        }
    }

    let leader_map = leaders
        .iter()
        .filter_map(|(handle, leader)| leader.map(|leader| (*handle, leader)))
        .collect::<HashMap<_, _>>();
    let cross_partition_dependencies = leader_map
        .iter()
        .filter(|(follower, leader)| {
            let follower = world.vehicle(**follower).expect("follower must exist");
            let leader = world.vehicle(leader.leader).expect("leader must exist");
            mapping.vehicle_owner(world, follower) != mapping.vehicle_owner(world, leader)
        })
        .count();
    let dependency = assert_component_projection_exact(world, mapping, permutation);

    Ok(PartitionedObservation {
        edges,
        leaders,
        metrics: ResearchMetrics {
            owned_min: owned_counts.iter().copied().min().unwrap_or(0),
            owned_max: owned_counts.iter().copied().max().unwrap_or(0),
            remote_slices: remote_slices.len(),
            halo_unique_vehicles: halo_vehicles.len(),
            halo_total_copies: halo_copies.len(),
            cross_partition_dependencies,
            dependency,
            oracle_occupancy_bytes: oracle.retained_bytes(),
            retained_occupancy_bytes: views.iter().map(OccupancyScratch::retained_bytes).sum(),
        },
    })
}

fn boundary_migrations(before: &CoreWorld, after: &CoreWorld, mapping: &TestPartitionMap) -> usize {
    stable_handles(before)
        .into_iter()
        .filter(|handle| {
            let Some(before_vehicle) = before.vehicle(*handle) else {
                return false;
            };
            let Some(after_vehicle) = after.vehicle(*handle) else {
                return false;
            };
            mapping.vehicle_owner(before, before_vehicle)
                != mapping.vehicle_owner(after, after_vehicle)
        })
        .count()
}

fn run_scenario_matrix(name: &str, base: &CoreWorld) -> ScenarioEvidence {
    let oracle = oracle_occupancy(base).expect("scenario oracle must succeed");
    let mut evidence = ScenarioEvidence::default();

    for partition_count in PARTITION_COUNTS {
        for seed in ASSIGNMENT_SEEDS {
            let mapping =
                TestPartitionMap::fixture(base.lane_graph.edges().len(), partition_count, seed);
            let mut expected_metrics = None;
            let mut serial_world = base.clone();
            let serial_result = serial_world
                .step(TickInput::new(base.fixed_delta_time_ms))
                .expect("scenario serial step must succeed");
            let migrations = boundary_migrations(base, &serial_world, &mapping);

            for permutation in COMPLETION_PERMUTATIONS {
                let before_observation = base.clone();
                let observation = partitioned_observation(base, &mapping, permutation)
                    .expect("partitioned observation must succeed");
                assert_eq!(
                    base, &before_observation,
                    "research observation is read-only"
                );
                assert_eq!(
                    observation.edges, oracle.edges,
                    "{name}: occupancy mismatch"
                );
                assert_eq!(
                    observation.leaders, oracle.leaders,
                    "{name}: leader mismatch"
                );
                if let Some(expected) = &expected_metrics {
                    assert_eq!(
                        &observation.metrics, expected,
                        "{name}: metrics must ignore completion permutation"
                    );
                } else {
                    expected_metrics = Some(observation.metrics.clone());
                }

                let mut observed_world = base.clone();
                let observed_result = observed_world
                    .step(TickInput::new(base.fixed_delta_time_ms))
                    .expect("observed scenario step must succeed");
                assert_eq!(
                    observed_result, serial_result,
                    "{name}: event/result mismatch"
                );
                assert_eq!(
                    observed_world, serial_world,
                    "{name}: committed world mismatch"
                );

                evidence.merge(ScenarioEvidence {
                    saw_halo: observation.metrics.halo_total_copies > 0,
                    saw_multi_edge_halo: observation.metrics.remote_slices > 1,
                    saw_cross_partition_dependency: observation
                        .metrics
                        .cross_partition_dependencies
                        > 0,
                    saw_cross_partition_component: observation
                        .metrics
                        .dependency
                        .cross_partition_components
                        > 0,
                    saw_cycle: observation.metrics.dependency.cycle_count > 0,
                    saw_boundary_migration: migrations > 0,
                });
            }

            let metrics = expected_metrics.expect("permutation matrix must produce metrics");
            println!(
                "P2_METRICS scenario={name} partitions={partition_count} seed={seed:#x} owned={}-{} remote_slices={} halo_unique={} halo_copies={} cross_dependencies={} components={} cross_components={} cycles={} max_depth={} migrations={} oracle_bytes={} retained_bytes={}",
                metrics.owned_min,
                metrics.owned_max,
                metrics.remote_slices,
                metrics.halo_unique_vehicles,
                metrics.halo_total_copies,
                metrics.cross_partition_dependencies,
                metrics.dependency.component_count,
                metrics.dependency.cross_partition_components,
                metrics.dependency.cycle_count,
                metrics.dependency.max_depth,
                migrations,
                metrics.oracle_occupancy_bytes,
                metrics.retained_occupancy_bytes,
            );
        }
    }

    evidence
}

fn edge(id: &str, length: f64, next: &[&str]) -> LaneEdge {
    LaneEdge::new(
        id,
        EdgeLength::try_new(length).expect("edge length"),
        SpeedLimit::try_new(f64::MAX).expect("speed limit"),
        next.iter().copied(),
    )
}

fn profile(id: &str, desired_speed: f64) -> (VehicleProfileRegistry, VehicleProfileHandle) {
    let profiles = VehicleProfileRegistry::try_new([VehicleProfile::try_new_iidm(
        id,
        IidmProfileSpec {
            length: 4.0,
            desired_speed,
            min_gap: 2.0,
            time_headway: 1.5,
            max_acceleration: 2.0,
            comfortable_deceleration: 2.0,
            emergency_deceleration: 8.0,
        },
    )
    .expect("vehicle profile")])
    .expect("vehicle profile registry");
    let handle = profiles.profile_handle(id).expect("vehicle profile handle");
    (profiles, handle)
}

fn active(
    id: &str,
    profile: VehicleProfileHandle,
    route: &str,
    route_edge_index: usize,
    progress: f64,
    speed: f64,
) -> VehicleSpawnInput {
    VehicleSpawnInput::active(
        id,
        profile,
        route,
        route_edge_index,
        EdgeProgress::try_new(progress).expect("progress"),
        Speed::try_new(speed).expect("speed"),
    )
}

fn stopped(
    id: &str,
    profile: VehicleProfileHandle,
    route: &str,
    route_edge_index: usize,
    progress: f64,
) -> VehicleSpawnInput {
    VehicleSpawnInput::stopped(
        id,
        profile,
        route,
        route_edge_index,
        EdgeProgress::try_new(progress).expect("progress"),
    )
}

fn corridor_world() -> CoreWorld {
    let graph = LaneGraph::try_new([
        edge("A", 10.0, &["B"]),
        edge("B", 10.0, &["C"]),
        edge("C", 10.0, &["D"]),
        edge("D", 10.0, &["E"]),
        edge("E", 10.0, &["F"]),
        edge("F", 10.0, &[]),
        edge("X", 10.0, &["C"]),
        edge("T0", 10.0, &["T1"]),
        edge("T1", 10.0, &["T2"]),
        edge("T2", 10.0, &[]),
    ])
    .expect("corridor graph");
    let routes = [
        Route::try_new("main", ["A", "B", "C", "D", "E", "F"]).expect("main route"),
        Route::try_new("branch", ["X", "C", "D", "E", "F"]).expect("branch route"),
        Route::try_new("transition", ["T0", "T1", "T2"]).expect("transition route"),
    ];
    let (profiles, profile) = profile("corridor-profile", 30.0);
    let traffic = InitialTrafficData::try_new(graph, routes, profiles).expect("traffic data");
    CoreWorld::with_traffic_data(
        1_000,
        traffic,
        vec![
            active("long-follower", profile, "main", 0, 9.0, 18.0),
            stopped("remote-leader", profile, "main", 3, 4.0),
            active("branch-follower", profile, "branch", 0, 9.0, 16.0),
            stopped("congestion-rear", profile, "main", 4, 2.0),
            stopped("congestion-front", profile, "main", 4, 8.0),
            active("multi-edge-transition", profile, "transition", 0, 9.0, 20.0),
            active(
                "simultaneous-completion",
                profile,
                "transition",
                2,
                9.0,
                20.0,
            ),
        ],
    )
    .expect("corridor world")
}

fn cycle_world() -> CoreWorld {
    let graph = LaneGraph::try_new([
        edge("loop-a", 20.0, &["loop-b"]),
        edge("loop-b", 20.0, &["loop-a"]),
    ])
    .expect("loop graph");
    let routes = [
        Route::try_new("route-a", ["loop-a", "loop-b", "loop-a"]).expect("route A"),
        Route::try_new("route-b", ["loop-b", "loop-a", "loop-b"]).expect("route B"),
    ];
    let (profiles, profile) = profile("cycle-profile", 30.0);
    let traffic = InitialTrafficData::try_new(graph, routes, profiles).expect("traffic data");
    CoreWorld::with_traffic_data(
        1_000,
        traffic,
        vec![
            active("cycle-a", profile, "route-a", 0, 15.0, 12.0),
            active("cycle-b", profile, "route-b", 0, 15.0, 11.0),
        ],
    )
    .expect("cycle world")
}

fn phase(id: &str, duration_ms: u64, states: &[(&str, SignalAspect)]) -> SignalPhase {
    SignalPhase::new(
        id,
        duration_ms,
        states
            .iter()
            .map(|(group, aspect)| SignalGroupState::new(*group, *aspect)),
    )
}

fn signal_world() -> CoreWorld {
    let graph = LaneGraph::try_new([
        edge("signal-a", 10.0, &["signal-b"]),
        edge("signal-b", 10.0, &[]),
    ])
    .expect("signal graph");
    let signals = SignalRegistry::try_new(
        &graph,
        [StopLine::new(
            "signal-stop",
            "signal-a",
            StopLineLocation::EdgeEnd,
        )],
        [SignalGroup::new("signal-group")],
        [SignalController::new_fixed_time(
            "signal-controller",
            0,
            ["signal-group"],
            [
                phase("red", 1_000, &[("signal-group", SignalAspect::Red)]),
                phase("green", 1_000, &[("signal-group", SignalAspect::Green)]),
            ],
        )],
        [MovementGate::new(
            "signal-a",
            "signal-b",
            "signal-stop",
            SignalControlInput::Group("signal-group".to_owned()),
        )],
    )
    .expect("signal registry");
    let (profiles, profile) = profile("signal-profile", 20.0);
    let traffic = InitialTrafficData::try_new_with_signals(
        graph,
        [Route::try_new("signal-route", ["signal-a", "signal-b"]).expect("signal route")],
        profiles,
        signals,
    )
    .expect("signal traffic");
    CoreWorld::with_traffic_data(
        1_000,
        traffic,
        vec![active(
            "signal-vehicle",
            profile,
            "signal-route",
            0,
            5.0,
            20.0,
        )],
    )
    .expect("signal world")
}

fn parking_world() -> CoreWorld {
    let graph = LaneGraph::try_new([edge("parking-edge", 200.0, &[])]).expect("parking graph");
    let parking = ParkingRegistry::try_new(
        &graph,
        [],
        [ParkingSpace::new(
            "parking-space",
            None,
            "parking-edge",
            20.0,
            "parking-edge",
            40.0,
            ParkingSpaceGeometry::new(-3.0, 0.0, 4.5, 2.4),
        )],
    )
    .expect("parking registry");
    let (profiles, profile) = profile("parking-profile", 30.0);
    let traffic = InitialTrafficData::try_new_with_signals_and_parking(
        graph,
        [Route::try_new("parking-route", ["parking-edge"]).expect("parking route")],
        profiles,
        SignalRegistry::empty(),
        parking,
    )
    .expect("parking traffic");
    let mut world = CoreWorld::with_traffic_data(
        1_000,
        traffic,
        vec![active(
            "parking-vehicle",
            profile,
            "parking-route",
            0,
            15.0,
            20.0,
        )],
    )
    .expect("parking world");
    let vehicle = world
        .vehicle_handle("parking-vehicle")
        .expect("parking vehicle");
    let space = world
        .parking()
        .space_handle("parking-space")
        .expect("parking space");
    world
        .reserve_parking_space(vehicle, space)
        .expect("parking reservation");
    world
}

fn non_finite_horizon_world() -> CoreWorld {
    let graph = LaneGraph::try_new([
        edge("overflow-a", f64::MAX, &[]),
        edge("overflow-z", f64::MAX, &[]),
    ])
    .expect("overflow graph");
    let routes = [
        Route::try_new("overflow-route-a", ["overflow-a"]).expect("overflow route A"),
        Route::try_new("overflow-route-z", ["overflow-z"]).expect("overflow route Z"),
    ];
    let (profiles, profile) = profile("overflow-profile", 30.0);
    let traffic = InitialTrafficData::try_new(graph, routes, profiles).expect("traffic data");
    CoreWorld::with_traffic_data(
        2_000,
        traffic,
        vec![
            active(
                "a-overflow-follower",
                profile,
                "overflow-route-a",
                0,
                0.0,
                f64::MAX,
            ),
            stopped("a-overflow-leader", profile, "overflow-route-a", 0, 10.0),
            active(
                "z-overflow-follower",
                profile,
                "overflow-route-z",
                0,
                0.0,
                f64::MAX,
            ),
            stopped("z-overflow-leader", profile, "overflow-route-z", 0, 10.0),
        ],
    )
    .expect("overflow world")
}

#[test]
fn partitioned_halo_and_global_components_match_the_production_oracle() {
    let mut evidence = ScenarioEvidence::default();
    evidence.merge(run_scenario_matrix("corridor", &corridor_world()));
    evidence.merge(run_scenario_matrix("cycle", &cycle_world()));
    evidence.merge(run_scenario_matrix("signals", &signal_world()));
    evidence.merge(run_scenario_matrix("parking", &parking_world()));

    assert!(
        evidence.saw_halo,
        "matrix must exercise a read-only remote halo"
    );
    assert!(
        evidence.saw_multi_edge_halo,
        "matrix must carry halo slices across more than one route edge"
    );
    assert!(
        evidence.saw_cross_partition_dependency,
        "matrix must exercise cross-partition leader dependencies"
    );
    assert!(
        evidence.saw_cross_partition_component,
        "matrix must reconstruct cross-partition logical components"
    );
    assert!(evidence.saw_cycle, "matrix must exercise a leader cycle");
    assert!(
        evidence.saw_boundary_migration,
        "matrix must observe a committed ownership change"
    );
}

#[test]
fn scenario_fixtures_reach_required_route_signal_parking_and_cycle_paths() {
    let mut corridor = corridor_world();
    let transition = corridor
        .vehicle_handle("multi-edge-transition")
        .expect("transition vehicle");
    let completing = corridor
        .vehicle_handle("simultaneous-completion")
        .expect("completing vehicle");
    let corridor_result = corridor.step(TickInput::new(1_000)).expect("corridor step");
    let transition_count = corridor_result
        .events
        .iter()
        .filter(|event| {
            matches!(
                event,
                CoreEvent::VehicleChangedEdge(event) if event.vehicle == transition
            )
        })
        .count();
    assert!(
        transition_count >= 2,
        "fixture must cross multiple route edges in one tick"
    );
    assert!(corridor_result.events.iter().any(|event| {
        matches!(
            event,
            CoreEvent::VehicleCompletedRoute(event) if event.vehicle == completing
        )
    }));

    let cycle = cycle_world();
    let cycle_a = cycle.vehicle_handle("cycle-a").expect("cycle A");
    let cycle_b = cycle.vehicle_handle("cycle-b").expect("cycle B");
    let cycle_oracle = oracle_occupancy(&cycle).expect("cycle oracle");
    let leaders = cycle_oracle.leaders.into_iter().collect::<HashMap<_, _>>();
    assert_eq!(leaders[&cycle_a].expect("cycle A leader").leader, cycle_b);
    assert_eq!(leaders[&cycle_b].expect("cycle B leader").leader, cycle_a);

    let mut signals = signal_world();
    let signal_result = signals.step(TickInput::new(1_000)).expect("signal step");
    assert!(
        signal_result
            .events
            .iter()
            .any(|event| matches!(event, CoreEvent::VehicleSignalStopProjectionApplied(_)))
    );

    let mut parking = parking_world();
    let parking_result = parking.step(TickInput::new(1_000)).expect("parking step");
    assert!(
        parking_result
            .events
            .iter()
            .any(|event| matches!(event, CoreEvent::VehicleParkingStopProjectionApplied(_)))
    );
    assert!(
        parking_result
            .events
            .iter()
            .any(|event| matches!(event, CoreEvent::VehicleParkingArrivalReached(_)))
    );
}

#[test]
fn first_error_is_canonical_and_non_finite_failure_is_atomic() {
    let base = non_finite_horizon_world();
    let follower = base
        .vehicle_handle("a-overflow-follower")
        .expect("overflow follower");
    let before = base.clone();
    let candidates = stable_handles(&base)
        .into_iter()
        .filter_map(|handle| {
            let vehicle = base.vehicle(handle)?;
            base.leader_horizon(vehicle)
                .err()
                .map(|error| (handle, error))
        })
        .collect::<Vec<_>>();
    assert_eq!(
        candidates.len(),
        2,
        "fixture must expose two independently failing leader candidates"
    );

    for partition_count in PARTITION_COUNTS {
        for seed in ASSIGNMENT_SEEDS {
            let mapping =
                TestPartitionMap::fixture(base.lane_graph.edges().len(), partition_count, seed);
            for permutation in COMPLETION_PERMUTATIONS {
                let error = partitioned_observation(&base, &mapping, permutation)
                    .expect_err("partitioned observation must preserve the oracle error");
                std::assert_matches!(
                    error,
                    CoreError::NonFiniteLeaderComputation {
                        vehicle,
                        stage: "travel_upper",
                        value
                    } if vehicle == follower && value.to_bits() == f64::INFINITY.to_bits()
                );
            }
        }
    }

    let mut failed = base.clone();
    let error = failed
        .step(TickInput::new(2_000))
        .expect_err("production oracle must reject the non-finite horizon");
    std::assert_matches!(
        error,
        CoreError::NonFiniteLeaderComputation {
            vehicle,
            stage: "travel_upper",
            value
        } if vehicle == follower && value.to_bits() == f64::INFINITY.to_bits()
    );
    assert_eq!(
        failed, before,
        "failed step must not commit authority state"
    );
}

#[test]
fn injected_late_failure_retries_like_a_fresh_replay() {
    let base = corridor_world();
    let failure_vehicle = base
        .vehicle_handle("simultaneous-completion")
        .expect("failure vehicle");
    let mut failed = base.clone();
    failed.step_failure_after_vehicle = Some(failure_vehicle);
    let before_failure = failed.clone();

    let error = failed
        .step(TickInput::new(1_000))
        .expect_err("injected late failure must abort the step");
    std::assert_matches!(
        error,
        CoreError::ParkingBindingInvariantViolation {
            stage: "test_after_vehicle_advance",
            vehicle: Some(vehicle),
            space: None,
        } if vehicle == failure_vehicle
    );
    assert_eq!(failed, before_failure, "late failure must be atomic");

    failed.step_failure_after_vehicle = None;
    let retry = failed
        .step(TickInput::new(1_000))
        .expect("retry must succeed after clearing injection");
    let mut fresh = base;
    let replay = fresh
        .step(TickInput::new(1_000))
        .expect("fresh replay must succeed");
    assert_eq!(retry, replay, "retry events/result must match fresh replay");
    assert_eq!(failed, fresh, "retry world must match fresh replay world");
}
