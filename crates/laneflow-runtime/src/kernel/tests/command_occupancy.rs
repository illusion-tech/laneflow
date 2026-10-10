//! 命令在占用索引重建点让出、借执行资源并行重建（`traffic-runtime-vehicle-placement.md`
//! 第 8 节）：多 worker 世界与单 worker 世界逐次得到相同的命令结果与索引。

use std::num::NonZeroU32;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;

use crate::kernel::occupancy::{
    occupancy_full_fingerprint, set_parallel_occupancy_fault, take_parallel_occupancy_rebuilds,
};
use crate::kernel::world::take_command_occupancy_yields;
use crate::{TickInput, TrafficWorld, VehicleHandle, VehicleSpawnInput};

fn with_workers(world: &mut TrafficWorld, workers: u32) {
    world.execution = crate::kernel::execution::WorldExecution::start_private(
        crate::ExecutionConfig::new(NonZeroU32::new(workers).expect("nonzero workers")),
        &world.state,
    );
}

fn step(world: &mut TrafficWorld) {
    let input = TickInput::new(world.config().fixed_delta_time_ms());
    world.step(input).expect("step");
}

fn fingerprint(world: &TrafficWorld) -> String {
    occupancy_full_fingerprint(&world.state.derived.occupancy)
}

fn digest(world: &TrafficWorld) -> String {
    format!(
        "{:x}",
        crate::deterministic_state_digest(&world.capture_snapshot().expect("snapshot"))
            .expect("digest")
    )
}

/// 同一规模世界的单 worker 与 16 worker 两份，各腾出 3 个槽位供生成。
fn scale_pair() -> (TrafficWorld, TrafficWorld) {
    let revision = crate::admin::cutover_migration::tests::conflict_scale_revision();
    let mut serial =
        crate::admin::cutover_migration::tests::conflict_scale_world(Arc::clone(&revision), 16);
    let mut parallel = crate::admin::cutover_migration::tests::conflict_scale_world(revision, 16);
    with_workers(&mut parallel, 16);
    for world in [&mut serial, &mut parallel] {
        let spares: Vec<_> = world
            .live_vehicles()
            .iter()
            .rev()
            .take(3)
            .copied()
            .collect();
        for spare in spares {
            world.despawn_vehicle(spare).expect("free a slot");
        }
    }
    (serial, parallel)
}

fn spawn_input(world: &TrafficWorld, batch: u32, command: u32) -> VehicleSpawnInput {
    let first = world
        .vehicle(world.live_vehicles()[0])
        .expect("first vehicle");
    VehicleSpawnInput::new(
        first.profile(),
        first.route(),
        (batch + command) % 3,
        ((batch * 7 + command) * 1_900) % 6_000,
        2_000 + command * 2_500,
    )
    .with_open_entrance()
}

/// 步进之后的每批生成：多 worker 世界让出并并行重建，结果、索引与单 worker 世界逐次
/// 相同；一批里最多让出一次。
#[test]
fn parallel_command_rebuild_matches_serial_world() {
    let _lock = crate::kernel::execution::RESOURCE_TEST_LOCK.lock().unwrap();
    let (mut serial, mut parallel) = scale_pair();
    let mut outcomes = std::collections::BTreeSet::new();
    let mut yields = 0;
    for batch in 0..12u32 {
        step(&mut serial);
        step(&mut parallel);
        take_command_occupancy_yields();
        take_parallel_occupancy_rebuilds();
        let mut added: Vec<(VehicleHandle, VehicleHandle)> = Vec::new();
        for command in 0..4u32 {
            let input = spawn_input(&serial, batch, command);
            let expected = serial.spawn_vehicle(input);
            let got = parallel.spawn_vehicle(input);
            assert_eq!(got, expected, "batch {batch} command {command}");
            assert_eq!(
                fingerprint(&parallel),
                fingerprint(&serial),
                "batch {batch}"
            );
            outcomes.insert(got.is_ok());
            if let (Ok(serial_handle), Ok(parallel_handle)) = (expected, got) {
                added.push((serial_handle, parallel_handle));
            }
        }
        let batch_yields = take_command_occupancy_yields();
        assert!(
            batch_yields <= 1,
            "batch {batch} yielded {batch_yields} times"
        );
        assert_eq!(take_parallel_occupancy_rebuilds(), batch_yields);
        yields += batch_yields;
        for (serial_handle, parallel_handle) in added {
            serial.despawn_vehicle(serial_handle).expect("despawn");
            parallel.despawn_vehicle(parallel_handle).expect("despawn");
        }
    }
    assert!(yields > 0, "the parallel world must yield");
    assert_eq!(outcomes.len(), 2, "the fixture must both accept and reject");
    assert_eq!(digest(&parallel), digest(&serial));
}

