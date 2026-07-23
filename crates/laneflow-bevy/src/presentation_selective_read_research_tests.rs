//! #210 P3 selected Adapter frame 研究原型。
//!
//! 本模块只在 `laneflow-bevy` 的单元测试构建中存在。它以 production
//! `rebuild_pose_inputs`、Spatial canonical pose batch 与 Bevy local Transform commit
//! 为 exact oracle，证明 caller-owned selection 不需要改变任何 production API。

use std::{
    alloc::System,
    hint::black_box,
    mem::{size_of, swap},
    num::NonZeroU32,
    sync::Mutex,
    time::Instant,
};

use bevy_ecs::{entity::Entity, hierarchy::ChildOf, world::World};
use bevy_transform::components::Transform;
use laneflow_core::{
    CoreWorld, EdgeLength, EdgeProgress, IidmProfileSpec, InitialTrafficData, LaneEdge, LaneGraph,
    ParkedVehicleSpawnInput, ParkingRegistry, ParkingSpace, ParkingSpaceGeometry, Route,
    SignalRegistry, Speed, SpeedLimit, VehicleHandle, VehicleParkingState, VehicleProfile,
    VehicleProfileHandle, VehicleProfileRegistry, VehicleSpawnInput, VehicleStatus,
};
use laneflow_spatial::{
    CanonicalFrameId, CanonicalPoint3F32, CanonicalPoseBatchF32, CanonicalPoseBatchScratch,
    FramePlacementToken, PoseInputRecord, PoseSource, SpatialEdgeInput, SpatialRegistry,
};
use stats_alloc::{INSTRUMENTED_SYSTEM, Region, Stats, StatsAlloc};

use super::*;
use crate::LaneFlowSessionConfig;

#[global_allocator]
static GLOBAL: &StatsAlloc<System> = &INSTRUMENTED_SYSTEM;
static ALLOCATION_LOCK: Mutex<()> = Mutex::new(());

const SCALE_COUNTS: [usize; 2] = [10_000, 100_000];
const SELECTION_BASIS_POINTS: [usize; 6] = [0, 10, 100, 1_000, 5_000, 10_000];

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

#[derive(Clone, Debug, PartialEq, Eq)]
struct AdapterSelection {
    membership: Vec<bool>,
}

impl AdapterSelection {
    fn all(vehicle_count: usize) -> Self {
        Self {
            membership: vec![true; vehicle_count],
        }
    }

    fn from_handles(core: &CoreWorld, handles: impl IntoIterator<Item = VehicleHandle>) -> Self {
        let mut selection = Self {
            membership: vec![false; core.vehicles().count()],
        };
        for handle in handles {
            let position = core
                .vehicles()
                .position(|vehicle| vehicle.handle == handle)
                .expect("selected vehicle must be live");
            selection.membership[position] = true;
        }
        selection
    }

