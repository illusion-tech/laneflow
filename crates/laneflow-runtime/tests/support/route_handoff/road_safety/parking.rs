use super::{Scene, TICK};
use laneflow_runtime::{
    LeaveParkingTarget, ParkedVehicleSpawnInput, ParkingError, ParkingTarget, ReserveParkingTarget,
    RouteRegisterInput, encode_lfrs,
};

#[test]
fn reserved_source_entry_keeps_priority_over_handoff() {
    let mut scene = Scene::new(1);
    let change = scene.changer(0, "car", 83_750, 10_000);
    let source = ReserveParkingTarget::ExplicitSpace {
        space: scene.space_at(0, 348_500),
        entry_route_occurrence: 1,
    };
    scene.world.reserve_parking(change.vehicle, source).unwrap();
    let binding = scene.world.parking_binding(change.vehicle);
    let request = scene.request(&change);
    scene
        .world
        .step_with_route_handoffs(TICK, &[request])
        .unwrap();
    for _ in 0..20 {
        scene.world.step(TICK).unwrap();
    }
    assert!(!scene.held(change.vehicle));
    assert_eq!(scene.world.parking_binding(change.vehicle), binding);
    assert_eq!(
        scene
            .world
            .vehicle(change.vehicle)
            .unwrap()
            .route_edge_index(),
        1
    );
}

#[test]
fn entered_handoff_refuses_later_reservation_on_abandoned_source() {
    let mut scene = Scene::new(1);
    let change = scene.changer(0, "car", 83_750, 10_000);
    let request = scene.request(&change);
    scene
        .world
        .step_with_route_handoffs(TICK, &[request])
        .unwrap();
    assert!(scene.held(change.vehicle));
    let source = ReserveParkingTarget::ExplicitSpace {
        space: scene.space_at(0, 348_500),
        entry_route_occurrence: 1,
    };
    let cursor = scene.world.command_cursor();
    assert_eq!(
        scene.world.reserve_parking(change.vehicle, source),
        Err(ParkingError::HandoffConflict)
    );
    assert_eq!(scene.world.command_cursor(), cursor);
    assert_eq!(scene.world.parking_binding(change.vehicle), None);
}

#[test]
fn parking_at_or_after_handoff_release_uses_the_committed_cursor_for_arrival() {
    for (occurrence, lane, station) in [
        (2, 1, 113_750),
        (2, 1, 113_800),
        (2, 1, 135_000),
        (3, 0, 348_500),
    ] {
        for reserve_first in [true, false] {
            let mut scene = Scene::new(1);
            let change = scene.changer_on(0, "car", 83_750, 10_000, occurrence == 3);
            let target = ReserveParkingTarget::ExplicitSpace {
                space: scene.space_at(lane, station),
                entry_route_occurrence: occurrence,
            };
            if reserve_first {
                scene.world.reserve_parking(change.vehicle, target).unwrap();
            }
            let request = scene.request(&change);
            scene
                .world
                .step_with_route_handoffs(TICK, &[request])
                .unwrap();
            assert!(scene.held(change.vehicle));
            if !reserve_first {
                scene.world.reserve_parking(change.vehicle, target).unwrap();
            }
            // 冷恢复须接受同一横移与停车预约的组合，下一拍起仍按实际游标停车。
            if occurrence == 3 && reserve_first {
                // 预约仍引用同一个物理泊位，但源出现 1 将被放弃。
                let mut bytes =
                    laneflow_runtime::encode_lfrs(&scene.world.capture_snapshot().unwrap());
                let root =
                    crate::snapshot_wire::size_prefixed_root_as_runtime_snapshot(&bytes).unwrap();
                let binding = root.vehicles().get(0).parking().unwrap();
                let offset = crate::wire_table_field_offset(
                    binding._tab,
                    crate::snapshot_wire::ParkingBinding::VT_ENTRY_ROUTE_OCCURRENCE,
                );
                bytes[offset..offset + 4].copy_from_slice(&1_u32.to_le_bytes());
                assert_eq!(
                    laneflow_runtime::restore_lfrs(
                        &bytes,
                        scene.world.revision(),
                        scene.world.committed_source().clone(),
                        scene.world.config(),
                        scene.world.execution_config(),
                        laneflow_runtime::SnapshotRestoreLimits::new(4 << 20, 4096)
                    )
                    .unwrap_err(),
                    laneflow_runtime::SnapshotRestoreError::InvalidHandoffState
                );
            }
            scene.world = super::restore(&scene, std::num::NonZeroU32::MIN);
            let mut cleared = false;
            let mut arrivals = 0;
            for _ in 0..700 {
                let step = scene.world.step(TICK).unwrap();
                if !scene.held(change.vehicle) {
                    cleared = true;
                }
                let vehicle = scene.world.vehicle(change.vehicle).unwrap();
                if !step.parking_arrivals().is_empty() {
                    assert_eq!(step.parking_arrivals().len(), 1);
                    assert_eq!(step.parking_arrivals()[0].vehicle, change.vehicle);
                    assert!(cleared);
                    assert_eq!(
                        (
                            vehicle.route_edge_index(),
                            vehicle.progress_mm(),
                            vehicle.speed_mm_s()
                        ),
                        (occurrence, station - 40_000, 0)
                    );
                    arrivals += 1;
                    break;
                }
                if occurrence == 3 && cleared {
                    break;
                }
            }
            assert!(cleared);
            assert_eq!(arrivals, usize::from(occurrence == 2), "停车入口 {station}");
            assert!(scene.world.parking_binding(change.vehicle).is_some());
            assert!(
                scene
                    .world
                    .step(TICK)
                    .unwrap()
                    .parking_arrivals()
                    .is_empty()
            );
        }
    }
}

