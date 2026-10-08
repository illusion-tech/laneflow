//! Bounded spawn, replace, and steady-step account. Not a city run.

#[path = "support/policy.rs"]
#[cfg(feature = "placement-fixtures")]
mod test_policy;

#[path = "support/admission_distant_fixture.rs"]
#[cfg(feature = "placement-fixtures")]
mod admission_distant_fixture;

#[cfg(feature = "placement-fixtures")]
use std::alloc::System;
#[cfg(feature = "placement-fixtures")]
use std::sync::Arc;
#[cfg(feature = "placement-fixtures")]
use std::time::Instant;

#[cfg(feature = "placement-fixtures")]
use laneflow_format::{FormatLimits, check_canonical_network_input};
#[cfg(feature = "placement-fixtures")]
use laneflow_runtime::{
    CommittedNetworkSource, ExecutionConfig, PublishedLfcaReference, RouteRegisterInput, TickInput,
    TrafficWorld, VehicleSpawnInput, VehicleStatus, WorldConfig,
};
#[cfg(feature = "placement-fixtures")]
use laneflow_static_contract::{ParticipantStreamOrdinal, VehicleProfileOrdinal};
#[cfg(feature = "placement-fixtures")]
use laneflow_static_network::{
    SharedNetworkBuildLimits, SharedNetworkBuildOptions, SpatialBuildOption,
    build_shared_network_revision,
};
#[cfg(feature = "placement-fixtures")]
use stats_alloc::{INSTRUMENTED_SYSTEM, Region, StatsAlloc};

#[global_allocator]
#[cfg(feature = "placement-fixtures")]
static GLOBAL: &StatsAlloc<System> = &INSTRUMENTED_SYSTEM;

#[cfg(feature = "placement-fixtures")]
const FULL_SPATIAL: &[u8] = include_bytes!(
    "../../laneflow-compiler/tests/fixtures/portable/lfca-world-policies/full-spatial.lfca"
);
#[cfg(feature = "placement-fixtures")]
fn account_world(
    distant_drop: bool,
) -> (
    TrafficWorld,
    laneflow_runtime::RouteHandle,
    Vec<laneflow_static_contract::LaneEdgeOrdinal>,
) {
    let (revision, distant_edges) = if distant_drop {
        let (revision, edges) = admission_distant_fixture::distant_drop_fixture();
        (revision, Some(edges))
    } else {
        let input = check_canonical_network_input(FULL_SPATIAL, FormatLimits::HARD)
            .expect("checked fixture");
        let revision = build_shared_network_revision(
            input,
            SharedNetworkBuildOptions::new(
                SpatialBuildOption::Omit,
                SharedNetworkBuildLimits::new(64 * 1_024 * 1_024, 16 * 1_024 * 1_024),
            ),
        )
        .expect("revision");
        (revision, None)
    };
    let origin = *revision.canonical_origin();
    let mut world = TrafficWorld::install(
        Arc::clone(&revision),
        WorldConfig::new(32, 8, 256, 64, 100),
        ExecutionConfig::new(std::num::NonZeroU32::MIN),
        CommittedNetworkSource::Published {
            reference: PublishedLfcaReference::new(
                "fixture://placement-account",
                origin.canonical_artifact_digest(),
                origin.canonical_artifact_byte_length(),
                origin.network_revision(),
            )
            .expect("source"),
        },
        742,
        test_policy::selection(&revision),
    )
    .expect("world");
    let edges = distant_edges.unwrap_or_else(|| {
        let stream = revision
            .conflict()
            .participant_stream(ParticipantStreamOrdinal::from_raw(0))
            .expect("stream");
        revision
            .traffic()
            .maneuvers()
            .maneuver_path(stream.maneuver_path())
            .expect("path")
            .edges()
            .to_vec()
    });
    let route = world
        .register_route(RouteRegisterInput::new(edges.clone()))
        .expect("route");
    (world, route, edges)
}

#[cfg(feature = "placement-fixtures")]
fn placement_bounded_account() {
    let (mut world, route, edges) = account_world(false);
    let started = Instant::now();
    let region = Region::new(GLOBAL);
    let approach_length = world.traffic().lane_lengths_millimetres()[edges[0].index()];
    let mut progress = 0u32;
    while progress + 4_500 < approach_length && progress / 5_000 < 8 {
        world
            .spawn_vehicle(
                VehicleSpawnInput::new(VehicleProfileOrdinal::from_raw(0), route, 0, progress, 0)
                    .with_open_entrance(),
            )
            .expect("spawn");
        progress = progress.saturating_add(5_000);
    }
    let last = u32::try_from(edges.len() - 1).expect("index");
    let last_length = world.traffic().lane_lengths_millimetres()[edges[last as usize].index()];
    let completed = world
        .spawn_vehicle(
            VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                last,
                last_length,
                0,
            )
            .with_open_entrance(),
        )
        .expect("end spawn");
    world.step(TickInput::new(100)).expect("complete step");
    if world.vehicle(completed).map(|state| state.status()) == Some(VehicleStatus::Completed) {
        world
            .replace_completed_vehicle(
                completed,
                VehicleSpawnInput::new(
                    VehicleProfileOrdinal::from_raw(0),
                    route,
                    last,
                    last_length,
                    0,
                )
                .with_open_entrance(),
            )
            .expect("replace");
    }
    for _ in 0..4 {
        world.step(TickInput::new(100)).expect("steady");
    }
    let stats = region.change();
    let elapsed_us = started.elapsed().as_micros();
    eprintln!(
        "ACCOUNT elapsed_us={elapsed_us} allocs={} retained_bytes={} vehicles={}",
        stats.allocations,
        stats
            .bytes_allocated
            .saturating_sub(stats.bytes_deallocated),
        world.live_vehicles().len()
    );
}

