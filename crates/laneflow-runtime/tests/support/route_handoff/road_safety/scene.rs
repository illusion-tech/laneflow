use super::super::*;
use laneflow_runtime::{
    BoundRouteHandoff, ExecutionConfig, RouteHandle, RouteHandoffApproaches, RouteHandoffRequest,
    TickInput, TrafficWorld, VehicleHandle, VehicleSpawnInput, WorldInstallInput,
};
use laneflow_static_contract::{LaneEdgeOrdinal, VehicleProfileOrdinal};
use laneflow_static_network::SharedNetworkRevision;
use std::{num::NonZeroU32, sync::OnceLock};

pub(super) const CUT: u32 = 40_000;
const ROAD: u64 = 350_000;
pub(super) const TICK: TickInput = TickInput::new(100);

fn network() -> Arc<SharedNetworkRevision> {
    static REVISION: OnceLock<Arc<SharedNetworkRevision>> = OnceLock::new();
    Arc::clone(REVISION.get_or_init(|| {
        use RoadEditingDeclaration as D;
        let limits = CompileProfile::single_network_10k_stable_entities();
        let mut builder = RoadEditingSourceModuleBuilder::new(
            RoadEditingModuleHeader::try_new(
                "city/runtime-conflict",
                "runtime-conflict.lfre",
                vec![],
                RoadEditingProvenance::direct("dynamic road safety").unwrap(),
            )
            .unwrap(),
            GeometryAccuracyProfile::Fine2Cm,
            GeometryDirectionProfile::Smooth1Deg,
            &limits,
        )
        .unwrap();
        builder
            .add_alignment(
                RoadAlignmentInput::try_new(
                    "road",
                    CanonicalFrameReference::local("frame").unwrap(),
                    road_editing_line((0., 0.), (0., 350.)),
                )
                .unwrap(),
            )
            .unwrap();
        let mut add = |d| {
            builder.add_declaration(d).unwrap();
        };
        add(D::CanonicalFrame(
            CanonicalFrameInput::try_new("frame").unwrap(),
        ));
        let section = RoadSectionReference::owner_scoped(vec!["road".into()], "section").unwrap();
        let names = ["left", "middle", "right"];
        let lanes = names.map(|name| {
            AuthoringLaneReference::owner_scoped(vec!["road".into(), "section".into()], name)
                .unwrap()
        });
        add(D::RoadCorridor(
            RoadCorridorInput::try_new(
                "road",
                RoadAlignmentReference::try_new("road").unwrap(),
                0.,
                RoadEditingStationEnd::AlignmentEnd,
                section.clone(),
                lanes[1].clone(),
                vec![RoadEditingCorridorElement::RoadSection(section.clone())],
            )
            .unwrap()
            .with_cross_section(CrossSectionCoverage::Complete, vec![]),
        ));
        let boundaries = lanes
            .windows(2)
            .enumerate()
            .map(|(i, pair)| BoundaryExtentInput {
                left: BoundaryMemberInput::AuthoringLane(pair[0].clone()),
                right: BoundaryMemberInput::AuthoringLane(pair[1].clone()),
                kind: BoundaryExtentKind::Rule,
                local_key: format!("line-{i}").into(),
                low: BoundaryStation::CorridorEndpoint,
                high: BoundaryStation::CorridorEndpoint,
                payload: Some(BoundaryPayloadInput::CrossingRule {
                    directions: 3,
                    effect: 1,
                }),
                canvas_selection: None,
            })
            .collect();
        add(D::RoadSection(
            RoadSectionInput::try_new(
                "section",
                "motorLane",
                lanes.to_vec(),
                RoadCorridorReference::local("road").unwrap(),
            )
            .unwrap()
            .with_boundary_extents(boundaries),
        ));
        for (name, limit) in names.into_iter().zip([10., 13.9, 10.]) {
            let first = format!("{name}-up");
            let last = LaneEdgeReference::local(name).unwrap();
            add(D::LaneEdge(
                LaneEdgeInput::try_new(&first, limit, vec![last.clone()], None).unwrap(),
            ));
            add(D::LaneEdge(
                LaneEdgeInput::try_new(name, limit, vec![], None).unwrap(),
            ));
            let first = LaneEdgeReference::local(first).unwrap();
            add(D::AuthoringLane(
                AuthoringLaneInput::try_new(
                    name,
                    first.clone(),
                    RoadEditingLaneDirection::Forward,
                    LinearWidthProfile::try_new(3.5, 3.5).unwrap(),
                    None,
                    section.clone(),
                )
                .unwrap()
                .with_edge_chain(vec![first, last], vec![40.])
                .unwrap()
                .with_wheel_support(Some(true)),
            ));
        }
        add(D::ParticipantClass(
            ParticipantClassInput::try_new("motor").unwrap(),
        ));
        for (key, length, wheelbase, rear_overhang) in
            [("car", 4.5, 2.65, 0.75), ("long", 12., 6., 2.)]
        {
            add(D::VehicleProfile(
                VehicleProfileInput::try_new(
                    key,
                    ParticipantClassReference::local("motor").unwrap(),
                    IidmVehicleProfileInput::try_new(length, 13.9, 2.5, 1.2, 2., 4.5, 9.).unwrap(),
                    RoadVehicleGeometryInput::RigidTwoAxle(RigidTwoAxleGeometryInput {
                        wheelbase_m: wheelbase,
                        track_width_m: 1.6,
                        reference_height_m: 0.55,
                        rear_overhang_m: rear_overhang,
                        width_m: 1.9,
                        bottom_height_m: 0.,
                        top_height_m: 1.5,
                        max_steering_angle_rad: 0.65,
                        max_steering_rate_rad_s: 1.,
                    }),
                )
                .unwrap(),
            ));
        }
        compile_road_editing_revision(builder.finish().unwrap())
    }))
}

