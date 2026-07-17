#[path = "support/parking_runtime_scenarios.rs"]
#[allow(dead_code, reason = "shared helper also exposes benchmark scenarios")]
mod parking_scenarios;

use parking_scenarios::{
    parking_six_command_scenario, run_six_command_batch, warm_six_command_batch,
    warm_six_command_spawn_capacity,
};

#[test]
fn all_six_parking_command_kinds_replay_in_order_with_identical_committed_state() {
    let seed = parking_six_command_scenario(32, 4);
    let mut warmed = seed.clone();
    assert_eq!(warm_six_command_batch(&mut warmed), 4 * 4);
    assert_eq!(warmed.world, seed.world);
    assert_eq!(warm_six_command_spawn_capacity(&mut warmed), 4 * 2);
    assert_eq!(warmed.world.vehicles().count(), 32);
    assert_eq!(warmed.world.parking_snapshot().counts().vacant, 12);
    let mut actual = seed.clone();
    let mut replay = seed;

    assert_eq!(run_six_command_batch(&mut actual), 4 * 7);
    assert_eq!(run_six_command_batch(&mut replay), 4 * 7);
    assert_eq!(actual.world, replay.world);

    let counts = actual.world.parking_snapshot().counts();
    assert_eq!(counts.capacity, 12);
    assert_eq!(counts.vacant, 8);
    assert_eq!(counts.reserved, 0);
    assert_eq!(counts.occupied, 4);
}
