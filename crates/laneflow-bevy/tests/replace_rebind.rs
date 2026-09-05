use std::{num::NonZeroU32, sync::Arc, time::Duration};

use bevy_app::App;
use bevy_time::{TimePlugin, TimeUpdateStrategy};
use bevy_transform::{TransformPlugin, components::Transform};
use laneflow_bevy::{
    LaneFlowPlugin, LaneFlowSession, LaneFlowSessionConfig, LaneFlowVehicleReplaceOutcome,
    despawn_vehicle, replace_completed_vehicle,
};
use laneflow_compiler::{
    CompilationUnitBuilder, CompileLimits, Compiler, IidmVehicleProfileInput, LaneEdgeInput,
    LaneEdgeReference, ParkingFacilityInput, ParkingLaneAnchorInput, ParticipantClassInput,
    ParticipantClassReference, PortableDiffBase, PortableEmissionProvenance, SourceModuleHeader,
    SourceModuleHeaderInput, SyntheticModuleBuilder, VehicleProfileInput, emit_portable_candidate,
};
use laneflow_format::{FormatLimits, check_canonical_network_input, check_post_emission_bundle};
use laneflow_runtime::{
    LeaveParkingTarget, ParkedVehicleSpawnInput, ParkingError, ParkingTarget, ReserveParkingTarget,
    RouteHandle, RouteRegisterInput, TickInput, TrafficWorld, VehicleSpawnInput, VehicleStatus,
    VirtualEntryAnchorSelector, VirtualExitAnchorSelector, WorldConfig,
};
use laneflow_spatial::SpatialSession;
use laneflow_static_contract::{
    LaneEdgeOrdinal, ParkingFacilityOrdinal, ParkingSpaceOrdinal, VehicleProfileOrdinal,
};
use laneflow_static_network::{
    SharedNetworkBuildLimits, SharedNetworkBuildOptions, SpatialBuildOption,
    build_shared_network_revision,
};

fn install_fixture(
    revision: std::sync::Arc<laneflow_static_network::SharedNetworkRevision>,
    config: laneflow_runtime::WorldConfig,
) -> Result<laneflow_runtime::TrafficWorld, laneflow_runtime::InstallError> {
    install_with_policy(
        revision,
        config,
        laneflow_runtime::WorldPolicySelection::Pinned(laneflow_runtime::PolicyPin {
            policy: laneflow_static_contract::RightOfWayPolicySetId::from_untyped(
                laneflow_compiler::derive_canonical_stable_id_v1(
                    laneflow_static_contract::EntityKind::RightOfWayPolicySet,
                    "runtime-fixture-policy",
                    "fixture-policy",
                    &CompileLimits::p100_initial_v1(),
                )
                .expect("full-spatial fixture policy identity"),
            ),
        }),
    )
}

fn install_with_policy(
    revision: std::sync::Arc<laneflow_static_network::SharedNetworkRevision>,
    config: laneflow_runtime::WorldConfig,
    policy_selection: laneflow_runtime::WorldPolicySelection,
) -> Result<laneflow_runtime::TrafficWorld, laneflow_runtime::InstallError> {
    let origin = *revision.canonical_origin();
    laneflow_runtime::TrafficWorld::install(
        std::sync::Arc::clone(&revision),
        config,
        laneflow_runtime::CommittedNetworkSource::Published {
            reference: laneflow_runtime::PublishedLfcaReference::new(
                "fixture://in-process",
                origin.canonical_artifact_digest(),
                origin.canonical_artifact_byte_length(),
                origin.network_revision(),
            )
            .expect("non-empty fixture key"),
        },
        0,
        policy_selection,
    )
}

const FULL_SPATIAL: &[u8] = include_bytes!(
    "../../laneflow-compiler/tests/fixtures/portable/lfca-world-policies/full-spatial.lfca"
);

fn revision() -> Arc<laneflow_static_network::SharedNetworkRevision> {
    let input = check_canonical_network_input(FULL_SPATIAL, FormatLimits::HARD)
        .expect("checked canonical network input");
    build_shared_network_revision(
        input,
        SharedNetworkBuildOptions::new(
            SpatialBuildOption::RetainAvailable,
            SharedNetworkBuildLimits::new(64 * 1_024 * 1_024, 16 * 1_024 * 1_024),
        ),
    )
    .expect("shared network revision")
}

