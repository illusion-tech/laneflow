use std::sync::Arc;

use laneflow_compiler::{
    AccessRuleInput, AccessRuleTargetInput, CompilationUnitBuilder, CompileLimits, Compiler,
    IidmVehicleProfileInput, LaneEdgeInput, LaneEdgeReference, ParkingLaneAnchorInput,
    ParkingSpaceGeometryInput, ParkingSpaceInput, ParticipantClassInput, ParticipantClassReference,
    PortableDiffBase, PortableEmissionProvenanceV1, SourceModuleHeader, SourceModuleHeaderInput,
    StaticRouteInput, SyntheticModuleBuilder, VehicleProfileInput, emit_portable_candidate,
};
use laneflow_format::{FormatLimits, check_post_emission_bundle_v1};
use laneflow_runtime::{
    ParkingError, PoseSource, RouteRegisterInput, SpawnError, TickInput, TrafficWorld,
    VehicleSpawnInput, WorldConfig,
};
use laneflow_static_contract::{AccessEffect, ParkingSpaceOrdinal, VehicleProfileOrdinal};
use laneflow_static_network::{
    SharedNetworkBuildLimits, SharedNetworkBuildOptions, SharedNetworkRevision, SpatialBuildOption,
    build_shared_network_revision,
};

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
        .expect("compiled output");
    let provenance = PortableEmissionProvenanceV1::try_new("laneflow-runtime-coverage-v1")
        .expect("portable provenance");
    let candidate = emit_portable_candidate(
        &output,
        &provenance,
        FormatLimits::V1_HARD,
        PortableDiffBase::Genesis,
    )
    .expect("portable candidate");
    let checked = check_post_emission_bundle_v1(
        candidate.canonical_artifact().bytes(),
        candidate.source_map().bytes(),
        candidate.semantic_diff().bytes(),
        candidate.expected_semantic_diff_base(),
        FormatLimits::V1_HARD,
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

#[test]
fn spawn_access_denied_on_static_and_dynamic_routes_leaves_no_vehicle() {
    let revision = compile_revision(|module| {
        add_standard_profiles(module);
        module
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: "stem",
                length_meters: 10.0,
                speed_limit_meters_per_second: 10.0,
                successors: &[LaneEdgeReference::local("tail")],
            })
            .expect("stem")
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: "tail",
                length_meters: 10.0,
                speed_limit_meters_per_second: 10.0,
                successors: &[],
            })
            .expect("tail")
            .add_access_rule(AccessRuleInput {
                access_rule_key: "deny-on-tail",
                target: AccessRuleTargetInput::LaneEdge(LaneEdgeReference::local("tail")),
                effect: AccessEffect::Deny,
                participant_classes: &[ParticipantClassReference::local("road-user")],
                regulation: None,
                priority: 0,
            })
            .expect("deny rule")
            .add_static_route(StaticRouteInput {
                static_route_key: "through",
                edge_sequence: &[
                    LaneEdgeReference::local("stem"),
                    LaneEdgeReference::local("tail"),
                ],
            })
            .expect("static route");
    });
    let mut world =
        TrafficWorld::install(revision, WorldConfig::new(8, 4, 1, 100)).expect("install");
    let static_route = world
        .static_route(laneflow_static_contract::StaticRouteOrdinal::from_raw(0))
        .expect("static");
    assert_eq!(
        world
            .spawn_vehicle(VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                static_route,
                0,
                0.0,
                0.0,
            ))
            .unwrap_err(),
        SpawnError::AccessDenied
    );
    assert!(world.committed_pose_sources().as_slice().is_empty());

    let edges: Vec<_> = world
        .traffic()
        .relations()
        .static_route_edges(laneflow_static_contract::StaticRouteOrdinal::from_raw(0))
        .expect("edges")
        .to_vec();
    let dynamic = world
        .register_route(RouteRegisterInput::new(edges))
        .expect("dynamic");
    assert_eq!(
        world
            .spawn_vehicle(VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                dynamic,
                0,
                0.0,
                0.0,
            ))
            .unwrap_err(),
        SpawnError::AccessDenied
    );
    assert!(world.committed_pose_sources().as_slice().is_empty());
}

