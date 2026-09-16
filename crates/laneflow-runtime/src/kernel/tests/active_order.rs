use super::*;
use crate::kernel::parking_command_research::fixture::{self, Case};
use crate::{ParkedVehicleSpawnInput, ParkingTarget, TickInput, VehicleSpawnInput, VehicleStatus};
use laneflow_static_contract::{ParkingFacilityOrdinal, VehicleProfileOrdinal};

fn case(success_percent: usize) -> Case {
    Case {
        active: 128,
        parked: 64,
        commands: 64,
        success_percent,
        order: 0,
    }
}

fn projection(world: &TrafficWorld) {
    let expected: Vec<_> = world
        .live_vehicles()
        .iter()
        .copied()
        .filter(|h| world.vehicle(*h).unwrap().status() == VehicleStatus::Active)
        .collect();
    assert_eq!(world.derived.active_order, expected);
}

fn published(world: &TrafficWorld) -> String {
    format!(
        "{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{}|{}",
        world.latest_waiting_decisions(),
        world.latest_conflict_decisions(),
        world.latest_transition_events(),
        world.committed_signal_groups(),
        world.waiting_zone_members(),
        world.observation_state_sequence(),
        world.command_cursor(),
        world.event_cursor()
    )
}

fn leave(f: &mut fixture::Fixture, index: usize) {
    f.world
        .leave_parking(f.vehicles[index], f.targets[index])
        .unwrap();
    projection(&f.world);
}

#[test]
fn live_prefix_extends_and_reused_slot_keeps_append_order() {
    let mut f = fixture::fixture_with_capacity(&fixture::revision(), case(100), 1_024);
    leave(&mut f, 31);
    let prefix = f.world.derived.live_order_index.indexed_len;
    let route = f.world.vehicle_state(f.vehicles[20]).unwrap().route;
    let extra = f
        .world
        .spawn_parked_vehicle(
            ParkedVehicleSpawnInput::new(VehicleProfileOrdinal::from_raw(0), route, 0, 0),
            ParkingTarget::VirtualPool(ParkingFacilityOrdinal::from_raw(0)),
        )
        .unwrap()
        .vehicle;
    assert_eq!(f.world.derived.live_order_index.indexed_len, prefix);
    assert_eq!(f.world.committed.live_order.len(), prefix + 1);
    leave(&mut f, 30);
    assert_eq!(f.world.derived.live_order_index.indexed_len, prefix + 1);
    f.world.despawn_vehicle(f.vehicles[20]).unwrap();
    assert_eq!(f.world.derived.live_order_index.indexed_len, 0);
    let reused = f
        .world
        .spawn_parked_vehicle(
            ParkedVehicleSpawnInput::new(VehicleProfileOrdinal::from_raw(0), route, 0, 0),
            ParkingTarget::VirtualPool(ParkingFacilityOrdinal::from_raw(0)),
        )
        .unwrap()
        .vehicle;
    assert_eq!(reused.index(), f.vehicles[20].index());
    assert_ne!(reused, f.vehicles[20]);
    f.world.leave_parking(reused, f.targets[20]).unwrap();
    assert_eq!(f.world.derived.active_order.last(), Some(&reused));
    projection(&f.world);
    assert_eq!(
        f.world
            .derived
            .live_order_index
            .position(f.vehicles[20], &f.world.committed.live_order),
        None
    );
    assert!(
        f.world
            .derived
            .live_order_index
            .position(extra, &f.world.committed.live_order)
            .is_some()
    );
    let snapshot = f.world.capture_snapshot().unwrap();
    assert_eq!(
        f.world.leave_parking(f.vehicles[20], f.targets[20]),
        Err(crate::ParkingError::StaleVehicle)
    );
    assert_eq!(f.world.capture_snapshot().unwrap(), snapshot);
}

#[test]
fn optional_allocation_failure_matches_success_and_retry() {
    let revision = fixture::revision();
    let mut normal = fixture::fixture_with_capacity(&revision, case(100), 1_024);
    let mut fallback = fixture::fixture_with_capacity(&revision, case(100), 1_024);
    with_position_allocation_failure(|| leave(&mut fallback, 31));
    leave(&mut normal, 31);
    assert_eq!(fallback.world.derived.live_order_index.indexed_len, 0);
    assert_eq!(published(&fallback.world), published(&normal.world));
    assert_eq!(
        fallback.world.capture_snapshot().unwrap(),
        normal.world.capture_snapshot().unwrap()
    );
    leave(&mut fallback, 30);
    leave(&mut normal, 30);
    assert_eq!(
        fallback.world.capture_snapshot().unwrap(),
        normal.world.capture_snapshot().unwrap()
    );
    // 已有空间的热缓存不调用分配器，故 failpoint 不触发降级。
    with_position_allocation_failure(|| leave(&mut fallback, 29));
    leave(&mut normal, 29);
    assert_eq!(
        fallback.world.derived.live_order_index.indexed_len,
        fallback.world.live_vehicles().len()
    );
    assert_eq!(
        fallback.world.capture_snapshot().unwrap(),
        normal.world.capture_snapshot().unwrap()
    );
    // append 使 slot 表需要扩容；失败后的全量投影仍保留相同提交结果。
    for f in [&mut normal, &mut fallback] {
        let route = f.world.vehicle_state(f.vehicles[28]).unwrap().route;
        f.world
            .spawn_parked_vehicle(
                ParkedVehicleSpawnInput::new(VehicleProfileOrdinal::from_raw(0), route, 0, 0),
                ParkingTarget::VirtualPool(ParkingFacilityOrdinal::from_raw(0)),
            )
            .unwrap();
    }
    with_position_allocation_failure(|| leave(&mut fallback, 28));
    leave(&mut normal, 28);
    assert_eq!(fallback.world.derived.live_order_index.indexed_len, 0);
    assert_eq!(
        fallback.world.capture_snapshot().unwrap(),
        normal.world.capture_snapshot().unwrap()
    );
    leave(&mut fallback, 27);
    leave(&mut normal, 27);
    assert_eq!(
        fallback.world.capture_snapshot().unwrap(),
        normal.world.capture_snapshot().unwrap()
    );
}

