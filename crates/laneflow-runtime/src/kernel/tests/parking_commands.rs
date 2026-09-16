use std::cell::Cell;

#[path = "parking_commands/fixture.rs"]
mod fixture;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct Counts {
    pub calls: usize,
    pub followers: usize,
    pub occupancy_builds: usize,
    pub occupancy_inputs: usize,
    pub occupancy_records: usize,
    pub active_builds: usize,
    pub live_visits: usize,
    pub active_writes: usize,
}

thread_local! { static COUNTS: Cell<Counts> = Cell::new(Counts::default()); }

pub(crate) fn note(update: impl FnOnce(&mut Counts)) {
    COUNTS.with(|cell| {
        let mut counts = cell.get();
        update(&mut counts);
        cell.set(counts);
    });
}

fn measure(
    case: fixture::Case,
    revision: &std::sync::Arc<laneflow_static_network::SharedNetworkRevision>,
) {
    let mut f = fixture::fixture(revision, case);
    COUNTS.set(Counts::default());
    let mut expected = Counts::default();
    let mut successes = 0;
    let mut dirty = true;
    for i in case.indices() {
        expected.calls += 1;
        if dirty {
            expected.occupancy_builds += 1;
            expected.occupancy_inputs += case.active + successes;
            expected.occupancy_records += case.active + successes;
        }
        expected.followers += successes
            + if case.succeeds(i) {
                case.active
            } else {
                case.active - fixture::EXITS + i + 1
            };
        let result = f.world.leave_parking(f.vehicles[i], f.targets[i]);
        assert_eq!(result.is_ok(), case.succeeds(i));
        if !case.succeeds(i) {
            assert_eq!(
                result,
                Err(crate::ParkingError::LeaveUnsafeFollower {
                    follower: f.followers[i]
                })
            );
        }
        dirty = result.is_ok();
        if dirty {
            successes += 1;
            expected.active_builds += 1;
            expected.live_visits += case.active + case.parked;
            expected.active_writes += case.active + successes;
        }
        let active: Vec<_> = f
            .world
            .live_vehicles()
            .iter()
            .copied()
            .filter(|h| f.world.vehicle(*h).unwrap().status() == crate::VehicleStatus::Active)
            .collect();
        assert_eq!(f.world.derived.active_order, active);
    }
    let actual = COUNTS.get();
    assert_eq!(actual, expected);
    let memory = f.world.retained_memory();
    let digest = crate::deterministic_state_digest(&f.world.capture_snapshot().unwrap()).unwrap();
    println!(
        "parking-work case={case:?} counts={actual:?} digest={digest:x} world_bytes={} occupancy_bytes={} active_bytes={}",
        memory.world_owned_bytes(),
        f.world.derived.occupancy.retained_logical_bytes(),
        crate::kernel::state::vec_bytes(&f.world.derived.active_order)
    );
}

#[test]
fn parking_command_counts_match_committed_prefix() {
    let revision = fixture::revision();
    for success_percent in [0, 50, 100] {
        for order in 0..3 {
            measure(
                fixture::Case {
                    active: 128,
                    parked: 64,
                    commands: 16,
                    success_percent,
                    order,
                },
                &revision,
            );
        }
    }
}

#[test]
#[ignore = "manual command work and retained memory matrix, not latency evidence"]
fn parking_command_work_matrix() {
    let revision = fixture::revision();
    for case in fixture::cases() {
        measure(case, &revision);
    }
}
