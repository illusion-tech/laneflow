//! #541 虚拟停车稀疏规模账本。
//!
//! 单独的 integration-test 进程避免 `stats_alloc::Region` 与并行测试串账。10k / 100k
//! 两个制品仅改变一个设施的声明容量；静态 retained 与相同稀疏 binding 的每世界
//! live bytes 都必须保持相同。

#[path = "support/policy.rs"]
mod test_policy;

use std::alloc::System;
use std::hint::black_box;
use std::sync::Arc;

use laneflow_compiler::{
    CompilationUnitBuilder, CompileLimits, Compiler, IidmVehicleProfileInput, LaneEdgeInput,
    LaneEdgeReference, ParkingFacilityInput, ParkingLaneAnchorInput, ParticipantClassInput,
    ParticipantClassReference, PortableDiffBase, PortableEmissionProvenance, SourceModuleHeader,
    SourceModuleHeaderInput, SyntheticModuleBuilder, VehicleProfileInput,
    derive_canonical_stable_id_v1, emit_portable_candidate,
};
use laneflow_format::{FormatLimits, check_post_emission_bundle};
use laneflow_runtime::{
    CommittedNetworkSource, ParkingFacilityOrdinal, ParkingTarget, PublishedLfcaReference,
    ReserveParkingTarget, RouteRegisterInput, TrafficWorld, VehicleSpawnInput,
    VirtualEntryAnchorSelector, WorldConfig,
};
use laneflow_static_contract::{EntityKind, LaneEdgeId, VehicleProfileOrdinal};
use laneflow_static_network::{
    SharedNetworkBuildLimits, SharedNetworkBuildOptions, SharedNetworkRevision, SpatialBuildOption,
    build_shared_network_revision,
};
use stats_alloc::{INSTRUMENTED_SYSTEM, Region, StatsAlloc};

#[global_allocator]
static GLOBAL: &StatsAlloc<System> = &INSTRUMENTED_SYSTEM;

fn compile_capacity(capacity: u32) -> Arc<SharedNetworkRevision> {
    let limits = CompileLimits::p100_initial_v1();
    let header = SourceModuleHeader::new(
        SourceModuleHeaderInput {
            authoring_namespace_id: "city/parking-scale",
            source_document_key: "parking-scale.document",
            generator_build_id: "git:0123456789abcdef",
            parameters_and_inputs_digest: [0x31; 32],
            frontend_options_digest: [0x32; 32],
            random_seed: Some(541),
            provenance: "repository:laneflow",
        },
        &limits,
    )
    .expect("source header");
    let mut module = SyntheticModuleBuilder::new(header, &limits).expect("module");
    let entries = [ParkingLaneAnchorInput {
        lane_edge: LaneEdgeReference::local("edge"),
        progress_meters: 20.0,
    }];
    let exits = [ParkingLaneAnchorInput {
        lane_edge: LaneEdgeReference::local("edge"),
        progress_meters: 80.0,
    }];
    module
        .add_participant_class(ParticipantClassInput {
            participant_class_key: "road-user",
            extends: None,
        })
        .expect("class")
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
        .expect("profile")
        .add_lane_edge(LaneEdgeInput {
            lane_edge_key: "edge",
            length_meters: 100.0,
            speed_limit_meters_per_second: 15.0,
            successors: &[],
        })
        .expect("edge")
        .add_parking_facility(ParkingFacilityInput {
            parking_facility_key: "facility",
            virtual_capacity: capacity,
            virtual_entries: &entries,
            virtual_exits: &exits,
        })
        .expect("facility");

    let mut unit = CompilationUnitBuilder::new(limits);
    unit.add_synthetic_module(module.finish().expect("finished module"))
        .expect("compilation module");
    let output = Compiler::new()
        .compile(unit.build().expect("compilation unit"))
        .expect("compiled output");
    let provenance = PortableEmissionProvenance::try_new("laneflow-parking-scale-v1")
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
    .expect("checked bundle");
    build_shared_network_revision(
        checked.canonical_network_input(),
        SharedNetworkBuildOptions::new(
            SpatialBuildOption::Omit,
            SharedNetworkBuildLimits::new(64 * 1_024 * 1_024, 16 * 1_024 * 1_024),
        ),
    )
    .expect("shared revision")
}

