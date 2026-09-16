//! #678 的生产命令与测试计数共用同一个正式编译夹具。
use laneflow_compiler::{
    CompilationUnitBuilder, CompileLimits, Compiler, IidmVehicleProfileInput, LaneEdgeInput,
    LaneEdgeReference, ParkingFacilityInput, ParkingLaneAnchorInput, ParticipantClassInput,
    ParticipantClassReference, PortableDiffBase, PortableEmissionProvenance, SourceModuleHeader,
    SourceModuleHeaderInput, SyntheticModuleBuilder, VehicleProfileInput,
    derive_canonical_stable_id_v1, emit_portable_candidate,
};
use laneflow_format::{FormatLimits, check_post_emission_bundle};
use laneflow_runtime::{
    CommittedNetworkSource, LeaveParkingTarget, ParkedVehicleSpawnInput, ParkingTarget,
    PublishedLfcaReference, RouteHandle, RouteRegisterInput, TrafficWorld, VehicleHandle,
    VehicleSpawnInput, VirtualExitAnchorSelector, WorldConfig, WorldPolicySelection,
};
use laneflow_static_contract::{
    EntityKind, LaneEdgeId, ParkingFacilityOrdinal, VehicleProfileOrdinal,
};
use laneflow_static_network::{
    SharedNetworkBuildLimits, SharedNetworkBuildOptions, SharedNetworkRevision, SpatialBuildOption,
    build_shared_network_revision,
};
use std::sync::Arc;
fn iidm() -> IidmVehicleProfileInput {
    IidmVehicleProfileInput {
        length_meters: 4.5,
        desired_speed_meters_per_second: 13.75,
        min_gap_meters: 2.0,
        time_headway_seconds: 1.4,
        max_acceleration_meters_per_second_squared: 1.8,
        comfortable_deceleration_meters_per_second_squared: 2.0,
        emergency_deceleration_meters_per_second_squared: 4.5,
    }
}

fn compile_revision_with_limits(
    limits: CompileLimits,
    configure: impl FnOnce(&mut SyntheticModuleBuilder),
) -> Arc<SharedNetworkRevision> {
    let header = SourceModuleHeader::new(
        SourceModuleHeaderInput {
            authoring_namespace_id: "city/runtime-coverage",
            source_document_key: "runtime-coverage.document",
            generator_build_id: "git:0123456789abcdef",
            parameters_and_inputs_digest: [0x11; 32],
            frontend_options_digest: [0x22; 32],
            random_seed: Some(42),
            provenance: "repository:laneflow",
        },
        &limits,
    )
    .expect("source header");
    let mut module = SyntheticModuleBuilder::new(header, &limits).expect("synthetic module");
    configure(&mut module);
    let mut unit = CompilationUnitBuilder::new(limits);
    unit.add_synthetic_module(module.finish().expect("finished module"))
        .expect("compilation module");
    let output = Compiler::new()
        .compile(unit.build().expect("compilation unit"))
        .unwrap_or_else(|bundle| {
            panic!(
                "compiled output diagnostics: {:?}",
                bundle
                    .diagnostics()
                    .iter()
                    .map(|diagnostic| (diagnostic.code(), diagnostic.payload()))
                    .collect::<Vec<_>>()
            )
        });
    let provenance = PortableEmissionProvenance::try_new("laneflow-runtime-coverage-v1")
        .expect("portable provenance");
    let candidate = emit_portable_candidate(
        &output,
        &provenance,
        FormatLimits::HARD,
        PortableDiffBase::Genesis,
    )
    .expect("portable candidate");
    let checked = check_post_emission_bundle(
        candidate.canonical_artifact().bytes(),
        candidate.source_map().bytes(),
        candidate.semantic_diff().bytes(),
        candidate.expected_semantic_diff_base(),
        FormatLimits::HARD,
    )
    .expect("post-emission checked bundle");
    build_shared_network_revision(
        checked.canonical_network_input(),
        SharedNetworkBuildOptions::new(
            SpatialBuildOption::Omit,
            SharedNetworkBuildLimits::new(64 * 1_024 * 1_024, 16 * 1_024 * 1_024),
        ),
    )
    .expect("shared network revision")
}

