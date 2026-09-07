//! Public-API reduction of the 10k tick-172 finding. Returns nonzero on an invalid cursor.
use std::path::Path;

use laneflow_runtime::{
    CommittedNetworkSource, PolicyPin, PublishedLfcaReference, RouteRegisterInput, TickInput,
    TrafficWorld, VehicleSpawnInput, WorldConfig, WorldPolicySelection,
};
use laneflow_static_contract::{EntityKind, LaneEdgeId, RightOfWayPolicySetId, VehicleProfileId};
use laneflow_urban_harness::Artifacts;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let directory = args
        .first()
        .ok_or("usage: boundary_probe <10k-artifact-directory>")?;
    let artifacts = Artifacts::load(Path::new(directory))?;
    let spec = artifacts
        .catalog
        .routes
        .iter()
        .find(|r| r.key == "t000.c01.cross.w")
        .ok_or("missing route")?;
    let origin = artifacts.revision.canonical_origin();
    let policy: RightOfWayPolicySetId = format!(
        "lfid1_{}_{}",
        EntityKind::RightOfWayPolicySet.slug(),
        artifacts.catalog.policy_id
    )
    .parse::<RightOfWayPolicySetId>()
    .map_err(|e| e.to_string())?;
    let source = PublishedLfcaReference::new(
        "fixture://lf-cn-urban-v1",
        origin.canonical_artifact_digest(),
        origin.canonical_artifact_byte_length(),
        origin.network_revision(),
    )?;
    let mut world = TrafficWorld::install(
        artifacts.revision.clone(),
        WorldConfig::new(1, 1, spec.edge_keys.len() as u64, 1_000, 1, 16),
        CommittedNetworkSource::Published { reference: source },
        544,
        WorldPolicySelection::Pinned(PolicyPin { policy }),
    )?;
    let edges: Vec<_> = spec
        .edge_keys
        .iter()
        .map(|key| {
            let id: LaneEdgeId = format!(
                "lfid1_{}_{}",
                EntityKind::LaneEdge.slug(),
                artifacts.catalog.edge_ids[key]
            )
            .parse()
            .unwrap();
            artifacts.revision.identity().ordinal(id).unwrap()
        })
        .collect();
    let route = world.register_route(RouteRegisterInput::new(edges.clone()))?;
    for _ in 0..171 {
        world.step(TickInput::new(16))?;
    }
    let id: VehicleProfileId = format!(
        "lfid1_{}_{}",
        EntityKind::VehicleProfile.slug(),
        artifacts.catalog.profile_ids["compact"]
    )
    .parse::<VehicleProfileId>()
    .map_err(|e| e.to_string())?;
    let profile = artifacts
        .revision
        .identity()
        .ordinal(id)
        .ok_or("missing profile")?;
    let vehicle = world.spawn_vehicle(VehicleSpawnInput::new(profile, route, 3, 95_000, 0))?;
    println!(
        "before: tick={} state={:?}",
        world.tick_index(),
        world.vehicle(vehicle)
    );
    world.step(TickInput::new(16))?;
    let after = world.vehicle(vehicle).ok_or("lost vehicle")?;
    println!("after: tick={} state={after:?}", world.tick_index());
    let snapshot = world.capture_snapshot()?;
    let bytes = laneflow_runtime::encode_lfrs(&snapshot);
    let restore_source = world.committed_source().clone();
    println!("capture: captured and encoded {} bytes", bytes.len());
    println!(
        "following step: {:?}",
        world
            .step(TickInput::new(16))
            .map(|outcome| outcome.tick_index())
    );
    drop(world);
    println!(
        "restore tick 172: {:?}",
        laneflow_runtime::restore_lfrs(
            &bytes,
            artifacts.revision.clone(),
            restore_source,
            WorldConfig::new(1, 1, spec.edge_keys.len() as u64, 1_000, 1, 16),
            laneflow_runtime::SnapshotRestoreLimits::new(16_777_216, 1_024)
        )
        .map(|_| "restored")
    );
    let length = artifacts.revision.traffic().lane_lengths_millimetres()
        [edges[after.route_edge_index() as usize].index()];
    if u64::from(after.progress_mm()) * 1_000 + u64::from(after.carry_um())
        > u64::from(length) * 1_000
    {
        return Err("successful step committed a front beyond the current route edge".into());
    }
    Ok(())
}
