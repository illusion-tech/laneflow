//! #216 唯一候选：一次生成占用记录，再从连续暂存区分桶。
//! 只在单元测试显式选择时启用；既有默认算法和生产内存布局不变。

use super::*;
use crate::kernel::exact_path_research::{Stage, begin, with_candidate};

thread_local! {
    static FAIL_PENDING_RESERVE: Cell<Option<usize>> = const { Cell::new(None) };
}

fn pending_reserve_allowed() -> bool {
    FAIL_PENDING_RESERVE.with(|remaining| match remaining.get() {
        Some(0) => false,
        Some(count) => {
            remaining.set(Some(count - 1));
            true
        }
        None => true,
    })
}

pub(super) fn rebuild(
    binding: &crate::kernel::state::WorldBindingState,
    committed: &crate::kernel::state::CommittedWorldState,
    active_order: &[VehicleHandle],
    occupancy: &mut OccupancyIndex,
    scratch: &mut OccupancyScratch,
) -> Result<(), StepError> {
    let count_timer = begin(Stage::OccupancyCount);
    let bucket_count = binding.revision.traffic().lane_edge_count() as usize;
    let ceiling = occupancy_record_limit(binding.config.vehicle_capacity());
    occupancy.reset_inspections();
    occupancy.try_prepare_scratch(scratch, bucket_count)?;
    scratch.exact_pending.clear();
    let mut allocation_failed = false;
    visit_occupancy_records(
        active_order,
        &committed.vehicles,
        &binding.revision,
        &committed.routes,
        |record| {
            if let Some(count) = scratch.positions.get_mut(record.bucket.index()) {
                *count += 1;
            }
            // 即使暂存分配失败，也走完原来的完整性检查；不改变原算法的首错优先级。
            if allocation_failed || scratch.exact_pending.len() == ceiling {
                return;
            }
            if scratch.exact_pending.len() == scratch.exact_pending.capacity()
                && (!pending_reserve_allowed() || scratch.exact_pending.try_reserve(1).is_err())
            {
                allocation_failed = true;
                return;
            }
            scratch.exact_pending.push(record);
        },
    )?;
    let total = scratch.record_total(bucket_count);
    if total > ceiling {
        return Err(StepError::OccupancyCapacityExceeded);
    }
    if allocation_failed {
        return Err(StepError::OccupancyAllocFailed);
    }
    drop(count_timer);
    let layout_timer = begin(Stage::OccupancyLayout);
    occupancy.try_reserve_records(total)?;
    occupancy.finish_layout(scratch, bucket_count);
    drop(layout_timer);
    let fill_timer = begin(Stage::OccupancyFill);
    for index in 0..scratch.exact_pending.len() {
        occupancy.write_record(scratch, scratch.exact_pending[index]);
    }
    drop(fill_timer);
    let _sort_timer = begin(Stage::OccupancySortSuffix);
    occupancy.sort_buckets(bucket_count);
    Ok(())
}

pub(crate) fn multi_edge_revision() -> std::sync::Arc<SharedNetworkRevision> {
    use laneflow_compiler::{LaneEdgeInput, LaneEdgeReference};

    super::tests::compile_revision(|module| {
        super::tests::add_car_profile(module);
        for ring in 0..64 {
            for edge in 0..32 {
                module
                    .add_lane_edge(LaneEdgeInput {
                        lane_edge_key: &format!("ring-{ring}-edge-{edge}"),
                        length_meters: 10.0,
                        speed_limit_meters_per_second: 15.0,
                        successors: &[LaneEdgeReference::local(&format!(
                            "ring-{ring}-edge-{}",
                            (edge + 1) % 32,
                        ))],
                    })
                    .unwrap();
            }
        }
    })
}

pub(crate) fn multi_edge_world(revision: &std::sync::Arc<SharedNetworkRevision>) -> TrafficWorld {
    use crate::{RouteRegisterInput, TickInput, VehicleSpawnInput, WorldConfig};
    use laneflow_static_contract::VehicleProfileOrdinal;

    let origin = *revision.canonical_origin();
    let mut world = TrafficWorld::install(
        std::sync::Arc::clone(revision),
        WorldConfig::new(1_000, 64, 4_096, 1_024, 1, 100),
        crate::CommittedNetworkSource::Published {
            reference: crate::PublishedLfcaReference::new(
                "fixture://issue-216-multi-edge",
                origin.canonical_artifact_digest(),
                origin.canonical_artifact_byte_length(),
                origin.network_revision(),
            )
            .unwrap(),
        },
        216,
        crate::WorldPolicySelection::NotRequired,
    )
    .unwrap();
    let mut seen = vec![false; 2_048];
    let mut routes = Vec::new();
    for raw in 0..2_048 {
        let start = LaneEdgeOrdinal::from_raw(raw);
        if seen[start.index()] {
            continue;
        }
        let mut edge = start;
        let mut cycle = Vec::new();
        loop {
            assert!(!seen[edge.index()]);
            seen[edge.index()] = true;
            cycle.push(edge);
            let next = revision.traffic().successors(edge).unwrap();
            assert_eq!(next.len(), 1);
            edge = next[0];
            if edge == start {
                break;
            }
        }
        assert_eq!(cycle.len(), 32);
        let repeated = cycle.repeat(2);
        routes.push(
            world
                .register_route(RouteRegisterInput::new(repeated))
                .unwrap(),
        );
    }
    assert_eq!(routes.len(), 64);
    for vehicle in 0..1_000 {
        let distance = (vehicle / 64) * 20_000 + 5_000;
        world
            .spawn_vehicle(VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                routes[(vehicle % 64) as usize],
                distance / 10_000,
                distance % 10_000,
                0,
            ))
            .unwrap();
    }
    for _ in 0..40 {
        world.step(TickInput::new(100)).unwrap();
    }
    world
}

