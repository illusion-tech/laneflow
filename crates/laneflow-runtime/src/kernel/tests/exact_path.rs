//! #216 有限研究：批次重建计时与每 67 次查询一次的确定性采样。
//! 查询计时不是 sampled CPU，外推值也不是未插桩整步延迟。

use stats_alloc::{INSTRUMENTED_SYSTEM, Region, StatsAlloc};
use std::alloc::System;
use std::cell::{Cell, RefCell};
use std::time::Instant;

// 仅单元测试二进制。配对墙钟同时报告分配计数；零分配稳态不触发分配器记账。
// 未插桩生产库墙钟仍使用独立的 runtime_profile_evidence 二进制。
#[global_allocator]
static GLOBAL: &StatsAlloc<System> = &INSTRUMENTED_SYSTEM;

use crate as runtime_types;
#[path = "performance_profile/runtime_profile.rs"]
mod support;

#[path = "performance_profile/exact_query_replay.rs"]
mod query_replay;

use crate::kernel::occupancy::exact_candidate;

#[derive(Clone, Copy)]
enum Scene {
    Road(support::Scene),
    MultiEdge,
}

const CASES: [Scene; 4] = [
    Scene::Road(support::CASES[0]),
    Scene::Road(support::CASES[1]),
    Scene::Road(support::CASES[2]),
    Scene::MultiEdge,
];

impl Scene {
    fn name(self) -> String {
        match self {
            Self::Road(scene) => scene.name(),
            Self::MultiEdge => "multi-edge-1000-64-rings-32-edges".into(),
        }
    }

    fn count(self) -> usize {
        match self {
            Self::Road(scene) => scene.count(),
            Self::MultiEdge => 1_000,
        }
    }
}

struct Fixtures {
    roads: support::Fixtures,
    multi: std::sync::Arc<laneflow_static_network::SharedNetworkRevision>,
}

impl Fixtures {
    fn new() -> Self {
        Self {
            roads: support::Fixtures::new(),
            multi: exact_candidate::multi_edge_revision(),
        }
    }

    fn world(&self, scene: Scene) -> Window {
        match scene {
            Scene::Road(scene) => Window::Road(self.roads.world(scene)),
            Scene::MultiEdge => Window::MultiEdge(exact_candidate::multi_edge_world(&self.multi)),
        }
    }
}

enum Window {
    Road(support::Harness),
    MultiEdge(crate::TrafficWorld),
}

impl Window {
    fn world(&self) -> &crate::TrafficWorld {
        match self {
            Self::Road(harness) => &harness.world,
            Self::MultiEdge(world) => world,
        }
    }

    fn world_mut(&mut self) -> &mut crate::TrafficWorld {
        match self {
            Self::Road(harness) => &mut harness.world,
            Self::MultiEdge(world) => world,
        }
    }

    fn step(&mut self) {
        match self {
            Self::Road(harness) => harness.step(),
            Self::MultiEdge(world) => {
                std::hint::black_box(world.step(crate::TickInput::new(100)).unwrap());
            }
        }
    }

    fn validate(&self, scene: Scene) {
        if let Self::Road(harness) = self {
            assert_eq!(harness.steps, 64);
            harness.validate();
        }
        assert_eq!(self.world().live_vehicles().len(), scene.count());
        assert!(self.world().live_vehicles().iter().all(|handle| {
            self.world().vehicle(*handle).unwrap().status() == crate::VehicleStatus::Active
        }));
    }

    fn digest(&self) -> String {
        if let Self::Road(harness) = self {
            return harness.digest();
        }
        format!(
            "{:x}",
            crate::deterministic_state_digest(&self.world().capture_snapshot().unwrap()).unwrap()
        )
    }
}

fn assert_same_world(left: &crate::TrafficWorld, right: &crate::TrafficWorld) {
    assert_eq!(
        left.capture_snapshot().unwrap(),
        right.capture_snapshot().unwrap()
    );
    assert_eq!(left.live_vehicles(), right.live_vehicles());
    for handle in left.live_vehicles() {
        assert_eq!(left.vehicle_state(*handle), right.vehicle_state(*handle));
    }
    assert_eq!(
        left.latest_transition_events(),
        right.latest_transition_events()
    );
    assert_eq!(
        left.latest_waiting_decisions(),
        right.latest_waiting_decisions()
    );
    assert_eq!(
        left.latest_conflict_decisions(),
        right.latest_conflict_decisions()
    );
    exact_candidate::assert_same_index(left, right);
}

#[test]
fn candidate_multi_edge_states_events_and_index_match_each_tick() {
    let revision = exact_candidate::multi_edge_revision();
    let mut baseline = with_candidate(false, || exact_candidate::multi_edge_world(&revision));
    let mut candidate = with_candidate(true, || exact_candidate::multi_edge_world(&revision));
    assert_same_world(&baseline, &candidate);
    for _ in 0..16 {
        let input = crate::TickInput::new(100);
        let left = with_candidate(false, || baseline.step(input));
        let right = with_candidate(true, || candidate.step(input));
        assert_eq!(left, right);
        assert_same_world(&baseline, &candidate);
    }
}

