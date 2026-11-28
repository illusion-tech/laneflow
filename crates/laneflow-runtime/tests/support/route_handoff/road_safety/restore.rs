use super::{Scene, TICK, deterministic_state_digest};
use crate::{snapshot_wire as wire, wire_table_field_offset};
use laneflow_runtime::{
    LeaveParkingTarget, ParkedVehicleSpawnInput, ParkingTarget, SnapshotRestoreError,
    SnapshotRestoreLimits, SpawnError, VehicleSpawnInput, encode_lfrs, restore_lfrs,
};
use std::sync::OnceLock;

#[test]
fn restore_checks_the_body_on_both_lanes_after_the_front_changes_occurrence() {
    let revision = OnceLock::new();
    for changer_first in [true, false] {
        let mut scene = Scene::with_cuts(1, [40, 94, 40], &revision, 100);
        let (change, far) = if changer_first {
            let change = scene.changer(0, "car", 83_750, 10_000);
            (change, scene.straight(0, 346_800, 0))
        } else {
            let far = scene.straight(0, 346_800, 0);
            (scene.changer(0, "car", 83_750, 10_000), far)
        };
        let middle = scene.straight(0, 220_000, 1_000);
        let request = scene.request(&change);
        scene
            .world
            .step_with_route_handoffs(TICK, &[request])
            .unwrap();
        for _ in 0..30 {
            if scene
                .world
                .vehicle(change.vehicle)
                .unwrap()
                .route_edge_index()
                == 2
            {
                break;
            }
            scene.world.step(TICK).unwrap();
        }
        let changing = scene.world.vehicle(change.vehicle).unwrap();
        assert_eq!(changing.route_edge_index(), 2);
        assert!(changing.progress_mm() < changing.length_mm());
        let geometry = scene
            .world
            .committed_handoff_geometry(change.vehicle)
            .unwrap()
            .unwrap();
        let source = geometry.footprint.sides[0].as_ref().unwrap();
        assert!(
            source
                .enclosure
                .addresses()
                .all(|a| a.progress_mm[1] < 65_000)
        );
        // 目标侧局部进度尚不足一车长。从普通路线倒推会误占源车道 350 米末端。
        assert!(scene.front(far) > 350_000 - (changing.length_mm() - changing.progress_mm()));
        let bytes = encode_lfrs(&scene.world.capture_snapshot().unwrap());
        let restore = |bytes: &[u8]| {
            restore_lfrs(
                bytes,
                scene.world.revision(),
                scene.world.committed_source().clone(),
                scene.world.config(),
                scene.world.execution_config(),
                SnapshotRestoreLimits::new(4 << 20, 4096),
            )
        };
        let restored = restore(&bytes).unwrap();
        // 将直行车辆改到源侧真实车身中，确认重叠仍会拒绝。
        let root = wire::size_prefixed_root_as_runtime_snapshot(&bytes).unwrap();
        let far_progress = scene.world.vehicle(far).unwrap().progress_mm();
        let other = root
            .vehicles()
            .iter()
            .find(|v| v.progress_mm() == far_progress)
            .unwrap();
        let offset = wire_table_field_offset(other._tab, wire::SnapshotVehicle::VT_PROGRESS_MM);
        let occupied = source
            .enclosure
            .addresses()
            .find(|a| a.edge == scene.lanes[0][1])
            .unwrap();
        let mut overlapping = bytes.clone();
        overlapping[offset..offset + 4].copy_from_slice(&occupied.progress_mm[1].to_le_bytes());
        assert!(matches!(
            restore(&overlapping),
            Err(SnapshotRestoreError::InvalidHandoffState
                | SnapshotRestoreError::Vehicle {
                    error: SpawnError::Overlap,
                    ..
                })
        ));
        // 同一份快照内的两辆普通车也须彼此无重叠。
        let middle_progress = scene.world.vehicle(middle).unwrap().progress_mm();
        overlapping[offset..offset + 4].copy_from_slice(&middle_progress.to_le_bytes());
        assert!(matches!(
            restore(&overlapping),
            Err(SnapshotRestoreError::Vehicle {
                error: SpawnError::Overlap,
                ..
            })
        ));
        let resumed_far = restored
            .vehicle_handle(other.snapshot_vehicle_id())
            .unwrap();
        let mut resumed = restored.into_world();
        let target_lane = scene.lanes[1].clone();
        let space = scene.space_at(0, 348_500);
        for (world, far) in [(&mut scene.world, far), (&mut resumed, resumed_far)] {
            let far_state = world.vehicle(far).unwrap();
            world.despawn_vehicle(far).unwrap();
            let spawn =
                |at| VehicleSpawnInput::new(far_state.profile(), far_state.route(), 1, at, 0);
            assert_eq!(
                world.spawn_vehicle(spawn(occupied.progress_mm[1])),
                Err(SpawnError::Overlap)
            );
            let target_route = world
                .register_route(laneflow_runtime::RouteRegisterInput::new(
                    target_lane.clone(),
                ))
                .unwrap();
            // 目标侧 112 米在实际车身之前，仍处于未释放的目标保护内。
            let reserved = VehicleSpawnInput::new(far_state.profile(), target_route, 1, 18_000, 0);
            assert_eq!(
                world.spawn_vehicle(reserved),
                Err(SpawnError::StopConstraintUnsatisfiable)
            );
            let spawned = world
                .spawn_vehicle(spawn(far_progress))
                .expect("源侧远端已腾空");
            world.despawn_vehicle(spawned).unwrap();
            let parked = world
                .spawn_parked_vehicle(
                    ParkedVehicleSpawnInput::new(
                        far_state.profile(),
                        far_state.route(),
                        1,
                        far_progress,
                    ),
                    ParkingTarget::ExplicitSpace(space),
                )
                .unwrap();
            world
                .leave_parking(
                    parked.vehicle,
                    LeaveParkingTarget::ExplicitSpace {
                        space,
                        route: far_state.route(),
                        exit_route_occurrence: 1,
                    },
                )
                .expect("源侧 348.5 米的空位可以驶出");
        }
        resumed.step(TICK).unwrap();
        scene.world.step(TICK).unwrap();
        assert_eq!(
            deterministic_state_digest(&resumed.capture_snapshot().unwrap()).unwrap(),
            deterministic_state_digest(&scene.world.capture_snapshot().unwrap()).unwrap()
        );
    }
}
