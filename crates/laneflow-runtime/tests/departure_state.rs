#[path = "support/policy.rs"]
mod test_policy;

use std::sync::Arc;

use laneflow_compiler::{
    CompilationUnitBuilder, CompileLimits, Compiler, IidmVehicleProfileInput, LaneEdgeInput,
    ParticipantClassInput, ParticipantClassReference, PortableDiffBase, PortableEmissionProvenance,
    SourceModuleHeader, SourceModuleHeaderInput, SyntheticModuleBuilder, VehicleProfileInput,
    derive_canonical_stable_id_v1, emit_portable_candidate,
};
use laneflow_format::{FormatLimits, check_post_emission_bundle};
use laneflow_runtime::{
    CommittedNetworkSource, DepartureStateError, ExecutionConfig, PublishedLfcaReference,
    ReplaceError, RouteRegisterInput, SpawnError, TickInput, TrafficWorld, VehicleDepartureState,
    VehicleSpawnInput, VehicleStatus, WorldConfig,
};
use laneflow_static_contract::{EntityKind, LaneEdgeId};
use laneflow_static_network::{
    SharedNetworkBuildLimits, SharedNetworkBuildOptions, SharedNetworkRevision, SpatialBuildOption,
    build_shared_network_revision,
};

fn install(delta_ms: u32) -> TrafficWorld {
    let revision = compile_road();
    TrafficWorld::install(
        Arc::clone(&revision),
        WorldConfig::new(8, 4, 64, 4, u64::from(delta_ms)),
        ExecutionConfig::new(std::num::NonZeroU32::MIN),
        published(&revision),
        0,
        test_policy::selection(&revision),
    )
    .expect("install")
}

fn published(revision: &SharedNetworkRevision) -> CommittedNetworkSource {
    let origin = *revision.canonical_origin();
    CommittedNetworkSource::Published {
        reference: PublishedLfcaReference::new(
            "fixture://departure",
            origin.canonical_artifact_digest(),
            origin.canonical_artifact_byte_length(),
            origin.network_revision(),
        )
        .expect("key"),
    }
}

fn compile_road() -> Arc<SharedNetworkRevision> {
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
    .expect("header");
    let mut module = SyntheticModuleBuilder::new(header, &limits).expect("module");
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
            lane_edge_key: "stem",
            length_meters: 20.0,
            speed_limit_meters_per_second: 15.0,
            successors: &[],
        })
        .expect("stem")
        .add_lane_edge(LaneEdgeInput {
            lane_edge_key: "road",
            length_meters: 250.0,
            speed_limit_meters_per_second: 15.0,
            successors: &[],
        })
        .expect("road");
    let mut unit = CompilationUnitBuilder::new(limits);
    unit.add_synthetic_module(module.finish().expect("module"))
        .expect("unit");
    let output = Compiler::new()
        .compile(unit.build().expect("unit"))
        .expect("compile");
    let provenance =
        PortableEmissionProvenance::try_new("laneflow-runtime-coverage-v1").expect("provenance");
    let candidate = emit_portable_candidate(
        &output,
        &provenance,
        FormatLimits::HARD,
        PortableDiffBase::Genesis,
    )
    .expect("candidate");
    let checked = check_post_emission_bundle(
        candidate.canonical_artifact().bytes(),
        candidate.source_map().bytes(),
        candidate.semantic_diff().bytes(),
        candidate.expected_semantic_diff_base(),
        FormatLimits::HARD,
    )
    .expect("checked");
    build_shared_network_revision(
        checked.canonical_network_input(),
        SharedNetworkBuildOptions::new(
            SpatialBuildOption::Omit,
            SharedNetworkBuildLimits::new(64 * 1_024 * 1_024, 16 * 1_024 * 1_024),
        ),
    )
    .expect("revision")
}

fn route(world: &mut TrafficWorld, keys: &[&str]) -> laneflow_runtime::RouteHandle {
    let limits = CompileLimits::p100_initial_v1();
    let edges: Vec<_> = keys
        .iter()
        .map(|key| {
            let stable = derive_canonical_stable_id_v1(
                EntityKind::LaneEdge,
                "city/runtime-coverage",
                key,
                &limits,
            )
            .expect("id");
            world
                .revision()
                .identity()
                .ordinal(LaneEdgeId::from_untyped(stable))
                .expect(key)
        })
        .collect();
    world
        .register_route(RouteRegisterInput::new(edges))
        .expect("route")
}