fn virtual_parking_revision() -> Arc<laneflow_static_network::SharedNetworkRevision> {
    let limits = CompileLimits::p100_initial_v1();
    let header = SourceModuleHeader::new(
        SourceModuleHeaderInput {
            authoring_namespace_id: "bevy/virtual-parking-acceptance",
            source_document_key: "virtual-parking.document",
            generator_build_id: "git:0123456789abcdef",
            parameters_and_inputs_digest: [0x31; 32],
            frontend_options_digest: [0x32; 32],
            random_seed: Some(541),
            provenance: "repository:laneflow",
        },
        &limits,
    )
    .expect("source header");
    let mut module = SyntheticModuleBuilder::new(header, &limits).expect("synthetic module");
    let virtual_entries = [ParkingLaneAnchorInput {
        lane_edge: LaneEdgeReference::local("edge"),
        progress_meters: 20.0,
    }];
    let virtual_exits = [ParkingLaneAnchorInput {
        lane_edge: LaneEdgeReference::local("edge"),
        progress_meters: 70.0,
    }];
    module
        .add_participant_class(ParticipantClassInput {
            participant_class_key: "road-user",
            extends: None,
        })
        .expect("participant class")
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
        .expect("vehicle profile")
        .add_lane_edge(LaneEdgeInput {
            lane_edge_key: "edge",
            length_meters: 100.0,
            speed_limit_meters_per_second: 15.0,
            successors: &[],
        })
        .expect("lane edge")
        .add_parking_facility(ParkingFacilityInput {
            parking_facility_key: "facility",
            virtual_capacity: 1,
            virtual_entries: &virtual_entries,
            virtual_exits: &virtual_exits,
        })
        .expect("parking facility");
    let mut unit = CompilationUnitBuilder::new(limits);
    unit.add_synthetic_module(module.finish().expect("finished module"))
        .expect("compilation module");
    let output = Compiler::new()
        .compile(unit.build().expect("compilation unit"))
        .unwrap_or_else(|bundle| panic!("compile diagnostics: {:?}", bundle.diagnostics()));
    let provenance =
        PortableEmissionProvenance::try_new("laneflow-bevy-virtual-parking-acceptance-v1")
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
    .expect("virtual parking shared revision")
}

fn edge_for_length(world: &TrafficWorld, length: u32) -> LaneEdgeOrdinal {
    let index = world
        .traffic()
        .lane_lengths_millimetres()
        .iter()
        .position(|actual| *actual == length)
        .expect("fixture lane length");
    LaneEdgeOrdinal::try_from_usize(index).expect("fixture lane ordinal")
}

fn register_preview_route(world: &mut TrafficWorld) -> RouteHandle {
    world
        .register_route(RouteRegisterInput::new(vec![
            edge_for_length(world, 10_000),
            edge_for_length(world, 8_000),
            edge_for_length(world, 12_000),
        ]))
        .expect("register")
}

fn drive_to_completed(world: &mut TrafficWorld) -> (laneflow_runtime::VehicleHandle, RouteHandle) {
    let route = register_preview_route(world);
    let edges = world.route_edges(route).expect("edges").to_vec();
    let last = *edges.last().expect("route has edges");
    let last_length = world.traffic().lane_lengths_millimetres()[last.index()];
    let speed_limit = world.traffic().lane_speed_limits_millimetres_per_second()[last.index()];
    let last_index = u32::try_from(edges.len() - 1).expect("index");
    let vehicle = world
        .spawn_vehicle(VehicleSpawnInput::new(
            VehicleProfileOrdinal::from_raw(0),
            route,
            last_index,
            last_length.saturating_sub(500),
            speed_limit,
        ))
        .expect("spawn near end");
    for _ in 0..8 {
        world.step(TickInput::new(100)).expect("step");
        if world
            .vehicle(vehicle)
            .is_some_and(|state| state.status() == VehicleStatus::Completed)
        {
            break;
        }
    }
    (vehicle, route)
}