/// 并行重建分配失败：命令退回原位置串行重建，结果不变，世界仍可用。
#[test]
fn failed_parallel_rebuild_falls_back_to_the_serial_rebuild() {
    let _lock = crate::kernel::execution::RESOURCE_TEST_LOCK.lock().unwrap();
    let (mut serial, mut parallel) = scale_pair();
    let mut yields = 0;
    for batch in 0..6u32 {
        step(&mut serial);
        step(&mut parallel);
        take_command_occupancy_yields();
        set_parallel_occupancy_fault(Some(false));
        let mut added = Vec::new();
        for command in 0..4u32 {
            let input = spawn_input(&serial, batch, command);
            let expected = serial.spawn_vehicle(input);
            let got = parallel.spawn_vehicle(input);
            assert_eq!(got, expected, "batch {batch} command {command}");
            assert_eq!(
                fingerprint(&parallel),
                fingerprint(&serial),
                "batch {batch}"
            );
            if let (Ok(serial_handle), Ok(parallel_handle)) = (expected, got) {
                added.push((serial_handle, parallel_handle));
            }
        }
        set_parallel_occupancy_fault(None);
        yields += take_command_occupancy_yields();
        for (serial_handle, parallel_handle) in added {
            serial.despawn_vehicle(serial_handle).expect("despawn");
            parallel.despawn_vehicle(parallel_handle).expect("despawn");
        }
    }
    assert!(yields > 0, "the parallel world must yield");
    assert_eq!(digest(&parallel), digest(&serial));
}

/// 并行重建里 worker panic：世界退出可用状态，之后的命令不再执行。
#[test]
fn panicking_parallel_rebuild_invalidates_the_world() {
    let _lock = crate::kernel::execution::RESOURCE_TEST_LOCK.lock().unwrap();
    let (_, mut parallel) = scale_pair();
    step(&mut parallel);
    let input = spawn_input(&parallel, 0, 0);
    set_parallel_occupancy_fault(Some(true));
    let panicked = catch_unwind(AssertUnwindSafe(|| parallel.spawn_vehicle(input)));
    set_parallel_occupancy_fault(None);
    assert!(panicked.is_err(), "the injected panic must propagate");
    let after = catch_unwind(AssertUnwindSafe(|| parallel.spawn_vehicle(input)));
    assert!(after.is_err(), "an invalidated world must refuse commands");
}

/// 驶离停车：成功离开后索引过期，下一次离开在重建点让出，结果与单 worker 世界相同。
#[test]
fn leave_parking_yields_and_matches_the_serial_world() {
    use crate::kernel::parking_command_research::fixture;
    let _lock = crate::kernel::execution::RESOURCE_TEST_LOCK.lock().unwrap();
    let revision = fixture::revision();
    let case = fixture::Case {
        active: 128,
        parked: 64,
        commands: 16,
        success_percent: 50,
        order: 0,
    };
    let mut serial = fixture::fixture(&revision, case);
    let mut parallel = fixture::fixture(&revision, case);
    with_workers(&mut parallel.world, 16);
    take_command_occupancy_yields();
    let mut outcomes = std::collections::BTreeSet::new();
    for index in case.indices() {
        let expected = serial
            .world
            .leave_parking(serial.vehicles[index], serial.targets[index]);
        let got = parallel
            .world
            .leave_parking(parallel.vehicles[index], parallel.targets[index]);
        assert_eq!(got, expected, "command {index}");
        assert_eq!(fingerprint(&parallel.world), fingerprint(&serial.world));
        outcomes.insert(got.is_ok());
    }
    assert!(take_command_occupancy_yields() > 0, "leaving must yield");
    assert_eq!(outcomes.len(), 2, "the fixture must both accept and reject");
    assert_eq!(digest(&parallel.world), digest(&serial.world));
}