fn register_named(world: &mut TrafficWorld, keys: &[&str]) -> RouteHandle {
    const NS: &str = "city/runtime-coverage";
    let limits = CompileLimits::p100_initial_v1();
    let edges: Vec<_> = keys
        .iter()
        .map(|key| {
            let stable =
                derive_canonical_stable_id_v1(EntityKind::LaneEdge, NS, key, &limits).expect("id");
            world
                .revision()
                .identity()
                .ordinal(LaneEdgeId::from_untyped(stable))
                .expect(key)
        })
        .collect();
    world
        .register_route(RouteRegisterInput::new(edges))
        .expect("register")
}

fn add_standard_profiles(module: &mut SyntheticModuleBuilder) {
    module
        .add_participant_class(ParticipantClassInput {
            participant_class_key: "road-user",
            extends: None,
        })
        .expect("class")
        .add_vehicle_profile(VehicleProfileInput {
            vehicle_profile_key: "car",
            participant_class: ParticipantClassReference::local("road-user"),
            iidm: iidm(),
        })
        .expect("profile");
}

fn install_fixture(
    revision: std::sync::Arc<laneflow_static_network::SharedNetworkRevision>,
    config: laneflow_runtime::WorldConfig,
) -> Result<laneflow_runtime::TrafficWorld, laneflow_runtime::InstallError> {
    assert!(
        [
            EntityKind::ManeuverGate,
            EntityKind::ConflictZone,
            EntityKind::ParticipantStream,
        ]
        .iter()
        .all(|kind| revision.traffic().entity_counts().count(*kind) == 0),
        "parking command fixture must remain policy-free"
    );
    laneflow_runtime::TrafficWorld::install(
        Arc::clone(&revision),
        config,
        published_source(&revision, "fixture://in-process"),
        0,
        WorldPolicySelection::NotRequired,
    )
}

fn published_source(revision: &SharedNetworkRevision, key: &str) -> CommittedNetworkSource {
    let origin = *revision.canonical_origin();
    CommittedNetworkSource::Published {
        reference: PublishedLfcaReference::new(
            key,
            origin.canonical_artifact_digest(),
            origin.canonical_artifact_byte_length(),
            origin.network_revision(),
        )
        .expect("non-empty fixture key"),
    }
}

pub const EXITS: usize = 64;

#[derive(Clone, Copy, Debug)]
pub struct Case {
    pub active: usize,
    pub parked: usize,
    pub commands: usize,
    pub success_percent: usize,
    /// 0：交替；1：成功在前；2：拒绝在前。
    pub order: usize,
}

impl Case {
    pub fn succeeds(self, index: usize) -> bool {
        self.success_percent == 100 || (self.success_percent == 50 && index.is_multiple_of(2))
    }

    pub fn indices(self) -> Vec<usize> {
        let mut indices: Vec<_> = (0..self.commands).collect();
        if self.order != 0 {
            indices.sort_by_key(|index| self.succeeds(*index) != (self.order == 1));
        }
        indices
    }
}

pub fn cases() -> Vec<Case> {
    let mut cases = Vec::new();
    for active in [128, 1_024] {
        for parked in [64, 4_096] {
            for commands in [1, 16, 64] {
                for success_percent in [0, 50, 100] {
                    if commands == 1 && success_percent == 50 {
                        continue;
                    }
                    cases.push(Case {
                        active,
                        parked,
                        commands,
                        success_percent,
                        order: 0,
                    });
                }
            }
        }
    }
    for order in [1, 2] {
        cases.push(Case {
            active: 1_024,
            parked: 4_096,
            commands: 64,
            success_percent: 50,
            order,
        });
    }
    cases
}