pub(crate) fn pending_bytes(world: &TrafficWorld) -> u64 {
    crate::kernel::state::vec_bytes(&world.workspace.occupancy_scratch.exact_pending)
}

pub(crate) fn assert_same_index(left: &TrafficWorld, right: &TrafficWorld) {
    let left = &left.derived.occupancy;
    let right = &right.derived.occupancy;
    assert_eq!(left.offsets, right.offsets);
    assert_eq!(left.records, right.records);
    assert_eq!(left.suffix_min_lo, right.suffix_min_lo);
    assert_eq!(left.suffix_second_lo, right.suffix_second_lo);
}

fn with_pending_failure<R>(after: usize, run: impl FnOnce() -> R) -> R {
    struct Restore(Option<usize>);
    impl Drop for Restore {
        fn drop(&mut self) {
            FAIL_PENDING_RESERVE.set(self.0);
        }
    }
    let _restore = Restore(FAIL_PENDING_RESERVE.replace(Some(after)));
    run()
}

#[test]
fn pending_allocation_failures_preserve_published_state_and_retry_matches_fresh() {
    let revision = multi_edge_revision();
    let mut failures = 0;
    for after in 0..16 {
        let mut world = with_candidate(false, || multi_edge_world(&revision));
        let before = world.capture_snapshot().unwrap();
        let events = world.latest_transition_events().to_vec();
        let records = world.derived.occupancy.records.clone();
        let input = crate::TickInput::new(100);
        let result = with_candidate(true, || with_pending_failure(after, || world.step(input)));
        if result.is_ok() {
            break;
        }
        assert_eq!(result, Err(StepError::OccupancyAllocFailed));
        failures += 1;
        assert_eq!(world.capture_snapshot().unwrap(), before);
        assert_eq!(world.latest_transition_events(), events);
        assert_eq!(world.derived.occupancy.records, records);
        with_candidate(true, || world.step(input)).unwrap();
        let mut fresh = with_candidate(false, || multi_edge_world(&revision));
        fresh.step(input).unwrap();
        assert_eq!(
            world.capture_snapshot().unwrap(),
            fresh.capture_snapshot().unwrap()
        );
        assert_eq!(
            world.latest_transition_events(),
            fresh.latest_transition_events()
        );
        assert_same_index(&world, &fresh);
    }
    assert!(
        failures > 1 && failures < 16,
        "bounded sequence reaches success"
    );
}

#[test]
fn incomplete_route_keeps_priority_over_pending_allocation_failure() {
    let revision = multi_edge_revision();
    let mut world = with_candidate(false, || multi_edge_world(&revision));
    let before = world.capture_snapshot().unwrap();
    let records = world.derived.occupancy.records.clone();
    let handle = *world.live_vehicles().last().unwrap();
    let previous = *world.vehicle_state(handle).unwrap();
    world.committed.vehicles[handle.index() as usize]
        .state
        .as_mut()
        .unwrap()
        .route_edge_index = u32::MAX;
    let input = crate::TickInput::new(100);
    assert_eq!(
        with_candidate(false, || world.step(input)),
        Err(StepError::OccupancyIntervalIncomplete)
    );
    assert_eq!(
        with_candidate(true, || with_pending_failure(0, || world.step(input))),
        Err(StepError::OccupancyIntervalIncomplete)
    );
    assert_eq!(world.derived.occupancy.records, records);
    world.committed.vehicles[handle.index() as usize].state = Some(previous);
    assert_eq!(world.capture_snapshot().unwrap(), before);
    with_candidate(true, || world.step(input)).unwrap();
    let mut fresh = with_candidate(false, || multi_edge_world(&revision));
    fresh.step(input).unwrap();
    assert_eq!(
        world.capture_snapshot().unwrap(),
        fresh.capture_snapshot().unwrap()
    );
    assert_same_index(&world, &fresh);
}

#[test]
fn candidate_selector_restores_on_unwind_and_nested_calls() {
    assert!(!crate::kernel::exact_path_research::candidate_enabled());
    let unwind = std::panic::catch_unwind(|| {
        with_candidate(true, || {
            assert!(crate::kernel::exact_path_research::candidate_enabled());
            with_candidate(false, || {
                assert!(!crate::kernel::exact_path_research::candidate_enabled());
            });
            assert!(crate::kernel::exact_path_research::candidate_enabled());
            panic!("test candidate unwind");
        });
    });
    assert!(unwind.is_err());
    assert!(!crate::kernel::exact_path_research::candidate_enabled());
}