#[test]
#[ignore = "manual #216 every-tick exact state, event and index comparison at measured scales"]
fn occupancy_exact_equivalence_windows() {
    let fixtures = Fixtures::new();
    for scene in CASES {
        let mut baseline = with_candidate(false, || fixtures.world(scene));
        let mut candidate = with_candidate(true, || fixtures.world(scene));
        assert_same_world(baseline.world(), candidate.world());
        for _ in 0..64 {
            let input = crate::TickInput::new(100);
            let left = with_candidate(false, || baseline.world_mut().step(input));
            let right = with_candidate(true, || candidate.world_mut().step(input));
            assert_eq!(left, right);
            assert_same_world(baseline.world(), candidate.world());
        }
        println!(
            "exact-equivalence scene={} steps=64 digest={}",
            scene.name(),
            candidate.digest()
        );
    }
}

const STAGES: usize = 8;
const QUERY_STRIDE: u64 = 67;
const NAMES: [&str; STAGES] = [
    "occupancy_count",
    "occupancy_layout",
    "occupancy_fill",
    "occupancy_sort_suffix",
    "route_profile_inputs",
    "leader_horizon",
    "leader_gap",
    "route_stop_queries",
];

#[derive(Clone, Copy)]
pub(crate) enum Stage {
    OccupancyCount,
    OccupancyLayout,
    OccupancyFill,
    OccupancySortSuffix,
    RouteProfileInputs,
    LeaderHorizon,
    LeaderGap,
    RouteStopQueries,
}

#[derive(Clone, Default)]
struct Profile {
    enabled: bool,
    phase: u64,
    calls: [u64; STAGES],
    samples: [u64; STAGES],
    nanos: [u128; STAGES],
}

thread_local! {
    static PROFILE: RefCell<Profile> = RefCell::new(Profile::default());
    static CANDIDATE: Cell<bool> = const { Cell::new(false) };
}

pub(crate) fn candidate_enabled() -> bool {
    CANDIDATE.get()
}

pub(crate) fn with_candidate<R>(enabled: bool, run: impl FnOnce() -> R) -> R {
    struct Restore(bool);
    impl Drop for Restore {
        fn drop(&mut self) {
            CANDIDATE.set(self.0);
        }
    }
    let _restore = Restore(CANDIDATE.replace(enabled));
    run()
}

pub(crate) struct Span(Option<(Stage, Instant)>);

pub(crate) fn begin(stage: Stage) -> Span {
    let sampled = PROFILE.with_borrow_mut(|profile| {
        if !profile.enabled {
            return false;
        }
        let index = stage as usize;
        let ordinal = profile.calls[index];
        profile.calls[index] += 1;
        index < Stage::RouteProfileInputs as usize
            || (ordinal + profile.phase).is_multiple_of(QUERY_STRIDE)
    });
    Span(sampled.then(|| (stage, Instant::now())))
}

impl Drop for Span {
    fn drop(&mut self) {
        if let Some((stage, started)) = self.0 {
            let elapsed = started.elapsed().as_nanos();
            PROFILE.with_borrow_mut(|profile| {
                profile.samples[stage as usize] += 1;
                profile.nanos[stage as usize] += elapsed;
            });
        }
    }
}

struct Session;

impl Session {
    fn start(phase: u64) -> Self {
        PROFILE.with_borrow_mut(|profile| {
            assert!(!profile.enabled, "research sessions must not nest");
            *profile = Profile {
                enabled: true,
                phase,
                ..Profile::default()
            };
        });
        Self
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        PROFILE.with_borrow_mut(|profile| profile.enabled = false);
    }
}

#[test]
fn attribution_sampling_counts_calls_and_disables_on_unwind() {
    let unwind = std::panic::catch_unwind(|| {
        let _session = Session::start(0);
        for _ in 0..(QUERY_STRIDE * 2) {
            drop(begin(Stage::LeaderGap));
        }
        panic!("test session unwind");
    });
    assert!(unwind.is_err());
    PROFILE.with_borrow(|profile| {
        assert!(!profile.enabled);
        assert_eq!(profile.calls[Stage::LeaderGap as usize], QUERY_STRIDE * 2);
        assert_eq!(profile.samples[Stage::LeaderGap as usize], 2);
    });
    drop(begin(Stage::LeaderGap));
    PROFILE.with_borrow(|profile| {
        assert_eq!(profile.calls[Stage::LeaderGap as usize], QUERY_STRIDE * 2);
    });
}