#[test]
fn rejected_command_preserves_error_snapshot_and_cold_cache() {
    let mut f = fixture::fixture(&fixture::revision(), case(0));
    let before = f.world.capture_snapshot().unwrap();
    let batches = published(&f.world);
    let error =
        with_position_allocation_failure(|| f.world.leave_parking(f.vehicles[0], f.targets[0]));
    assert_eq!(
        error,
        Err(crate::ParkingError::LeaveUnsafeFollower {
            follower: f.followers[0]
        })
    );
    assert_eq!(f.world.capture_snapshot().unwrap(), before);
    assert_eq!(published(&f.world), batches);
    assert_eq!(f.world.derived.live_order_index.positions.capacity(), 0);
    f.world.despawn_vehicle(f.followers[0]).unwrap();
    let cursor = f.world.committed.command_cursor;
    f.world.committed.command_cursor = u64::MAX;
    let exhausted = f.world.capture_snapshot().unwrap();
    let batches = published(&f.world);
    assert_eq!(
        with_position_allocation_failure(|| f.world.leave_parking(f.vehicles[0], f.targets[0])),
        Err(crate::ParkingError::CommandCursorExhausted)
    );
    assert_eq!(f.world.capture_snapshot().unwrap(), exhausted);
    assert_eq!(published(&f.world), batches);
    assert_eq!(f.world.derived.live_order_index.positions.capacity(), 0);
    f.world.committed.command_cursor = cursor;
    leave(&mut f, 0);
    let before = f.world.capture_snapshot().unwrap();
    assert!(f.world.step(TickInput::new(1)).is_err());
    assert_eq!(f.world.capture_snapshot().unwrap(), before);
    projection(&f.world);
    let batches = published(&f.world);
    crate::kernel::tick::STEP_FAILPOINT
        .with(|slot| slot.set(Some(crate::kernel::tick::StepFailpoint::AfterTransitions)));
    assert_eq!(
        f.world.step(TickInput::new(100)),
        Err(crate::StepError::ParkingObservationAllocFailed)
    );
    assert_eq!(f.world.capture_snapshot().unwrap(), before);
    assert_eq!(published(&f.world), batches);
    projection(&f.world);
    f.world.step(TickInput::new(100)).unwrap();
    projection(&f.world);
}

#[test]
fn completed_replacement_invalidates_and_parking_removes_stably() {
    let mut f = fixture::fixture_with_capacity(&fixture::revision(), case(100), 1_024);
    let route = f.world.vehicle_state(f.vehicles[0]).unwrap().route;
    let completing = f
        .world
        .spawn_vehicle(VehicleSpawnInput::new(
            VehicleProfileOrdinal::from_raw(0),
            route,
            0,
            99_999,
            1_000,
        ))
        .unwrap();
    leave(&mut f, 31);
    let prefix = f.world.derived.live_order_index.indexed_len;
    for _ in 0..100 {
        if f.world.vehicle(completing).unwrap().status() == VehicleStatus::Completed {
            break;
        }
        f.world.step(TickInput::new(100)).unwrap();
    }
    assert_eq!(
        f.world.vehicle(completing).unwrap().status(),
        VehicleStatus::Completed
    );
    assert_eq!(f.world.derived.live_order_index.indexed_len, prefix);
    projection(&f.world);
    let replacement = f
        .world
        .replace_completed_vehicle(
            completing,
            VehicleSpawnInput::new(VehicleProfileOrdinal::from_raw(0), route, 0, 0, 0),
        )
        .unwrap();
    assert_eq!(f.world.derived.live_order_index.indexed_len, 0);
    assert_eq!(f.world.derived.active_order.last(), Some(&replacement.new));
    leave(&mut f, 30);
    // background 首车前进后入口释放；停车删除 Active 成员不改变 live 顺序。
    let background = f
        .world
        .vehicle_state(f.world.live_vehicles()[64])
        .unwrap()
        .route;
    let entering = f
        .world
        .spawn_vehicle(VehicleSpawnInput::new(
            VehicleProfileOrdinal::from_raw(0),
            background,
            0,
            0,
            0,
        ))
        .unwrap();
    let facility = ParkingFacilityOrdinal::from_raw(0);
    f.world
        .reserve_parking(
            entering,
            crate::ReserveParkingTarget::VirtualPool {
                facility,
                entry_anchor: crate::VirtualEntryAnchorSelector::from_raw(0),
                entry_route_occurrence: 0,
            },
        )
        .unwrap();
    // 使用正常 step 抵达入口；不修改已提交状态来制造停车条件。
    for _ in 0..200 {
        if f.world.vehicle_state(entering).unwrap().progress_mm == 10_000 {
            break;
        }
        f.world.step(TickInput::new(100)).unwrap();
    }
    f.world
        .park_vehicle(entering, ParkingTarget::VirtualPool(facility))
        .unwrap();
    projection(&f.world);
    f.world.despawn_vehicle(f.followers[0]).unwrap();
    f.world.despawn_vehicle(replacement.new).unwrap();
    f.world.leave_parking(entering, f.targets[0]).unwrap();
    assert_eq!(f.world.derived.active_order.last(), Some(&entering));
    projection(&f.world);
}