#[test]
fn replace_reuses_bound_entity_and_keeps_transform_on_blocked() {
    let mut world =
        install_fixture(revision(), WorldConfig::new(8, 4, 1_024, 1_024, 1, 100)).expect("install");
    let (old, route) = drive_to_completed(&mut world);
    let spatial = SpatialSession::bind(world.revision())
        .expect("bind")
        .expect("spatial");
    let session = LaneFlowSession::new(
        world,
        Some(spatial),
        LaneFlowSessionConfig::new(NonZeroU32::new(8).expect("non-zero")),
    )
    .expect("session");

    let mut app = App::new();
    app.add_plugins((TimePlugin, TransformPlugin, LaneFlowPlugin));
    app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(
        16,
    )));
    app.insert_resource(session);
    let entity = app
        .world_mut()
        .spawn(Transform::from_xyz(1.0, 2.0, 3.0))
        .id();
    app.world_mut()
        .resource_mut::<LaneFlowSession>()
        .bind_vehicle_entity(old, entity)
        .expect("bind");

    app.world_mut()
        .resource_mut::<LaneFlowSession>()
        .world_mut()
        .spawn_vehicle(VehicleSpawnInput::new(
            VehicleProfileOrdinal::from_raw(0),
            route,
            0,
            0,
            0,
        ))
        .expect("blocker");
    let before = *app.world().get::<Transform>(entity).expect("transform");
    let outcome = replace_completed_vehicle(
        app.world_mut(),
        old,
        VehicleSpawnInput::new(VehicleProfileOrdinal::from_raw(0), route, 0, 0, 0),
    )
    .expect("blocked is success-path for adapter");
    assert!(matches!(outcome, LaneFlowVehicleReplaceOutcome::Blocked(_)));
    assert_eq!(
        app.world()
            .resource::<LaneFlowSession>()
            .vehicle_entity(old),
        Some(entity)
    );
    assert_eq!(
        *app.world().get::<Transform>(entity).expect("transform"),
        before
    );

    let outcome = replace_completed_vehicle(
        app.world_mut(),
        old,
        VehicleSpawnInput::new(VehicleProfileOrdinal::from_raw(0), route, 0, 8_000, 0),
    )
    .expect("replace");
    let LaneFlowVehicleReplaceOutcome::Replaced(record) = outcome else {
        panic!("expected replaced");
    };
    assert_eq!(record.entity, Some(entity));
    assert_ne!(record.new, old);
    assert!(
        app.world()
            .resource::<LaneFlowSession>()
            .vehicle_entity(old)
            .is_none()
    );
    assert_eq!(
        app.world()
            .resource::<LaneFlowSession>()
            .vehicle_entity(record.new),
        Some(entity)
    );
    assert_eq!(
        *app.world().get::<Transform>(entity).expect("transform"),
        before,
        "presentation updates on a later outer frame"
    );
}

#[test]
fn unbound_replace_stays_unbound() {
    let mut world =
        install_fixture(revision(), WorldConfig::new(8, 4, 1_024, 1_024, 1, 100)).expect("install");
    let (old, route) = drive_to_completed(&mut world);
    let session = LaneFlowSession::new(
        world,
        None,
        LaneFlowSessionConfig::new(NonZeroU32::new(8).expect("non-zero")),
    )
    .expect("session");
    let mut app = App::new();
    app.add_plugins((TimePlugin, LaneFlowPlugin));
    app.insert_resource(session);
    let outcome = replace_completed_vehicle(
        app.world_mut(),
        old,
        VehicleSpawnInput::new(VehicleProfileOrdinal::from_raw(0), route, 0, 0, 0),
    )
    .expect("replace");
    let LaneFlowVehicleReplaceOutcome::Replaced(record) = outcome else {
        panic!("expected replaced");
    };
    assert_eq!(record.entity, None);
    assert!(
        app.world()
            .resource::<LaneFlowSession>()
            .vehicle_entity(record.new)
            .is_none()
    );
}

