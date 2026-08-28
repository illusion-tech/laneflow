//! 与 `LFRS` 容器编码无关的 Runtime 逻辑状态摘要。

use std::collections::BTreeMap;

use laneflow_static_contract::Sha256Digest;
use sha2::{Digest, Sha256};

use crate::{CapturedRoute, CapturedSnapshot, CapturedVehicle, VehicleStatus};

/// 确定性状态摘要规范化版本。
pub const RUNTIME_STATE_DIGEST_VERSION: u16 = 1;
/// SHA-256 域分隔前缀；尾随 NUL 属于前缀字节。
pub const RUNTIME_STATE_DIGEST_DOMAIN: &[u8] = b"laneflow:runtime-state-digest:v1\0";

/// 对捕获的 Runtime 逻辑状态计算与 `LFRS` wire 编码、进程句柄和快照局部 ID 无关的
/// SHA-256 摘要。
///
/// 规范输入包括：摘要版本、世界身份、语义路网修订与六轴静态契约版本、行为语义
/// 配置（容量 + fixed dt）、tick/时间/双游标、路线内容多重集、车辆内容多重集与 live
/// 更新顺序。LFCA exact-byte digest/length、Published asset 审计来源、worker 计划、
/// `WorldGeneration` 与观测 session 不进入逻辑摘要。
#[must_use]
pub fn deterministic_state_digest(snapshot: &CapturedSnapshot) -> Sha256Digest {
    let route_by_id = snapshot
        .routes
        .iter()
        .map(|route| (route.snapshot_route_id, route))
        .collect::<BTreeMap<_, _>>();

    let mut route_records = snapshot
        .routes
        .iter()
        .map(canonical_route_record)
        .collect::<Vec<_>>();
    route_records.sort_unstable();

    let vehicle_by_id = snapshot
        .vehicles
        .iter()
        .map(|vehicle| {
            (
                vehicle.snapshot_vehicle_id,
                canonical_vehicle_record(vehicle, &route_by_id),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let mut vehicle_records = vehicle_by_id.values().cloned().collect::<Vec<_>>();
    vehicle_records.sort_unstable();

    let mut canonical = Vec::new();
    canonical.extend_from_slice(RUNTIME_STATE_DIGEST_DOMAIN);
    push_u16(&mut canonical, RUNTIME_STATE_DIGEST_VERSION);
    push_u16(&mut canonical, crate::RUNTIME_STATE_VERSION);
    push_u64(&mut canonical, snapshot.world_id);
    canonical.extend_from_slice(snapshot.origin.network_revision().as_digest().as_bytes());
    let contracts = snapshot.origin.static_contract_versions();
    push_u16(&mut canonical, contracts.canonical_format_version());
    push_u16(&mut canonical, contracts.identity_encoding_version());
    push_u16(&mut canonical, contracts.identity_registry_revision());
    push_u16(
        &mut canonical,
        contracts.network_revision_derivation_version(),
    );
    push_u16(&mut canonical, contracts.constraint_contract_version());
    push_u16(
        &mut canonical,
        contracts.static_execution_contract_version(),
    );
    push_u32(&mut canonical, snapshot.config.vehicle_capacity());
    push_u32(&mut canonical, snapshot.config.route_capacity());
    push_u64(
        &mut canonical,
        snapshot.config.route_edge_occurrence_capacity(),
    );
    push_u64(&mut canonical, snapshot.config.fixed_delta_time_ms());
    push_u64(&mut canonical, snapshot.tick);
    push_u64(&mut canonical, snapshot.time_ms);
    push_u64(&mut canonical, snapshot.command_cursor);
    push_u64(&mut canonical, snapshot.event_cursor);

    push_u64(
        &mut canonical,
        u64::try_from(route_records.len()).expect("route count fits u64"),
    );
    for record in route_records {
        push_record(&mut canonical, &record);
    }
    push_u64(
        &mut canonical,
        u64::try_from(vehicle_records.len()).expect("vehicle count fits u64"),
    );
    for record in vehicle_records {
        push_record(&mut canonical, &record);
    }
    push_u64(
        &mut canonical,
        u64::try_from(snapshot.live_order.len()).expect("live order count fits u64"),
    );
    for snapshot_vehicle_id in &snapshot.live_order {
        let record = vehicle_by_id
            .get(snapshot_vehicle_id)
            .expect("captured live_order closes over captured vehicles");
        push_record(&mut canonical, record);
    }

    let mut hasher = Sha256::new();
    hasher.update(&canonical);
    Sha256Digest::from_bytes(hasher.finalize().into())
}

fn canonical_route_record(route: &CapturedRoute) -> Vec<u8> {
    let mut record = Vec::new();
    push_u64(
        &mut record,
        u64::try_from(route.edges.len()).expect("route edge count fits u64"),
    );
    for edge in &route.edges {
        record.extend_from_slice(edge.as_bytes());
    }
    record
}

fn canonical_vehicle_record(
    vehicle: &CapturedVehicle,
    route_by_id: &BTreeMap<u64, &CapturedRoute>,
) -> Vec<u8> {
    let route = route_by_id
        .get(&vehicle.snapshot_route_id)
        .expect("captured vehicle route closes over captured routes");
    let mut record = canonical_route_record(route);
    push_u32(&mut record, vehicle.route_edge_index);
    push_u32(&mut record, vehicle.progress_mm);
    push_u16(&mut record, vehicle.carry_um);
    push_u32(&mut record, vehicle.speed_mm_s);
    record.push(match vehicle.status {
        VehicleStatus::Active => 1,
        VehicleStatus::Parked => 2,
        VehicleStatus::Completed => 3,
    });
    record.extend_from_slice(vehicle.profile.as_bytes());
    record.extend_from_slice(vehicle.class.as_bytes());
    if let Some(parking_space) = vehicle.parking_space {
        record.push(1);
        record.extend_from_slice(parking_space.as_bytes());
    } else {
        record.push(0);
    }
    record
}

fn push_record(target: &mut Vec<u8>, record: &[u8]) {
    push_u64(
        target,
        u64::try_from(record.len()).expect("canonical record length fits u64"),
    );
    target.extend_from_slice(record);
}

fn push_u16(target: &mut Vec<u8>, value: u16) {
    target.extend_from_slice(&value.to_le_bytes());
}

fn push_u32(target: &mut Vec<u8>, value: u32) {
    target.extend_from_slice(&value.to_le_bytes());
}

fn push_u64(target: &mut Vec<u8>, value: u64) {
    target.extend_from_slice(&value.to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cutover::tests::transaction_tests::world_with_vehicle;
    use crate::{
        SnapshotRestoreLimits, TickInput, VehicleSpawnInput, WorldConfig, encode_lfrs, restore_lfrs,
    };

    #[test]
    fn digest_is_stable_across_restore_handle_and_local_id_reassignment() {
        let (mut world, route, _) = world_with_vehicle(true);
        world.step(TickInput::new(100)).expect("step");
        // 捕获槽位序为 Active / Parked；restore 为避免 transient 非占用重叠会先
        // staging Parked，再 staging Active，因此新进程槽位与局部 ID 会反转。
        let parked = world
            .spawn_vehicle(VehicleSpawnInput::new(
                laneflow_static_contract::VehicleProfileOrdinal::from_raw(0),
                route,
                0,
                10_000,
                0,
            ))
            .expect("second");
        world
            .occupy_parking(
                parked,
                laneflow_static_contract::ParkingSpaceOrdinal::from_raw(0),
            )
            .expect("parking");
        let before = world.capture_snapshot();
        let before_digest = deterministic_state_digest(&before);
        let restored = restore_lfrs(
            &encode_lfrs(&before),
            world.revision(),
            world.committed_source().clone(),
            world.config(),
            SnapshotRestoreLimits::new(16 * 1_024 * 1_024, 4 * 1_024),
        )
        .expect("restore");
        let after = restored.world().capture_snapshot();
        assert_ne!(before.vehicles[0].status, before.vehicles[1].status);
        assert_ne!(before.vehicles[0].status, after.vehicles[0].status);
        assert_eq!(before_digest, deterministic_state_digest(&after));
    }

    #[test]
    fn digest_ignores_local_ids_source_audit_and_worker_plan() {
        let (world, _, _) = world_with_vehicle(true);
        let original = world.capture_snapshot();
        let expected = deterministic_state_digest(&original);
        assert_eq!(
            expected,
            Sha256Digest::from_bytes([
                0x4a, 0x96, 0x66, 0x2b, 0x75, 0x0c, 0xff, 0xd8, 0x67, 0x02, 0x28, 0xf1, 0x13, 0xef,
                0xc1, 0x10, 0xe7, 0x4c, 0x7b, 0x38, 0xcf, 0x67, 0x85, 0xb4, 0x7f, 0x99, 0xd2, 0x8d,
                0x03, 0x6b, 0x50, 0xdb,
            ])
        );
        let mut equivalent = original.clone();
        equivalent.routes[0].snapshot_route_id = 91;
        equivalent.vehicles[0].snapshot_vehicle_id = 72;
        equivalent.vehicles[0].snapshot_route_id = 91;
        equivalent.live_order[0] = 72;
        equivalent.config = WorldConfig::new(
            equivalent.config.vehicle_capacity(),
            equivalent.config.route_capacity(),
            equivalent.config.route_edge_occurrence_capacity(),
            99,
            equivalent.config.fixed_delta_time_ms(),
        );
        let crate::CommittedNetworkSource::Published { reference } = &original.source;
        equivalent.source = crate::CommittedNetworkSource::Published {
            reference: crate::PublishedLfcaReference::new(
                "asset://republished-audit-name",
                laneflow_static_contract::Sha256Digest::from_bytes([0xa5; 32]),
                laneflow_static_contract::ExactByteLength::new(777),
                reference.network_revision(),
            )
            .expect("source"),
        };
        assert_eq!(expected, deterministic_state_digest(&equivalent));
    }

    #[test]
    fn semantic_capacity_and_live_order_change_the_digest() {
        let (mut world, route, first) = world_with_vehicle(true);
        world
            .occupy_parking(
                first,
                laneflow_static_contract::ParkingSpaceOrdinal::from_raw(0),
            )
            .expect("park first");
        world
            .spawn_vehicle(VehicleSpawnInput::new(
                laneflow_static_contract::VehicleProfileOrdinal::from_raw(0),
                route,
                0,
                1_000,
                0,
            ))
            .expect("second");
        let original = world.capture_snapshot();
        let expected = deterministic_state_digest(&original);

        let mut larger_capacity = original.clone();
        larger_capacity.config = WorldConfig::new(
            larger_capacity.config.vehicle_capacity() + 1,
            larger_capacity.config.route_capacity(),
            larger_capacity.config.route_edge_occurrence_capacity(),
            larger_capacity.config.worker_count(),
            larger_capacity.config.fixed_delta_time_ms(),
        );
        assert_ne!(expected, deterministic_state_digest(&larger_capacity));

        let mut swapped_live_order = original;
        swapped_live_order.live_order.swap(0, 1);
        assert_ne!(expected, deterministic_state_digest(&swapped_live_order));
    }
}