fn departed(input: VehicleSpawnInput, index: u32, progress: u32, speed: u32) -> VehicleSpawnInput {
    input.with_departure(VehicleDepartureState::new(index, progress, speed))
}

#[test]
fn undeclared_fast_spawn_stays_accepted_and_a_zero_departure_rejects_it() {
    let mut world = install(100);
    let road = route(&mut world, &["road"]);
    let fast = VehicleSpawnInput::new(
        laneflow_static_contract::VehicleProfileOrdinal::from_raw(0),
        road,
        0,
        5_000,
        10_000,
    );
    world
        .spawn_vehicle(fast)
        .expect("no departure does not invent a zero history");
    assert_eq!(world.live_vehicles().len(), 1);

    let mut again = install(100);
    let road = route(&mut again, &["road"]);
    let cursor = again.command_cursor();
    let rejected = again.spawn_vehicle(departed(
        VehicleSpawnInput::new(
            laneflow_static_contract::VehicleProfileOrdinal::from_raw(0),
            road,
            0,
            5_000,
            10_000,
        ),
        0,
        0,
        0,
    ));
    assert_eq!(rejected, Err(SpawnError::InitialSpeedExceedsDepartureBound));
    assert_eq!(again.command_cursor(), cursor);
    assert_eq!(again.live_vehicles().len(), 0);
    let repeated = again.spawn_vehicle(departed(fast, 0, 0, 0));
    assert_eq!(repeated, rejected);
    assert_eq!(again.command_cursor(), cursor);
}

#[test]
fn a_departure_at_least_as_fast_as_the_initial_speed_does_not_add_a_bound() {
    let mut plain = install(100);
    let road = route(&mut plain, &["road"]);
    let profile = laneflow_static_contract::VehicleProfileOrdinal::from_raw(0);
    let input = VehicleSpawnInput::new(profile, road, 0, 80_000, 10_000);
    let first = plain.spawn_vehicle(input).expect("undeclared");
    let plain_speed = plain.vehicle(first).expect("live").speed_mm_s();

    let mut declared = install(100);
    let road = route(&mut declared, &["road"]);
    let input = VehicleSpawnInput::new(profile, road, 0, 80_000, 10_000);
    let kept = departed(input, 0, 0, 10_000);
    assert_eq!(kept.departure().expect("kept").speed_mm_s(), 10_000);
    let second = declared
        .spawn_vehicle(kept)
        .expect("departure speed is enough");
    assert_eq!(
        declared.vehicle(second).expect("live").speed_mm_s(),
        plain_speed
    );
}

#[test]
fn invalid_departure_reasons_stay_distinct_and_do_not_commit() {
    let mut world = install(100);
    let road = route(&mut world, &["road"]);
    let profile = laneflow_static_contract::VehicleProfileOrdinal::from_raw(0);
    let input = VehicleSpawnInput::new(profile, road, 0, 1_000, 0);
    let cursor = world.command_cursor();
    assert_eq!(
        world.spawn_vehicle(departed(input, 3, 0, 0)),
        Err(SpawnError::InvalidDepartureState(
            DepartureStateError::RouteIndexOutOfRange
        ))
    );
    assert_eq!(
        world.spawn_vehicle(departed(input, 0, 250_000 + 1, 0)),
        Err(SpawnError::InvalidDepartureState(
            DepartureStateError::InvalidProgress
        ))
    );
    assert_eq!(
        world.spawn_vehicle(departed(input, 0, 2_000, 0)),
        Err(SpawnError::InvalidDepartureState(
            DepartureStateError::AfterPlacement
        ))
    );
    assert_eq!(
        world.spawn_vehicle(departed(input, 0, 2_000, 5_000)),
        Err(SpawnError::InvalidDepartureState(
            DepartureStateError::AfterPlacement
        ))
    );
    assert_eq!(
        world.spawn_vehicle(departed(input, 0, 0, 100_001)),
        Err(SpawnError::InvalidDepartureState(
            DepartureStateError::SpeedOutOfRange
        ))
    );
    assert_eq!(world.command_cursor(), cursor);
    assert_eq!(world.live_vehicles().len(), 0);
}

