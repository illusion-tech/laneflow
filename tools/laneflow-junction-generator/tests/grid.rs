use std::sync::Arc;

use laneflow_format::{FormatLimits, check_canonical_network_input};
use laneflow_junction_generator::{JunctionConfig, generate_grid};
use laneflow_runtime::{
    CommittedNetworkSource, PublishedLfcaReference, TickInput, TrafficWorld, VehicleSpawnInput,
    WorldConfig,
};
use laneflow_scenario::complex_junction::{VEHICLE_PROFILE_KEY, bind};
use laneflow_static_network::{
    SharedNetworkBuildLimits, SharedNetworkBuildOptions, SpatialBuildOption,
    build_shared_network_revision,
};

#[test]
fn multiple_cells_share_one_policy_and_keep_distinct_formal_resource_owners() {
    let config = JunctionConfig::parse(include_str!(
        "../../../examples/config/v0.1-complex-junction.toml"
    ))
    .unwrap();
    let first = generate_grid(&config, 2).unwrap();
    let second = generate_grid(&config, 2).unwrap();
    assert_eq!(first.lfca, second.lfca);
    assert_eq!(
        toml::to_string(&first.catalog).unwrap(),
        toml::to_string(&second.catalog).unwrap()
    );
    let revision = build_shared_network_revision(
        check_canonical_network_input(first.lfca.as_slice(), FormatLimits::HARD).unwrap(),
        SharedNetworkBuildOptions::new(
            SpatialBuildOption::RetainAvailable,
            SharedNetworkBuildLimits::new(64 * 1024 * 1024, 16 * 1024 * 1024),
        ),
    )
    .unwrap();
    let bound: Vec<_> = first
        .catalog
        .cells
        .iter()
        .map(|catalog| bind(catalog, &revision).unwrap())
        .collect();
    assert_eq!(bound[0].policy_selection, bound[1].policy_selection);
    assert_ne!(bound[0].route_exits[0].edges, bound[1].route_exits[0].edges);
    let origin = revision.canonical_origin();
    let mut world = TrafficWorld::install(
        Arc::clone(&revision),
        WorldConfig::new(8, 22, 4096, 4096, 16),
        laneflow_runtime::ExecutionConfig::new(std::num::NonZeroU32::MIN),
        CommittedNetworkSource::Published {
            reference: PublishedLfcaReference::new(
                "fixture://junction-grid",
                origin.canonical_artifact_digest(),
                origin.canonical_artifact_byte_length(),
                origin.network_revision(),
            )
            .unwrap(),
        },
        0,
        bound[0].policy_selection,
    )
    .unwrap();
    let mut vehicles = Vec::new();
    for cell in &bound {
        let routes = cell.install_routes(&mut world).unwrap();
        for lane in [0, 2, 4, 6] {
            let slot = &cell.spawn_slots[cell.portal_lanes[lane].entry_slot_index];
            vehicles.push(
                world
                    .spawn_vehicle(VehicleSpawnInput::new(
                        cell.profiles[VEHICLE_PROFILE_KEY],
                        routes[slot.route_index],
                        0,
                        slot.progress_mm,
                        0,
                    ))
                    .unwrap(),
            );
        }
    }
    let mut decisions = [false; 2];
    for _ in 0..6_000 {
        world.step(TickInput::new(16)).unwrap();
        for decision in world.latest_conflict_decisions() {
            if let Some(index) = vehicles
                .iter()
                .position(|&vehicle| vehicle == decision.vehicle())
            {
                decisions[index / 4] = true;
            }
        }
    }
    assert_eq!(
        decisions,
        [true, true],
        "both cells must exercise the shared policy through production stepping"
    );
}
