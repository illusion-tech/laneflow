//! #210 P3 committed selective read 研究原型。
//!
//! 本模块只在 `laneflow-core` 的测试构建中存在。它把 current `vehicles()` stable scan
//! 作为 oracle，比较 caller-owned filtered materialization、ordered dirty delta +
//! retained cache，以及只在单一 committed epoch 内有效的 stable cursor/page。
//! 所有 record、epoch、cursor 与 selection shape 都是研究工具，不进入 production API。

use std::{alloc::System, hint::black_box, mem::size_of, sync::Mutex, time::Instant};

use stats_alloc::{INSTRUMENTED_SYSTEM, Region, Stats, StatsAlloc};

use super::*;
use crate::{
    EdgeLength, IidmProfileSpec, InitialTrafficData, LaneEdge, LeaveParkingInput,
    ParkedVehicleSpawnInput, ParkingRegistry, ParkingSpace, ParkingSpaceGeometry,
    ParkingSpaceHandle, Route, SignalRegistry, SpeedLimit, VehicleParkingState, VehicleProfile,
    VehicleProfileHandle, VehicleProfileRegistry,
};

#[global_allocator]
static GLOBAL: &StatsAlloc<System> = &INSTRUMENTED_SYSTEM;
static ALLOCATION_LOCK: Mutex<()> = Mutex::new(());

