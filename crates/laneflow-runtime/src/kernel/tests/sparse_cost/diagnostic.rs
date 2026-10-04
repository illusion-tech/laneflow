//! #682 正交账本与嵌套批次诊断。绝不把本二进制当未插桩生产延迟。
use crate as runtime_types;
pub(crate) mod fixture;
use crate::kernel::{exact_path_research, performance_profile};
use stats_alloc::{INSTRUMENTED_SYSTEM, Region};
use std::{cell::Cell, time::Instant};

thread_local! {
    static ENABLED: Cell<bool> = const { Cell::new(false) };
    static NANOS: Cell<[u128; 5]> = const { Cell::new([0; 5]) };
    static ITEMS: Cell<[usize; 5]> = const { Cell::new([0; 5]) };
    static CALLS: Cell<[usize; 5]> = const { Cell::new([0; 5]) };
}
crate::kernel::execution::carry_hooks!(carry_test_hooks: ENABLED, NANOS, ITEMS, CALLS);

pub(crate) struct Span(Option<(usize, Instant)>);
pub(crate) fn begin(stage: usize, items: usize) -> Span {
    if !ENABLED.get() {
        return Span(None);
    }
    let mut counts = ITEMS.get();
    counts[stage] += items;
    ITEMS.set(counts);
    let mut calls = CALLS.get();
    calls[stage] += 1;
    CALLS.set(calls);
    Span(Some((stage, Instant::now())))
}
impl Drop for Span {
    fn drop(&mut self) {
        if let Some((stage, start)) = self.0 {
            let mut nanos = NANOS.get();
            nanos[stage] += start.elapsed().as_nanos();
            NANOS.set(nanos);
        }
    }
}
struct Session;
impl Session {
    fn start() -> Self {
        assert!(!ENABLED.replace(true), "nested sparse session");
        NANOS.set([0; 5]);
        ITEMS.set([0; 5]);
        CALLS.set([0; 5]);
        Self
    }
}

#[test]
#[ignore = "manual #682 small resource fixture sorting attribution; not city latency"]
fn resource_sort_diagnostic() {
    println!();
    use performance_profile::support::{Fixtures, Scene};
    let fixtures = Fixtures::new();
    for round in 0..3 {
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
            for stage in 2..5 {
                println!(
                    "resource-sort round={round} scene={} stage={stage} steps={} calls={} items={} ns={} digest={}",
                    scene.name(),
                    harness.steps,
                    CALLS.get()[stage],
                    ITEMS.get()[stage],
                    NANOS.get()[stage],
                    harness.digest()
                );
            }
        }
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        ENABLED.set(false);
    }
}

fn assert_axes(world: &crate::TrafficWorld, case: fixture::Case) {
    fixture::validate(world, case);
    assert_eq!(
        world.state.committed.vehicles.len(),
        case.high_water as usize
    );
    assert_eq!(world.state.binding.config.vehicle_capacity(), case.capacity);
    assert_eq!(world.state.derived.active_order.len(), case.active as usize);
    assert!(world.state.conflict_state_valid());
}