#[test]
fn target_parking_before_handoff_release_is_rejected_in_both_orders() {
    for reserve_first in [true, false] {
        let mut scene = Scene::new(1);
        let change = scene.changer(0, "car", 83_750, 10_000);
        let target = ReserveParkingTarget::ExplicitSpace {
            space: scene.space_at(1, 100_000),
            entry_route_occurrence: 2,
        };
        if reserve_first {
            scene.world.reserve_parking(change.vehicle, target).unwrap();
        }
        let request = scene.request(&change);
        scene
            .world
            .step_with_route_handoffs(TICK, &[request])
            .unwrap();
        assert_eq!(scene.held(change.vehicle), !reserve_first);
        if !reserve_first {
            assert_eq!(
                scene.world.reserve_parking(change.vehicle, target),
                Err(ParkingError::HandoffConflict)
            );
        }
    }
}

#[test]
fn terminal_tick_with_a_near_parking_entry_stops_once_at_that_entry() {
    let mut scene = Scene::at_interval(1, 1000);
    let tick = laneflow_runtime::TickInput::new(1000);
    let change = scene.changer(0, "car", 83_750, 10_000);
    let request = scene.request(&change);
    scene
        .world
        .step_with_route_handoffs(tick, &[request])
        .unwrap();
    scene.world.step(tick).unwrap();
    scene.world.step(tick).unwrap();
    assert!(scene.held(change.vehicle));
    let target = ReserveParkingTarget::ExplicitSpace {
        space: scene.space_at(1, 113_800),
        entry_route_occurrence: 2,
    };
    scene.world.reserve_parking(change.vehicle, target).unwrap();
    let released = scene.world.step(tick).unwrap();
    let v = scene.world.vehicle(change.vehicle).unwrap();
    assert!(
        !scene.held(change.vehicle),
        "前杠 {} mm, 速度 {} mm/s",
        scene.front(change.vehicle),
        v.speed_mm_s()
    );
    assert_eq!((scene.front(change.vehicle), v.speed_mm_s()), (113_800, 0));
    assert_eq!(released.parking_arrivals().len(), 1);
    assert_eq!(released.parking_arrivals()[0].vehicle, change.vehicle);
    assert!(
        scene
            .world
            .step(tick)
            .unwrap()
            .parking_arrivals()
            .is_empty()
    );
}

