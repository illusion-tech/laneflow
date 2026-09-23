//! Finite physical cases for the two authored outer-loop merge resources.
//!
//! 车已经在准入线末端并带着速度，下一拍才分出谁先进入。新鲜生成要求这一拍停得住，
//! 所以这里用测试特性保留已有场面。
#![cfg(feature = "placement-fixtures")]

use laneflow_format::{FormatLimits, check_canonical_network_input};
use laneflow_junction_generator::{JunctionCatalog, JunctionConfig, generate};
use laneflow_runtime::{
    CommittedNetworkSource, ConflictDecisionOutcome, PublishedLfcaReference, RouteHandle,
    TickInput, TrafficWorld, VehicleSpawnInput, WorldConfig,
};
use laneflow_scenario::complex_junction::{VEHICLE_PROFILE_KEY, bind};
use laneflow_static_contract::VehicleProfileOrdinal;
use laneflow_static_network::{
    SharedNetworkBuildLimits, SharedNetworkBuildOptions, SpatialBuildOption,
    build_shared_network_revision,
};

fn fixture(corner: &str) -> (TrafficWorld, VehicleProfileOrdinal, [(RouteHandle, u32); 2]) {
    let generated = generate(
        &JunctionConfig::parse(include_str!(
            "../../../examples/config/v0.1-complex-junction.toml"
        ))
        .unwrap(),
    )
    .unwrap();
    let catalog: JunctionCatalog =
        toml::from_str(std::str::from_utf8(generated.catalog_bytes()).unwrap()).unwrap();
    let revision = build_shared_network_revision(
        check_canonical_network_input(generated.lfca_bytes(), FormatLimits::HARD).unwrap(),
        SharedNetworkBuildOptions::new(
            SpatialBuildOption::RetainAvailable,
            SharedNetworkBuildLimits::new(64 * 1_024 * 1_024, 16 * 1_024 * 1_024),
        ),
    )
    .unwrap();
    let bound = bind(&catalog, &revision).unwrap();
    let origin = *revision.canonical_origin();
    let mut world = TrafficWorld::install(
        revision,
        WorldConfig::new(8, 32, 1_024, 1_024, 16),
        laneflow_runtime::ExecutionConfig::new(std::num::NonZeroU32::MIN),
        CommittedNetworkSource::Published {
            reference: PublishedLfcaReference::new(
                "fixture://merge-admission",
                origin.canonical_artifact_digest(),
                origin.canonical_artifact_byte_length(),
                origin.network_revision(),
            )
            .unwrap(),
        },
        0,
        bound.policy_selection,
    )
    .unwrap();
    let routes = bound.install_routes(&mut world).unwrap();
    let selected = std::array::from_fn(|lane| {
        let feed = format!("loop-{corner}-i{lane}");
        let (index, route) = catalog
            .routes
            .iter()
            .enumerate()
            .find(|(_, route)| route.edge_ids[0] == feed)
            .unwrap();
        assert_eq!(route.edge_ids[1], format!("{feed}.admission"));
        assert_eq!(route.edge_ids[2], format!("{feed}.merge"));
        (routes[index], 1)
    });
    (world, bound.profiles[VEHICLE_PROFILE_KEY], selected)
}

fn at_admission(
    world: &mut TrafficWorld,
    profile: VehicleProfileOrdinal,
    route: RouteHandle,
    hop: u32,
) -> laneflow_runtime::VehicleHandle {
    let edge = world.route_edges(route).unwrap()[hop as usize];
    let length = world.traffic().lane_lengths_millimetres()[edge.index()];
    world
        .place_existing_active_vehicle(VehicleSpawnInput::new(profile, route, hop, length, 2_000))
        .unwrap()
}

#[test]
fn simultaneous_incoming_branches_are_serialized_until_the_first_body_clears() {
    for corner in ["es", "wn"] {
        let (mut world, profile, routes) = fixture(corner);
        let vehicles = routes.map(|(route, hop)| at_admission(&mut world, profile, route, hop));
        world.step(TickInput::new(16)).unwrap();
        let passed = vehicles.map(|vehicle| world.vehicle(vehicle).unwrap().route_edge_index() > 1);
        assert_eq!(
            passed.iter().filter(|value| **value).count(),
            1,
            "{corner}: exactly one branch passes admission"
        );
        let winner = usize::from(!passed[0]);
        let loser = 1 - winner;
        assert!(world.conflict_reservation(vehicles[winner]).is_some());
        assert!(world.conflict_reservation(vehicles[loser]).is_none());
        assert!(
            world
                .latest_conflict_decisions()
                .iter()
                .any(|decision| decision.vehicle() == vehicles[loser]
                    && matches!(decision.outcome(), ConflictDecisionOutcome::NoGrant(_)))
        );
        for waited in 0..2_000 {
            world.step(TickInput::new(16)).unwrap();
            let first = world.vehicle(vehicles[winner]).unwrap();
            let second = world.vehicle(vehicles[loser]).unwrap();
            if second.route_edge_index() > 1 {
                assert!(waited > 100, "must exercise sustained exclusion");
                assert!(first.route_edge_index() >= 3);
                if first.route_edge_index() == 3 {
                    assert!(first.progress_mm() >= first.length_mm() + 2_000);
                }
                assert!(world.conflict_reservation(vehicles[winner]).is_none());
                break;
            }
        }
        assert!(
            world.vehicle(vehicles[loser]).unwrap().route_edge_index() > 1,
            "waiting branch must eventually proceed"
        );
    }
}

#[test]
fn occupied_shared_edge_prevents_both_branches_from_entering_the_taper() {
    for corner in ["es", "wn"] {
        let (mut world, profile, routes) = fixture(corner);
        // Rear at 3.5 m is outside the 2 m conflict exit but still intersects
        // the declared downstream storage needed by a 4.5 m merging car.
        world
            .spawn_vehicle(VehicleSpawnInput::new(profile, routes[0].0, 3, 8_000, 0))
            .unwrap();
        let vehicles = routes.map(|(route, hop)| at_admission(&mut world, profile, route, hop));
        for _ in 0..10 {
            world.step(TickInput::new(16)).unwrap();
            for vehicle in vehicles {
                assert_eq!(
                    world.vehicle(vehicle).unwrap().route_edge_index(),
                    1,
                    "{corner}: blocked downstream cannot admit either branch"
                );
                assert!(world.conflict_reservation(vehicle).is_none());
            }
        }
    }
}