fn install(revision: Arc<SharedNetworkRevision>) -> TrafficWorld {
    let origin = *revision.canonical_origin();
    TrafficWorld::install(
        std::sync::Arc::clone(&revision),
        WorldConfig::new(8, 4, 1_024, 1_024, 1, 100),
        CommittedNetworkSource::Published {
            reference: PublishedLfcaReference::new(
                "fixture://parking-scale",
                origin.canonical_artifact_digest(),
                origin.canonical_artifact_byte_length(),
                origin.network_revision(),
            )
            .expect("source"),
        },
        541,
        test_policy::selection(&revision),
    )
    .expect("install")
}

fn register_route(world: &mut TrafficWorld) -> laneflow_runtime::RouteHandle {
    let limits = CompileLimits::p100_initial_v1();
    let stable =
        derive_canonical_stable_id_v1(EntityKind::LaneEdge, "city/parking-scale", "edge", &limits)
            .expect("edge stable id");
    let edge = world
        .revision()
        .identity()
        .ordinal(LaneEdgeId::from_untyped(stable))
        .expect("edge ordinal");
    world
        .register_route(RouteRegisterInput::new(vec![edge]))
        .expect("route")
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Sample {
    allocations: usize,
    reallocations: usize,
    live_bytes: usize,
}

fn sample_sparse_world(revision: Arc<SharedNetworkRevision>) -> (Sample, u64) {
    let region = Region::new(GLOBAL);
    let mut world = install(revision);
    let route = register_route(&mut world);
    let vehicle = world
        .spawn_vehicle(VehicleSpawnInput::new(
            VehicleProfileOrdinal::from_raw(0),
            route,
            0,
            0,
            0,
        ))
        .expect("vehicle");
    world
        .reserve_parking(
            vehicle,
            ReserveParkingTarget::VirtualPool {
                facility: ParkingFacilityOrdinal::from_raw(0),
                entry_anchor: VirtualEntryAnchorSelector::from_raw(0),
                entry_route_occurrence: 0,
            },
        )
        .expect("single sparse binding");
    assert_eq!(
        world
            .parking_binding(vehicle)
            .map(|binding| binding.target()),
        Some(ParkingTarget::VirtualPool(
            ParkingFacilityOrdinal::from_raw(0)
        ))
    );
    let declared = world
        .parking_facility_counts(ParkingFacilityOrdinal::from_raw(0))
        .expect("counts")
        .virtual_pool
        .capacity;
    black_box(&world);
    let stats = region.change();
    (
        Sample {
            allocations: stats.allocations,
            reallocations: stats.reallocations,
            live_bytes: stats
                .bytes_allocated
                .saturating_sub(stats.bytes_deallocated),
        },
        declared,
    )
}

#[test]
fn declared_capacity_10k_and_100k_do_not_expand_static_or_per_world_storage() {
    let ten_thousand = compile_capacity(10_000);
    let hundred_thousand = compile_capacity(100_000);
    assert_eq!(
        ten_thousand.retained_logical_bytes(),
        hundred_thousand.retained_logical_bytes(),
        "static retained must depend on F + S + A, not declared capacity C"
    );

    // 热身相同路径，隔离一次性 allocator/runtime 初始化。
    black_box(sample_sparse_world(Arc::clone(&ten_thousand)));
    let (small, small_declared) = sample_sparse_world(ten_thousand);
    let (large, large_declared) = sample_sparse_world(hundred_thousand);
    assert_eq!((small_declared, large_declared), (10_000, 100_000));
    assert_eq!(
        small, large,
        "one sparse binding must retain the same world allocation shape regardless of C"
    );
    println!(
        "parking-sparse-evidence capacities=10000/100000 allocations={} reallocations={} live_bytes={}",
        small.allocations, small.reallocations, small.live_bytes
    );
}