#[test]
fn lowering_the_initial_speed_keeps_the_departure_declaration() {
    let mut world = install(100);
    let road = route(&mut world, &["road"]);
    let profile = laneflow_static_contract::VehicleProfileOrdinal::from_raw(0);
    let fast = departed(
        VehicleSpawnInput::new(profile, road, 0, 5_000, 10_000),
        0,
        0,
        0,
    );
    assert!(world.spawn_vehicle(fast).is_err());
    let slower = departed(VehicleSpawnInput::new(profile, road, 0, 5_000, 0), 0, 0, 0);
    assert_eq!(slower.departure().expect("still declared").progress_mm(), 0);
    world
        .spawn_vehicle(slower)
        .expect("zero speed is inside a zero departure");
}

#[test]
fn a_longer_tick_still_rejects_a_departure_that_is_too_close() {
    let profile = laneflow_static_contract::VehicleProfileOrdinal::from_raw(0);
    let mut coarse = install(1_000);
    let road = route(&mut coarse, &["road"]);
    let near = departed(
        VehicleSpawnInput::new(profile, road, 0, 2_000, 4_000),
        0,
        0,
        0,
    );
    assert_eq!(
        coarse.spawn_vehicle(near),
        Err(SpawnError::InitialSpeedExceedsDepartureBound)
    );
}

#[test]
fn replace_rejects_the_same_departure_bound_without_retiring_the_old_vehicle() {
    let mut world = install(100);
    let road = route(&mut world, &["road"]);
    let profile = laneflow_static_contract::VehicleProfileOrdinal::from_raw(0);
    let length = world.traffic().lane_lengths_millimetres()
        [world.route_edges(road).expect("edges")[0].index()];
    let completed = world
        .spawn_vehicle(VehicleSpawnInput::new(profile, road, 0, length, 0))
        .expect("at the end");
    world.step(TickInput::new(100)).expect("complete");
    assert_eq!(
        world.vehicle(completed).expect("kept").status(),
        VehicleStatus::Completed
    );
    let cursor = world.command_cursor();
    let replaced = world.replace_completed_vehicle(
        completed,
        departed(
            VehicleSpawnInput::new(profile, road, 0, 5_000, 10_000),
            0,
            0,
            0,
        ),
    );
    assert_eq!(
        replaced,
        Err(ReplaceError::InitialSpeedExceedsDepartureBound)
    );
    assert_eq!(world.command_cursor(), cursor);
    assert_eq!(
        world.vehicle(completed).expect("still completed").status(),
        VehicleStatus::Completed
    );
}

#[test]
fn a_bad_departure_is_not_reported_as_overlap() {
    let mut world = install(100);
    let road = route(&mut world, &["road"]);
    let profile = laneflow_static_contract::VehicleProfileOrdinal::from_raw(0);
    world
        .spawn_vehicle(VehicleSpawnInput::new(profile, road, 0, 20_000, 0))
        .expect("first vehicle");
    let occupied = departed(VehicleSpawnInput::new(profile, road, 0, 20_000, 0), 9, 0, 0);
    assert_eq!(
        world.spawn_vehicle(occupied),
        Err(SpawnError::InvalidDepartureState(
            DepartureStateError::RouteIndexOutOfRange
        ))
    );
}

#[cfg(feature = "placement-fixtures")]
#[test]
fn an_existing_state_fixture_does_not_apply_the_departure_bound() {
    let mut world = install(100);
    let road = route(&mut world, &["road"]);
    let profile = laneflow_static_contract::VehicleProfileOrdinal::from_raw(0);
    let input = departed(
        VehicleSpawnInput::new(profile, road, 0, 5_000, 10_000),
        0,
        0,
        0,
    );
    world
        .place_existing_active_vehicle(input)
        .expect("fixtures skip the fresh departure bound");
}
