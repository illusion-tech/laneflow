#[path = "support/policy.rs"]
mod test_policy;

use std::sync::Arc;

use laneflow_compiler::{
    CompilationUnitBuilder, CompileLimits, Compiler, IidmVehicleProfileInput, LaneEdgeInput,
    LaneEdgeReference, ParticipantClassInput, ParticipantClassReference, PortableDiffBase,
    PortableEmissionProvenance, SourceModuleHeader, SourceModuleHeaderInput,
    SyntheticModuleBuilder, VehicleProfileInput, derive_canonical_stable_id_v1,
    emit_portable_candidate,
};
use laneflow_format::{FormatLimits, check_post_emission_bundle};
use laneflow_runtime::{
    CommittedNetworkSource, EntranceBodyError, EntranceDirection, ExecutionConfig,
    PublishedLfcaReference, ReplaceError, RouteHandle, RouteRegisterInput, SpawnError, TickInput,
    TrafficWorld, VehicleEntrance, VehicleSpawnInput, VehicleStatus, WorldConfig,
};
use laneflow_static_contract::{EntityKind, LaneEdgeId, VehicleProfileOrdinal};
use laneflow_static_network::{
    SharedNetworkBuildLimits, SharedNetworkBuildOptions, SharedNetworkRevision, SpatialBuildOption,
    build_shared_network_revision,
};

fn install() -> TrafficWorld {
    let revision = compile_network();
    TrafficWorld::install(
        Arc::clone(&revision),
        WorldConfig::new(8, 8, 64, 8, 100),
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
            "fixture://entrance",
            origin.canonical_artifact_digest(),
            origin.canonical_artifact_byte_length(),
            origin.network_revision(),
        )
        .expect("key"),
    }
}