#[test]
fn virtual_parking_echoes_typed_selectors_and_keeps_mapping_without_pose() {
    let mut world = install_with_policy(
        virtual_parking_revision(),
        WorldConfig::new(8, 4, 1_024, 1_024, 1, 100),
        laneflow_runtime::WorldPolicySelection::NotRequired,
    )
    .expect("install virtual parking fixture");
    let route = world
        .register_route(RouteRegisterInput::new(vec![LaneEdgeOrdinal::from_raw(0)]))
        .expect("virtual parking route");
    let vehicle = world
        .spawn_vehicle(VehicleSpawnInput::new(
            VehicleProfileOrdinal::from_raw(0),
            route,
            0,
            20_000,
            0,
        ))
        .expect("spawn at virtual entry");
    let session = LaneFlowSession::new(
        world,
        None,
        LaneFlowSessionConfig::new(NonZeroU32::new(8).expect("non-zero")),
    )
    .expect("session");
    let mut app = App::new();
    app.add_plugins((TimePlugin, TransformPlugin, LaneFlowPlugin));
    app.insert_resource(session);
    let entity = app.world_mut().spawn(Transform::IDENTITY).id();
    app.world_mut()
        .resource_mut::<LaneFlowSession>()
        .bind_vehicle_entity(vehicle, entity)
        .expect("bind virtual parking vehicle");

    let facility = ParkingFacilityOrdinal::from_raw(0);
    let target = ParkingTarget::VirtualPool(facility);
    let entry_selector = VirtualEntryAnchorSelector::from_raw(0);
    let reserve = app
        .world_mut()
        .resource_mut::<LaneFlowSession>()
        .world_mut()
        .reserve_parking(
            vehicle,
            ReserveParkingTarget::VirtualPool {
                facility,
                entry_anchor: entry_selector,
                entry_route_occurrence: 0,
            },
        )
        .expect("reserve virtual target")
        .into_record();
    assert_eq!(reserve.vehicle, vehicle);
    assert_eq!(reserve.target, target);
    assert_eq!(reserve.route, route);
    assert_eq!(reserve.entry_route_occurrence, 0);
    assert_eq!(reserve.virtual_entry_selector, Some(entry_selector));
    assert!(reserve.arrived);

    let park = app
        .world_mut()
        .resource_mut::<LaneFlowSession>()
        .world_mut()
        .park_vehicle(vehicle, target)
        .expect("park at virtual target")
        .into_record();
    assert_eq!(park.vehicle, vehicle);
    assert_eq!(park.target, target);
    {
        let session = app.world().resource::<LaneFlowSession>();
        assert_eq!(session.vehicle_entity(vehicle), Some(entity));
        assert_eq!(
            session
                .world()
                .vehicle(vehicle)
                .expect("parked live")
                .status(),
            VehicleStatus::Parked
        );
        assert!(
            session
                .world()
                .committed_pose_sources()
                .as_slice()
                .iter()
                .all(|(handle, _)| *handle != vehicle),
            "virtual Parked vehicle has no committed pose but remains mapped"
        );
    }
    assert!(app.world().get_entity(entity).is_ok());

    let exit_selector = VirtualExitAnchorSelector::from_raw(0);
    let leave = app
        .world_mut()
        .resource_mut::<LaneFlowSession>()
        .world_mut()
        .leave_parking(
            vehicle,
            LeaveParkingTarget::VirtualPool {
                facility,
                route,
                exit_anchor: exit_selector,
                exit_route_occurrence: 0,
            },
        )
        .expect("leave caller-selected virtual exit");
    assert_eq!(leave.vehicle, vehicle);
    assert_eq!(leave.target, target);
    assert_eq!(leave.route, route);
    assert_eq!(leave.exit_route_occurrence, 0);
    assert_eq!(leave.virtual_exit_selector, Some(exit_selector));
    let session = app.world().resource::<LaneFlowSession>();
    assert_eq!(session.vehicle_entity(vehicle), Some(entity));
    assert_eq!(
        session
            .world()
            .vehicle(vehicle)
            .expect("active live")
            .status(),
        VehicleStatus::Active
    );
}