#[test]
#[ignore = "manual release #216 batch and sampled query attribution"]
fn occupancy_exact_attribution() {
    let mut clock_ns = 0;
    let mut clock_zero = 0;
    for _ in 0..10_000 {
        let started = Instant::now();
        let elapsed = started.elapsed().as_nanos();
        clock_ns += elapsed;
        clock_zero += usize::from(elapsed == 0);
    }
    println!("exact-clock calls=10000 ns={clock_ns} zero_samples={clock_zero}");
    let fixtures = Fixtures::new();
    for scene in CASES {
        for round in 0..3 {
            for candidate in [false, true] {
                with_candidate(candidate, || {
                    let mut harness = fixtures.world(scene);
                    harness.validate(scene);
                    let mut inspections = 0;
                    let mut walks = 0;
                    let mut records = 0;
                    let session = Session::start(round * 19);
                    let started = Instant::now();
                    for _ in 0..64 {
                        harness.step();
                        inspections += harness.world().derived.occupancy.inspections();
                        walks += harness.world().derived.occupancy.occurrence_walks();
                        records += harness.world().derived.occupancy.records_len();
                    }
                    let instrumented_ns = started.elapsed().as_nanos();
                    drop(session);
                    harness.validate(scene);
                    let profile = PROFILE.with_borrow(Clone::clone);
                    for (index, name) in NAMES.iter().enumerate() {
                        let expected = 64
                            * if index < Stage::RouteProfileInputs as usize {
                                1
                            } else {
                                scene.count() as u64
                            };
                        assert_eq!(profile.calls[index], expected, "{name}");
                        assert!(profile.samples[index] > 0);
                        println!(
                            "exact-stage scene={} candidate={candidate} round={round} stage={name} calls={} samples={} ns={}",
                            scene.name(),
                            profile.calls[index],
                            profile.samples[index],
                            profile.nanos[index],
                        );
                    }
                    println!(
                        "exact-work scene={} candidate={candidate} round={round} steps=64 active={} records={records} inspections={inspections} occurrence_walks={walks} instrumented_ns={instrumented_ns} digest={}",
                        scene.name(),
                        scene.count(),
                        harness.digest(),
                    );
                    if matches!(scene, Scene::MultiEdge) {
                        assert!(
                            walks > 0,
                            "multi-edge attribution must execute later occurrences"
                        );
                        assert!(
                            records > 64 * scene.count(),
                            "multi-edge bodies must span boundaries"
                        );
                    }
                });
            }
        }
    }
}

#[test]
#[ignore = "manual release #216 paired test-build whole-step and allocation comparison"]
fn occupancy_exact_paired_windows() {
    let fixtures = Fixtures::new();
    for scene in CASES {
        let mut reference_digest = None;
        for round in 0..6 {
            let order = if round % 2 == 0 {
                [false, true]
            } else {
                [true, false]
            };
            for (position, candidate) in order.into_iter().enumerate() {
                with_candidate(candidate, || {
                    let mut harness = fixtures.world(scene);
                    harness.validate(scene);
                    let mut samples = [0; 64];
                    let mut records = 0;
                    let mut inspections = 0;
                    let mut walks = 0;
                    let region = Region::new(GLOBAL);
                    for sample in &mut samples {
                        let started = Instant::now();
                        harness.step();
                        *sample = started.elapsed().as_nanos();
                        records += harness.world().derived.occupancy.records_len();
                        inspections += harness.world().derived.occupancy.inspections();
                        walks += harness.world().derived.occupancy.occurrence_walks();
                    }
                    let allocation = region.change();
                    harness.validate(scene);
                    let digest = harness.digest();
                    if let Some(expected) = &reference_digest {
                        assert_eq!(
                            &digest, expected,
                            "all variants and rounds use the same trace"
                        );
                    } else {
                        reference_digest = Some(digest.clone());
                    }
                    samples.sort_unstable();
                    let percentile =
                        |percent: usize| samples[(samples.len() * percent).div_ceil(100) - 1];
                    let memory = harness.world().retained_memory();
                    println!(
                        "exact-pair scene={} candidate={candidate} round={round} position={position} steps=64 active={} p50_ns={} p95_ns={} p99_ns={} max_ns={} sum_ns={} records={records} inspections={inspections} occurrence_walks={walks} allocations={} reallocations={} allocated_bytes={} deallocated_bytes={} reallocated_bytes={} source_world_owned={} shared_root={} binding={} committed={} derived={} workspace={} administrative={} pending_bytes={} digest={digest}",
                        scene.name(),
                        scene.count(),
                        percentile(50),
                        percentile(95),
                        percentile(99),
                        samples.last().unwrap(),
                        samples.iter().sum::<u128>(),
                        allocation.allocations,
                        allocation.reallocations,
                        allocation.bytes_allocated,
                        allocation.bytes_deallocated,
                        allocation.bytes_reallocated,
                        memory.world_owned_bytes(),
                        memory.shared_network,
                        memory.partitions[0],
                        memory.partitions[1],
                        memory.partitions[2],
                        memory.partitions[3],
                        memory.partitions[4],
                        exact_candidate::pending_bytes(harness.world()),
                    );
                });
            }
        }
    }
}
