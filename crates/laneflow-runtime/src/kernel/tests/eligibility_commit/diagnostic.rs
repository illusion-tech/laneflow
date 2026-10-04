//! #777 资格表复制与全空扫描归因，不是生产库延迟。
use crate as runtime_types;
mod input;
use crate::kernel::sparse_cost_research::fixture as sparse;
use stats_alloc::{INSTRUMENTED_SYSTEM, Region};
use std::{cell::Cell, time::Instant};

pub(crate) const NAMES: [&str; 5] = [
    "clear_motion",
    "clear_eligibility",
    "copy",
    "scan",
    "eligibility_commit",
];
thread_local! {
    static ENABLED: Cell<bool> = const { Cell::new(false) };
    static NANOS: Cell<[u128; 5]> = const { Cell::new([0;5]) };
    static ITEMS: Cell<[usize; 5]> = const { Cell::new([0;5]) };
    static CALLS: Cell<[usize; 5]> = const { Cell::new([0;5]) };
}
crate::kernel::execution::carry_hooks!(carry_test_hooks: ENABLED, NANOS, ITEMS, CALLS);

pub(crate) struct Span(Option<(usize, Instant)>);
pub(crate) fn begin(stage: usize, items: usize) -> Span {
    if !ENABLED.get() {
        return Span(None);
    }
    let mut v = ITEMS.get();
    v[stage] += items;
    ITEMS.set(v);
    let mut v = CALLS.get();
    v[stage] += 1;
    CALLS.set(v);
    Span(Some((stage, Instant::now())))
}
impl Drop for Span {
    fn drop(&mut self) {
        if let Some((stage, start)) = self.0 {
            let mut v = NANOS.get();
            v[stage] += start.elapsed().as_nanos();
            NANOS.set(v);
        }
    }
}
struct Session;
impl Session {
    fn start() -> Self {
        assert!(!ENABLED.replace(true));
        NANOS.set([0; 5]);
        ITEMS.set([0; 5]);
        CALLS.set([0; 5]);
        Self
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        ENABLED.set(false);
    }
}

/// 表级位置/密度对照；合成槽位只验证提交拷贝，不冒充合法交通世界。
#[cfg(feature = "placement-fixtures")]
#[test]
fn eligibility_table_commit_matches_dense_oracle() {
    let (mut world, _) = input::create("conflict-small");
    let value = world
        .state
        .committed
        .conflict_eligibility
        .iter()
        .copied()
        .flatten()
        .next()
        .expect("real eligible vehicle");
    let size = world.state.workspace.conflict_next_eligibility.len();
    let additional = size.saturating_sub(world.state.committed.conflict_eligibility.len());
    world
        .state
        .committed
        .conflict_eligibility
        .reserve(additional);
    for positions in [vec![0], vec![size - 1], (0..size).collect(), Vec::new()] {
        world.state.workspace.conflict_next_eligibility.fill(None);
        for slot in positions {
            world.state.workspace.conflict_next_eligibility[slot] = Some(value);
        }
        let mut expected = world.state.workspace.conflict_next_eligibility.to_vec();
        if expected.iter().all(Option::is_none) {
            expected.clear();
        }
        world.state.committed_mut().commit_conflict_step();
        assert_eq!(world.state.committed.conflict_eligibility, expected);
    }
    assert!(
        world.state.committed.conflict_eligibility.is_empty(),
        "old eligibility revoked"
    );
}

