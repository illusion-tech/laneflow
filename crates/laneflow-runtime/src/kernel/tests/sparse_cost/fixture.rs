//! #682 正交输入；安装、生命周期命令和摘要均在 step 计时外。
use super::runtime_types as rt;
use laneflow_compiler::{
    CompilationUnitBuilder, CompileLimits, Compiler, IidmVehicleProfileInput, LaneEdgeInput,
    LaneEdgeReference, ParkingFacilityInput, ParkingLaneAnchorInput, ParticipantClassInput,
    ParticipantClassReference, PortableDiffBase, PortableEmissionProvenance, SourceModuleHeader,
    SourceModuleHeaderInput, SyntheticModuleBuilder, VehicleProfileInput, emit_portable_candidate,
};
use laneflow_format::{FormatLimits, check_post_emission_bundle};
use laneflow_static_contract::{LaneEdgeOrdinal, ParkingFacilityOrdinal, VehicleProfileOrdinal};
use laneflow_static_network::{
    SharedNetworkBuildLimits, SharedNetworkBuildOptions, SharedNetworkRevision, SpatialBuildOption,
    build_shared_network_revision,
};
use std::sync::Arc;

pub const WARM: usize = 40;
pub const STEPS: usize = 128;

#[derive(Clone, Copy, Debug)]
pub struct Case {
    pub name: &'static str,
    pub active: u32,
    pub live: u32,
    pub high_water: u32,
    pub capacity: u32,
    pub edges: u32,
    pub ring_mm: u32,
}

pub const CASES: [Case; 9] = [
    Case {
        name: "compact",
        active: 1_000,
        live: 1_000,
        high_water: 1_000,
        capacity: 10_000,
        edges: 256,
        ring_mm: 0,
    },
    Case {
        name: "vacant",
        active: 1_000,
        live: 1_000,
        high_water: 10_000,
        capacity: 10_000,
        edges: 256,
        ring_mm: 0,
    },
    Case {
        name: "parked",
        active: 1_000,
        live: 10_000,
        high_water: 10_000,
        capacity: 10_000,
        edges: 256,
        ring_mm: 0,
    },
    Case {
        name: "capacity",
        active: 1_000,
        live: 1_000,
        high_water: 1_000,
        capacity: 100_000,
        edges: 256,
        ring_mm: 0,
    },
    Case {
        name: "active",
        active: 10_000,
        live: 10_000,
        high_water: 10_000,
        capacity: 10_000,
        edges: 256,
        ring_mm: 0,
    },
    Case {
        name: "edges-small",
        active: 1_000,
        live: 1_000,
        high_water: 1_000,
        capacity: 10_000,
        edges: 16,
        ring_mm: 0,
    },
    Case {
        name: "edges-large",
        active: 1_000,
        live: 1_000,
        high_water: 1_000,
        capacity: 10_000,
        edges: 4_096,
        ring_mm: 0,
    },
    Case {
        name: "ring-10m",
        active: 64,
        live: 64,
        high_water: 64,
        capacity: 10_000,
        edges: 4_096,
        ring_mm: 10_000,
    },
    Case {
        name: "ring-1m",
        active: 64,
        live: 64,
        high_water: 64,
        capacity: 10_000,
        edges: 4_096,
        ring_mm: 1_000,
    },
];

pub fn revision(case: Case) -> Arc<SharedNetworkRevision> {
    let limits = CompileLimits::single_network_1m_v2();
    let header = SourceModuleHeader::new(
        SourceModuleHeaderInput {
            authoring_namespace_id: "research/sparse-cost",
            source_document_key: "sparse.document",
            generator_build_id: "sparse-cost-v1",
            parameters_and_inputs_digest: [0x68; 32],
            frontend_options_digest: [0x02; 32],
            random_seed: Some(682),
            provenance: "repository:laneflow",
        },
        &limits,
    )
    .unwrap();
    let mut module = SyntheticModuleBuilder::new(header, &limits).unwrap();
    module
        .add_participant_class(ParticipantClassInput {
            participant_class_key: "road-user",
            extends: None,
        })
        .unwrap();
    module
        .add_vehicle_profile(VehicleProfileInput {
            vehicle_profile_key: "car",
            participant_class: ParticipantClassReference::local("road-user"),
            iidm: IidmVehicleProfileInput {
                length_meters: 4.5,
                desired_speed_meters_per_second: 13.75,
                min_gap_meters: 2.0,
                time_headway_seconds: 1.4,
                max_acceleration_meters_per_second_squared: 1.8,
                comfortable_deceleration_meters_per_second_squared: 2.0,
                emergency_deceleration_meters_per_second_squared: 4.5,
            },
        })
        .unwrap();
    for edge in 0..case.edges {
        let next = format!("edge-{}", edge / 64 * 64 + (edge + 1) % 64);
        let successors = [LaneEdgeReference::local(&next)];
        module
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: &format!("edge-{edge}"),
                length_meters: if case.ring_mm == 0 {
                    8_000.0
                } else {
                    f64::from(case.ring_mm) / 1_000.0
                },
                speed_limit_meters_per_second: 15.0,
                successors: if case.ring_mm == 0 { &[] } else { &successors },
            })
            .unwrap();
    }
    module
        .add_parking_facility(ParkingFacilityInput {
            parking_facility_key: "pool",
            virtual_capacity: 100_000,
            virtual_entries: &[ParkingLaneAnchorInput {
                lane_edge: LaneEdgeReference::local("edge-0"),
                progress_meters: 0.5,
            }],
            virtual_exits: &[ParkingLaneAnchorInput {
                lane_edge: LaneEdgeReference::local("edge-0"),
                progress_meters: 0.5,
            }],
        })
        .unwrap();
    let mut unit = CompilationUnitBuilder::new(limits);
    unit.add_synthetic_module(module.finish().unwrap()).unwrap();
    let output = Compiler::new().compile(unit.build().unwrap()).unwrap();
    let provenance = PortableEmissionProvenance::try_new("sparse-cost-v1").unwrap();
    let artifact = emit_portable_candidate(
        &output,
        &provenance,
        FormatLimits::HARD,
        PortableDiffBase::Genesis,
    )
    .unwrap();
    let checked = check_post_emission_bundle(
        artifact.canonical_artifact().bytes(),
        artifact.source_map().bytes(),
        artifact.semantic_diff().bytes(),
        artifact.expected_semantic_diff_base(),
        FormatLimits::HARD,
    )
    .unwrap();
    build_shared_network_revision(
        checked.canonical_network_input(),
        SharedNetworkBuildOptions::new(
            SpatialBuildOption::Omit,
            SharedNetworkBuildLimits::new(256 * 1_024 * 1_024, 64 * 1_024 * 1_024),
        ),
    )
    .unwrap()
}

