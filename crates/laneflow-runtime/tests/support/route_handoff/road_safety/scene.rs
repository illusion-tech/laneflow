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

fn network(
    cuts_m: [&[u32]; 3],
    length_m: u32,
    car_length_m: f64,
    spaces: &[(&str, usize, f64)],
    cache: &OnceLock<Arc<SharedNetworkRevision>>,
) -> Arc<SharedNetworkRevision> {
    Arc::clone(cache.get_or_init(|| {
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
                    road_editing_line((0., 0.), (0., f64::from(length_m))),
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
        for ((name, limit), cuts) in names.into_iter().zip([10., 13.9, 10.]).zip(cuts_m) {
            let mut keys = vec![format!("{name}-up")];
            keys.extend((1..cuts.len()).map(|i| format!("{name}-part-{i}")));
            keys.push(name.to_owned());
            let chain: Vec<_> = keys
                .iter()
                .map(|k| LaneEdgeReference::local(k.as_str()).unwrap())
                .collect();
            for (i, key) in keys.iter().enumerate() {
                let next = chain.get(i + 1).cloned().into_iter().collect();
                add(D::LaneEdge(
                    LaneEdgeInput::try_new(key.as_str(), limit, next, None).unwrap(),
                ));
            }
            add(D::AuthoringLane(
                AuthoringLaneInput::try_new(
                    name,
                    chain[0].clone(),
                    RoadEditingLaneDirection::Forward,
                    LinearWidthProfile::try_new(3.5, 3.5).unwrap(),
                    None,
                    section.clone(),
                )
                .unwrap()
                .with_edge_chain(chain, cuts.iter().map(|&m| f64::from(m)).collect())
                .unwrap()
                .with_wheel_support(Some(true)),
            ));
        }
        if !spaces.is_empty() {
            add(D::ParkingFacility(
                ParkingFacilityInput::try_new("parking").unwrap(),
            ));
        }
        for &(space, lane, station) in spaces {
            let anchor = ParkingLaneAnchor::try_new(
                LaneEdgeReference::local(names[lane]).unwrap(),
                station - f64::from(*cuts_m[lane].last().unwrap()),
            )
            .unwrap();
            add(D::ParkingSpace(
                ParkingSpaceInput::try_new(
                    space,
                    anchor.clone(),
                    anchor,
                    ParkingSpaceGeometry::try_new(1.5, 0., 5., 2.5).unwrap(),
                )
                .unwrap()
                .with_parking_facility(ParkingFacilityReference::local("parking").unwrap()),
            ));
        }
        add(D::ParticipantClass(
            ParticipantClassInput::try_new("motor").unwrap(),
        ));
        for (key, length, wheelbase, rear_overhang, gap) in [
            ("car", car_length_m, 2.65, 0.75, 2.5),
            ("long", 12., 6., 2., 2.5),
            ("zero-gap", 4.5, 2.65, 0.75, 0.),
        ] {
            add(D::VehicleProfile(
                VehicleProfileInput::try_new(
                    key,
                    ParticipantClassReference::local("motor").unwrap(),
                    IidmVehicleProfileInput::try_new(length, 13.9, gap, 1.2, 2., 4.5, 9.).unwrap(),
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
    pub(super) binding: BoundRouteHandoff,
    profile: VehicleProfileOrdinal,
    front_over_axle: u32,
    span: u64,
}

impl Scene {
    pub(super) fn new(workers: u32) -> Self {
        Self::at_interval(workers, 100)
    }

    pub(super) fn at_interval(workers: u32, dt: u64) -> Self {
        static REVISION: OnceLock<Arc<SharedNetworkRevision>> = OnceLock::new();
        Self::with_cuts(workers, [40; 3], &REVISION, dt)
    }

    pub(super) fn with_cuts(
        workers: u32,
        cuts_m: [u32; 3],
        cache: &OnceLock<Arc<SharedNetworkRevision>>,
        dt: u64,
    ) -> Self {
        let spaces = [
            ("space", 0, 348.5),
            ("source-exit-near", 0, 79.039),
            ("source-exit-short", 0, 77.540),
            ("source-exit-exact", 0, 77.539),
            ("source-exit-touch", 0, 80.039),
            ("source-exit-one-mm", 0, 80.038),
            ("target-skipped", 1, 100.),
            ("target-end", 1, 113.75),
            ("target-near", 1, 113.8),
            ("target-far", 1, 135.),
            ("target-exit-tail-inside", 1, 125.363),
            ("target-exit-tail-clear", 1, 125.364),
        ];
        let revision = network(
            cuts_m.each_ref().map(std::slice::from_ref),
            350,
            4.5,
            &spaces,
            cache,
        );
        Self::install(workers, dt, revision)
    }

    pub(super) fn split_middle_lane() -> Self {
        static REVISION: OnceLock<Arc<SharedNetworkRevision>> = OnceLock::new();
        let revision = network([&[148], &[10, 150], &[24]], 200, 4.15, &[], &REVISION);
        Self::install(1, 1_000, revision)
    }

    fn install(workers: u32, dt: u64, revision: Arc<SharedNetworkRevision>) -> Self {
        let rel = revision.traffic().relations();
        let section = rel.section_lanes(RoadSectionOrdinal::from_raw(0)).unwrap();
        let lanes = std::array::from_fn(|i| rel.authoring_edge_chain(section[i]).unwrap().to_vec());
        let config = WorldConfig::try_new(WorldConfigInput {
            vehicle_capacity: 4,
            route_capacity: 4,
            route_edge_occurrence_capacity: 32,
            route_conflict_occurrence_capacity: 16,
            fixed_delta_time_ms: dt,
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
        let cut = self.cut(lane);
        let (index, progress) = if front >= cut {
            (1, front - cut)
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
        self.changer_on(lane, profile_key, front, speed, false)
    }

    pub(super) fn changer_on(
        &mut self,
        lane: usize,
        profile_key: &str,
        front: u32,
        speed: u32,
        returns_to_source: bool,
    ) -> Change {
        let profile = conflict_profile(&self.revision, profile_key);
        let mut edges = vec![self.lanes[lane][0], self.lanes[lane][1], self.lanes[1][1]];
        let occurrence = RouteHandoffOccurrence {
            revision: self.revision.network_revision(),
            source_index: 1,
            owner: BoundaryOwner::Section(RoadSectionOrdinal::from_raw(0)),
            left_index: u32::from(lane == 2),
            stations_mm: [u64::from(self.cut(lane).max(self.cut(1))) + 1, ROAD - 1],
        };
        let mut occurrences = vec![occurrence];
        if returns_to_source {
            edges.push(self.lanes[lane][1]);
            occurrences.push(RouteHandoffOccurrence {
                source_index: 2,
                stations_mm: [345_000, ROAD - 1],
                ..occurrence
            });
        }
        let input = RouteRegisterInput::new(edges);
        let binding = self
            .world
            .bind_route_handoffs(&input, &occurrences)
            .unwrap();
        let route = self
            .world
            .register_route(input.with_handoffs(occurrences))
            .unwrap();
        let vehicle = self
            .world
            .spawn_vehicle(VehicleSpawnInput::new(
                profile,
                route,
                1,
                front - self.cut(lane),
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
        let edge =
            self.world.route_edges(state.route()).unwrap()[state.route_edge_index() as usize];
        let cut = self
            .lanes
            .iter()
            .position(|lane| lane[1] == edge)
            .map_or(0, |lane| self.cut(lane));
        state.rounded_front_progress_mm() + cut
    }

    fn cut(&self, lane: usize) -> u32 {
        self.revision.traffic().lane_lengths_millimetres()[self.lanes[lane][0].index()]
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

    pub(super) fn trial(
        &self,
        change: &Change,
        request: &RouteHandoffRequest,
    ) -> RouteHandoffTrial {
        RouteHandoffTrial {
            profile: self.revision.identity().stable_id(change.profile).unwrap(),
            station_window_mm: request.station_window_mm,
            rear_stations_mm: request.rear_stations_mm,
            speed_m_s: request.speed_m_s,
            limits: request.limits,
        }
    }

    pub(super) fn approaches(
        &self,
        change: &Change,
        request: &RouteHandoffRequest,
    ) -> RouteHandoffApproaches {
        let trial = self.trial(change, request);
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

    pub(super) fn space_at(
        &self,
        lane: usize,
        station: u32,
    ) -> laneflow_static_contract::ParkingSpaceOrdinal {
        let relations = self.revision.traffic().relations();
        relations
            .parking_facility_spaces(laneflow_static_contract::ParkingFacilityOrdinal::from_raw(
                0,
            ))
            .unwrap()
            .iter()
            .copied()
            .find(|&space| {
                relations.parking_space_entry(space)
                    == Some((self.lanes[lane][1], station - self.cut(lane)))
            })
            .unwrap()
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