#[test]
fn typed_completed_despawn_removes_mapping_but_leaves_host_entity_to_caller() {
    let mut world =
        install_fixture(revision(), WorldConfig::new(8, 4, 1_024, 1_024, 1, 100)).expect("install");
    let (completed, _route) = drive_to_completed(&mut world);
    let session = LaneFlowSession::new(
        world,
        None,
        LaneFlowSessionConfig::new(NonZeroU32::new(8).expect("non-zero")),
    )
    .expect("session");
    let mut app = App::new();
    app.add_plugins((TimePlugin, TransformPlugin, LaneFlowPlugin));
    app.insert_resource(session);
    let entity = app.world_mut().spawn(Transform::IDENTITY).id();
    app.world_mut()
        .resource_mut::<LaneFlowSession>()
        .bind_vehicle_entity(completed, entity)
        .expect("bind completed vehicle");

    let record = despawn_vehicle(app.world_mut(), completed).expect("typed Completed despawn");
    assert_eq!(record.entity, Some(entity));
    assert_eq!(record.runtime.vehicle, completed);
    assert_eq!(record.runtime.status, VehicleStatus::Completed);
    assert_eq!(record.runtime.parking_binding, None);
    let session = app.world().resource::<LaneFlowSession>();
    assert!(session.world().vehicle(completed).is_none());
    assert!(session.vehicle_entity(completed).is_none());
    assert!(app.world().get_entity(entity).is_ok());
}

#[test]
fn identical_bind_is_duplicate_error() {
    let mut world =
        install_fixture(revision(), WorldConfig::new(8, 4, 1_024, 1_024, 1, 100)).expect("install");
    let (vehicle, _route) = drive_to_completed(&mut world);
    let session = LaneFlowSession::new(
        world,
        None,
        LaneFlowSessionConfig::new(NonZeroU32::new(8).expect("non-zero")),
    )
    .expect("session");
    let mut app = App::new();
    app.add_plugins((TimePlugin, TransformPlugin, LaneFlowPlugin));
    app.insert_resource(session);
    let entity = app.world_mut().spawn(Transform::IDENTITY).id();
    app.world_mut()
        .resource_mut::<LaneFlowSession>()
        .bind_vehicle_entity(vehicle, entity)
        .expect("first bind");
    let err = app
        .world_mut()
        .resource_mut::<LaneFlowSession>()
        .bind_vehicle_entity(vehicle, entity)
        .expect_err("repeat bind");
    assert!(matches!(
        err,
        laneflow_bevy::LaneFlowAdapterError::DuplicateVehicleBinding { .. }
    ));
}

#[test]
fn typed_despawn_rejects_stale_entity_without_runtime_or_mapping_changes() {
    let mut world =
        install_fixture(revision(), WorldConfig::new(8, 4, 1_024, 1_024, 1, 100)).expect("install");
    let route = register_preview_route(&mut world);
    let vehicle = world
        .spawn_vehicle(VehicleSpawnInput::new(
            VehicleProfileOrdinal::from_raw(0),
            route,
            0,
            0,
            0,
        ))
        .expect("active vehicle");
    let session = LaneFlowSession::new(
        world,
        None,
        LaneFlowSessionConfig::new(NonZeroU32::new(8).expect("non-zero")),
    )
    .expect("session");
    let mut app = App::new();
    app.add_plugins((TimePlugin, TransformPlugin, LaneFlowPlugin));
    app.insert_resource(session);
    let entity = app.world_mut().spawn(Transform::IDENTITY).id();
    app.world_mut()
        .resource_mut::<LaneFlowSession>()
        .bind_vehicle_entity(vehicle, entity)
        .expect("bind vehicle");
    assert!(app.world_mut().despawn(entity));

    let (before_state, before_live, before_cursor, before_sequence, before_binding) = {
        let session = app.world().resource::<LaneFlowSession>();
        (
            session.world().vehicle(vehicle),
            session.world().live_vehicles().to_vec(),
            session.world().command_cursor(),
            session.world().observation_state_sequence(),
            session.world().parking_binding(vehicle),
        )
    };
    let error = despawn_vehicle(app.world_mut(), vehicle).expect_err("stale Entity fails closed");
    assert!(matches!(
        error,
        laneflow_bevy::LaneFlowAdapterError::StaleLifecycleEntity {
            vehicle: actual_vehicle,
            entity: actual_entity,
        } if actual_vehicle == vehicle && actual_entity == entity
    ));

    let session = app.world().resource::<LaneFlowSession>();
    assert_eq!(session.world().vehicle(vehicle), before_state);
    assert_eq!(session.world().live_vehicles(), before_live);
    assert_eq!(session.world().command_cursor(), before_cursor);
    assert_eq!(
        session.world().observation_state_sequence(),
        before_sequence
    );
    assert_eq!(session.world().parking_binding(vehicle), before_binding);
    assert_eq!(session.vehicle_entity(vehicle), Some(entity));
}

