//! 已提交位姿来源的单句柄查询与成员资格语义测试（#720）。
//!
//! 夹具是进程内编译的合成修订：一条 100 m 车道，其上同时有一个虚拟池停车设施
//! 和一个显式泊位。全部期望值按成员资格规则显式写出，不靠两个入口互查。

#[path = "support/policy.rs"]
mod test_policy;

use std::sync::Arc;

use laneflow_compiler::{
    CompilationUnitBuilder, CompileLimits, Compiler, IidmVehicleProfileInput, LaneEdgeInput,
    LaneEdgeReference, ParkingFacilityInput, ParkingFacilityReference, ParkingLaneAnchorInput,
    ParkingSpaceGeometryInput, ParkingSpaceInput, ParticipantClassInput, ParticipantClassReference,
    PortableDiffBase, PortableEmissionProvenance, SourceModuleHeader, SourceModuleHeaderInput,
    SyntheticModuleBuilder, VehicleProfileInput, emit_portable_candidate,
};
use laneflow_format::{FormatLimits, check_post_emission_bundle};
use laneflow_runtime::{
    CommittedNetworkSource, CommittedPoseSourceError, ParkedVehicleSpawnInput, ParkingTarget,
    PoseSource, PublishedLfcaReference, ReserveParkingTarget, RouteRegisterInput, TickInput,
    TrafficWorld, VehicleSpawnInput, VehicleStatus, WorldConfig, deterministic_state_digest,
};
use laneflow_static_contract::{
    LaneEdgeOrdinal, ParkingFacilityOrdinal, ParkingSpaceOrdinal, VehicleProfileOrdinal,
};
use laneflow_static_network::{
    SharedNetworkBuildLimits, SharedNetworkBuildOptions, SharedNetworkRevision, SpatialBuildOption,
    build_shared_network_revision,
};

const EDGE_LENGTH_MM: u32 = 100_000;
const PROFILE: VehicleProfileOrdinal = VehicleProfileOrdinal::from_raw(0);
const FACILITY: ParkingFacilityOrdinal = ParkingFacilityOrdinal::from_raw(0);
const SPACE: ParkingSpaceOrdinal = ParkingSpaceOrdinal::from_raw(0);
const EDGE: LaneEdgeOrdinal = LaneEdgeOrdinal::from_raw(0);

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