pub(super) struct Scene {
    pub(super) world: TrafficWorld,
    revision: Arc<SharedNetworkRevision>,
    pub(super) lanes: [Vec<LaneEdgeOrdinal>; 3],
}

pub(super) struct Change {
    pub(super) vehicle: VehicleHandle,
    route: RouteHandle,
    binding: BoundRouteHandoff,
    profile: VehicleProfileOrdinal,
    front_over_axle: u32,
    span: u64,
}

impl Scene {
    pub(super) fn new(workers: u32) -> Self {
        let revision = network();
        let rel = revision.traffic().relations();
        let section = rel.section_lanes(RoadSectionOrdinal::from_raw(0)).unwrap();
        let lanes = std::array::from_fn(|i| rel.authoring_edge_chain(section[i]).unwrap().to_vec());
        let config = WorldConfig::try_new(WorldConfigInput {
            vehicle_capacity: 4,
            route_capacity: 4,
            route_edge_occurrence_capacity: 32,
            route_conflict_occurrence_capacity: 16,
            fixed_delta_time_ms: 100,
        })
        .unwrap()
        .with_handoff_driving(registration::driving());
        let world = TrafficWorld::install(WorldInstallInput {
            revision: revision.clone(),
            config,
            execution: ExecutionConfig::new(NonZeroU32::new(workers).unwrap()),
            source: crate::published_source(&revision, "fixture://dynamic-road-safety"),
            world_id: crate::WorldId::new(0),
            policy_selection: crate::test_policy::selection(&revision),
        })
        .unwrap();
        Self {
            world,
            revision,
            lanes,
        }
    }

    pub(super) fn straight(&mut self, lane: usize, front: u32, speed: u32) -> VehicleHandle {
        let route = self
            .world
            .register_route(RouteRegisterInput::new(self.lanes[lane].clone()))
            .unwrap();
        let (index, progress) = if front >= CUT {
            (1, front - CUT)
        } else {
            (0, front)
        };
        self.world
            .spawn_vehicle(VehicleSpawnInput::new(
                conflict_profile(&self.revision, "car"),
                route,
                index,
                progress,
                speed,
            ))
            .unwrap()
    }

    pub(super) fn changer(
        &mut self,
        lane: usize,
        profile_key: &str,
        front: u32,
        speed: u32,
    ) -> Change {
        let profile = conflict_profile(&self.revision, profile_key);
        let input = RouteRegisterInput::new(vec![
            self.lanes[lane][0],
            self.lanes[lane][1],
            self.lanes[1][1],
        ]);
        let occurrence = RouteHandoffOccurrence {
            revision: self.revision.network_revision(),
            source_index: 1,
            owner: BoundaryOwner::Section(RoadSectionOrdinal::from_raw(0)),
            left_index: u32::from(lane == 2),
            stations_mm: [u64::from(CUT) + 1, ROAD - 1],
        };
        let binding = self.world.bind_route_handoff(&input, occurrence).unwrap();
        let route = self
            .world
            .register_route(input.with_handoffs(vec![occurrence]))
            .unwrap();
        let vehicle = self
            .world
            .spawn_vehicle(VehicleSpawnInput::new(
                profile,
                route,
                1,
                front - CUT,
                speed,
            ))
            .unwrap();
        let (front_over_axle, span) = if profile_key == "long" {
            (10_000, 60_000)
        } else {
            (3_750, 30_000)
        };
        Change {
            vehicle,
            route,
            binding,
            profile,
            front_over_axle,
            span,
        }
    }

    pub(super) fn front(&self, vehicle: VehicleHandle) -> u32 {
        let state = self.world.vehicle(vehicle).unwrap();
        state.rounded_front_progress_mm()
            + if state.route_edge_index() == 0 {
                0
            } else {
                CUT
            }
    }

    pub(super) fn request(&self, change: &Change) -> RouteHandoffRequest {
        let rear = u64::from(self.front(change.vehicle) - change.front_over_axle);
        RouteHandoffRequest {
            vehicle: change.vehicle,
            route: change.route,
            revision: self.revision.network_revision(),
            source_index: 1,
            station_window_mm: [0, ROAD],
            rear_stations_mm: [rear, rear + change.span],
            speed_m_s: 10.,
            limits: GeometryWorkLimits::STANDARD,
        }
    }

    pub(super) fn approaches(
        &self,
        change: &Change,
        request: &RouteHandoffRequest,
    ) -> RouteHandoffApproaches {
        let trial = RouteHandoffTrial {
            profile: self.revision.identity().stable_id(change.profile).unwrap(),
            station_window_mm: request.station_window_mm,
            rear_stations_mm: request.rear_stations_mm,
            speed_m_s: request.speed_m_s,
            limits: request.limits,
        };
        self.world
            .preview_route_handoff_approaches(
                &change.binding,
                1,
                &trial,
                0..=change.span,
                TRAVEL_TOLERANCE_MM,
                u32::MAX,
            )
            .unwrap()
    }

    pub(super) fn held(&self, vehicle: VehicleHandle) -> bool {
        self.world.vehicle_route_handoffs(vehicle).next().is_some()
    }
}

pub(super) fn restore(scene: &Scene, workers: NonZeroU32) -> TrafficWorld {
    let bytes = laneflow_runtime::encode_lfrs(&scene.world.capture_snapshot().unwrap());
    laneflow_runtime::restore_lfrs(
        &bytes,
        scene.revision.clone(),
        scene.world.committed_source().clone(),
        scene.world.config(),
        ExecutionConfig::new(workers),
        laneflow_runtime::SnapshotRestoreLimits::new(4 << 20, 4096),
    )
    .unwrap()
    .into_world()
}