#[test]
fn occupy_other_parking_space_fails_when_already_parked() {
    let revision = compile_revision(|module| {
        add_standard_profiles(module);
        module
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: "edge",
                length_meters: 20.0,
                speed_limit_meters_per_second: 10.0,
                successors: &[],
            })
            .expect("edge")
            .add_static_route(StaticRouteInput {
                static_route_key: "route",
                edge_sequence: &[LaneEdgeReference::local("edge")],
            })
            .expect("route")
            .add_parking_space(ParkingSpaceInput {
                parking_space_key: "space-a",
                parking_area: None,
                entry: ParkingLaneAnchorInput {
                    lane_edge: LaneEdgeReference::local("edge"),
                    progress_meters: 4.0,
                },
                exit: ParkingLaneAnchorInput {
                    lane_edge: LaneEdgeReference::local("edge"),
                    progress_meters: 5.0,
                },
                geometry: ParkingSpaceGeometryInput {
                    lateral_offset_meters: -3.0,
                    heading_offset_radians: 0.25,
                    length_meters: 5.5,
                    width_meters: 2.6,
                },
            })
            .expect("space-a")
            .add_parking_space(ParkingSpaceInput {
                parking_space_key: "space-b",
                parking_area: None,
                entry: ParkingLaneAnchorInput {
                    lane_edge: LaneEdgeReference::local("edge"),
                    progress_meters: 12.0,
                },
                exit: ParkingLaneAnchorInput {
                    lane_edge: LaneEdgeReference::local("edge"),
                    progress_meters: 13.0,
                },
                geometry: ParkingSpaceGeometryInput {
                    lateral_offset_meters: -3.0,
                    heading_offset_radians: 0.25,
                    length_meters: 5.5,
                    width_meters: 2.6,
                },
            })
            .expect("space-b");
    });
    let mut world =
        TrafficWorld::install(revision, WorldConfig::new(8, 4, 1, 100)).expect("install");
    let route = world
        .static_route(laneflow_static_contract::StaticRouteOrdinal::from_raw(0))
        .expect("route");
    let vehicle = world
        .spawn_vehicle(VehicleSpawnInput::new(
            VehicleProfileOrdinal::from_raw(0),
            route,
            0,
            0.0,
            0.0,
        ))
        .expect("spawn");
    world
        .occupy_parking(vehicle, ParkingSpaceOrdinal::from_raw(0))
        .expect("first space");
    assert_eq!(
        world
            .occupy_parking(vehicle, ParkingSpaceOrdinal::from_raw(1))
            .unwrap_err(),
        ParkingError::VehicleBoundToOtherSpace
    );
}

#[test]
fn follower_on_diverge_respects_leader_overhang_on_shared_stem() {
    let revision = compile_revision(|module| {
        add_standard_profiles(module);
        module
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: "stem",
                length_meters: 10.0,
                speed_limit_meters_per_second: 10.0,
                successors: &[
                    LaneEdgeReference::local("left"),
                    LaneEdgeReference::local("right"),
                ],
            })
            .expect("stem")
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: "left",
                length_meters: 20.0,
                speed_limit_meters_per_second: 10.0,
                successors: &[],
            })
            .expect("left")
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: "right",
                length_meters: 20.0,
                speed_limit_meters_per_second: 10.0,
                successors: &[],
            })
            .expect("right");
    });
    let traffic = revision.traffic();
    let count = traffic.lane_edge_count();
    let stem = (0..count)
        .map(laneflow_static_contract::LaneEdgeOrdinal::from_raw)
        .find(|edge| {
            traffic
                .successors(*edge)
                .is_some_and(|successors| successors.len() == 2)
        })
        .expect("stem");
    let branches = traffic.successors(stem).expect("branches");
    let left = branches[0];
    let right = branches[1];
    let mut world =
        TrafficWorld::install(revision, WorldConfig::new(8, 4, 1, 100)).expect("install");
    let leader_route = world
        .register_route(RouteRegisterInput::new(vec![stem, left]))
        .expect("left route");
    let follower_route = world
        .register_route(RouteRegisterInput::new(vec![stem, right]))
        .expect("right route");
    world
        .spawn_vehicle(VehicleSpawnInput::new(
            VehicleProfileOrdinal::from_raw(0),
            leader_route,
            1,
            0.5,
            0.0,
        ))
        .expect("leader on left, tail on stem");
    let follower = world
        .spawn_vehicle(VehicleSpawnInput::new(
            VehicleProfileOrdinal::from_raw(0),
            follower_route,
            0,
            5.0,
            10.0,
        ))
        .expect("follower on stem");
    world.step(TickInput::new(100)).expect("step");
    let PoseSource::Lane { progress, .. } = world
        .committed_pose_sources()
        .as_slice()
        .iter()
        .find(|(handle, _)| *handle == follower)
        .expect("follower pose")
        .1
    else {
        panic!("follower must stay on lane");
    };
    assert!(
        progress < 6.0 - 1e-6,
        "follower must not enter leader overhang on stem, progress={progress}"
    );
}