fn compile_revision(
    configure: impl FnOnce(&mut SyntheticModuleBuilder),
) -> Arc<SharedNetworkRevision> {
    let limits = CompileLimits::p100_initial_v1();
    let header = SourceModuleHeader::new(
        SourceModuleHeaderInput {
            authoring_namespace_id: "city/pose-source-query",
            source_document_key: "pose-source-query.document",
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
        .expect("compiled output");
    let provenance = PortableEmissionProvenance::try_new("laneflow-pose-source-query-v1")
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

/// 单车道 + 虚拟池设施（容量 4）+ 显式泊位，三者共用同一条边。
fn parking_revision() -> Arc<SharedNetworkRevision> {
    compile_revision(|module| {
        let anchor = |progress: f64| ParkingLaneAnchorInput {
            lane_edge: LaneEdgeReference::local("edge"),
            progress_meters: progress,
        };
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
                virtual_capacity: 4,
                virtual_entries: &[anchor(20.0)],
                virtual_exits: &[anchor(70.0)],
            })
            .expect("facility")
            .add_parking_space(ParkingSpaceInput {
                parking_space_key: "space",
                parking_facility: Some(ParkingFacilityReference::local("facility")),
                entry: anchor(90.0),
                exit: anchor(95.0),
                geometry: ParkingSpaceGeometryInput {
                    lateral_offset_meters: -3.0,
                    heading_offset_radians: 0.25,
                    length_meters: 5.5,
                    width_meters: 2.6,
                },
            })
            .expect("space");
    })
}

fn world() -> TrafficWorld {
    let revision = parking_revision();
    let origin = *revision.canonical_origin();
    TrafficWorld::install(
        Arc::clone(&revision),
        WorldConfig::new(8, 4, 1_024, 1_024, 100),
        laneflow_runtime::ExecutionConfig::new(std::num::NonZeroU32::MIN),
        CommittedNetworkSource::Published {
            reference: PublishedLfcaReference::new(
                "fixture://pose-source-query",
                origin.canonical_artifact_digest(),
                origin.canonical_artifact_byte_length(),
                origin.network_revision(),
            )
            .expect("fixture key"),
        },
        0,
        test_policy::selection(&revision),
    )
    .expect("install")
}

fn route(world: &mut TrafficWorld) -> laneflow_runtime::RouteHandle {
    world
        .register_route(RouteRegisterInput::new(vec![EDGE]))
        .expect("register single-edge route")
}

fn spawn_active(
    world: &mut TrafficWorld,
    route: laneflow_runtime::RouteHandle,
    progress_mm: u32,
) -> laneflow_runtime::VehicleHandle {
    world
        .spawn_vehicle(
            VehicleSpawnInput::new(PROFILE, route, 0, progress_mm, 0).with_open_entrance(),
        )
        .expect("spawn active vehicle")
}

fn spawn_virtual_parked(
    world: &mut TrafficWorld,
    route: laneflow_runtime::RouteHandle,
) -> laneflow_runtime::VehicleHandle {
    world
        .spawn_parked_vehicle(
            ParkedVehicleSpawnInput::new(PROFILE, route, 0, 0),
            ParkingTarget::VirtualPool(FACILITY),
        )
        .expect("spawn virtual parked vehicle")
        .vehicle
}

fn lane_source(progress_mm: u32) -> PoseSource {
    PoseSource::Lane {
        edge: EDGE,
        progress_mm,
    }
}

fn full_sources(world: &TrafficWorld) -> Vec<(laneflow_runtime::VehicleHandle, PoseSource)> {
    world.committed_pose_sources().collect::<Vec<_>>()
}

/// 迭代器作用域内消费完毕后，世界恢复可步进（compile_fail 的正向对照）。
#[test]
fn iterator_scope_ends_then_world_can_step() {
    let mut world = world();
    let route = route(&mut world);
    let _vehicle = spawn_active(&mut world, route, 10_000);

    let count = {
        let sources = world.committed_pose_sources();
        sources.count()
    };
    assert_eq!(count, 1);
    world
        .step(TickInput::new(100))
        .expect("step after iterator scope ends");
}

/// Active 车辆映射为当前边与进度的车道来源；期望值显式写出。
#[test]
fn active_vehicle_maps_to_expected_lane_source() {
    let mut world = world();
    let route = route(&mut world);
    let vehicle = spawn_active(&mut world, route, 40_000);

    assert_eq!(
        world.committed_pose_source(vehicle),
        Ok(Some(lane_source(40_000)))
    );
    assert_eq!(full_sources(&world), vec![(vehicle, lane_source(40_000))]);
}

/// 已预约显式泊位但尚未停入的 Active 车辆仍是车道来源。
#[test]
fn reserved_active_vehicle_keeps_lane_source() {
    let mut world = world();
    let route = route(&mut world);
    let vehicle = spawn_active(&mut world, route, 80_000);

    world
        .reserve_parking(
            vehicle,
            ReserveParkingTarget::ExplicitSpace {
                space: SPACE,
                entry_route_occurrence: 0,
            },
        )
        .expect("reserve explicit space");
    assert_eq!(
        world
            .parking_binding(vehicle)
            .map(|binding| binding.target()),
        Some(ParkingTarget::ExplicitSpace(SPACE))
    );

    assert_eq!(
        world.committed_pose_source(vehicle),
        Ok(Some(lane_source(80_000)))
    );
}

/// 停入显式泊位的车辆映射为该泊位来源，并保留在 live 顺序中。
#[test]
fn explicitly_parked_vehicle_maps_to_expected_parking_source() {
    let mut world = world();
    let route = route(&mut world);
    let vehicle = spawn_active(&mut world, route, 90_000);
    world
        .reserve_parking(
            vehicle,
            ReserveParkingTarget::ExplicitSpace {
                space: SPACE,
                entry_route_occurrence: 0,
            },
        )
        .expect("reserve explicit space");
    world
        .park_vehicle(vehicle, ParkingTarget::ExplicitSpace(SPACE))
        .expect("park into reserved space");

    let parking = PoseSource::Parking { space: SPACE };
    assert_eq!(world.committed_pose_source(vehicle), Ok(Some(parking)));
    assert_eq!(full_sources(&world), vec![(vehicle, parking)]);
}

/// 虚拟池停入的车辆合法但当前无 pose 来源；全量查询省略它。
#[test]
fn virtual_pool_parked_vehicle_has_no_source() {
    let mut world = world();
    let route = route(&mut world);
    let vehicle = spawn_virtual_parked(&mut world, route);

    assert_eq!(world.committed_pose_source(vehicle), Ok(None));
    assert!(full_sources(&world).is_empty());
}

/// Completed 车辆句柄仍可读，但当前无 pose 来源。
#[test]
fn completed_vehicle_is_queryable_but_has_no_source() {
    let mut world = world();
    let route = route(&mut world);
    let vehicle = spawn_active(&mut world, route, EDGE_LENGTH_MM - 500);

    for _ in 0..16 {
        world.step(TickInput::new(100)).expect("step");
        if full_sources(&world).is_empty() {
            break;
        }
    }
    assert!(full_sources(&world).is_empty(), "vehicle must complete");
    assert_eq!(
        world.vehicle(vehicle).expect("retained").status(),
        VehicleStatus::Completed
    );
    assert_eq!(world.committed_pose_source(vehicle), Ok(None));
}

/// despawn 后的句柄返回结构化错误；同槽位复用的新车辆不接管旧句柄。
#[test]
fn stale_handles_from_despawn_and_slot_reuse_return_structured_error() {
    let mut world = world();
    let route = route(&mut world);
    let parked = spawn_virtual_parked(&mut world, route);
    world
        .despawn_vehicle(parked)
        .expect("despawn virtual parked vehicle");

    assert_eq!(
        world.committed_pose_source(parked),
        Err(CommittedPoseSourceError::UnknownVehicle { handle: parked })
    );
    assert!(full_sources(&world).is_empty());

    let replacement = spawn_active(&mut world, route, 12_345);
    assert_ne!(replacement, parked, "slot reuse must mint a new generation");
    assert_eq!(
        world.committed_pose_source(replacement),
        Ok(Some(lane_source(12_345)))
    );
    assert_eq!(
        world.committed_pose_source(parked),
        Err(CommittedPoseSourceError::UnknownVehicle { handle: parked })
    );
    assert_eq!(
        full_sources(&world),
        vec![(replacement, lane_source(12_345))]
    );
}

/// 混合状态世界按 live 顺序输出显式期望序列：virtual Parked 被省略、
/// 其余成员按产生顺序排列。
#[test]
fn mixed_world_full_sequence_has_explicit_expected_values() {
    let mut world = world();
    let route = route(&mut world);
    let active = spawn_active(&mut world, route, 10_000);
    let virtual_parked = spawn_virtual_parked(&mut world, route);
    let mut explicit = spawn_active(&mut world, route, 90_000);
    let _ = &mut explicit;
    world
        .reserve_parking(
            explicit,
            ReserveParkingTarget::ExplicitSpace {
                space: SPACE,
                entry_route_occurrence: 0,
            },
        )
        .expect("reserve explicit space");
    world
        .park_vehicle(explicit, ParkingTarget::ExplicitSpace(SPACE))
        .expect("park into reserved space");

    assert_eq!(
        full_sources(&world),
        vec![
            (active, lane_source(10_000)),
            (explicit, PoseSource::Parking { space: SPACE }),
        ]
    );
    assert_eq!(
        world.committed_pose_source(virtual_parked),
        Ok(None),
        "virtual parked member is omitted from the sequence"
    );
}

/// 完整遍历与单句柄查询都是只读操作：已提交状态摘要不变。
#[test]
fn queries_do_not_change_committed_state() {
    let mut world = world();
    let route = route(&mut world);
    let active = spawn_active(&mut world, route, 30_000);
    let parked = spawn_virtual_parked(&mut world, route);
    let _ = (active, parked);

    let before_digest = deterministic_state_digest(&world.capture_snapshot().expect("snapshot"))
        .expect("digest before");
    let before_sources = full_sources(&world);
    let handles: Vec<_> = world.live_vehicles().to_vec();
    for handle in &handles {
        let _ = world.committed_pose_source(*handle);
    }
    let after_digest = deterministic_state_digest(&world.capture_snapshot().expect("snapshot"))
        .expect("digest after");

    assert_eq!(before_digest, after_digest);
    assert_eq!(full_sources(&world), before_sources);
}