#[cfg(feature = "placement-fixtures")]
fn admission_filter_allocation_account(distant_drop: bool) {
    let measure = || {
        let (mut world, route, edges) = account_world(distant_drop);
        let last = u32::try_from(edges.len() - 1).unwrap();
        let last_length = world.traffic().lane_lengths_millimetres()[edges[last as usize].index()];
        world
            .place_existing_active_vehicle(
                VehicleSpawnInput::new(
                    VehicleProfileOrdinal::from_raw(0),
                    route,
                    if distant_drop { 0 } else { last },
                    if distant_drop {
                        1_000
                    } else {
                        last_length - 1_000
                    },
                    0,
                )
                .with_open_entrance(),
            )
            .unwrap();
        let before = world.capture_snapshot().unwrap();
        laneflow_runtime::reset_contender_filter_totals();
        let region = Region::new(GLOBAL);
        world.force_rebuild_contenders_for_test();
        let cold = region.change();
        let hits = laneflow_runtime::contender_filter_counts()[0];
        let kinds = laneflow_runtime::contender_filter_speed_drop_kinds();
        assert_eq!(kinds[usize::from(distant_drop)], hits);
        assert_eq!(kinds[usize::from(!distant_drop)], 0);
        let retained = world.contender_retained_bytes_for_test();
        let region = Region::new(GLOBAL);
        for _ in 0..64 {
            world.force_rebuild_contenders_for_test();
        }
        let warm = region.change();
        assert_eq!(
            warm.allocations, 0,
            "warm empty contributions must not allocate"
        );
        assert_eq!(warm.bytes_allocated, 0);
        assert_eq!(world.contender_retained_bytes_for_test(), retained);
        for kind in [
            laneflow_runtime::AdmissionReserve::Best,
            laneflow_runtime::AdmissionReserve::Cell,
            laneflow_runtime::AdmissionReserve::Waiting,
        ] {
            laneflow_runtime::set_admission_reserve_failure(kind, true);
            world.force_rebuild_contenders_for_test();
            laneflow_runtime::set_admission_reserve_failure(kind, false);
            assert!(
                world
                    .contender_full_fingerprint_for_test()
                    .ends_with("None)")
            );
            assert_eq!(world.capture_snapshot().unwrap(), before);
            world.force_rebuild_contenders_for_test();
            assert_eq!(world.contender_retained_bytes_for_test(), retained);
        }
        let fingerprint = world.contender_full_fingerprint_for_test();
        assert_eq!(world.capture_snapshot().unwrap(), before);
        (cold, warm, retained, hits, fingerprint)
    };
    let candidate = measure();
    let reference = laneflow_runtime::with_contender_filter_reference(measure);
    assert!(candidate.3 > 0);
    assert_eq!(reference.3, 0);
    assert_eq!(candidate.0, reference.0, "cold allocation calls and bytes");
    assert_eq!(candidate.1, reference.1, "warm allocation calls and bytes");
    assert_eq!(
        candidate.2, reference.2,
        "retained owner and outer-table capacity"
    );
    assert_eq!(candidate.4, reference.4, "retry rebuild values");
    eprintln!(
        "FILTER_ACCOUNT distant_drop={} cold={:?} warm={:?} retained={} hits={}",
        distant_drop, candidate.0, candidate.1, candidate.2, candidate.3
    );
}

fn main() {
    let mut args = libtest_mimic::Arguments::from_args();
    // 全进程计数不能与框架调度并发；命令行指定更多线程也不能改变测量边界。
    args.test_threads = Some(1);
    #[cfg_attr(not(feature = "placement-fixtures"), allow(unused_variables))]
    let main_thread = std::thread::current().id();
    #[cfg(feature = "placement-fixtures")]
    let tests = vec![
        libtest_mimic::Trial::test("placement_bounded_account", move || {
            assert_eq!(std::thread::current().id(), main_thread);
            placement_bounded_account();
            Ok(())
        }),
        libtest_mimic::Trial::test("admission_filter_allocation_account", move || {
            assert_eq!(std::thread::current().id(), main_thread);
            admission_filter_allocation_account(false);
            admission_filter_allocation_account(true);
            Ok(())
        }),
    ];
    #[cfg(not(feature = "placement-fixtures"))]
    let tests: Vec<libtest_mimic::Trial> = Vec::new();
    libtest_mimic::run(&args, tests).exit();
}
