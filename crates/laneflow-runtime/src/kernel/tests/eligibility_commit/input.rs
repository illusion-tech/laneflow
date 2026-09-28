//! #777 墙钟与诊断共用输入。资源组为八个独立、相同的 16 拍窗口。
use super::{runtime_types as rt, sparse};
use laneflow_static_contract::{
    EntityKind, ParticipantStreamOrdinal, RightOfWayPolicySetId, VehicleProfileOrdinal,
};
use laneflow_static_network::{
    SharedNetworkBuildLimits, SharedNetworkBuildOptions, SpatialBuildOption,
    build_shared_network_revision,
};
use std::sync::Arc;

pub const CASES: [&str; 6] = [
    "compact",
    "capacity",
    "active",
    "parked",
    "conflict-small",
    "conflict-large",
];
pub const TICKS: usize = 128;

pub fn selected() -> String {
    let name = std::env::var("LANEFLOW_ELIGIBILITY_CASE").unwrap_or_else(|_| "compact".into());
    assert!(CASES.contains(&name.as_str()), "unknown eligibility case");
    name
}
pub fn windows(case: &str) -> usize {
    if case.starts_with("conflict-") { 8 } else { 1 }
}
pub fn delta_ms(case: &str) -> u64 {
    if case.starts_with("conflict-") {
        4
    } else {
        100
    }
}
pub fn create(case: &str) -> (rt::TrafficWorld, String) {
    if let Some(case) = sparse::CASES.iter().find(|v| v.name == case) {
        let root = sparse::revision(*case);
        let digest = format!("{:?}", root.canonical_origin().canonical_artifact_digest());
        return (sparse::world(&root, *case), digest);
    }
    assert!(case.starts_with("conflict-"));
    const INPUT: &[u8] = include_bytes!(
        "../../../../../laneflow-compiler/tests/fixtures/portable/lfca-world-policies/full-spatial.lfca"
    );
    let input =
        laneflow_format::check_canonical_network_input(INPUT, laneflow_format::FormatLimits::HARD)
            .unwrap();
    let root = build_shared_network_revision(
        input,
        SharedNetworkBuildOptions::new(
            SpatialBuildOption::Omit,
            SharedNetworkBuildLimits::new(64 * 1_024 * 1_024, 16 * 1_024 * 1_024),
        ),
    )
    .unwrap();
    let origin = *root.canonical_origin();
    let policy = RightOfWayPolicySetId::from_untyped(
        laneflow_compiler::derive_canonical_stable_id_v1(
            EntityKind::RightOfWayPolicySet,
            "runtime-fixture-policy",
            "fixture-policy",
            &laneflow_compiler::CompileLimits::p100_initial_v1(),
        )
        .unwrap(),
    );
    let capacity = if case == "conflict-small" { 8 } else { 100_000 };
    let mut world = rt::TrafficWorld::install(
        Arc::clone(&root),
        rt::WorldConfig::new(capacity, 4, 1_024, 1_024, 4),
        rt::ExecutionConfig::new(std::num::NonZeroU32::MIN),
        rt::CommittedNetworkSource::Published {
            reference: rt::PublishedLfcaReference::new(
                "fixture://eligibility-commit",
                origin.canonical_artifact_digest(),
                origin.canonical_artifact_byte_length(),
                origin.network_revision(),
            )
            .unwrap(),
        },
        777,
        rt::WorldPolicySelection::Pinned(rt::PolicyPin { policy }),
    )
    .unwrap();
    for raw in [0, 1] {
        let path = root
            .conflict()
            .participant_stream(ParticipantStreamOrdinal::from_raw(raw))
            .unwrap()
            .maneuver_path();
        let edges = root
            .traffic()
            .maneuvers()
            .maneuver_path(path)
            .unwrap()
            .edges()
            .to_vec();
        let progress = root.traffic().lane_lengths_millimetres()[edges[0].index()];
        let route = world
            .register_route(rt::RouteRegisterInput::new(edges))
            .unwrap();
        let spawn = rt::VehicleSpawnInput::new(
            VehicleProfileOrdinal::from_raw(0),
            route,
            0,
            progress,
            8_000,
        )
        .with_open_entrance();
        #[cfg(feature = "placement-fixtures")]
        world.place_existing_active_vehicle(spawn).unwrap();
        #[cfg(not(feature = "placement-fixtures"))]
        world.spawn_vehicle(spawn).unwrap();
    }
    for _ in 0..9 {
        step(&mut world, case);
    }
    validate(&world, case);
    (world, format!("{:?}", origin.canonical_artifact_digest()))
}
pub fn step(world: &mut rt::TrafficWorld, case: &str) {
    std::hint::black_box(world.step(rt::TickInput::new(delta_ms(case))).unwrap());
}
pub fn validate(world: &rt::TrafficWorld, case: &str) {
    if let Some(case) = sparse::CASES.iter().find(|v| v.name == case) {
        sparse::validate(world, *case);
    } else {
        assert_eq!(world.live_vehicles().len(), 2);
        assert_eq!(
            world
                .live_vehicles()
                .iter()
                .filter(|h| world.conflict_reservation(**h).is_some())
                .count(),
            1
        );
        assert!(
            world
                .latest_conflict_decisions()
                .iter()
                .any(|d| matches!(d.outcome(), rt::ConflictDecisionOutcome::NoGrant(_)))
        );
    }
}
pub fn end(case: &str, window: usize, world: &rt::TrafficWorld, input: &str) {
    println!(
        "elig-end case={case} window={window} windows={} ticks={} input={input} digest={}",
        windows(case),
        TICKS / windows(case),
        sparse::digest(world)
    );
}