pub fn revision() -> Arc<SharedNetworkRevision> {
    compile_revision_with_limits(CompileLimits::p100_initial_v1(), |module| {
        add_standard_profiles(module);
        module
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: "background",
                length_meters: 10_000.0,
                speed_limit_meters_per_second: 15.0,
                successors: &[],
            })
            .unwrap();
        let keys: Vec<_> = (0..EXITS).map(|i| format!("exit-{i}")).collect();
        for key in &keys {
            module
                .add_lane_edge(LaneEdgeInput {
                    lane_edge_key: key,
                    length_meters: 100.0,
                    speed_limit_meters_per_second: 15.0,
                    successors: &[],
                })
                .unwrap();
        }
        let exits: Vec<_> = keys
            .iter()
            .map(|key| ParkingLaneAnchorInput {
                lane_edge: LaneEdgeReference::local(key),
                progress_meters: 70.0,
            })
            .collect();
        module
            .add_parking_facility(ParkingFacilityInput {
                parking_facility_key: "facility",
                virtual_capacity: 4_096,
                virtual_entries: &[ParkingLaneAnchorInput {
                    lane_edge: LaneEdgeReference::local("background"),
                    progress_meters: 10.0,
                }],
                virtual_exits: &exits,
            })
            .unwrap();
    })
}

pub struct Fixture {
    pub world: TrafficWorld,
    pub vehicles: Vec<VehicleHandle>,
    pub followers: Vec<VehicleHandle>,
    pub targets: Vec<LeaveParkingTarget>,
}

pub fn fixture(revision: &Arc<SharedNetworkRevision>, case: Case) -> Fixture {
    fixture_with_capacity(revision, case, (case.active + case.parked) as u32)
}

pub fn fixture_with_capacity(
    revision: &Arc<SharedNetworkRevision>,
    case: Case,
    vehicle_capacity: u32,
) -> Fixture {
    assert!(case.active >= EXITS && case.parked >= EXITS && case.commands <= EXITS);
    let mut world = install_fixture(
        Arc::clone(revision),
        WorldConfig::new(vehicle_capacity, 65, 65, 1, 1, 100),
    )
    .unwrap();
    let facility = ParkingFacilityOrdinal::from_raw(0);
    let profile = VehicleProfileOrdinal::from_raw(0);
    let anchors = revision
        .traffic()
        .relations()
        .parking_facility(facility)
        .unwrap()
        .virtual_exits();
    let routes: Vec<_> = anchors
        .iter()
        .map(|a| {
            world
                .register_route(RouteRegisterInput::new(vec![a.lane_edge()]))
                .unwrap()
        })
        .collect();
    let background = register_named(&mut world, &["background"]);
    let vehicles = (0..EXITS)
        .map(|i| {
            world
                .spawn_parked_vehicle(
                    ParkedVehicleSpawnInput::new(profile, routes[i], 0, 0),
                    ParkingTarget::VirtualPool(facility),
                )
                .unwrap()
                .vehicle
        })
        .collect();
    for _ in EXITS..case.parked {
        world
            .spawn_parked_vehicle(
                ParkedVehicleSpawnInput::new(profile, background, 0, 0),
                ParkingTarget::VirtualPool(facility),
            )
            .unwrap();
    }
    for i in 0..case.active - EXITS {
        world
            .spawn_vehicle(VehicleSpawnInput::new(
                profile,
                background,
                0,
                (i as u32 + 1) * 10_000,
                10_000,
            ))
            .unwrap();
    }
    let followers = routes
        .iter()
        .enumerate()
        .map(|(i, route)| {
            world
                .spawn_vehicle(VehicleSpawnInput::new(
                    profile,
                    *route,
                    0,
                    if case.succeeds(i) { 10_000 } else { 61_000 },
                    10_000,
                ))
                .unwrap()
        })
        .collect();
    let targets = routes
        .iter()
        .enumerate()
        .map(|(i, route)| LeaveParkingTarget::VirtualPool {
            facility,
            route: *route,
            exit_anchor: VirtualExitAnchorSelector::from_raw(i as u32),
            exit_route_occurrence: 0,
        })
        .collect();
    Fixture {
        world,
        vehicles,
        followers,
        targets,
    }
}
