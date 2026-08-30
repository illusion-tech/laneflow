//! 与 `LFRS` 容器编码无关的 Runtime 逻辑状态摘要。

use laneflow_static_contract::Sha256Digest;
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::{CapturedRoute, CapturedSnapshot, CapturedVehicle, VehicleStatus};

/// 确定性状态摘要规范化版本。
pub const RUNTIME_STATE_DIGEST_VERSION: u16 = 3;
/// SHA-256 域分隔前缀；尾随 NUL 属于前缀字节。
pub const RUNTIME_STATE_DIGEST_DOMAIN: &[u8] = b"laneflow:runtime-state-digest:v1\0";

/// 状态摘要规范化失败（#532 摘要轴错误族）。
///
/// 摘要只读已捕获快照，唯一失败模式是规范化缓冲或分组表的容量预留
/// 失败；失败无副作用，宿主清点后可直接重试。
#[derive(Clone, Copy, Debug, Eq, PartialEq, Error)]
pub enum SnapshotDigestError {
    /// 规范化缓冲或分组表容量预留失败。
    #[error("状态摘要容量预留失败")]
    ReservationFailed,
}

/// 固定前缀的总字节数（一次预留）。计数字段与记录统一走可失败写入
/// （`try_push_u64` / `try_push_record`）——`try_reserve_exact` 只保证当次
/// len + additional，前缀预留的冗余会被后续精确重预留冲销，动态写入不能
/// 依赖前缀预算。
const fn canonical_prefix_len() -> usize {
    RUNTIME_STATE_DIGEST_DOMAIN.len()
        + 2
        + 2 // 摘要版本 / Runtime 状态版本
        + 8 // 世界身份
        + 32 // 语义路网修订 digest
        + 2 * 6 // 六轴静态契约版本
        + 4
        + 4
        + 8 // 三个容量字段
        + 8 // fixed dt
        + 8 * 4 // tick / 时间 / 双游标
}

/// 摘要轴可失败精确预留（注入点与 capture 侧共用同一快照轴计数器）。
fn digest_try_reserve_exact<T>(
    values: &mut Vec<T>,
    additional: usize,
) -> Result<(), SnapshotDigestError> {
    if additional == 0 {
        return Ok(());
    }
    #[cfg(test)]
    if crate::snapshot::snapshot_reservation_injected_failure() {
        return Err(SnapshotDigestError::ReservationFailed);
    }
    values
        .try_reserve_exact(additional)
        .map_err(|_| SnapshotDigestError::ReservationFailed)
}