#[test]
#[ignore = "manual #682 release memory and batch diagnostic; not production latency"]
fn sparse_cost_diagnostic() {
    println!();
    for case in fixture::selected() {
        let root = fixture::revision(case);
        let mut world = fixture::world(&root, case);
        assert_axes(&world, case);
        let mut records = 0;
        let mut inspections = 0;
        let mut walks = 0;
        let region = Region::new(&INSTRUMENTED_SYSTEM);
        let session = Session::start();
        let (((), phases), occupancy) = exact_path_research::measure_batches(|| {
            performance_profile::measure(|| {
                for _ in 0..fixture::STEPS {
                    {
                        let _whole =
                            performance_profile::begin(performance_profile::Stage::WholeStep);
                        fixture::step(&mut world);
                    }
                    records += world.state.derived.occupancy.records_len();
                    inspections += world.state.derived.occupancy.inspections();
                    walks += world.state.derived.occupancy.occurrence_walks();
                }
            })
        });
        drop(session);
        let allocations = region.change();
        assert_axes(&world, case);
        let memory = world.state.retained_memory();
        for (stage, ns) in performance_profile::stage_names().iter().zip(phases) {
            println!("sparse-stage case={} stage={stage} ns={ns}", case.name);
        }
        for (stage, ns) in ["count", "layout", "fill", "sort_suffix"]
            .iter()
            .zip(occupancy)
        {
            println!("sparse-occupancy case={} stage={stage} ns={ns}", case.name);
        }
        println!(
            "sparse-clear case={} waiting_ns={} conflict_ns={} waiting_slots={} conflict_slots={}",
            case.name,
            NANOS.get()[0],
            NANOS.get()[1],
            ITEMS.get()[0],
            ITEMS.get()[1]
        );
        println!(
            "sparse-memory case={} world={} shared={} binding={} committed={} derived={} workspace={} admin={} vehicle_slot_size={} vehicle_state_size={} slots_len={} slots_capacity={}",
            case.name,
            memory.world_owned_bytes(),
            memory.shared_network,
            memory.partitions[0],
            memory.partitions[1],
            memory.partitions[2],
            memory.partitions[3],
            memory.partitions[4],
            size_of::<crate::kernel::tables::VehicleSlot>(),
            size_of::<crate::VehicleState>(),
            world.state.committed.vehicles.len(),
            world.state.committed.vehicles.capacity()
        );
        println!(
            "sparse-work case={} records={records} inspections={inspections} occurrence_walks={walks} allocations={} reallocations={} bytes_allocated={}",
            case.name,
            allocations.allocations,
            allocations.reallocations,
            allocations.bytes_allocated
        );
        println!(
            "sparse-end case={} active={} live={} high_water={} capacity={} edges={} warm={} steps={} digest={} input={:?}",
            case.name,
            case.active,
            case.live,
            case.high_water,
            case.capacity,
            case.edges,
            fixture::WARM,
            fixture::STEPS,
            fixture::digest(&world),
            root.canonical_origin().canonical_artifact_digest()
        );
    }
}

#[test]
fn sparse_high_water_failure_retry_and_generation() {
    let case = fixture::Case {
        active: 8,
        live: 8,
        high_water: 64,
        capacity: 128,
        ..fixture::CASES[0]
    };
    let root = fixture::revision(case);
    let mut world = fixture::world(&root, case);
    let mut fresh = fixture::world(&root, case);
    assert_axes(&world, case);
    let parked_input = crate::ParkedVehicleSpawnInput::new(
        laneflow_static_contract::VehicleProfileOrdinal::from_raw(0),
        world.vehicle(world.live_vehicles()[0]).unwrap().route(),
        0,
        0,
    );
    let target = crate::ParkingTarget::VirtualPool(
        laneflow_static_contract::ParkingFacilityOrdinal::from_raw(0),
    );
    let old = world
        .spawn_parked_vehicle(parked_input, target)
        .unwrap()
        .vehicle;
    world.despawn_vehicle(old).unwrap();
    let new = world
        .spawn_parked_vehicle(parked_input, target)
        .unwrap()
        .vehicle;
    assert_eq!(old.index(), new.index());
    assert_ne!(old.generation(), new.generation());
    assert!(world.vehicle(old).is_none());
    world.despawn_vehicle(new).unwrap();
    for _ in 0..2 {
        let handle = fresh
            .spawn_parked_vehicle(parked_input, target)
            .unwrap()
            .vehicle;
        fresh.despawn_vehicle(handle).unwrap();
    }
    assert_eq!(
        world.capture_snapshot().unwrap(),
        fresh.capture_snapshot().unwrap()
    );
    for point in [
        crate::kernel::tick::StepFailpoint::AfterGrants,
        crate::kernel::tick::StepFailpoint::AfterTransitions,
    ] {
        let before = world.capture_snapshot().unwrap();
        crate::kernel::tick::STEP_FAILPOINT.set(Some(point));
        let failed = world.step(crate::TickInput::new(100));
        crate::kernel::tick::STEP_FAILPOINT.set(None);
        assert!(failed.is_err());
        assert_eq!(world.capture_snapshot().unwrap(), before);
        fixture::step(&mut world);
        fixture::step(&mut fresh);
        assert_eq!(
            world.capture_snapshot().unwrap(),
            fresh.capture_snapshot().unwrap()
        );
        assert_eq!(
            world.latest_transition_events(),
            fresh.latest_transition_events()
        );
        assert_axes(&world, case);
    }
}