#[test]
fn parking_departure_leaves_the_minimum_stopping_gap_before_a_handoff() {
    for (exit_mm, gap_mm, profile_key, can_leave) in [
        (79_039, 1_000, "car", false),
        (77_540, 2_499, "car", false),
        (77_539, 2_500, "car", true),
        (80_039, 0, "zero-gap", false),
        (80_038, 1, "zero-gap", true),
    ] {
        let mut scene = Scene::new(1);
        let change = scene.changer(0, "car", 83_750, 10_000);
        let request = scene.request(&change);
        scene
            .world
            .step_with_route_handoffs(TICK, &[request])
            .unwrap();
        let bytes = encode_lfrs(&scene.world.capture_snapshot().unwrap());
        let root = crate::snapshot_wire::size_prefixed_root_as_runtime_snapshot(&bytes).unwrap();
        let handoff = root.vehicles().get(0).handoffs().get(0);
        let source = handoff
            .protections()
            .iter()
            .find(|p| p.role() == crate::snapshot_wire::HandoffProtectionRole::Source)
            .unwrap();
        assert_eq!(super::CUT + source.start_mm() - exit_mm, gap_mm);
        let route = scene
            .world
            .register_route(RouteRegisterInput::new(scene.lanes[0].clone()))
            .unwrap();
        let profile = crate::conflict_profile(&scene.world.revision(), profile_key);
        let space = scene.space_at(0, exit_mm);
        let parked = scene
            .world
            .spawn_parked_vehicle(
                ParkedVehicleSpawnInput::new(profile, route, 1, exit_mm - super::CUT),
                ParkingTarget::ExplicitSpace(space),
            )
            .unwrap()
            .vehicle;
        let before = encode_lfrs(&scene.world.capture_snapshot().unwrap());
        let leave = scene.world.leave_parking(
            parked,
            LeaveParkingTarget::ExplicitSpace {
                space,
                route,
                exit_route_occurrence: 1,
            },
        );
        if !can_leave {
            assert_eq!(leave, Err(ParkingError::HandoffConflict));
            assert_eq!(
                before,
                encode_lfrs(&scene.world.capture_snapshot().unwrap())
            );
        } else {
            leave.unwrap();
            assert_eq!(scene.front(parked), exit_mm);
        }
        for _ in 0..60 {
            scene.world.step(TICK).unwrap();
            if !scene.held(change.vehicle) {
                break;
            }
        }
        assert!(!scene.held(change.vehicle));
    }
}

#[test]
fn a_parked_vehicle_can_leave_with_its_tail_at_the_end_of_half_open_downstream_protection() {
    let mut scene = Scene::new(1);
    let change = scene.changer(0, "car", 83_750, 10_000);
    let request = scene.request(&change);
    scene
        .world
        .step_with_route_handoffs(TICK, &[request])
        .unwrap();
    let bytes = encode_lfrs(&scene.world.capture_snapshot().unwrap());
    let root = crate::snapshot_wire::size_prefixed_root_as_runtime_snapshot(&bytes).unwrap();
    let held = root.vehicles().get(0).handoffs().get(0);
    let downstream = held
        .protections()
        .iter()
        .find(|p| p.role() == crate::snapshot_wire::HandoffProtectionRole::Downstream)
        .unwrap();
    let car = scene.world.vehicle(change.vehicle).unwrap();
    let first_clear = super::CUT + downstream.end_mm() + car.length_mm();
    let route = scene
        .world
        .register_route(RouteRegisterInput::new(scene.lanes[1].clone()))
        .unwrap();
    for exit in [first_clear - 1, first_clear] {
        let space = scene.space_at(1, exit);
        let parked = scene
            .world
            .spawn_parked_vehicle(
                ParkedVehicleSpawnInput::new(car.profile(), route, 1, exit - super::CUT),
                ParkingTarget::ExplicitSpace(space),
            )
            .unwrap()
            .vehicle;
        let leave = scene.world.leave_parking(
            parked,
            LeaveParkingTarget::ExplicitSpace {
                space,
                route,
                exit_route_occurrence: 1,
            },
        );
        if exit < first_clear {
            assert_eq!(leave, Err(ParkingError::HandoffConflict));
        } else {
            leave.unwrap();
            assert_eq!(
                scene.front(parked) - car.length_mm(),
                super::CUT + downstream.end_mm()
            );
        }
    }
    scene.world.step(TICK).unwrap();
}