/// 对捕获的 Runtime 逻辑状态计算与 `LFRS` wire 编码、进程句柄和快照局部 ID 无关的
/// SHA-256 摘要。
///
/// 规范输入包括：摘要版本、世界身份、语义路网修订与六轴静态契约版本、行为语义
/// 配置（容量 + fixed dt）、tick/时间/双游标、路线实例分组多重集（路线内容 +
/// 车辆绑定）与 live 更新顺序（条目绑定所属路线记录）。LFCA exact-byte
/// digest/length、Published asset 审计来源、worker 计划、`WorldGeneration` 与观测
/// session 不进入逻辑摘要。全部缓冲按计数可失败预留（#532）。
pub fn deterministic_state_digest(
    snapshot: &CapturedSnapshot,
) -> Result<Sha256Digest, SnapshotDigestError> {
    // 车辆记录表：`BTreeMap` 无可失败预留 API，等价换「按局部 ID 排序的
    // 扁平表 + 二分查找」（查找语义与输出字节不变）。
    let mut vehicle_records: Vec<(u64, Vec<u8>, u64)> = Vec::new();
    digest_try_reserve_exact(&mut vehicle_records, snapshot.vehicles.len())?;
    for vehicle in &snapshot.vehicles {
        vehicle_records.push((
            vehicle.snapshot_vehicle_id,
            canonical_vehicle_record(vehicle)?,
            vehicle.snapshot_route_id,
        ));
    }
    vehicle_records.sort_unstable_by_key(|entry| entry.0);
    let vehicle_index = |snapshot_vehicle_id: u64| -> usize {
        vehicle_records
            .binary_search_by_key(&snapshot_vehicle_id, |entry| entry.0)
            .expect("captured live_order closes over captured vehicles")
    };

    let mut route_records: Vec<(u64, Vec<u8>)> = Vec::new();
    digest_try_reserve_exact(&mut route_records, snapshot.routes.len())?;
    for route in &snapshot.routes {
        route_records.push((route.snapshot_route_id, canonical_route_record(route)?));
    }
    route_records.sort_unstable_by_key(|entry| entry.0);
    let route_index = |snapshot_route_id: u64| -> usize {
        route_records
            .binary_search_by_key(&snapshot_route_id, |entry| entry.0)
            .expect("captured vehicle route closes over captured routes")
    };

    // 路线实例分组：路线内容 + 绑定其上的车辆记录多重集。内容相同但绑定
    // 不同的实例因此可区分（remove 语义不同）；内容与绑定均相同的实例
    // 可交换（全部后续命令行为一致），保持摘要相等。
    let mut route_groups: Vec<Vec<u8>> = Vec::new();
    digest_try_reserve_exact(&mut route_groups, snapshot.routes.len())?;
    for route in &snapshot.routes {
        let bound_count = snapshot
            .vehicles
            .iter()
            .filter(|vehicle| vehicle.snapshot_route_id == route.snapshot_route_id)
            .count();
        let mut bound: Vec<&[u8]> = Vec::new();
        digest_try_reserve_exact(&mut bound, bound_count)?;
        for vehicle in &snapshot.vehicles {
            if vehicle.snapshot_route_id == route.snapshot_route_id {
                let index = vehicle_index(vehicle.snapshot_vehicle_id);
                bound.push(vehicle_records[index].1.as_slice());
            }
        }
        bound.sort_unstable();
        let route_bytes = &route_records[route_index(route.snapshot_route_id)].1;
        let mut group = Vec::new();
        digest_try_reserve_exact(&mut group, route_bytes.len() + 8)?;
        group.extend_from_slice(route_bytes);
        push_u64(
            &mut group,
            u64::try_from(bound.len()).expect("vehicle count fits u64"),
        );
        for record in bound {
            try_push_record(&mut group, record)?;
        }
        route_groups.push(group);
    }
    route_groups.sort_unstable();

    let mut canonical = Vec::new();
    digest_try_reserve_exact(&mut canonical, canonical_prefix_len())?;
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

    try_push_u64(
        &mut canonical,
        u64::try_from(route_groups.len()).expect("route group count fits u64"),
    )?;
    for group in &route_groups {
        try_push_record(&mut canonical, group)?;
    }
    try_push_u64(
        &mut canonical,
        u64::try_from(snapshot.live_order.len()).expect("live order count fits u64"),
    )?;
    // live 序条目 = 车辆记录 + 所属路线记录（内容）。跨路线换序因此可区分
    // （pose 批次按 live 序产出，顺序可观测）；所属路线内容相同的同值车辆
    // 换序保持等价（有序对内容相同）。
    for snapshot_vehicle_id in &snapshot.live_order {
        let index = vehicle_index(*snapshot_vehicle_id);
        let snapshot_route_id = vehicle_records[index].2;
        try_push_record(&mut canonical, &vehicle_records[index].1)?;
        let route_bytes = &route_records[route_index(snapshot_route_id)].1;
        try_push_record(&mut canonical, route_bytes)?;
    }

    let mut hasher = Sha256::new();
    hasher.update(&canonical);
    Ok(Sha256Digest::from_bytes(hasher.finalize().into()))
}

fn canonical_route_record(route: &CapturedRoute) -> Result<Vec<u8>, SnapshotDigestError> {
    let mut record = Vec::new();
    digest_try_reserve_exact(&mut record, 8 + route.edges.len() * 16)?;
    push_u64(
        &mut record,
        u64::try_from(route.edges.len()).expect("route edge count fits u64"),
    );
    for edge in &route.edges {
        record.extend_from_slice(edge.as_bytes());
    }
    Ok(record)
}