#[test]
fn position_memory_follows_actual_slots_and_is_counted_once() {
    let revision = fixture::revision();
    for high_water in [false, true] {
        let mut f = fixture::fixture_with_capacity(
            &revision,
            Case {
                parked: if high_water { 4_096 } else { 64 },
                ..case(100)
            },
            65_536,
        );
        if high_water {
            let removed = f.world.live_vehicles()[64..4_096].to_vec();
            for h in removed {
                f.world.despawn_vehicle(h).unwrap();
            }
        }
        let before = f.world.derived.retained_logical_bytes();
        let snapshot = f.world.capture_snapshot().unwrap();
        assert!(f.world.prepare_active_insertion(f.vehicles[0]).is_some());
        let cache = &f.world.derived.live_order_index;
        assert_eq!(cache.positions.len(), f.world.committed.vehicles.len());
        assert_eq!(
            cache.positions.capacity(),
            if high_water { 4_224 } else { 192 }
        );
        assert_eq!(
            f.world.derived.retained_logical_bytes() - before,
            cache.retained_logical_bytes()
        );
        assert_eq!(f.world.capture_snapshot().unwrap(), snapshot);
        println!(
            "active-memory high_water={high_water} slots={} live={} heap_bytes={} inline_bytes={}",
            cache.positions.len(),
            cache.indexed_len,
            cache.retained_logical_bytes(),
            core::mem::size_of::<LiveOrderIndex>()
        );
    }
}

#[test]
fn restore_starts_cold_and_same_revision_cutover_preserves_valid_prefix() {
    let revision = fixture::revision();
    let mut f = fixture::fixture(&revision, case(100));
    leave(&mut f, 31);
    let captured = f.world.capture_snapshot().unwrap();
    let restored = crate::restore_lfrs(
        &crate::encode_lfrs(&captured),
        revision.clone(),
        f.world.committed_source().clone(),
        f.world.config(),
        crate::SnapshotRestoreLimits::new(1_048_576, 1_024),
    )
    .unwrap();
    let mut world = restored.into_world();
    assert_eq!(
        crate::deterministic_state_digest(&world.capture_snapshot().unwrap()).unwrap(),
        crate::deterministic_state_digest(&captured).unwrap()
    );
    assert_eq!(world.derived.live_order_index.positions.capacity(), 0);
    projection(&world);
    let parked = world.live_vehicles()[30];
    assert!(world.prepare_active_insertion(parked).is_some());
    let descriptor = crate::NetworkRevisionCutoverDescriptor::new(
        crate::LfcaOriginBinding::from_canonical_origin(*world.revision().canonical_origin()),
        crate::LfcaOriginBinding::from_canonical_origin(*revision.canonical_origin()),
        None,
        crate::MigrationPolicyKind::SameRevisionRestore,
        world.world_binding(),
    );
    let _commit = world
        .cutover_same_revision(
            revision,
            world.committed_source().clone(),
            &descriptor,
            &crate::CutoverPreflightLimits::new(1_048_576),
        )
        .unwrap();
    assert_eq!(
        world.derived.live_order_index.indexed_len,
        world.live_vehicles().len()
    );
    projection(&world);
    // 原地同 revision 切换保留 live 句柄与顺序，位置表继续有效。
    let vehicle = world.live_vehicles()[30];
    let route = world.vehicle_state(vehicle).unwrap().route;
    world
        .leave_parking(
            vehicle,
            crate::LeaveParkingTarget::VirtualPool {
                facility: ParkingFacilityOrdinal::from_raw(0),
                route,
                exit_anchor: crate::VirtualExitAnchorSelector::from_raw(30),
                exit_route_occurrence: 0,
            },
        )
        .unwrap();
    projection(&world);
}