#[test]
fn parking_failures_keep_entity_mapping_and_only_typed_despawn_removes_it() {
    let mut world =
        install_fixture(revision(), WorldConfig::new(8, 4, 1_024, 1_024, 1, 100)).expect("install");
    let route = register_preview_route(&mut world);
    let profile = VehicleProfileOrdinal::from_raw(0);
    let active = world
        .spawn_vehicle(VehicleSpawnInput::new(profile, route, 0, 0, 0))
        .expect("active vehicle");
    let session = LaneFlowSession::new(
        world,
        None,
        LaneFlowSessionConfig::new(NonZeroU32::new(8).expect("non-zero")),
    )
    .expect("session");
    let mut app = App::new();
    app.add_plugins((TimePlugin, TransformPlugin, LaneFlowPlugin));
    app.insert_resource(session);
    let active_entity = app.world_mut().spawn(Transform::IDENTITY).id();
    app.world_mut()
        .resource_mut::<LaneFlowSession>()
        .bind_vehicle_entity(active, active_entity)
        .expect("bind active");

    let space = ParkingSpaceOrdinal::from_raw(0);
    assert_eq!(
        app.world_mut()
            .resource_mut::<LaneFlowSession>()
            .world_mut()
            .park_vehicle(active, ParkingTarget::ExplicitSpace(space))
            .unwrap_err(),
        ParkingError::NotReserved
    );
    assert_eq!(
        app.world()
            .resource::<LaneFlowSession>()
            .vehicle_entity(active),
        Some(active_entity),
        "failed park cannot remove the host mapping"
    );

    let active_despawn = despawn_vehicle(app.world_mut(), active).expect("typed active despawn");
    assert_eq!(active_despawn.entity, Some(active_entity));
    assert_eq!(active_despawn.runtime.status, VehicleStatus::Active);
    assert!(
        app.world()
            .resource::<LaneFlowSession>()
            .vehicle_entity(active)
            .is_none()
    );
    assert!(app.world().get_entity(active_entity).is_ok());

    let parked = app
        .world_mut()
        .resource_mut::<LaneFlowSession>()
        .world_mut()
        .spawn_parked_vehicle(
            ParkedVehicleSpawnInput::new(profile, route, 0, 0),
            ParkingTarget::ExplicitSpace(space),
        )
        .expect("parked vehicle")
        .vehicle;
    let parked_entity = app.world_mut().spawn(Transform::IDENTITY).id();
    app.world_mut()
        .resource_mut::<LaneFlowSession>()
        .bind_vehicle_entity(parked, parked_entity)
        .expect("bind parked");
    assert_eq!(
        app.world_mut()
            .resource_mut::<LaneFlowSession>()
            .world_mut()
            .leave_parking(
                parked,
                LeaveParkingTarget::ExplicitSpace {
                    space: ParkingSpaceOrdinal::from_raw(u32::MAX),
                    route,
                    exit_route_occurrence: 0,
                },
            )
            .unwrap_err(),
        ParkingError::UnknownSpace
    );
    assert_eq!(
        app.world()
            .resource::<LaneFlowSession>()
            .vehicle_entity(parked),
        Some(parked_entity),
        "failed leave keeps the hidden/live entity association"
    );
    assert_eq!(
        app.world()
            .resource::<LaneFlowSession>()
            .world()
            .vehicle(parked)
            .expect("parked remains live")
            .status(),
        VehicleStatus::Parked
    );

    let parked_despawn = despawn_vehicle(app.world_mut(), parked).expect("typed parked despawn");
    assert_eq!(parked_despawn.entity, Some(parked_entity));
    assert_eq!(parked_despawn.runtime.status, VehicleStatus::Parked);
    assert!(
        app.world()
            .resource::<LaneFlowSession>()
            .vehicle_entity(parked)
            .is_none()
    );
    assert!(matches!(
        despawn_vehicle(app.world_mut(), parked),
        Err(laneflow_bevy::LaneFlowAdapterError::VehicleDespawn {
            source: ParkingError::StaleVehicle,
            ..
        })
    ));
}