const SCALE_COUNTS: [usize; 2] = [10_000, 100_000];
const SELECTION_BASIS_POINTS: [usize; 6] = [0, 10, 100, 1_000, 5_000, 10_000];
const CURSOR_PAGE_SIZE: usize = 1_024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ResearchPoseSource {
    Lane {
        edge: EdgeHandle,
        progress_bits: u64,
    },
    Parking {
        space: ParkingSpaceHandle,
    },
    None,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ResearchVehicleRecord {
    stable_position: usize,
    handle: VehicleHandle,
    profile: VehicleProfileHandle,
    status: VehicleStatus,
    route: RouteHandle,
    route_edge_index: usize,
    edge: EdgeHandle,
    edge_progress_bits: u64,
    current_speed_bits: u64,
    applied_acceleration_bits: u64,
    parking: VehicleParkingState,
    pose_source: ResearchPoseSource,
}

impl ResearchVehicleRecord {
    fn from_committed(world: &CoreWorld, stable_position: usize, vehicle: &VehicleState) -> Self {
        let edge = world
            .route_edges(vehicle.route)
            .and_then(|edges| edges.get(vehicle.route_edge_index))
            .copied()
            .expect("live vehicle route occurrence must resolve");
        let parking = world
            .parking_snapshot()
            .vehicle_state(vehicle.handle)
            .expect("live vehicle must have a Parking view");
        let pose_source = match (vehicle.status, parking) {
            (VehicleStatus::Active | VehicleStatus::Stopped, _) => ResearchPoseSource::Lane {
                edge,
                progress_bits: vehicle.edge_progress.value().to_bits(),
            },
            (VehicleStatus::Parked, VehicleParkingState::Occupied { space }) => {
                ResearchPoseSource::Parking { space }
            }
            (VehicleStatus::Completed, _) => ResearchPoseSource::None,
            (VehicleStatus::Parked, _) => {
                panic!("Parked vehicle must have an Occupied Parking binding")
            }
        };

        Self {
            stable_position,
            handle: vehicle.handle,
            profile: vehicle.profile,
            status: vehicle.status,
            route: vehicle.route,
            route_edge_index: vehicle.route_edge_index,
            edge,
            edge_progress_bits: vehicle.edge_progress.value().to_bits(),
            current_speed_bits: vehicle.current_speed.value().to_bits(),
            applied_acceleration_bits: vehicle.applied_acceleration.value().to_bits(),
            parking,
            pose_source,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct ReadMetrics {
    scanned: usize,
    emitted: usize,
    pages: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ResearchSelection {
    membership: Vec<bool>,
}

impl ResearchSelection {
    fn none(vehicle_count: usize) -> Self {
        Self {
            membership: vec![false; vehicle_count],
        }
    }

    fn all(vehicle_count: usize) -> Self {
        Self {
            membership: vec![true; vehicle_count],
        }
    }

    fn from_handles(world: &CoreWorld, handles: impl IntoIterator<Item = VehicleHandle>) -> Self {
        let mut selection = Self::none(world.vehicles().count());
        for handle in handles {
            let position = world
                .vehicles()
                .position(|vehicle| vehicle.handle == handle)
                .expect("selection handle must be live");
            selection.membership[position] = true;
        }
        selection
    }

    fn for_shape(world: &CoreWorld, selected: usize, shape: SelectionShape) -> ResearchSelection {
        let vehicle_count = world.vehicles().count();
        assert!(selected <= vehicle_count);
        let mut selection = Self::none(vehicle_count);
        if selected == 0 {
            return selection;
        }

        match shape {
            SelectionShape::Contiguous => {
                selection.membership[..selected].fill(true);
            }
            SelectionShape::Alternating => {
                let mut previous = 0;
                for (position, included) in selection.membership.iter_mut().enumerate() {
                    let current = (position + 1) * selected / vehicle_count;
                    if current != previous {
                        *included = true;
                    }
                    previous = current;
                }
            }
            SelectionShape::StableHash => {
                let mut ranked = world
                    .vehicles()
                    .enumerate()
                    .map(|(position, vehicle)| {
                        let key = (vehicle.handle.index() as u64)
                            .wrapping_mul(0x9e37_79b9_7f4a_7c15)
                            .rotate_left(17)
                            ^ u64::from(vehicle.handle.generation());
                        (key, position)
                    })
                    .collect::<Vec<_>>();
                ranked.sort_unstable();
                for (_, position) in ranked.into_iter().take(selected) {
                    selection.membership[position] = true;
                }
            }
        }
        assert_eq!(
            selection
                .membership
                .iter()
                .filter(|included| **included)
                .count(),
            selected
        );
        selection
    }

    fn includes(&self, stable_position: usize) -> bool {
        self.membership
            .get(stable_position)
            .copied()
            .unwrap_or(false)
    }

    fn retained_bytes(&self) -> usize {
        self.membership.capacity() * size_of::<bool>()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SelectionShape {
    Contiguous,
    Alternating,
    StableHash,
}

impl SelectionShape {
    const ALL: [Self; 3] = [Self::Contiguous, Self::Alternating, Self::StableHash];

    const fn name(self) -> &'static str {
        match self {
            Self::Contiguous => "contiguous",
            Self::Alternating => "alternating",
            Self::StableHash => "stable-hash",
        }
    }
}

fn materialize_filtered(
    world: &CoreWorld,
    selection: &ResearchSelection,
    output: &mut Vec<ResearchVehicleRecord>,
) -> ReadMetrics {
    output.clear();
    let mut metrics = ReadMetrics::default();
    for (stable_position, vehicle) in world.vehicles().enumerate() {
        metrics.scanned += 1;
        if selection.includes(stable_position) {
            output.push(ResearchVehicleRecord::from_committed(
                world,
                stable_position,
                vehicle,
            ));
            metrics.emitted += 1;
        }
    }
    metrics
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DirtyOperation {
    Remove {
        stable_position: usize,
        handle: VehicleHandle,
    },
    Upsert(ResearchVehicleRecord),
}

impl DirtyOperation {
    const fn stable_position(self) -> usize {
        match self {
            Self::Remove {
                stable_position, ..
            } => stable_position,
            Self::Upsert(record) => record.stable_position,
        }
    }

    const fn operation_order(self) -> usize {
        match self {
            Self::Remove { .. } => 0,
            Self::Upsert(_) => 1,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct DirtyDelta {
    operations: Vec<DirtyOperation>,
}

impl DirtyDelta {
    fn with_capacity(capacity: usize) -> Self {
        Self {
            operations: Vec::with_capacity(capacity),
        }
    }

    fn retained_bytes(&self) -> usize {
        self.operations.capacity() * size_of::<DirtyOperation>()
    }

    fn counts(&self) -> (usize, usize) {
        self.operations
            .iter()
            .fold((0, 0), |(removes, upserts), operation| match operation {
                DirtyOperation::Remove { .. } => (removes + 1, upserts),
                DirtyOperation::Upsert(_) => (removes, upserts + 1),
            })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct DirtyIndex {
    previous_by_slot: Vec<Option<(VehicleHandle, usize)>>,
    current_by_slot: Vec<Option<(VehicleHandle, usize)>>,
}

impl DirtyIndex {
    fn with_slot_capacity(slot_capacity: usize) -> Self {
        Self {
            previous_by_slot: vec![None; slot_capacity],
            current_by_slot: vec![None; slot_capacity],
        }
    }

    fn prepare(&mut self, previous: &[ResearchVehicleRecord], current: &[ResearchVehicleRecord]) {
        let required = previous
            .iter()
            .chain(current)
            .map(|record| record.handle.index() + 1)
            .max()
            .unwrap_or(0);
        if self.previous_by_slot.len() < required {
            self.previous_by_slot.resize(required, None);
            self.current_by_slot.resize(required, None);
        }
        self.previous_by_slot.fill(None);
        self.current_by_slot.fill(None);
        for (position, record) in previous.iter().enumerate() {
            self.previous_by_slot[record.handle.index()] = Some((record.handle, position));
        }
        for (position, record) in current.iter().enumerate() {
            self.current_by_slot[record.handle.index()] = Some((record.handle, position));
        }
    }

    fn retained_bytes(&self) -> usize {
        (self.previous_by_slot.capacity() + self.current_by_slot.capacity())
            * size_of::<Option<(VehicleHandle, usize)>>()
    }
}

fn build_dirty_delta(
    previous: &[ResearchVehicleRecord],
    current: &[ResearchVehicleRecord],
    delta: &mut DirtyDelta,
    index: &mut DirtyIndex,
) {
    delta.operations.clear();
    index.prepare(previous, current);

    for old in previous {
        let current_record = index.current_by_slot[old.handle.index()]
            .filter(|(handle, _)| *handle == old.handle)
            .map(|(_, position)| &current[position]);
        if current_record.is_none() {
            delta.operations.push(DirtyOperation::Remove {
                stable_position: old.stable_position,
                handle: old.handle,
            });
        }
    }
    for record in current {
        let previous_record = index.previous_by_slot[record.handle.index()]
            .filter(|(handle, _)| *handle == record.handle)
            .map(|(_, position)| &previous[position]);
        if previous_record != Some(record) {
            delta.operations.push(DirtyOperation::Upsert(*record));
        }
    }

    delta
        .operations
        .sort_by_key(|operation| (operation.stable_position(), operation.operation_order()));
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DirtyApplyError {
    MissingRemove(VehicleHandle),
    StablePositionCollision(usize),
    ReconstructedSnapshotMismatch,
}

fn apply_dirty_atomically(
    cache: &mut Vec<ResearchVehicleRecord>,
    scratch: &mut Vec<ResearchVehicleRecord>,
    delta: &DirtyDelta,
    expected: &[ResearchVehicleRecord],
) -> Result<(), DirtyApplyError> {
    scratch.clear();
    scratch.extend_from_slice(cache);

    for operation in delta.operations.iter().copied() {
        match operation {
            DirtyOperation::Remove { handle, .. } => {
                let Some(position) = scratch.iter().position(|record| record.handle == handle)
                else {
                    return Err(DirtyApplyError::MissingRemove(handle));
                };
                scratch.remove(position);
            }
            DirtyOperation::Upsert(record) => {
                if let Some(position) = scratch
                    .iter()
                    .position(|current| current.handle == record.handle)
                {
                    scratch[position] = record;
                    continue;
                }
                let insertion = scratch
                    .partition_point(|current| current.stable_position < record.stable_position);
                if scratch
                    .get(insertion)
                    .is_some_and(|current| current.stable_position == record.stable_position)
                {
                    return Err(DirtyApplyError::StablePositionCollision(
                        record.stable_position,
                    ));
                }
                scratch.insert(insertion, record);
            }
        }
    }

    if scratch.as_slice() != expected {
        return Err(DirtyApplyError::ReconstructedSnapshotMismatch);
    }
    std::mem::swap(cache, scratch);
    scratch.clear();
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ResearchReadEpoch(u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ResearchCursor {
    epoch: ResearchReadEpoch,
    next_stable_position: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CursorError {
    Stale {
        cursor: ResearchReadEpoch,
        current: ResearchReadEpoch,
    },
    ZeroPageSize,
}

#[derive(Clone, Copy, Debug, Default)]
struct ResearchReader {
    committed_generation: u64,
}

impl ResearchReader {
    const fn epoch(self) -> ResearchReadEpoch {
        ResearchReadEpoch(self.committed_generation)
    }

    const fn cursor(self) -> ResearchCursor {
        ResearchCursor {
            epoch: self.epoch(),
            next_stable_position: 0,
        }
    }

    fn note_successful_mutation(&mut self) {
        self.committed_generation = self.committed_generation.wrapping_add(1);
    }

    fn page(
        self,
        world: &CoreWorld,
        selection: &ResearchSelection,
        cursor: ResearchCursor,
        page_size: usize,
        output: &mut Vec<ResearchVehicleRecord>,
    ) -> Result<(Option<ResearchCursor>, ReadMetrics), CursorError> {
        if cursor.epoch != self.epoch() {
            return Err(CursorError::Stale {
                cursor: cursor.epoch,
                current: self.epoch(),
            });
        }
        if page_size == 0 {
            return Err(CursorError::ZeroPageSize);
        }

        output.clear();
        let mut metrics = ReadMetrics {
            pages: 1,
            ..ReadMetrics::default()
        };
        let mut next_position = None;
        for (stable_position, vehicle) in world
            .vehicles()
            .enumerate()
            .skip(cursor.next_stable_position)
        {
            metrics.scanned += 1;
            if !selection.includes(stable_position) {
                continue;
            }
            if output.len() == page_size {
                next_position = Some(stable_position);
                break;
            }
            output.push(ResearchVehicleRecord::from_committed(
                world,
                stable_position,
                vehicle,
            ));
            metrics.emitted += 1;
        }

        Ok((
            next_position.map(|next_stable_position| ResearchCursor {
                epoch: cursor.epoch,
                next_stable_position,
            }),
            metrics,
        ))
    }
}

fn collect_cursor(
    reader: ResearchReader,
    world: &CoreWorld,
    selection: &ResearchSelection,
    page_size: usize,
    output: &mut Vec<ResearchVehicleRecord>,
    page: &mut Vec<ResearchVehicleRecord>,
) -> Result<ReadMetrics, CursorError> {
    output.clear();
    let mut cursor = Some(reader.cursor());
    let mut metrics = ReadMetrics::default();
    while let Some(current) = cursor {
        let (next, page_metrics) = reader.page(world, selection, current, page_size, page)?;
        output.extend_from_slice(page);
        metrics.scanned += page_metrics.scanned;
        metrics.emitted += page_metrics.emitted;
        metrics.pages += page_metrics.pages;
        cursor = next;
    }
    Ok(metrics)
}

fn research_profile(id: &str) -> (VehicleProfileRegistry, VehicleProfileHandle) {
    let profiles = VehicleProfileRegistry::try_new([VehicleProfile::try_new_iidm(
        id,
        IidmProfileSpec {
            length: 0.1,
            desired_speed: 20.0,
            min_gap: 0.0,
            time_headway: 1.0,
            max_acceleration: 1.0,
            comfortable_deceleration: 2.0,
            emergency_deceleration: 4.0,
        },
    )
    .expect("research profile")])
    .expect("research profile registry");
    let handle = profiles
        .profile_handle(id)
        .expect("research profile handle");
    (profiles, handle)
}

fn research_edge(id: &str, length: f64) -> LaneEdge {
    LaneEdge::new(
        id,
        EdgeLength::try_new(length).expect("research edge length"),
        SpeedLimit::try_new(f64::MAX).expect("research speed limit"),
        std::iter::empty::<&str>(),
    )
}

struct MixedFixture {
    world: CoreWorld,
    profile: VehicleProfileHandle,
    route: RouteHandle,
    space: ParkingSpaceHandle,
    active: VehicleHandle,
    completed: VehicleHandle,
    parked: VehicleHandle,
}

fn mixed_world() -> MixedFixture {
    let graph =
        LaneGraph::try_new([research_edge("research-edge", 1_000.0)]).expect("research graph");
    let parking = ParkingRegistry::try_new(
        &graph,
        [],
        [ParkingSpace::new(
            "research-space",
            None,
            "research-edge",
            700.0,
            "research-edge",
            750.0,
            ParkingSpaceGeometry::new(-3.0, 0.0, 4.5, 2.4),
        )],
    )
    .expect("research Parking registry");
    let (profiles, profile) = research_profile("research-profile");
    let traffic = InitialTrafficData::try_new_with_signals_and_parking(
        graph,
        [Route::try_new("research-route", ["research-edge"]).expect("research route")],
        profiles,
        SignalRegistry::empty(),
        parking,
    )
    .expect("research traffic data");
    let mut world = CoreWorld::with_traffic_data(
        16,
        traffic,
        vec![
            VehicleSpawnInput::active(
                "active",
                profile,
                "research-route",
                0,
                EdgeProgress::try_new(100.0).expect("active progress"),
                Speed::try_new(7.5).expect("active speed"),
            ),
            VehicleSpawnInput::completed(
                "completed",
                profile,
                "research-route",
                0,
                EdgeProgress::try_new(1_000.0).expect("completed progress"),
            ),
            VehicleSpawnInput::stopped(
                "stopped",
                profile,
                "research-route",
                0,
                EdgeProgress::try_new(400.0).expect("stopped progress"),
            ),
        ],
    )
    .expect("mixed research world");
    let route = world
        .route_handle("research-route")
        .expect("research route handle");
    let space = world
        .parking()
        .space_handle("research-space")
        .expect("research space handle");
    let parked = world
        .spawn_parked_vehicle(ParkedVehicleSpawnInput {
            id: "parked".to_owned(),
            profile,
            route_id: "research-route".to_owned(),
            route_edge_index: 0,
            space,
        })
        .expect("parked research vehicle")
        .vehicle;

    MixedFixture {
        active: world.vehicle_handle("active").expect("active handle"),
        completed: world.vehicle_handle("completed").expect("completed handle"),
        world,
        profile,
        route,
        space,
        parked,
    }
}

fn scale_world(vehicle_count: usize) -> CoreWorld {
    let spacing = 0.25;
    let edge_length = spacing * vehicle_count as f64;
    let graph =
        LaneGraph::try_new([research_edge("scale-edge", edge_length)]).expect("scale graph");
    let (profiles, profile) = research_profile("scale-profile");
    let traffic = InitialTrafficData::try_new(
        graph,
        [Route::try_new("scale-route", ["scale-edge"]).expect("scale route")],
        profiles,
    )
    .expect("scale traffic");
    let vehicles = (0..vehicle_count)
        .map(|index| {
            VehicleSpawnInput::stopped(
                format!("vehicle-{index:06}"),
                profile,
                "scale-route",
                0,
                EdgeProgress::try_new(spacing * (index as f64 + 0.5)).expect("scale progress"),
            )
        })
        .collect();
    CoreWorld::with_traffic_data(16, traffic, vehicles).expect("scale world")
}

#[test]
fn filtered_dirty_and_cursor_preserve_committed_records_and_global_order() {
    let fixture = mixed_world();
    let world = &fixture.world;
    let handles = world
        .vehicles()
        .map(|vehicle| vehicle.handle)
        .collect::<Vec<_>>();
    let forward = ResearchSelection::from_handles(
        world,
        [handles[0], handles[2], handles[handles.len() - 1]],
    );
    let reverse = ResearchSelection::from_handles(
        world,
        [handles[handles.len() - 1], handles[2], handles[0]],
    );
    assert_eq!(
        forward, reverse,
        "selection construction order is not semantic"
    );

    let mut oracle = Vec::with_capacity(handles.len());
    let metrics = materialize_filtered(world, &forward, &mut oracle);
    assert_eq!(metrics.scanned, handles.len());
    assert_eq!(metrics.emitted, 3);
    assert!(
        oracle
            .windows(2)
            .all(|records| records[0].stable_position < records[1].stable_position)
    );
    assert!(oracle.iter().any(|record| {
        record.status == VehicleStatus::Parked
            && matches!(
                record.pose_source,
                ResearchPoseSource::Parking { space } if space == fixture.space
            )
    }));

    let reader = ResearchReader::default();
    let mut paged = Vec::with_capacity(oracle.len());
    let mut page = Vec::with_capacity(oracle.len());
    for page_size in [1, 64, 1_024, oracle.len().max(1), usize::MAX] {
        collect_cursor(reader, world, &forward, page_size, &mut paged, &mut page)
            .expect("same-epoch cursor");
        assert_eq!(paged, oracle, "page size {page_size} must preserve oracle");
    }

    let mut cache = Vec::with_capacity(handles.len());
    let mut cache_scratch = Vec::with_capacity(handles.len());
    let mut delta = DirtyDelta::with_capacity(handles.len() * 2);
    let mut dirty_index = DirtyIndex::with_slot_capacity(handles.len());
    build_dirty_delta(&cache, &oracle, &mut delta, &mut dirty_index);
    apply_dirty_atomically(&mut cache, &mut cache_scratch, &delta, &oracle)
        .expect("initial dirty cache build");
    assert_eq!(cache, oracle);

    build_dirty_delta(&cache, &oracle, &mut delta, &mut dirty_index);
    assert!(delta.operations.is_empty(), "no-change delta must be empty");
    apply_dirty_atomically(&mut cache, &mut cache_scratch, &delta, &oracle)
        .expect("no-change dirty cache apply");
    assert_eq!(cache, oracle);
}

#[test]
fn lifecycle_dirty_delta_covers_parking_despawn_spawn_and_atomic_replace() {
    let MixedFixture {
        mut world,
        profile,
        route,
        space,
        active,
        completed,
        parked,
    } = mixed_world();
    let selection = ResearchSelection::all(world.vehicles().count());
    let mut previous = Vec::with_capacity(16);
    let mut current = Vec::with_capacity(16);
    let mut cache_scratch = Vec::with_capacity(16);
    let mut delta = DirtyDelta::with_capacity(32);
    let mut dirty_index = DirtyIndex::with_slot_capacity(16);
    materialize_filtered(&world, &selection, &mut previous);

    world
        .leave_parking(LeaveParkingInput {
            vehicle: parked,
            space,
            route,
            route_edge_index: 0,
        })
        .expect("Parking source switch");
    let selection = ResearchSelection::all(world.vehicles().count());
    materialize_filtered(&world, &selection, &mut current);
    build_dirty_delta(&previous, &current, &mut delta, &mut dirty_index);
    assert!(delta.operations.iter().any(|operation| {
        matches!(
            operation,
            DirtyOperation::Upsert(record)
                if record.handle == parked
                    && matches!(record.pose_source, ResearchPoseSource::Lane { .. })
        )
    }));
    apply_dirty_atomically(&mut previous, &mut cache_scratch, &delta, &current)
        .expect("Parking source switch cache apply");

    let arriving = world
        .spawn_vehicle(VehicleSpawnInput::active(
            "arriving",
            profile,
            "research-route",
            0,
            EdgeProgress::try_new(700.0).expect("arrival progress"),
            Speed::ZERO,
        ))
        .expect("arrival spawn");
    world
        .reserve_parking_space(arriving, space)
        .expect("arrival reservation");
    world
        .commit_parking(arriving, space)
        .expect("Parking arrival commit");
    let selection = ResearchSelection::all(world.vehicles().count());
    materialize_filtered(&world, &selection, &mut current);
    build_dirty_delta(&previous, &current, &mut delta, &mut dirty_index);
    assert!(delta.operations.iter().any(|operation| {
        matches!(
            operation,
            DirtyOperation::Upsert(record)
                if record.handle == arriving
                    && matches!(
                        record.pose_source,
                        ResearchPoseSource::Parking { space: actual } if actual == space
                    )
        )
    }));
    apply_dirty_atomically(&mut previous, &mut cache_scratch, &delta, &current)
        .expect("Parking arrival cache apply");

    let removed_position = previous
        .iter()
        .find(|record| record.handle == active)
        .expect("active record")
        .stable_position;
    world.despawn_vehicle(active).expect("active despawn");
    let spawned = world
        .spawn_vehicle(VehicleSpawnInput::stopped(
            "spawned",
            profile,
            "research-route",
            0,
            EdgeProgress::try_new(200.0).expect("spawned progress"),
        ))
        .expect("replacement spawn");
    let selection = ResearchSelection::all(world.vehicles().count());
    materialize_filtered(&world, &selection, &mut current);
    build_dirty_delta(&previous, &current, &mut delta, &mut dirty_index);
    assert!(delta.operations.iter().any(|operation| {
        matches!(
            operation,
            DirtyOperation::Remove { handle, .. } if *handle == active
        )
    }));
    assert!(delta.operations.iter().any(|operation| {
        matches!(operation, DirtyOperation::Upsert(record) if record.handle == spawned)
    }));
    assert!(current.iter().all(|record| record.handle != active));
    assert!(
        current
            .iter()
            .any(|record| record.stable_position == removed_position),
        "compaction may reuse the logical position but ordering remains current-world defined"
    );
    apply_dirty_atomically(&mut previous, &mut cache_scratch, &delta, &current)
        .expect("despawn/spawn cache apply");

    let completed_position = previous
        .iter()
        .find(|record| record.handle == completed)
        .expect("completed record")
        .stable_position;
    let outcome = world
        .replace_completed_vehicle(
            completed,
            &VehicleReplaceInput::new(
                VehicleReplaceExternalId::Preserve,
                profile,
                route,
                0,
                EdgeProgress::try_new(850.0).expect("replacement progress"),
                Speed::ZERO,
            ),
        )
        .expect("completed replacement");
    let VehicleReplaceOutcome::Replaced(replaced) = outcome else {
        panic!("non-overlapping replacement must commit")
    };
    let selection = ResearchSelection::all(world.vehicles().count());
    materialize_filtered(&world, &selection, &mut current);
    build_dirty_delta(&previous, &current, &mut delta, &mut dirty_index);
    let old_operation = delta.operations.iter().position(|operation| {
        matches!(
            operation,
            DirtyOperation::Remove {
                stable_position,
                handle
            } if *stable_position == completed_position && *handle == replaced.old
        )
    });
    let new_operation = delta.operations.iter().position(|operation| {
        matches!(
            operation,
            DirtyOperation::Upsert(record)
                if record.stable_position == completed_position
                    && record.handle == replaced.new
        )
    });
    assert!(
        old_operation.is_some_and(|old| new_operation.is_some_and(|new| old < new)),
        "atomic replace must be old removal followed by new upsert at the stable position"
    );
    apply_dirty_atomically(&mut previous, &mut cache_scratch, &delta, &current)
        .expect("atomic replacement cache apply");
    assert_eq!(previous, current);
}

#[test]
fn committed_edge_transition_updates_exact_record_and_stales_old_cursor() {
    let graph = LaneGraph::try_new([
        LaneEdge::new(
            "transition-a",
            EdgeLength::try_new(10.0).expect("transition edge length"),
            SpeedLimit::try_new(f64::MAX).expect("transition speed limit"),
            ["transition-b"],
        ),
        research_edge("transition-b", 10.0),
    ])
    .expect("transition graph");
    let edge_a = graph.edge_handle("transition-a").expect("edge A");
    let edge_b = graph.edge_handle("transition-b").expect("edge B");
    let (profiles, profile) = research_profile("transition-profile");
    let traffic = InitialTrafficData::try_new(
        graph,
        [
            Route::try_new("transition-route", ["transition-a", "transition-b"])
                .expect("transition route"),
        ],
        profiles,
    )
    .expect("transition traffic");
    let mut world = CoreWorld::with_traffic_data(
        1_000,
        traffic,
        vec![VehicleSpawnInput::active(
            "transition-vehicle",
            profile,
            "transition-route",
            0,
            EdgeProgress::try_new(9.0).expect("transition progress"),
            Speed::try_new(2.0).expect("transition speed"),
        )],
    )
    .expect("transition world");
    let selection = ResearchSelection::all(1);
    let mut previous = Vec::with_capacity(1);
    let mut current = Vec::with_capacity(1);
    materialize_filtered(&world, &selection, &mut previous);
    assert_eq!(previous[0].edge, edge_a);

    let mut reader = ResearchReader::default();
    let cursor = reader.cursor();
    let tick_before = world.tick_index();
    world
        .step(TickInput::new(1_000))
        .expect("committed transition step");
    assert_eq!(world.tick_index(), tick_before + 1);
    reader.note_successful_mutation();
    assert!(matches!(
        reader.page(&world, &selection, cursor, 1, &mut current),
        Err(CursorError::Stale { .. })
    ));

    materialize_filtered(&world, &selection, &mut current);
    assert_eq!(current[0].edge, edge_b);
    assert!(matches!(
        current[0].pose_source,
        ResearchPoseSource::Lane { edge, .. } if edge == edge_b
    ));
    let mut delta = DirtyDelta::with_capacity(2);
    let mut dirty_index = DirtyIndex::with_slot_capacity(1);
    build_dirty_delta(&previous, &current, &mut delta, &mut dirty_index);
    assert_eq!(delta.counts(), (0, 1));
    let mut scratch = Vec::with_capacity(1);
    apply_dirty_atomically(&mut previous, &mut scratch, &delta, &current)
        .expect("edge transition cache apply");
    assert_eq!(previous, current);
}

#[test]
fn cursor_epoch_and_dirty_apply_reject_stale_or_partial_results_atomically() {
    let MixedFixture {
        mut world,
        profile,
        completed,
        route,
        ..
    } = mixed_world();
    let selection = ResearchSelection::all(world.vehicles().count());
    let mut reader = ResearchReader::default();
    let cursor = reader.cursor();
    let mut page = Vec::with_capacity(16);
    reader
        .page(&world, &selection, cursor, 1, &mut page)
        .expect("initial cursor page");

    world.replace_failure_after_prepare = true;
    let before_failed_replace = world.clone();
    let mut fresh = world.clone();
    fresh.replace_failure_after_prepare = false;
    let replacement = VehicleReplaceInput::new(
        VehicleReplaceExternalId::Preserve,
        profile,
        route,
        0,
        EdgeProgress::try_new(850.0).expect("replacement progress"),
        Speed::ZERO,
    );
    assert!(
        world
            .replace_completed_vehicle(completed, &replacement)
            .is_err()
    );
    assert_eq!(world, before_failed_replace);
    reader
        .page(&world, &selection, cursor, 1, &mut page)
        .expect("failed atomic command must not stale cursor");

    world.replace_failure_after_prepare = false;
    let retry = world
        .replace_completed_vehicle(completed, &replacement)
        .expect("retry replacement");
    let fresh_result = fresh
        .replace_completed_vehicle(completed, &replacement)
        .expect("fresh replacement");
    assert_eq!(retry, fresh_result);
    assert_eq!(world, fresh, "retry must equal fresh replay");
    reader.note_successful_mutation();
    assert!(matches!(
        reader.page(&world, &selection, cursor, 1, &mut page),
        Err(CursorError::Stale { .. })
    ));

    let current_selection = ResearchSelection::all(world.vehicles().count());
    let mut cache = Vec::with_capacity(16);
    let mut expected = Vec::with_capacity(16);
    materialize_filtered(&world, &current_selection, &mut cache);
    expected.extend_from_slice(&cache);
    let before_cache = cache.clone();
    let mut scratch = Vec::with_capacity(16);
    let handle = cache[0].handle;
    let malformed = DirtyDelta {
        operations: vec![
            DirtyOperation::Remove {
                stable_position: cache[0].stable_position,
                handle,
            },
            DirtyOperation::Remove {
                stable_position: cache[0].stable_position,
                handle,
            },
        ],
    };
    assert_eq!(
        apply_dirty_atomically(&mut cache, &mut scratch, &malformed, &expected),
        Err(DirtyApplyError::MissingRemove(handle))
    );
    assert_eq!(cache, before_cache, "failed dirty apply keeps old cache");
}

#[test]
#[ignore = "explicit release-mode 10k/100k research matrix"]
fn scale_matrix_reports_selection_dirty_cursor_memory_and_cost_shape() {
    for vehicle_count in SCALE_COUNTS {
        let world = scale_world(vehicle_count);
        let reader = ResearchReader::default();
        let mut filtered = Vec::with_capacity(vehicle_count);
        let mut cursor_output = Vec::with_capacity(vehicle_count);
        let mut page = Vec::with_capacity(CURSOR_PAGE_SIZE);
        let mut cache = Vec::with_capacity(vehicle_count);
        let mut cache_scratch = Vec::with_capacity(vehicle_count);
        let mut delta = DirtyDelta::with_capacity(vehicle_count * 2);
        let mut dirty_index = DirtyIndex::with_slot_capacity(vehicle_count);
        let mut churn = Vec::with_capacity(vehicle_count);

        for basis_points in SELECTION_BASIS_POINTS {
            let selected = vehicle_count * basis_points / 10_000;
            for shape in SelectionShape::ALL {
                let selection = ResearchSelection::for_shape(&world, selected, shape);
                let started = Instant::now();
                let filtered_metrics = materialize_filtered(&world, &selection, &mut filtered);
                let filtered_ns = started.elapsed().as_nanos();

                let started = Instant::now();
                let cursor_metrics = collect_cursor(
                    reader,
                    &world,
                    &selection,
                    CURSOR_PAGE_SIZE,
                    &mut cursor_output,
                    &mut page,
                )
                .expect("same-epoch scale cursor");
                let cursor_ns = started.elapsed().as_nanos();
                assert_eq!(cursor_output, filtered);

                cache.clear();
                cache.extend_from_slice(&filtered);
                let started = Instant::now();
                build_dirty_delta(&cache, &filtered, &mut delta, &mut dirty_index);
                let dirty_no_change_ns = started.elapsed().as_nanos();
                assert!(delta.operations.is_empty());
                apply_dirty_atomically(&mut cache, &mut cache_scratch, &delta, &filtered)
                    .expect("no-change scale dirty apply");

                let churn_shape = match shape {
                    SelectionShape::Contiguous => SelectionShape::Alternating,
                    SelectionShape::Alternating => SelectionShape::StableHash,
                    SelectionShape::StableHash => SelectionShape::Contiguous,
                };
                let churn_selection = ResearchSelection::for_shape(&world, selected, churn_shape);
                materialize_filtered(&world, &churn_selection, &mut churn);
                build_dirty_delta(&cache, &churn, &mut delta, &mut dirty_index);
                let (removes, upserts) = delta.counts();
                apply_dirty_atomically(&mut cache, &mut cache_scratch, &delta, &churn)
                    .expect("selection-churn scale dirty apply");

                println!(
                    "P3_READ_METRICS vehicles={vehicle_count} basis_points={basis_points} shape={} scanned={} emitted={} pages={} removes={} upserts={} selection_bytes={} output_bytes={} dirty_bytes={} dirty_index_bytes={} cache_bytes={} cursor_bytes={} filtered_ns={filtered_ns} cursor_ns={cursor_ns} dirty_no_change_ns={dirty_no_change_ns}",
                    shape.name(),
                    filtered_metrics.scanned,
                    filtered_metrics.emitted,
                    cursor_metrics.pages,
                    removes,
                    upserts,
                    selection.retained_bytes(),
                    filtered.capacity() * size_of::<ResearchVehicleRecord>(),
                    delta.retained_bytes(),
                    dirty_index.retained_bytes(),
                    cache.capacity() * size_of::<ResearchVehicleRecord>(),
                    size_of::<ResearchCursor>(),
                );
            }
        }
    }
}

#[test]
#[ignore = "global allocator measurement requires explicit serial execution"]
fn warm_10k_and_100k_filtered_dirty_cursor_paths_are_zero_allocation() {
    let _guard = ALLOCATION_LOCK.lock().expect("allocation lock");

    for vehicle_count in SCALE_COUNTS {
        let world = scale_world(vehicle_count);
        let selection =
            ResearchSelection::for_shape(&world, vehicle_count / 10, SelectionShape::StableHash);
        let reader = ResearchReader::default();
        let mut filtered = Vec::with_capacity(vehicle_count);
        let mut cache = Vec::with_capacity(vehicle_count);
        let mut cache_scratch = Vec::with_capacity(vehicle_count);
        let mut page = Vec::with_capacity(CURSOR_PAGE_SIZE);
        let mut delta = DirtyDelta::with_capacity(vehicle_count * 2);
        let mut dirty_index = DirtyIndex::with_slot_capacity(vehicle_count);

        materialize_filtered(&world, &selection, &mut filtered);
        cache.extend_from_slice(&filtered);
        build_dirty_delta(&cache, &filtered, &mut delta, &mut dirty_index);
        apply_dirty_atomically(&mut cache, &mut cache_scratch, &delta, &filtered)
            .expect("warm dirty apply");
        reader
            .page(
                &world,
                &selection,
                reader.cursor(),
                CURSOR_PAGE_SIZE,
                &mut page,
            )
            .expect("warm cursor page");

        let region = Region::new(GLOBAL);
        black_box(materialize_filtered(
            black_box(&world),
            black_box(&selection),
            black_box(&mut filtered),
        ));
        build_dirty_delta(
            black_box(&cache),
            black_box(&filtered),
            black_box(&mut delta),
            black_box(&mut dirty_index),
        );
        apply_dirty_atomically(
            black_box(&mut cache),
            black_box(&mut cache_scratch),
            black_box(&delta),
            black_box(&filtered),
        )
        .expect("measured dirty apply");
        black_box(
            reader
                .page(
                    black_box(&world),
                    black_box(&selection),
                    reader.cursor(),
                    CURSOR_PAGE_SIZE,
                    black_box(&mut page),
                )
                .expect("measured cursor page"),
        );
        let stats = region.change();
        assert_zero_allocation(vehicle_count, stats);
    }
}

fn assert_zero_allocation(vehicle_count: usize, stats: Stats) {
    assert_eq!(stats.allocations, 0, "{vehicle_count}: allocations");
    assert_eq!(stats.reallocations, 0, "{vehicle_count}: reallocations");
    assert_eq!(stats.bytes_allocated, 0, "{vehicle_count}: allocated bytes");
    assert_eq!(
        stats.bytes_reallocated, 0,
        "{vehicle_count}: reallocated bytes"
    );
}