#[test]
fn large_delta_travel_does_not_exceed_speed_limit_envelope() {
    let revision = compile_revision(|module| {
        add_standard_profiles(module);
        module
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: "edge",
                length_meters: 1_000.0,
                speed_limit_meters_per_second: 10.0,
                successors: &[],
            })
            .expect("edge")
            .add_static_route(StaticRouteInput {
                static_route_key: "route",
                edge_sequence: &[LaneEdgeReference::local("edge")],
            })
            .expect("route");
    });
    let mut world =
        TrafficWorld::install(revision, WorldConfig::new(8, 4, 1, 20_000)).expect("install");
    let route = world
        .static_route(laneflow_static_contract::StaticRouteOrdinal::from_raw(0))
        .expect("route");
    world
        .spawn_vehicle(VehicleSpawnInput::new(
            VehicleProfileOrdinal::from_raw(0),
            route,
            0,
            0.0,
            0.0,
        ))
        .expect("spawn");
    world.step(TickInput::new(20_000)).expect("step");
    let PoseSource::Lane { progress, .. } = world.committed_pose_sources().as_slice()[0].1 else {
        panic!("lane pose");
    };
    assert!(
        progress <= 10.0 * 20.0 + 1e-6,
        "travel must not exceed speed-limit envelope, progress={progress}"
    );
}

#[test]
fn speed_down_transition_caps_next_tick_travel() {
    let revision = compile_revision(|module| {
        add_standard_profiles(module);
        module
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: "fast",
                length_meters: 20.0,
                speed_limit_meters_per_second: 10.0,
                successors: &[LaneEdgeReference::local("slow")],
            })
            .expect("fast")
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: "slow",
                length_meters: 100.0,
                speed_limit_meters_per_second: 1.0,
                successors: &[],
            })
            .expect("slow")
            .add_static_route(StaticRouteInput {
                static_route_key: "route",
                edge_sequence: &[
                    LaneEdgeReference::local("fast"),
                    LaneEdgeReference::local("slow"),
                ],
            })
            .expect("route");
    });
    let mut world =
        TrafficWorld::install(revision, WorldConfig::new(8, 4, 1, 1_000)).expect("install");
    let route = world
        .static_route(laneflow_static_contract::StaticRouteOrdinal::from_raw(0))
        .expect("route");
    let vehicle = world
        .spawn_vehicle(VehicleSpawnInput::new(
            VehicleProfileOrdinal::from_raw(0),
            route,
            0,
            18.0,
            10.0,
        ))
        .expect("spawn near fast/slow boundary");
    world.step(TickInput::new(1_000)).expect("approach/cross");
    let PoseSource::Lane {
        edge: after_first,
        progress: first_progress,
    } = world
        .committed_pose_sources()
        .as_slice()
        .iter()
        .find(|(handle, _)| *handle == vehicle)
        .expect("pose")
        .1
    else {
        panic!("lane pose");
    };
    world.step(TickInput::new(1_000)).expect("slow edge tick");
    let PoseSource::Lane {
        edge: after_second,
        progress: second_progress,
    } = world
        .committed_pose_sources()
        .as_slice()
        .iter()
        .find(|(handle, _)| *handle == vehicle)
        .expect("pose")
        .1
    else {
        panic!("lane pose");
    };
    let travelled = if after_second == after_first {
        second_progress - first_progress
    } else {
        second_progress
    };
    assert!(
        travelled <= 1.0 + 1e-6,
        "tick on or after 1 m/s edge must not keep a 10 m/s envelope, travelled={travelled}, first={first_progress:?} {after_first:?}, second={second_progress:?} {after_second:?}"
    );
}