    fn for_shape(vehicle_count: usize, selected: usize, shape: SelectionShape) -> Self {
        assert!(selected <= vehicle_count);
        let mut membership = vec![false; vehicle_count];
        if selected == 0 {
            return Self { membership };
        }
        match shape {
            SelectionShape::Contiguous => membership[..selected].fill(true),
            SelectionShape::Alternating => {
                let mut previous = 0;
                for (position, included) in membership.iter_mut().enumerate() {
                    let current = (position + 1) * selected / vehicle_count;
                    if current != previous {
                        *included = true;
                    }
                    previous = current;
                }
            }
            SelectionShape::StableHash => {
                let mut ranked = (0..vehicle_count)
                    .map(|position| {
                        let key = (position as u64)
                            .wrapping_mul(0x9e37_79b9_7f4a_7c15)
                            .rotate_left(17);
                        (key, position)
                    })
                    .collect::<Vec<_>>();
                ranked.sort_unstable();
                for (_, position) in ranked.into_iter().take(selected) {
                    membership[position] = true;
                }
            }
        }
        assert_eq!(
            membership.iter().filter(|included| **included).count(),
            selected
        );
        Self { membership }
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

fn materialize_selected_pose_inputs(
    core: &CoreWorld,
    selection: &AdapterSelection,
    output: &mut Vec<PoseInputRecord>,
) -> Result<usize, LaneFlowAdapterError> {
    output.clear();
    let parking = core.parking_snapshot();
    let mut scanned = 0;

    for (input_index, vehicle) in core.vehicles().enumerate() {
        scanned += 1;
        if !selection.includes(input_index) {
            continue;
        }
        match vehicle.status {
            VehicleStatus::Active | VehicleStatus::Stopped => {
                let route_edges = core.route_edges(vehicle.route).ok_or(
                    LaneFlowAdapterError::MissingVehicleRoute {
                        input_index,
                        vehicle: vehicle.handle,
                        route: vehicle.route,
                    },
                )?;
                let edge = route_edges.get(vehicle.route_edge_index).copied().ok_or(
                    LaneFlowAdapterError::MissingVehicleRouteEdge {
                        input_index,
                        vehicle: vehicle.handle,
                        route_edge_index: vehicle.route_edge_index,
                    },
                )?;
                output.push(PoseInputRecord::lane(
                    vehicle.handle,
                    edge,
                    vehicle.edge_progress,
                ));
            }
            VehicleStatus::Parked => {
                if let Some(VehicleParkingState::Occupied { space }) =
                    parking.vehicle_state(vehicle.handle)
                {
                    output.push(PoseInputRecord::parking(vehicle.handle, space));
                } else {
                    return Err(LaneFlowAdapterError::MissingParkedVehicleBinding {
                        input_index,
                        vehicle: vehicle.handle,
                    });
                }
            }
            VehicleStatus::Completed => {}
            status => {
                return Err(LaneFlowAdapterError::UnsupportedVehicleStatus {
                    input_index,
                    vehicle: vehicle.handle,
                    status,
                });
            }
        }
    }
    Ok(scanned)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PoseInputFingerprint {
    Lane {
        vehicle: VehicleHandle,
        edge: laneflow_core::EdgeHandle,
        progress_bits: u64,
    },
    Parking {
        vehicle: VehicleHandle,
        space: laneflow_core::ParkingSpaceHandle,
    },
}

fn input_fingerprints(inputs: &[PoseInputRecord]) -> Vec<PoseInputFingerprint> {
    inputs
        .iter()
        .copied()
        .map(|input| match input.source() {
            PoseSource::Lane { edge, progress } => PoseInputFingerprint::Lane {
                vehicle: input.vehicle(),
                edge,
                progress_bits: progress.value().to_bits(),
            },
            PoseSource::Parking { space } => PoseInputFingerprint::Parking {
                vehicle: input.vehicle(),
                space,
            },
            _ => panic!("research oracle only supports current PoseSource variants"),
        })
        .collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct CanonicalPoseFingerprint {
    vehicle: VehicleHandle,
    position: [u32; 3],
    tangent: [u32; 3],
    up: [u32; 3],
}

fn canonical_fingerprints(batch: &CanonicalPoseBatchF32) -> Vec<CanonicalPoseFingerprint> {
    batch
        .records()
        .iter()
        .copied()
        .map(canonical_fingerprint)
        .collect()
}

fn canonical_fingerprint(
    record: laneflow_spatial::CanonicalPoseRecordF32,
) -> CanonicalPoseFingerprint {
    let pose = record.pose();
    let position = pose.position();
    let tangent = pose.tangent();
    let up = pose.up();
    CanonicalPoseFingerprint {
        vehicle: record.vehicle(),
        position: [
            position.x().to_bits(),
            position.y().to_bits(),
            position.z().to_bits(),
        ],
        tangent: [
            tangent.x().to_bits(),
            tangent.y().to_bits(),
            tangent.z().to_bits(),
        ],
        up: [up.x().to_bits(), up.y().to_bits(), up.z().to_bits()],
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct TransformFingerprint {
    translation: [u32; 3],
    rotation: [u32; 4],
    scale: [u32; 3],
}

impl TransformFingerprint {
    fn from_transform(transform: &Transform) -> Self {
        let translation = transform.translation.to_array().map(f32::to_bits);
        let rotation = transform.rotation.to_array().map(f32::to_bits);
        let scale = transform.scale.to_array().map(f32::to_bits);
        Self {
            translation,
            rotation,
            scale,
        }
    }
}

struct SelectedFrameBuffers {
    committed_inputs: Vec<PoseInputRecord>,
    candidate_inputs: Vec<PoseInputRecord>,
    candidate_pose: CanonicalPoseBatchF32,
    pose_scratch: CanonicalPoseBatchScratch,
}

impl SelectedFrameBuffers {
    fn with_capacity(session: &LaneFlowSession, capacity: usize) -> Self {
        Self {
            committed_inputs: Vec::with_capacity(capacity),
            candidate_inputs: Vec::with_capacity(capacity),
            candidate_pose: CanonicalPoseBatchF32::with_capacity(
                session.spatial.frame_id().clone(),
                FramePlacementToken::new(0),
                capacity,
            ),
            pose_scratch: CanonicalPoseBatchScratch::with_capacity(capacity),
        }
    }

    fn retained_bytes(&self) -> usize {
        (self.committed_inputs.capacity() + self.candidate_inputs.capacity())
            * size_of::<PoseInputRecord>()
            + (self.candidate_pose.capacity() + self.pose_scratch.capacity())
                * size_of::<laneflow_spatial::CanonicalPoseRecordF32>()
    }
}

fn apply_selected_frame_atomically(
    session: &mut LaneFlowSession,
    world: &mut World,
    placement: LaneFlowFramePlacement,
    buffers: &mut SelectedFrameBuffers,
) -> Result<LaneFlowPresentationReport, LaneFlowAdapterError> {
    session
        .spatial
        .extract_pose_batch(
            session.core.parking(),
            placement.token,
            &buffers.candidate_inputs,
            &mut buffers.candidate_pose,
            &mut buffers.pose_scratch,
        )
        .map_err(|source| LaneFlowAdapterError::SpatialBatch { source })?;

    swap(&mut session.pose_batch, &mut buffers.candidate_pose);
    session.presentation_report = LaneFlowPresentationReport {
        pose_records: session.pose_batch.len(),
        ..LaneFlowPresentationReport::default()
    };
    let result = session.validate_and_apply_presentation(world, placement);
    match result {
        Ok(()) => {
            swap(&mut buffers.committed_inputs, &mut buffers.candidate_inputs);
            Ok(session.presentation_report)
        }
        Err(error) => {
            swap(&mut session.pose_batch, &mut buffers.candidate_pose);
            session.presentation_report.applied_records = 0;
            session.transform_staging.clear();
            Err(error)
        }
    }
}

struct MixedAdapterFixture {
    session: LaneFlowSession,
    world: World,
    placement: LaneFlowFramePlacement,
    active: VehicleHandle,
    stopped: VehicleHandle,
    completed: VehicleHandle,
    parked: VehicleHandle,
    active_entity: Entity,
    parked_entity: Entity,
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
    .expect("research profiles");
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

fn mixed_adapter_fixture() -> MixedAdapterFixture {
    let graph =
        LaneGraph::try_new([research_edge("research-edge", 1_000.0)]).expect("research graph");
    let edge = graph.edge_handle("research-edge").expect("research edge");
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
            ParkingSpaceGeometry::new(-3.0, 0.25, 4.5, 2.4),
        )],
    )
    .expect("research Parking registry");
    let (profiles, profile) = research_profile("research-profile");
    let traffic = InitialTrafficData::try_new_with_signals_and_parking(
        graph.clone(),
        [Route::try_new("research-route", ["research-edge"]).expect("research route")],
        profiles,
        SignalRegistry::empty(),
        parking,
    )
    .expect("research traffic");
    let mut core = CoreWorld::with_traffic_data(
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
    .expect("mixed Adapter Core");
    let space = core
        .parking()
        .space_handle("research-space")
        .expect("research space");
    let parked = core
        .spawn_parked_vehicle(ParkedVehicleSpawnInput {
            id: "parked".to_owned(),
            profile,
            route_id: "research-route".to_owned(),
            route_edge_index: 0,
            space,
        })
        .expect("parked vehicle")
        .vehicle;
    let active = core.vehicle_handle("active").expect("active");
    let stopped = core.vehicle_handle("stopped").expect("stopped");
    let completed = core.vehicle_handle("completed").expect("completed");
    let points = [
        CanonicalPoint3F32::try_new(0.0, 0.0, 0.0).expect("research point"),
        CanonicalPoint3F32::try_new(1_000.0, 0.0, 0.0).expect("research point"),
    ];
    let spatial = SpatialRegistry::try_new(
        &graph,
        CanonicalFrameId::try_new("research:p3").expect("research frame"),
        [SpatialEdgeInput::new(edge, &points)],
    )
    .expect("research Spatial registry");
    let config = LaneFlowSessionConfig::new(NonZeroU32::new(1).expect("non-zero"));
    let mut session = LaneFlowSession::with_pose_capacity(core, spatial, config, 4);
    let mut world = World::new();
    let root = world.spawn(Transform::IDENTITY).id();
    let active_entity = world.spawn((Transform::IDENTITY, ChildOf(root))).id();
    let parked_entity = world.spawn((Transform::IDENTITY, ChildOf(root))).id();
    session
        .bind_vehicle_entity(active, active_entity)
        .expect("active binding");
    session
        .bind_vehicle_entity(parked, parked_entity)
        .expect("parked binding");
    let placement = LaneFlowFramePlacement::new(root, FramePlacementToken::new(2_103_001));
    session
        .set_frame_placement(placement)
        .expect("research placement");

    MixedAdapterFixture {
        session,
        world,
        placement,
        active,
        stopped,
        completed,
        parked,
        active_entity,
        parked_entity,
    }
}

struct ScaleAdapterFixture {
    session: LaneFlowSession,
    world: World,
    placement: LaneFlowFramePlacement,
    entities: Vec<Entity>,
}

impl ScaleAdapterFixture {
    fn new(vehicle_count: usize) -> Self {
        let spacing = 0.25;
        let edge_length = spacing * vehicle_count as f64;
        let graph =
            LaneGraph::try_new([research_edge("scale-edge", edge_length)]).expect("scale graph");
        let edge = graph.edge_handle("scale-edge").expect("scale edge");
        let (profiles, profile) = research_profile("scale-profile");
        let traffic = InitialTrafficData::try_new(
            graph.clone(),
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
        let core = CoreWorld::with_traffic_data(16, traffic, vehicles).expect("scale Core world");
        let handles = core
            .vehicles()
            .map(|vehicle| vehicle.handle)
            .collect::<Vec<_>>();
        let half_length = edge_length as f32 / 2.0;
        let points = [
            CanonicalPoint3F32::try_new(-half_length, 0.0, 0.0).expect("scale point"),
            CanonicalPoint3F32::try_new(half_length, 0.0, 0.0).expect("scale point"),
        ];
        let spatial = SpatialRegistry::try_new(
            &graph,
            CanonicalFrameId::try_new("research:p3-scale").expect("scale frame"),
            [SpatialEdgeInput::new(edge, &points)],
        )
        .expect("scale Spatial registry");
        let config = LaneFlowSessionConfig::new(NonZeroU32::new(1).expect("non-zero"));
        let mut session = LaneFlowSession::with_pose_capacity(core, spatial, config, vehicle_count);
        let mut world = World::new();
        let root = world.spawn(Transform::IDENTITY).id();
        let mut entities = Vec::with_capacity(vehicle_count);
        for handle in handles {
            let entity = world.spawn((Transform::IDENTITY, ChildOf(root))).id();
            session
                .bind_vehicle_entity(handle, entity)
                .expect("scale binding");
            entities.push(entity);
        }
        let placement = LaneFlowFramePlacement::new(root, FramePlacementToken::new(2_103_002));
        session
            .set_frame_placement(placement)
            .expect("scale placement");
        Self {
            session,
            world,
            placement,
            entities,
        }
    }
}

#[test]
fn selected_inputs_canonical_pose_counts_and_local_transforms_match_full_oracle() {
    let MixedAdapterFixture {
        mut session,
        mut world,
        placement,
        active,
        stopped,
        completed,
        parked,
        active_entity,
        parked_entity,
    } = mixed_adapter_fixture();

    session.rebuild_pose_inputs().expect("full pose inputs");
    let full_inputs = session.pose_inputs.clone();
    session
        .extract_presentation_batch(placement)
        .expect("full canonical pose");
    let full_pose = canonical_fingerprints(&session.pose_batch);
    session.presentation_report = LaneFlowPresentationReport {
        pose_records: session.pose_batch.len(),
        ..LaneFlowPresentationReport::default()
    };
    session
        .validate_and_apply_presentation(&mut world, placement)
        .expect("full Transform commit");
    let full_active_transform = TransformFingerprint::from_transform(
        world
            .get::<Transform>(active_entity)
            .expect("active Transform"),
    );
    let full_parked_transform = TransformFingerprint::from_transform(
        world
            .get::<Transform>(parked_entity)
            .expect("parked Transform"),
    );

    let mut buffers = SelectedFrameBuffers::with_capacity(&session, 4);
    let forward = AdapterSelection::from_handles(&session.core, [parked, active, stopped]);
    let reverse = AdapterSelection::from_handles(&session.core, [stopped, active, parked]);
    assert_eq!(
        forward, reverse,
        "selection construction order is not semantic"
    );
    assert!(
        !forward.includes(
            session
                .core
                .vehicles()
                .position(|vehicle| vehicle.handle == completed)
                .expect("completed position")
        )
    );
    materialize_selected_pose_inputs(&session.core, &forward, &mut buffers.candidate_inputs)
        .expect("selected pose inputs");
    let expected_inputs = full_inputs
        .iter()
        .copied()
        .filter(|input| [active, stopped, parked].contains(&input.vehicle()))
        .collect::<Vec<_>>();
    assert_eq!(
        input_fingerprints(&buffers.candidate_inputs),
        input_fingerprints(&expected_inputs)
    );
    let report = apply_selected_frame_atomically(&mut session, &mut world, placement, &mut buffers)
        .expect("selected frame");
    assert_eq!(report.pose_records(), 3);
    assert_eq!(report.mapped_records(), 2);
    assert_eq!(report.unbound_records(), 1);
    assert_eq!(report.applied_records(), 2);
    let selected_pose = canonical_fingerprints(&session.pose_batch);
    let expected_pose = full_pose
        .iter()
        .copied()
        .filter(|record| [active, stopped, parked].contains(&record.vehicle))
        .collect::<Vec<_>>();
    assert_eq!(selected_pose, expected_pose);
    assert_eq!(
        TransformFingerprint::from_transform(
            world
                .get::<Transform>(active_entity)
                .expect("active Transform")
        ),
        full_active_transform
    );
    assert_eq!(
        TransformFingerprint::from_transform(
            world
                .get::<Transform>(parked_entity)
                .expect("parked Transform")
        ),
        full_parked_transform
    );

    let parked_only = AdapterSelection::from_handles(&session.core, [parked]);
    *world
        .get_mut::<Transform>(active_entity)
        .expect("active Transform") = Transform::from_xyz(-10.0, -20.0, -30.0);
    materialize_selected_pose_inputs(&session.core, &parked_only, &mut buffers.candidate_inputs)
        .expect("parked-only inputs");
    let report = apply_selected_frame_atomically(&mut session, &mut world, placement, &mut buffers)
        .expect("parked-only frame");
    assert_eq!(report.pose_records(), 1);
    assert_eq!(report.mapped_records(), 1);
    assert_eq!(report.unbound_records(), 0);
    assert_eq!(report.applied_records(), 1);
    assert_eq!(
        TransformFingerprint::from_transform(
            world
                .get::<Transform>(parked_entity)
                .expect("parked Transform")
        ),
        full_parked_transform
    );
    assert_eq!(
        world
            .get::<Transform>(active_entity)
            .expect("unselected active Transform")
            .translation
            .x,
        -10.0,
        "unselected mapped Entity must not be written"
    );
}

#[test]
fn selected_frame_failure_preserves_cache_pose_and_prevalidated_transforms() {
    let MixedAdapterFixture {
        mut session,
        mut world,
        placement,
        active,
        parked,
        active_entity,
        parked_entity,
        ..
    } = mixed_adapter_fixture();
    let selection = AdapterSelection::from_handles(&session.core, [active, parked]);
    let mut buffers = SelectedFrameBuffers::with_capacity(&session, 4);
    materialize_selected_pose_inputs(&session.core, &selection, &mut buffers.candidate_inputs)
        .expect("initial inputs");
    apply_selected_frame_atomically(&mut session, &mut world, placement, &mut buffers)
        .expect("initial selected frame");
    let old_inputs = input_fingerprints(&buffers.committed_inputs);
    let old_pose = canonical_fingerprints(&session.pose_batch);

    buffers.candidate_inputs.clear();
    let edge = match buffers.committed_inputs[0].source() {
        PoseSource::Lane { edge, .. } => edge,
        PoseSource::Parking { .. } => panic!("first selected record must be lane-relative"),
        _ => panic!("research oracle only supports current PoseSource variants"),
    };
    buffers.candidate_inputs.push(PoseInputRecord::lane(
        active,
        edge,
        EdgeProgress::try_new(2_000.0).expect("finite invalid Spatial progress"),
    ));
    let before_active = TransformFingerprint::from_transform(
        world
            .get::<Transform>(active_entity)
            .expect("active Transform"),
    );
    let before_parked = TransformFingerprint::from_transform(
        world
            .get::<Transform>(parked_entity)
            .expect("parked Transform"),
    );
    assert!(matches!(
        apply_selected_frame_atomically(&mut session, &mut world, placement, &mut buffers),
        Err(LaneFlowAdapterError::SpatialBatch { .. })
    ));
    assert_eq!(input_fingerprints(&buffers.committed_inputs), old_inputs);
    assert_eq!(canonical_fingerprints(&session.pose_batch), old_pose);
    assert_eq!(
        TransformFingerprint::from_transform(
            world
                .get::<Transform>(active_entity)
                .expect("active Transform")
        ),
        before_active
    );
    assert_eq!(
        TransformFingerprint::from_transform(
            world
                .get::<Transform>(parked_entity)
                .expect("parked Transform")
        ),
        before_parked
    );

    materialize_selected_pose_inputs(&session.core, &selection, &mut buffers.candidate_inputs)
        .expect("valid retry inputs");
    *world
        .get_mut::<Transform>(active_entity)
        .expect("active Transform") = Transform::from_xyz(-1.0, -2.0, -3.0);
    let sentinel = TransformFingerprint::from_transform(
        world
            .get::<Transform>(active_entity)
            .expect("active Transform"),
    );
    assert!(world.despawn(parked_entity));
    assert!(matches!(
        apply_selected_frame_atomically(
            &mut session,
            &mut world,
            placement,
            &mut buffers
        ),
        Err(LaneFlowAdapterError::StaleMappedEntity { vehicle, .. })
            if vehicle == parked
    ));
    assert_eq!(input_fingerprints(&buffers.committed_inputs), old_inputs);
    assert_eq!(canonical_fingerprints(&session.pose_batch), old_pose);
    assert_eq!(
        TransformFingerprint::from_transform(
            world
                .get::<Transform>(active_entity)
                .expect("active Transform")
        ),
        sentinel,
        "stale second Entity must prevent the staged first write"
    );
    assert_eq!(session.presentation_report().applied_records(), 0);
}

#[test]
#[ignore = "explicit release-mode 10k/100k selected Adapter matrix"]
fn scale_matrix_reports_selected_adapter_cost_and_exact_counts() {
    for vehicle_count in SCALE_COUNTS {
        let mut fixture = ScaleAdapterFixture::new(vehicle_count);
        let mut buffers = SelectedFrameBuffers::with_capacity(&fixture.session, vehicle_count);
        let full_selection = AdapterSelection::all(vehicle_count);
        materialize_selected_pose_inputs(
            &fixture.session.core,
            &full_selection,
            &mut buffers.candidate_inputs,
        )
        .expect("scale full oracle inputs");
        apply_selected_frame_atomically(
            &mut fixture.session,
            &mut fixture.world,
            fixture.placement,
            &mut buffers,
        )
        .expect("scale full oracle frame");
        let full_pose = canonical_fingerprints(&fixture.session.pose_batch);
        let full_transforms = fixture
            .entities
            .iter()
            .map(|entity| {
                TransformFingerprint::from_transform(
                    fixture
                        .world
                        .get::<Transform>(*entity)
                        .expect("scale oracle Transform"),
                )
            })
            .collect::<Vec<_>>();

        for basis_points in SELECTION_BASIS_POINTS {
            let selected = vehicle_count * basis_points / 10_000;
            for shape in SelectionShape::ALL {
                let selection = AdapterSelection::for_shape(vehicle_count, selected, shape);
                let materialize_started = Instant::now();
                let scanned = materialize_selected_pose_inputs(
                    &fixture.session.core,
                    &selection,
                    &mut buffers.candidate_inputs,
                )
                .expect("scale selected inputs");
                let materialize_ns = materialize_started.elapsed().as_nanos();
                let frame_started = Instant::now();
                let report = apply_selected_frame_atomically(
                    &mut fixture.session,
                    &mut fixture.world,
                    fixture.placement,
                    &mut buffers,
                )
                .expect("scale selected frame");
                let frame_ns = frame_started.elapsed().as_nanos();
                assert_eq!(report.pose_records(), selected);
                assert_eq!(report.mapped_records(), selected);
                assert_eq!(report.unbound_records(), 0);
                assert_eq!(report.applied_records(), selected);
                assert!(
                    fixture
                        .session
                        .pose_batch
                        .records()
                        .iter()
                        .copied()
                        .map(canonical_fingerprint)
                        .eq(full_pose
                            .iter()
                            .copied()
                            .enumerate()
                            .filter(|(position, _)| selection.includes(*position))
                            .map(|(_, fingerprint)| fingerprint)),
                    "selected canonical pose must equal filtered full oracle"
                );
                for (position, entity) in fixture.entities.iter().copied().enumerate() {
                    if selection.includes(position) {
                        assert_eq!(
                            TransformFingerprint::from_transform(
                                fixture
                                    .world
                                    .get::<Transform>(entity)
                                    .expect("selected scale Transform")
                            ),
                            full_transforms[position],
                            "selected local Transform must equal full oracle"
                        );
                    }
                }

                println!(
                    "P3_ADAPTER_METRICS vehicles={vehicle_count} basis_points={basis_points} shape={} scanned={scanned} emitted={} mapped={} unbound={} applied={} selection_bytes={} retained_buffers={} pose_capacity={} staging_capacity={} materialize_ns={materialize_ns} extract_apply_ns={frame_ns}",
                    shape.name(),
                    report.pose_records(),
                    report.mapped_records(),
                    report.unbound_records(),
                    report.applied_records(),
                    selection.retained_bytes(),
                    buffers.retained_bytes(),
                    fixture.session.pose_batch.capacity(),
                    fixture.session.transform_staging.capacity(),
                );
            }
        }
    }
}

#[test]
#[ignore = "global allocator measurement requires explicit serial execution"]
fn warm_10k_and_100k_selected_materialize_extract_apply_is_zero_allocation() {
    let _guard = ALLOCATION_LOCK.lock().expect("allocation lock");

    for vehicle_count in SCALE_COUNTS {
        let mut fixture = ScaleAdapterFixture::new(vehicle_count);
        let selection = AdapterSelection::all(vehicle_count);
        let mut buffers = SelectedFrameBuffers::with_capacity(&fixture.session, vehicle_count);
        materialize_selected_pose_inputs(
            &fixture.session.core,
            &selection,
            &mut buffers.candidate_inputs,
        )
        .expect("warm selected inputs");
        apply_selected_frame_atomically(
            &mut fixture.session,
            &mut fixture.world,
            fixture.placement,
            &mut buffers,
        )
        .expect("warm selected frame");

        let region = Region::new(GLOBAL);
        black_box(
            materialize_selected_pose_inputs(
                black_box(&fixture.session.core),
                black_box(&selection),
                black_box(&mut buffers.candidate_inputs),
            )
            .expect("measured selected inputs"),
        );
        black_box(
            apply_selected_frame_atomically(
                black_box(&mut fixture.session),
                black_box(&mut fixture.world),
                fixture.placement,
                black_box(&mut buffers),
            )
            .expect("measured selected frame"),
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
