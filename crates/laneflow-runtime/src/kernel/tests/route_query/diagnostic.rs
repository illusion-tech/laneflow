//! #679 二分调用账本；site 数字绑定研究基线源码行，不随格式化重新编号。
use crate as runtime_types;
mod fixture;
mod index;
use crate::kernel::tick::{barrier_query_counts, reset_barrier_query_counts};
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;

thread_local! {
    static ENABLED: Cell<bool> = const { Cell::new(false) };
    static SEARCHES: RefCell<BTreeMap<&'static str, u64>> = const { RefCell::new(BTreeMap::new()) };
}
crate::kernel::execution::carry_hooks!(carry_test_hooks: ENABLED, SEARCHES);

pub(crate) fn note_search(site: &'static str) {
    if ENABLED.get() {
        SEARCHES.with_borrow_mut(|counts| *counts.entry(site).or_default() += 1);
    }
}

struct Session;
impl Session {
    fn start() -> Self {
        assert!(!ENABLED.replace(true));
        SEARCHES.with_borrow_mut(BTreeMap::clear);
        reset_barrier_query_counts();
        Self
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        ENABLED.set(false);
    }
}

fn report(case: &str) {
    SEARCHES.with_borrow(|counts| {
        for (site, calls) in counts {
            println!("route-search case={case} site={site} calls={calls}");
        }
    });
    let counts = barrier_query_counts();
    println!(
        "route-barrier case={case} signal_gates={} conflict_scans={} waiting_scans={}",
        counts.signal_gates, counts.conflict_scans, counts.waiting_entry_scans
    );
}

#[test]
fn bounded_signal_matches_reachable_unbounded_stop_across_loop_cursors() {
    use crate::kernel::tick::MotionReach;
    use laneflow_static_contract::SignalAspect;
    use laneflow_static_network::BoundedDistance;
    let root = fixture::revision(false);
    let (mut world, _) = fixture::world(&root, 8);
    let base = world.vehicle(world.live_vehicles()[0]).unwrap();
    let reach = MotionReach::from_tick(10_000, 1.8, 0.1).unwrap();
    for aspect in [SignalAspect::Green, SignalAspect::Red, SignalAspect::Yellow] {
        world.state.committed.signal_aspects.fill(aspect);
        let view = world.state.read_view();
        let compiled = view.compiled_route(base.route).unwrap();
        for cursor in 0..compiled.edges.len() {
            for progress in [0, 1, 7_999_999, 8_000_000] {
                let state = crate::VehicleState {
                    route_edge_index: cursor as u32,
                    progress_mm: progress,
                    speed_mm_s: 10_000,
                    ..base
                };
                let exact = view.signal_stop_distance(compiled, &state, cursor, None);
                let expected = exact.filter(|distance| match distance {
                    BoundedDistance::Finite(mm) => !reach.excludes(*mm),
                    BoundedDistance::BeyondFinite => false,
                });
                assert_eq!(
                    view.signal_stop_distance(compiled, &state, cursor, Some(reach)),
                    expected,
                    "cursor={cursor} progress={progress} aspect={aspect:?}"
                );
            }
        }
    }
}

fn class_policy_queries() {
    use crate::kernel::tick::MotionReach;
    use laneflow_static_network::BoundedDistance;
    let root = fixture::revision(false);
    let reach = MotionReach::from_tick(0, 1.8, 0.1).unwrap();
    for policy in ["policy", "deny-policy"] {
        let (world, _) = fixture::world_with_policy(&root, 8, policy);
        let base = world.vehicle(world.live_vehicles()[0]).unwrap();
        let view = world.state.read_view();
        let compiled = view.compiled_route(base.route).unwrap();
        for profile in ["car", "restricted"] {
            let state = crate::VehicleState {
                profile: fixture::profile(&root, profile),
                progress_mm: 7_999_999,
                speed_mm_s: 0,
                ..base
            };
            reset_barrier_query_counts();
            let stop = view.signal_stop_distance(compiled, &state, 0, Some(reach));
            let denied = policy == "deny-policy" || profile == "restricted";
            assert_eq!(stop, denied.then_some(BoundedDistance::Finite(1)));
            assert_eq!(barrier_query_counts().signal_gates, 1);
            println!(
                "route-policy policy={policy} profile={profile} signal_gates=1 denied={denied}"
            );
        }
    }
}

#[test]
fn class_policy_signal_queries_do_not_share_permissions() {
    class_policy_queries();
}

#[test]
#[ignore = "manual #679 counters and memory; not production latency"]
fn route_query_diagnostic() {
    println!();
    class_policy_queries();
    for red in [false, true] {
        let root = fixture::revision(red);
        for count in fixture::REPEATS {
            let (mut world, _) = fixture::world(&root, count);
            let session = Session::start();
            for _ in 0..fixture::STEPS {
                fixture::step(&mut world);
            }
            drop(session);
            let state_sum = fixture::validate(&world);
            assert!(world.state.conflict_state_valid());
            let route = world.vehicle(world.live_vehicles()[0]).unwrap().route();
            let compiled = world.state.read_view().compiled_route(route).unwrap();
            let case = format!("red-{red}-repeats-{count}");
            report(&case);
            println!(
                "route-memory case={case} world={} hops={} gates={} waiting={} conflicts={} state_sum={state_sum}",
                world.state.retained_memory().world_owned_bytes(),
                compiled.edges.len(),
                compiled.gate_hops.len(),
                compiled.waiting.len(),
                compiled.conflicts.len()
            );
            index::measure(compiled, &case);
            let state = world.vehicle(world.live_vehicles()[0]).unwrap();
            let view = world.state.read_view();
            reset_barrier_query_counts();
            let unbounded = view.signal_stop_distance(compiled, &state, 0, None);
            println!(
                "route-unbounded case={case} signal_gates={} stop={unbounded:?}",
                barrier_query_counts().signal_gates
            );
            assert_eq!(
                barrier_query_counts().signal_gates as usize,
                if red { 2 } else { count * 2 }
            );
        }
    }
    use crate::kernel::performance_profile::support::{Fixtures, Scene};
    let fixtures = Fixtures::new();
    for scene in [Scene::Waiting, Scene::Conflict] {
        let mut harness = fixtures.world(scene);
        harness.validate();
        let session = Session::start();
        for _ in 0..harness.steps {
            harness.step();
        }
        drop(session);
        harness.validate();
        assert!(harness.world.state.conflict_state_valid());
        report(&scene.name());
    }
}