#[test]
fn equal_limit_edge_boundary_does_not_stop_the_vehicle() {
    let revision = compile_revision(|module| {
        add_standard_profiles(module);
        module
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: "a",
                length_meters: 20.0,
                speed_limit_meters_per_second: 10.0,
                successors: &[LaneEdgeReference::local("b")],
            })
            .expect("a")
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: "b",
                length_meters: 100.0,
                speed_limit_meters_per_second: 10.0,
                successors: &[],
            })
            .expect("b")
            .add_static_route(StaticRouteInput {
                static_route_key: "route",
                edge_sequence: &[LaneEdgeReference::local("a"), LaneEdgeReference::local("b")],
            })
            .expect("route");
    });
    let mut world =
        TrafficWorld::install(revision, WorldConfig::new(8, 4, 1, 100)).expect("install");
    let route = world
        .static_route(laneflow_static_contract::StaticRouteOrdinal::from_raw(0))
        .expect("route");
    let vehicle = world
        .spawn_vehicle(VehicleSpawnInput::new(
            VehicleProfileOrdinal::from_raw(0),
            route,
            0,
            19.6,
            10.0,
        ))
        .expect("spawn near equal-limit boundary");
    world.step(TickInput::new(100)).expect("step");
    let PoseSource::Lane { edge, progress } = world
        .committed_pose_sources()
        .as_slice()
        .iter()
        .find(|(handle, _)| *handle == vehicle)
        .expect("pose")
        .1
    else {
        panic!("lane pose");
    };
    assert!(
        (progress - 20.0).abs() > 1e-6,
        "equal-limit crossing must not stop at the first-edge end, edge={edge:?} progress={progress}"
    );
}

#[test]
fn infeasible_stop_before_lower_limit_still_enters() {
    let revision = compile_revision(|module| {
        add_standard_profiles(module);
        module
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: "fast",
                length_meters: 10.0,
                speed_limit_meters_per_second: 10.0,
                successors: &[LaneEdgeReference::local("slower")],
            })
            .expect("fast")
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: "slower",
                length_meters: 100.0,
                speed_limit_meters_per_second: 8.0,
                successors: &[],
            })
            .expect("slower")
            .add_static_route(StaticRouteInput {
                static_route_key: "route",
                edge_sequence: &[
                    LaneEdgeReference::local("fast"),
                    LaneEdgeReference::local("slower"),
                ],
            })
            .expect("route");
    });
    let mut world =
        TrafficWorld::install(revision, WorldConfig::new(8, 4, 1, 1_000)).expect("install");
    let route = world
        .static_route(laneflow_static_contract::StaticRouteOrdinal::from_raw(0))
        .expect("route");
    let vehicle = world
        .spawn_vehicle(VehicleSpawnInput::new(
            VehicleProfileOrdinal::from_raw(0),
            route,
            0,
            9.0,
            10.0,
        ))
        .expect("spawn 1 m before a 10→8 drop");
    world.step(TickInput::new(1_000)).expect("step");
    let PoseSource::Lane { edge, progress } = world
        .committed_pose_sources()
        .as_slice()
        .iter()
        .find(|(handle, _)| *handle == vehicle)
        .expect("pose")
        .1
    else {
        panic!("lane pose");
    };
    let first = world
        .traffic()
        .relations()
        .static_route_edges(laneflow_static_contract::StaticRouteOrdinal::from_raw(0))
        .expect("edges")[0];
    assert!(
        edge != first || (progress - 10.0).abs() > 1e-6,
        "must enter when even a stop this tick overshoots the slower-edge start, edge={edge:?} progress={progress}"
    );
}