#[test]
#[ignore = "manual #777 release diagnostic"]
fn eligibility_commit_diagnostic() {
    println!();
    let case = input::selected();
    let mut nanos = [0_u128; 5];
    let mut items = [0_usize; 5];
    let mut calls = [0_usize; 5];
    let mut phases = [0_u128; 11];
    let mut allocations = 0;
    let mut reallocations = 0;
    let mut bytes = 0;
    let mut min_live_eligibility = usize::MAX;
    let mut max_live_eligibility = 0;
    for window in 0..input::windows(&case) {
        let (mut world, digest) = input::create(&case);
        assert!(world.state.conflict_state_valid());
        let session = Session::start();
        for _ in 0..input::TICKS / input::windows(&case) {
            let region = Region::new(&INSTRUMENTED_SYSTEM);
            let ((), measured) = crate::kernel::performance_profile::measure(|| {
                let _whole = crate::kernel::performance_profile::begin(
                    crate::kernel::performance_profile::Stage::WholeStep,
                );
                input::step(&mut world, &case);
            });
            let diff = region.change();
            allocations += diff.allocations;
            reallocations += diff.reallocations;
            bytes += diff.bytes_allocated;
            for i in 0..11 {
                phases[i] += measured[i];
            }
            let live = world
                .state
                .committed
                .conflict_eligibility
                .iter()
                .filter(|v| v.is_some())
                .count();
            min_live_eligibility = min_live_eligibility.min(live);
            max_live_eligibility = max_live_eligibility.max(live);
        }
        drop(session);
        for i in 0..5 {
            nanos[i] += NANOS.get()[i];
            items[i] += ITEMS.get()[i];
            calls[i] += CALLS.get()[i];
        }
        input::validate(&world, &case);
        assert!(world.state.conflict_state_valid());
        input::end(&case, window, &world, &digest);
        let m = world.state.retained_memory();
        println!(
            "elig-memory case={case} window={window} world={} shared={} binding={} committed={} derived={} workspace={} admin={}",
            m.world_owned_bytes(),
            m.shared_network,
            m.partitions[0],
            m.partitions[1],
            m.partitions[2],
            m.partitions[3],
            m.partitions[4]
        );
    }
    for i in 0..5 {
        println!(
            "elig-detail case={case} stage={} ns={} items={} calls={}",
            NAMES[i], nanos[i], items[i], calls[i]
        );
    }
    for (stage, ns) in crate::kernel::performance_profile::stage_names()
        .iter()
        .zip(phases)
    {
        println!("elig-phase case={case} stage={stage} ns={ns}");
    }
    println!(
        "elig-work case={case} allocations={allocations} reallocations={reallocations} bytes={bytes} min_eligible={min_live_eligibility} max_eligible={max_live_eligibility}"
    );
    if case == "conflict-large" {
        table_diagnostic(&case);
    }
}

fn table_diagnostic(case: &str) {
    let (mut world, _) = input::create(case);
    let value = world
        .state
        .committed
        .conflict_eligibility
        .iter()
        .copied()
        .flatten()
        .next()
        .unwrap();
    let size = world.state.workspace.conflict_next_eligibility.len();
    let additional = size.saturating_sub(world.state.committed.conflict_eligibility.len());
    world
        .state
        .committed
        .conflict_eligibility
        .reserve(additional);
    for pattern in ["empty", "first", "last", "dense"] {
        world.state.workspace.conflict_next_eligibility.fill(None);
        match pattern {
            "first" => world.state.workspace.conflict_next_eligibility[0] = Some(value),
            "last" => world.state.workspace.conflict_next_eligibility[size - 1] = Some(value),
            "dense" => world
                .state
                .workspace
                .conflict_next_eligibility
                .fill(Some(value)),
            _ => {}
        }
        let region = Region::new(&INSTRUMENTED_SYSTEM);
        let session = Session::start();
        for _ in 0..128 {
            std::hint::black_box(&mut world.state)
                .committed_mut()
                .commit_conflict_step();
        }
        drop(session);
        let diff = region.change();
        println!(
            "elig-table case={case} pattern={pattern} slots={size} calls=128 commit_ns={} copy_ns={} scan_ns={} copy_items={} scan_items={} allocations={} reallocations={}",
            NANOS.get()[4],
            NANOS.get()[2],
            NANOS.get()[3],
            ITEMS.get()[2],
            ITEMS.get()[3],
            diff.allocations,
            diff.reallocations
        );
    }
}