fn canonical_vehicle_record(vehicle: &CapturedVehicle) -> Result<Vec<u8>, SnapshotDigestError> {
    // 路线内容由所属实例分组承载，车辆记录不内嵌（内容相同的路线实例
    // 以其车辆绑定区分，见分组构造）。
    let record_len = 4
        + 4
        + 2
        + 4
        + 1
        + 16
        + 16
        + if vehicle.parking_space.is_some() {
            1 + 16
        } else {
            1
        };
    let mut record = Vec::new();
    digest_try_reserve_exact(&mut record, record_len)?;
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
    Ok(record)
}

/// 预留后写入长度前缀记录（预留成功则本次写入不再分配）。
fn try_push_record(target: &mut Vec<u8>, record: &[u8]) -> Result<(), SnapshotDigestError> {
    digest_try_reserve_exact(target, 8 + record.len())?;
    push_record(target, record);
    Ok(())
}

/// 预留后写入 `u64` 小字段（计数字段；前缀预留的冗余会被精确重预留
/// 冲销，不能依赖前缀预算）。
fn try_push_u64(target: &mut Vec<u8>, value: u64) -> Result<(), SnapshotDigestError> {
    digest_try_reserve_exact(target, 8)?;
    push_u64(target, value);
    Ok(())
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
    use crate::CapturedRoute;
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
        let before = world.capture_snapshot().expect("capture");
        let before_digest = deterministic_state_digest(&before).expect("digest");
        let restored = restore_lfrs(
            &encode_lfrs(&before),
            world.revision(),
            world.committed_source().clone(),
            world.config(),
            SnapshotRestoreLimits::new(16 * 1_024 * 1_024, 4 * 1_024),
        )
        .expect("restore");
        let after = restored.world().capture_snapshot().expect("capture");
        assert_ne!(before.vehicles[0].status, before.vehicles[1].status);
        assert_ne!(before.vehicles[0].status, after.vehicles[0].status);
        assert_eq!(
            before_digest,
            deterministic_state_digest(&after).expect("digest")
        );
    }

    #[test]
    fn digest_reservation_failure_is_retryable_and_byte_stable() {
        // #532：规范化的每个预留注入点都失败关闭（无副作用），清点后
        // 重试得到同一摘要；排序表替换 BTreeMap 不改变输出字节（由上方
        // known-vector 与等价类测试兜底）。
        let (mut world, _, _) = world_with_vehicle(true);
        world.step(TickInput::new(100)).expect("step");
        let snapshot = world.capture_snapshot().expect("capture");
        let expected = deterministic_state_digest(&snapshot).expect("baseline digest");
        for fail_after in 0..14 {
            let failed =
                crate::snapshot::with_snapshot_allocation_failure_after(fail_after, || {
                    deterministic_state_digest(&snapshot)
                });
            assert_eq!(
                failed,
                Err(SnapshotDigestError::ReservationFailed),
                "fail_after={fail_after}"
            );
        }
        assert_eq!(
            deterministic_state_digest(&snapshot).expect("retry after clearing"),
            expected
        );
    }

    #[test]
    fn digest_distinguishes_vehicle_binding_across_identical_routes() {
        // 两条内容相同的路线：车辆绑定分布不同 => 逻辑状态不同（remove 语义
        // 不同），摘要必须区分；内容与绑定均相同的实例互换 => 等价，摘要相等。
        let (world, _, _) = world_with_vehicle(true);
        let base = world.capture_snapshot().expect("capture");
        let second_vehicle = {
            let mut vehicle = base.vehicles[0].clone();
            vehicle.snapshot_vehicle_id = base.vehicles[0].snapshot_vehicle_id + 1;
            vehicle
        };
        let mut split = base.clone();
        split.routes.push(CapturedRoute {
            snapshot_route_id: base.routes[0].snapshot_route_id + 1,
            edges: base.routes[0].edges.clone(),
        });
        split.vehicles.push(second_vehicle.clone());
        split.vehicles[1].snapshot_route_id = split.routes[1].snapshot_route_id;
        split.live_order.push(second_vehicle.snapshot_vehicle_id);

        // A：两车分挂两条相同路线；B：两车同挂第一条、第二条空置。
        let mut merged = split.clone();
        merged.vehicles[1].snapshot_route_id = split.routes[0].snapshot_route_id;
        assert_ne!(
            deterministic_state_digest(&split).expect("digest"),
            deterministic_state_digest(&merged).expect("digest")
        );

        // 互换可交换实例的车辆绑定（分组不变）=> 摘要相等。
        let mut swapped = split.clone();
        swapped.vehicles[0].snapshot_route_id = split.routes[1].snapshot_route_id;
        swapped.vehicles[1].snapshot_route_id = split.routes[0].snapshot_route_id;
        assert_eq!(
            deterministic_state_digest(&split).expect("digest"),
            deterministic_state_digest(&swapped).expect("digest")
        );
    }

    #[test]
    fn digest_distinguishes_live_order_across_different_routes() {
        // 两条内容不同的路线、两辆同值车：live 序换序 => pose 批次顺序不同，
        // 摘要必须区分；所属路线内容相同时换序保持等价（边界登记）。
        let (world, _, _) = world_with_vehicle(true);
        let base = world.capture_snapshot().expect("capture");
        let second_route_id = base.routes[0].snapshot_route_id + 1;
        let second_vehicle_id = base.vehicles[0].snapshot_vehicle_id + 1;

        let mut forward = base.clone();
        let mut different_edges = base.routes[0].edges.clone();
        // 内容不同：追加重复首边（occurrence 语义允许重复边）。
        different_edges.push(base.routes[0].edges[0]);
        forward.routes.push(CapturedRoute {
            snapshot_route_id: second_route_id,
            edges: different_edges,
        });
        let mut second = base.vehicles[0].clone();
        second.snapshot_vehicle_id = second_vehicle_id;
        second.snapshot_route_id = second_route_id;
        forward.vehicles.push(second.clone());
        forward.live_order.push(second_vehicle_id);

        let mut reversed = forward.clone();
        reversed.live_order = vec![second_vehicle_id, base.vehicles[0].snapshot_vehicle_id];
        assert_ne!(
            deterministic_state_digest(&forward).expect("digest"),
            deterministic_state_digest(&reversed).expect("digest")
        );

        // 边界：两条路线内容改回相同 => 条目字节相同，换序等价。
        let mut equal_forward = forward.clone();
        equal_forward.routes[1].edges = base.routes[0].edges.clone();
        let mut equal_reversed = equal_forward.clone();
        equal_reversed.live_order = reversed.live_order.clone();
        assert_eq!(
            deterministic_state_digest(&equal_forward).expect("digest"),
            deterministic_state_digest(&equal_reversed).expect("digest")
        );
    }

    #[test]
    fn digest_ignores_local_ids_source_audit_and_worker_plan() {
        let (world, _, _) = world_with_vehicle(true);
        let original = world.capture_snapshot().expect("capture");
        let expected = deterministic_state_digest(&original).expect("digest");
        assert_eq!(
            expected,
            Sha256Digest::from_bytes([
                0x23, 0x17, 0x68, 0x5b, 0x70, 0x6f, 0xf4, 0x85, 0xda, 0x3f, 0x0a, 0x40, 0xf2, 0x33,
                0xaf, 0xf4, 0x1a, 0x73, 0xda, 0xae, 0x40, 0xcd, 0x54, 0x11, 0x08, 0x5d, 0x44, 0xc6,
                0x5e, 0x67, 0x1b, 0x55,
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
        assert_eq!(
            expected,
            deterministic_state_digest(&equivalent).expect("digest")
        );
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
        let original = world.capture_snapshot().expect("capture");
        let expected = deterministic_state_digest(&original).expect("digest");

        let mut larger_capacity = original.clone();
        larger_capacity.config = WorldConfig::new(
            larger_capacity.config.vehicle_capacity() + 1,
            larger_capacity.config.route_capacity(),
            larger_capacity.config.route_edge_occurrence_capacity(),
            larger_capacity.config.worker_count(),
            larger_capacity.config.fixed_delta_time_ms(),
        );
        assert_ne!(
            expected,
            deterministic_state_digest(&larger_capacity).expect("digest")
        );

        let mut swapped_live_order = original;
        swapped_live_order.live_order.swap(0, 1);
        assert_ne!(
            expected,
            deterministic_state_digest(&swapped_live_order).expect("digest")
        );
    }
}