pub fn world(root: &Arc<SharedNetworkRevision>, case: Case) -> rt::TrafficWorld {
    assert!(
        case.active <= case.live
            && case.live <= case.high_water
            && case.high_water <= case.capacity
    );
    let origin = root.canonical_origin();
    let mut world = rt::TrafficWorld::install(
        Arc::clone(root),
        rt::WorldConfig::new(case.capacity, 64, 16_384, 1_024, 100),
        rt::ExecutionConfig::new(std::num::NonZeroU32::MIN),
        rt::CommittedNetworkSource::Published {
            reference: rt::PublishedLfcaReference::new(
                "fixture://sparse-cost",
                origin.canonical_artifact_digest(),
                origin.canonical_artifact_byte_length(),
                origin.network_revision(),
            )
            .unwrap(),
        },
        682,
        rt::WorldPolicySelection::NotRequired,
    )
    .unwrap();
    let mut routes = Vec::new();
    if case.ring_mm == 0 {
        for raw in 0..16 {
            routes.push(
                world
                    .register_route(rt::RouteRegisterInput::new(vec![
                        LaneEdgeOrdinal::from_raw(raw),
                    ]))
                    .unwrap(),
            );
        }
    } else {
        let mut seen = vec![false; case.edges as usize];
        for raw in 0..case.edges {
            let start = LaneEdgeOrdinal::from_raw(raw);
            if seen[start.index()] {
                continue;
            }
            let mut edge = start;
            let mut cycle = Vec::new();
            loop {
                assert!(!seen[edge.index()]);
                seen[edge.index()] = true;
                cycle.push(edge);
                let next = root.traffic().successors(edge).unwrap();
                assert_eq!(next.len(), 1);
                edge = next[0];
                if edge == start {
                    break;
                }
            }
            assert_eq!(cycle.len(), 64);
            routes.push(
                world
                    .register_route(rt::RouteRegisterInput::new(cycle.repeat(4)))
                    .unwrap(),
            );
        }
        assert_eq!(routes.len(), 64);
    }
    for i in 0..case.active {
        let (route, cursor, progress) = if case.ring_mm == 0 {
            (routes[(i % 16) as usize], 0, (i / 16 + 1) * 10_000)
        } else {
            (routes[i as usize], 8, case.ring_mm / 2)
        };
        world
            .spawn_vehicle(
                rt::VehicleSpawnInput::new(
                    VehicleProfileOrdinal::from_raw(0),
                    route,
                    cursor,
                    progress,
                    0,
                )
                .with_open_entrance(),
            )
            .unwrap();
    }
    let mut removed = Vec::new();
    for i in case.active..case.high_water {
        let handle = world
            .spawn_parked_vehicle(
                rt::ParkedVehicleSpawnInput::new(
                    VehicleProfileOrdinal::from_raw(0),
                    routes[0],
                    0,
                    0,
                ),
                rt::ParkingTarget::VirtualPool(ParkingFacilityOrdinal::from_raw(0)),
            )
            .unwrap()
            .vehicle;
        if i >= case.live {
            removed.push(handle);
        }
    }
    for handle in removed {
        world.despawn_vehicle(handle).unwrap();
    }
    for _ in 0..WARM {
        step(&mut world);
    }
    validate(&world, case);
    world
}

pub fn step(world: &mut rt::TrafficWorld) {
    std::hint::black_box(world.step(rt::TickInput::new(100)).unwrap());
}

pub fn validate(world: &rt::TrafficWorld, case: Case) {
    assert_eq!(world.live_vehicles().len(), case.live as usize);
    assert_eq!(
        world
            .live_vehicles()
            .iter()
            .filter(|h| world.vehicle(**h).unwrap().status() == rt::VehicleStatus::Active)
            .count(),
        case.active as usize
    );
    assert_eq!(world.traffic().lane_edge_count(), case.edges);
}

pub fn digest(world: &rt::TrafficWorld) -> String {
    format!(
        "{:x}",
        rt::deterministic_state_digest(&world.capture_snapshot().unwrap()).unwrap()
    )
}

pub fn selected() -> Vec<Case> {
    match std::env::var("LANEFLOW_SPARSE_CASE") {
        Ok(name) => vec![
            *CASES
                .iter()
                .find(|c| c.name == name)
                .expect("unknown sparse case"),
        ],
        Err(_) => CASES.to_vec(),
    }
}