fn compile_network() -> Arc<SharedNetworkRevision> {
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
        .expect("profile");
    add_edge(&mut module, "gate", 40.0, &[]);
    add_edge(&mut module, "stem", 20.0, &["only"]);
    add_edge(&mut module, "only", 40.0, &[]);
    add_edge(&mut module, "west", 20.0, &["join"]);
    add_edge(&mut module, "east", 20.0, &["join"]);
    add_edge(&mut module, "join", 40.0, &[]);
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

fn add_edge(module: &mut SyntheticModuleBuilder, key: &str, length_m: f64, next: &[&str]) {
    let owned: Vec<LaneEdgeReference> = next
        .iter()
        .map(|name| LaneEdgeReference::local(name))
        .collect();
    module
        .add_lane_edge(LaneEdgeInput {
            lane_edge_key: key,
            length_meters: length_m,
            speed_limit_meters_per_second: 15.0,
            successors: &owned,
        })
        .expect(key);
}

fn route(world: &mut TrafficWorld, keys: &[&str]) -> RouteHandle {
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

fn profile() -> VehicleProfileOrdinal {
    VehicleProfileOrdinal::from_raw(0)
}

fn opened(input: VehicleSpawnInput) -> VehicleSpawnInput {
    input.with_entrance(VehicleEntrance::new(0, EntranceDirection::AlongLane))
}

fn edge_length(world: &TrafficWorld, route: RouteHandle) -> u32 {
    let edge = world.route_edges(route).expect("edges")[0];
    world.traffic().lane_lengths_millimetres()[edge.index()]
}

fn complete_at_end(
    world: &mut TrafficWorld,
    route: RouteHandle,
) -> laneflow_runtime::VehicleHandle {
    let length = edge_length(world, route);
    let handle = world
        .spawn_vehicle(VehicleSpawnInput::new(profile(), route, 0, length, 0))
        .expect("vehicle at the end fits on the route");
    world.step(TickInput::new(100)).expect("complete");
    assert_eq!(
        world.vehicle(handle).expect("kept").status(),
        VehicleStatus::Completed
    );
    handle
}

#[test]
fn bound_entrance_omits_an_out_of_domain_tail() {
    let mut world = install();
    let gate = route(&mut world, &["gate"]);
    let entering = opened(VehicleSpawnInput::new(profile(), gate, 0, 2_000, 10_000));
    let handle = world
        .spawn_vehicle(entering)
        .expect("bound entrance may omit the tail outside the network");
    assert_eq!(world.vehicle(handle).expect("live").speed_mm_s(), 10_000);
    assert_eq!(world.vehicle(handle).expect("live").progress_mm(), 2_000);
    assert_eq!(world.live_vehicles().len(), 1);

    let mut again = install();
    let gate = route(&mut again, &["gate"]);
    let old = complete_at_end(&mut again, gate);
    let replaced = again
        .replace_completed_vehicle(
            old,
            opened(VehicleSpawnInput::new(profile(), gate, 0, 2_000, 10_000)),
        )
        .expect("replace uses the same entrance binding");
    assert!(again.vehicle(old).is_none());
    assert_eq!(
        again.vehicle(replaced.new).expect("new").progress_mm(),
        2_000
    );
}

#[test]
fn an_in_domain_tail_is_rejected_until_the_route_includes_it() {
    let mut world = install();
    let only = route(&mut world, &["only"]);
    let hanging = VehicleSpawnInput::new(profile(), only, 0, 2_000, 0);
    let cursor = world.command_cursor();
    assert_eq!(
        world.spawn_vehicle(hanging),
        Err(SpawnError::EntranceBody(EntranceBodyError::InDomainTail))
    );
    assert_eq!(
        world.spawn_vehicle(opened(hanging)),
        Err(SpawnError::EntranceBody(EntranceBodyError::InDomainTail))
    );
    assert_eq!(world.command_cursor(), cursor);
    assert_eq!(world.live_vehicles().len(), 0);

    let accounted = route(&mut world, &["stem", "only"]);
    world
        .spawn_vehicle(VehicleSpawnInput::new(profile(), accounted, 1, 2_000, 0))
        .expect("the chosen upstream is already on the route");

    let gate = route(&mut world, &["gate"]);
    let old = complete_at_end(&mut world, gate);
    let replace_cursor = world.command_cursor();
    assert_eq!(
        world.replace_completed_vehicle(old, hanging),
        Err(ReplaceError::EntranceBody(EntranceBodyError::InDomainTail))
    );
    assert_eq!(world.command_cursor(), replace_cursor);
    assert_eq!(
        world.vehicle(old).expect("old").status(),
        VehicleStatus::Completed
    );
}

#[test]
fn multiple_upstreams_are_not_chosen() {
    let mut world = install();
    let join = route(&mut world, &["join"]);
    let hanging = VehicleSpawnInput::new(profile(), join, 0, 2_000, 1_000);
    let cursor = world.command_cursor();
    assert_eq!(
        world.spawn_vehicle(opened(hanging)),
        Err(SpawnError::EntranceBody(EntranceBodyError::InDomainTail))
    );
    assert_eq!(world.command_cursor(), cursor);
    assert!(world.live_vehicles().is_empty());

    let chosen = route(&mut world, &["west", "join"]);
    let chosen_vehicle = world
        .spawn_vehicle(VehicleSpawnInput::new(profile(), chosen, 1, 2_000, 0))
        .expect("the caller chooses west by putting it on the route");
    assert_eq!(
        world
            .vehicle(chosen_vehicle)
            .expect("live")
            .route_edge_index(),
        1
    );

    let gate = route(&mut world, &["gate"]);
    let old = complete_at_end(&mut world, gate);
    assert_eq!(
        world.replace_completed_vehicle(old, hanging),
        Err(ReplaceError::EntranceBody(EntranceBodyError::InDomainTail))
    );
    assert_eq!(
        world.vehicle(old).expect("old").status(),
        VehicleStatus::Completed
    );
}

#[test]
fn the_first_edge_a_predecessor_less_edge_or_the_viewport_is_not_an_entrance() {
    let mut world = install();
    let only = route(&mut world, &["only"]);
    let on_first_edge = VehicleSpawnInput::new(profile(), only, 0, 2_000, 0);
    assert_eq!(
        world.spawn_vehicle(on_first_edge),
        Err(SpawnError::EntranceBody(EntranceBodyError::InDomainTail))
    );

    let gate = route(&mut world, &["gate"]);
    let unbound = VehicleSpawnInput::new(profile(), gate, 0, 2_000, 10_000);
    assert!(unbound.entrance().is_none());
    assert_eq!(
        world.spawn_vehicle(unbound),
        Err(SpawnError::EntranceBody(EntranceBodyError::Unbound))
    );
    assert_eq!(
        world.spawn_vehicle(
            unbound.with_entrance(VehicleEntrance::new(0, EntranceDirection::AgainstLane))
        ),
        Err(SpawnError::EntranceBody(EntranceBodyError::InvalidBinding))
    );
    assert_eq!(
        world.spawn_vehicle(
            unbound.with_entrance(VehicleEntrance::new(1, EntranceDirection::AlongLane))
        ),
        Err(SpawnError::EntranceBody(EntranceBodyError::InvalidBinding))
    );
    assert_eq!(world.live_vehicles().len(), 0);

    let old = complete_at_end(&mut world, gate);
    assert_eq!(
        world.replace_completed_vehicle(old, unbound),
        Err(ReplaceError::EntranceBody(EntranceBodyError::Unbound))
    );
    assert_eq!(
        world.vehicle(old).expect("old").status(),
        VehicleStatus::Completed
    );
}

#[test]
fn an_in_domain_vehicle_needs_no_entrance_and_outside_length_is_not_an_input() {
    let mut first = install();
    let gate = route(&mut first, &["gate"]);
    let placed = VehicleSpawnInput::new(profile(), gate, 0, 20_000, 10_000);
    assert!(placed.entrance().is_none());
    let handle = first
        .spawn_vehicle(placed)
        .expect("the body already fits, so no entrance is required");
    assert_eq!(first.vehicle(handle).expect("live").progress_mm(), 20_000);

    let mut second = install();
    let gate = route(&mut second, &["gate"]);
    let again = second
        .spawn_vehicle(VehicleSpawnInput::new(profile(), gate, 0, 20_000, 10_000))
        .expect("the same in-domain input has no outside length to change");
    assert_eq!(
        second.vehicle(again).expect("live").speed_mm_s(),
        first.vehicle(handle).expect("live").speed_mm_s()
    );
}

#[cfg(feature = "placement-fixtures")]
#[test]
fn an_existing_state_fixture_does_not_apply_the_entrance_check() {
    let mut world = install();
    let gate = route(&mut world, &["gate"]);
    world
        .place_existing_active_vehicle(VehicleSpawnInput::new(profile(), gate, 0, 2_000, 10_000))
        .expect("fixtures keep a tail that fresh spawn would reject");
}
