use super::*;
use laneflow_runtime::{
    SnapshotDigestLimits, SnapshotRestoreError, SnapshotRestoreLimits, TrafficWorld,
    deterministic_state_digest, encode_lfrs, restore_lfrs,
};
use std::num::NonZeroU32;

#[test]
fn committed_handoff_keeps_each_interval_and_continues_from_saved_travel() {
    use crate::{snapshot_wire as wire, wire_table_field_offset};
    use laneflow_runtime::TickInput;
    let (mut world, request, config) = execution::drive_fixture(2_000);
    let tick = TickInput::new(config.fixed_delta_time_ms());
    world.step_with_route_handoffs(tick, &[request]).unwrap();
    let running = world
        .vehicle_route_handoffs(request.vehicle)
        .next()
        .unwrap();
    assert!(running.rear_travel_um > 0);
    let bytes = encode_lfrs(&world.capture_snapshot().unwrap());
    let restore = |bytes: &[u8]| {
        restore_lfrs(
            bytes,
            world.revision(),
            world.committed_source().clone(),
            config,
            world.execution_config(),
            SnapshotRestoreLimits::new(4 << 20, 4096),
        )
    };
    let root = wire::size_prefixed_root_as_runtime_snapshot(&bytes).unwrap();
    let h = root.vehicles().get(0).handoffs().get(0);
    let motion = wire_table_field_offset(h._tab, wire::CommittedHandoff::VT_MOTION);
    let protections = wire_table_field_offset(h._tab, wire::CommittedHandoff::VT_PROTECTIONS);
    let protections = protections
        + u32::from_le_bytes(bytes[protections..protections + 4].try_into().unwrap()) as usize;
    let n = h.protections().len();
    assert!((1..=4).all(|role| h.protections().iter().any(|p| p.role().0 == role)));
    let width = size_of::<wire::HandoffProtection>();
    for removed in 0..n {
        let mut damaged = bytes.clone();
        let start = protections + 4 + removed * width;
        damaged.copy_within(start + width..protections + 4 + n * width, start);
        damaged[protections..protections + 4]
            .copy_from_slice(&u32::try_from(n - 1).unwrap().to_le_bytes());
        assert_eq!(
            restore(&damaged).unwrap_err(),
            SnapshotRestoreError::InvalidHandoffState
        );
    }
    let window = root.routes().get(0).handoff_windows().get(0);
    assert_eq!([window.low_mm(), window.high_mm()], [24_000, 26_000]);
    let original = *h.motion();
    assert_eq!(
        [original.station_low_mm(), original.station_high_mm()],
        request.station_window_mm
    );
    assert_eq!(
        [original.rear_start_mm(), original.rear_end_mm()],
        request.rear_stations_mm
    );
    for change in 0..9 {
        let mut damaged = bytes.clone();
        let mut m = original;
        match change {
            0 => m.set_front_handed_off(1),
            1 => m.set_acquired_tick(root.tick() + 1),
            2 => m.set_admitted_speed_bits((-0.0_f64).to_bits()),
            3 => m.set_tracks(u32::MAX),
            4 => m.set_source_index(0),
            5 => m.set_rear_travel_um(u64::MAX),
            6 => m.set_station_high_mm(u64::MAX / 1_000_000 + 1),
            7 => m.set_rear_end_mm(original.station_high_mm() + 1),
            _ => m.set_rear_end_mm(original.rear_start_mm()),
        }
        damaged[motion..motion + m.0.len()].copy_from_slice(&m.0);
        assert_eq!(
            restore(&damaged).unwrap_err(),
            SnapshotRestoreError::InvalidHandoffState
        );
    }
    let restored = restore(&bytes).unwrap();
    let vehicle = restored.vehicle_handle(1).unwrap();
    let mut restored = restored.into_world();
    for n in 1..=5 {
        let captured = deterministic_state_digest(&world.capture_snapshot().unwrap()).unwrap();
        assert_eq!(
            captured,
            world
                .direct_state_digest(SnapshotDigestLimits::new(4 << 20))
                .unwrap()
        );
        assert_eq!(
            captured,
            restored
                .direct_state_digest(SnapshotDigestLimits::new(4 << 20))
                .unwrap()
        );
        assert_eq!(
            world.vehicle_route_handoffs(request.vehicle).next(),
            restored.vehicle_route_handoffs(vehicle).next()
        );
        if n < 5 {
            world.step(tick).unwrap();
            restored.step(tick).unwrap();
        }
    }
}

#[test]
fn empty_world_policy_and_unoccupied_windows_are_distinct_and_round_trip() {
    let revision = compile_road_editing_revision(module(0.));
    let (section, src, dst) = stop_line::road_lanes(&revision);
    let config = WorldConfig::try_new(WorldConfigInput {
        vehicle_capacity: 2,
        route_capacity: 3,
        route_edge_occurrence_capacity: 16,
        route_conflict_occurrence_capacity: 32,
        fixed_delta_time_ms: 100,
    })
    .unwrap();
    let policy = registration::driving();
    let mut changed = policy;
    changed.travel_tolerance_mm = NonZeroU32::new(policy.travel_tolerance_mm.get() + 1).unwrap();
    let enabled = config.with_handoff_driving(policy);
    let changed = config.with_handoff_driving(changed);
    let new_world = |c| install_fixture(Arc::clone(&revision), c).unwrap();
    let digest = |w: &TrafficWorld| {
        let captured = deterministic_state_digest(&w.capture_snapshot().unwrap()).unwrap();
        assert_eq!(
            captured,
            w.direct_state_digest(SnapshotDigestLimits::new(4 << 20))
                .unwrap()
        );
        captured
    };
    let restore = |w: &TrafficWorld, c| {
        restore_lfrs(
            &encode_lfrs(&w.capture_snapshot().unwrap()),
            w.revision(),
            w.committed_source().clone(),
            c,
            w.execution_config(),
            SnapshotRestoreLimits::new(4 << 20, 4096),
        )
    };
    let off = new_world(config);
    let mut a = new_world(enabled);
    let mut b = new_world(enabled);
    assert_ne!(digest(&a), digest(&off));
    assert_ne!(digest(&a), digest(&new_world(changed)));
    for c in [config, changed] {
        assert_eq!(
            restore(&a, c).unwrap_err(),
            SnapshotRestoreError::InvalidHandoffPolicy
        );
    }
    assert_eq!(
        digest(&a),
        digest(&restore(&a, enabled).unwrap().into_world())
    );
    let register = |w: &mut TrafficWorld, high| {
        w.register_route(
            RouteRegisterInput::new(vec![src[2], dst[1]]).with_handoffs(vec![
                RouteHandoffOccurrence {
                    revision: revision.network_revision(),
                    source_index: 0,
                    owner: BoundaryOwner::Section(section),
                    left_index: 0,
                    stations_mm: [24_000, high],
                },
            ]),
        )
        .unwrap();
    };
    register(&mut a, 26_000);
    register(&mut b, 25_999);
    assert_ne!(digest(&a), digest(&b));
    for w in [a, b] {
        let restored = restore(&w, enabled).unwrap();
        let handle = restored.route_handle(1).unwrap();
        let rebuilt = restored.into_world();
        assert_eq!(rebuilt.route_handoff_windows(handle).unwrap().len(), 1);
        assert_eq!(digest(&w), digest(&rebuilt));
    }
}
