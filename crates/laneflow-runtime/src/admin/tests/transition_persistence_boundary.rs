//! #1217 持久化边界封闭场景集（审计 P1-3/P1-4）。
//!
//! 合同锚点：换道合同 §9（A5，S5 #1200 交付完整格式前的版本轴）与冲突资源
//! 文档「静态面（安装/迁移/重绑定/恢复）遇 `Transition` 一律失败关闭」
//! （G2③ 持久边界拒绝）。场景集以**真实资源身份与历史**为断言面（逐格
//! `lag_reference` 比对 + 通道切断断言），不以同一套错误编码的摘要互相印证。
//!
//! 恢复面的过渡行资源种类检查（`format_admission`）为防御面：本版本 writer
//! 由保存侧结构化拒绝保证不再产出过渡行，v7 wire 也没有资源种类字段可检；
//! 手工构造混合 hop fixture 的触发测试留待 S5 #1200（findings.md 登记论证）。

use laneflow_static_contract::LaneEdgeOrdinal;

use crate::admin::snapshot::SnapshotCaptureError;
use crate::admin::snapshot_digest::{
    SnapshotDigestError, SnapshotDigestLimits, deterministic_state_digest,
};
use crate::kernel::conflict::{ConflictResource, UnsupportedTransitionState};
use crate::kernel::tables::compile_route_tests::s3_lane_change::{
    install, revision as sample_revision, stream_bearing_revision,
};
use crate::{
    ConflictEligibilityState, ConflictLagReference, ConflictPassageAddress, TickInput, TrafficWorld,
};

const DIGEST_LIMITS_MAX: u64 = 64 * 1_024 * 1_024;

fn limits() -> SnapshotDigestLimits {
    SnapshotDigestLimits::new(DIGEST_LIMITS_MAX)
}

/// 过渡资源格地址（安装口径与 `ConflictArbiter::install` 同型：每横向过渡段
/// 恰一格、表占位 stream 0/local 0）。
fn transition_addresses(world: &TrafficWorld) -> Vec<ConflictPassageAddress> {
    world
        .state
        .conflict_read()
        .addresses()
        .filter(|address| matches!(address.resource(), ConflictResource::Transition(_)))
        .collect()
}

fn static_addresses(world: &TrafficWorld) -> Vec<ConflictPassageAddress> {
    world
        .state
        .conflict_read()
        .addresses()
        .filter(|address| matches!(address.resource(), ConflictResource::Zone(_)))
        .collect()
}

/// 按真身份读取资源格的清空历史。
fn lag_of(world: &TrafficWorld, address: ConflictPassageAddress) -> ConflictLagReference {
    world
        .state
        .conflict_read()
        .lag_reference(address)
        .expect("installed address")
}

/// 按真身份直写清空历史（与运行态车尾清空写入同型的测试构造缝）。
fn write_lag(
    world: &mut TrafficWorld,
    address: ConflictPassageAddress,
    reference: ConflictLagReference,
) {
    crate::kernel::conflict::ConflictWrite::new(
        &mut world.state.committed.conflict,
        &mut world.state.derived.conflict,
        &mut world.state.workspace.conflict,
    )
    .restore_lag_reference(address, reference)
    .expect("write lag reference");
}

/// 流-bearing fixture 世界 + 含 T 路线 + 一辆活动车（无 traversal、无预约）。
fn t_world_with_route_and_vehicle() -> (TrafficWorld, crate::RouteHandle, crate::VehicleHandle) {
    let revision = stream_bearing_revision();
    let traffic = revision.traffic();
    let transition = (0..traffic.lane_edge_count())
        .map(LaneEdgeOrdinal::from_raw)
        .find(|edge| traffic.is_lateral_transition_segment(*edge))
        .expect("transition edge");
    let path_edges = traffic
        .maneuvers()
        .maneuver_path(laneflow_static_contract::ManeuverPathOrdinal::from_raw(0))
        .expect("path 0")
        .edges();
    let e2 = path_edges[0];
    let int_edge = path_edges[1];
    let exit = *path_edges.last().expect("path exit");
    let e0 = traffic.predecessors(transition).expect("predecessors")[0];
    let mut world = install(revision);
    let route = world
        .register_route(crate::RouteRegisterInput::new(vec![
            e0, transition, e2, int_edge, exit,
        ]))
        .expect("transition route");
    let vehicle = world
        .state
        .restore_unparked_vehicle(
            crate::VehicleSpawnInput::new(
                laneflow_static_contract::VehicleProfileOrdinal::from_raw(0),
                route,
                0,
                0,
                0,
            )
            .with_open_entrance(),
            0,
            crate::VehicleStatus::Active,
            None,
            None,
            true,
        )
        .expect("vehicle");
    world
        .state
        .rebuild_occupancy_index()
        .expect("occupancy index");
    (world, route, vehicle)
}

/// 该车当前是否持有过渡态权威（T `Clearing` 遍历或覆盖过渡出现记录的预约）。
/// 静态机动的预约/遍历不在此列——它们是可表达状态（零变化域）。
fn holds_transition_state(world: &TrafficWorld, vehicle: crate::VehicleHandle) -> bool {
    use crate::ManeuverTraversalPhase;

    let Some(state) = world.state.vehicle_state(vehicle) else {
        return false;
    };
    if let Some(traversal) = state.maneuver_traversal
        && let ManeuverTraversalPhase::Clearing { admission_gate_hop } = traversal.phase
    {
        let covered = world
            .state
            .compiled_route(traversal.route)
            .is_some_and(|compiled| {
                compiled.admission_hop_transition_covered(
                    traversal.maneuver_occurrence_index,
                    admission_gate_hop,
                )
            });
        if covered {
            return true;
        }
    }
    world
        .conflict_reservation(vehicle)
        .is_some_and(|reservation| {
            let range = reservation.passage_range();
            world
                .state
                .compiled_route(range.route())
                .is_some_and(|compiled| {
                    let start = range.first_conflict_occurrence_index() as usize;
                    let end = start.saturating_add(range.passage_count() as usize);
                    compiled
                        .conflicts
                        .get(start..end)
                        .is_some_and(|occurrences| {
                            occurrences.iter().any(|occurrence| {
                                matches!(occurrence.resource, ConflictResource::Transition(_))
                            })
                        })
                })
        })
}

/// S1–S3 生命周期回归（真实运行态，非构造输入）：换道车经 T 全程——
/// 入 T 前保存/摘要照常；持 T 预约与 `Clearing`（含车尾清空窗口）时按
/// `AdmissionHopWithoutGate` 结构化拒绝（旧实现于此 panic 传播宿主）；车尾
/// 清空、预约释放后按 `TransitionLagHistory` 拒绝（旧实现于此把 T 历史错绑
/// 成静态 Zone 身份）。全程：拒绝零状态污染、世界继续可运行。
#[test]
fn s1_s2_s3_real_tick_transition_lifecycle_rejects_structurally() {
    let revision = stream_bearing_revision();
    let traffic = revision.traffic();
    let transition = (0..traffic.lane_edge_count())
        .map(LaneEdgeOrdinal::from_raw)
        .find(|edge| traffic.is_lateral_transition_segment(*edge))
        .expect("transition edge");
    let path_edges = traffic
        .maneuvers()
        .maneuver_path(laneflow_static_contract::ManeuverPathOrdinal::from_raw(0))
        .expect("path 0")
        .edges();
    let e2 = path_edges[0];
    let int_edge = path_edges[1];
    let exit = *path_edges.last().expect("path exit");
    let e0 = traffic.predecessors(transition).expect("predecessors")[0];
    let e5 = traffic.successors(exit).expect("exit successors")[0];
    let mut world = install(revision);
    let route = world
        .register_route(crate::RouteRegisterInput::new(vec![
            e0, transition, e2, int_edge, exit, e5,
        ]))
        .expect("changer route");
    let vehicle = world
        .spawn_vehicle(
            crate::VehicleSpawnInput::new(
                laneflow_static_contract::VehicleProfileOrdinal::from_raw(0),
                route,
                0,
                15_000,
                10_000,
            )
            .with_open_entrance(),
        )
        .expect("spawn changer");
    world.state.rebuild_occupancy_index().expect("occupancy");

    let t_address = {
        let addresses = transition_addresses(&world);
        assert_eq!(addresses.len(), 1, "流-bearing fixture 恰一横向过渡段");
        addresses[0]
    };

    // 入 T 前：可表达（控制点，锚定零变化域）。
    let before_transition = world.capture_snapshot().expect("pre-transition capture");
    assert_eq!(
        world
            .direct_state_digest(limits())
            .expect("pre-transition digest"),
        deterministic_state_digest(&before_transition).expect("captured digest"),
        "直算与捕获摘要逐字节一致（零变化域）"
    );

    let mut seen_admission_reject = false;
    let mut seen_lag_reject = false;
    for tick in 1_u64..=80 {
        world.state.prepare_waiting_step(0.1).expect("prepare");
        world.step(TickInput::new(100)).expect("step");

        let transition_state = holds_transition_state(&world, vehicle);
        let lag = lag_of(&world, t_address);
        let tick_before = world.tick_index();
        let cursor_before = world.command_cursor();
        let result = world.capture_snapshot();
        assert_eq!(world.tick_index(), tick_before, "拒绝不推进 tick（{tick}）");
        assert_eq!(
            world.command_cursor(),
            cursor_before,
            "拒绝不动命令游标（{tick}）"
        );
        if transition_state {
            // S1/S2：活动过渡 / 车尾清空（Clearing）——准入身份槽无真实 Gate。
            assert_eq!(
                result,
                Err(SnapshotCaptureError::UnsupportedTransitionState(
                    UnsupportedTransitionState::AdmissionHopWithoutGate
                )),
                "tick {tick} 持 T 权威须按准入位点结构化拒绝"
            );
            assert_eq!(
                world.direct_state_digest(limits()),
                Err(SnapshotDigestError::UnsupportedTransitionState(
                    UnsupportedTransitionState::AdmissionHopWithoutGate
                )),
                "tick {tick} 摘要同口径，且不得经 Panicked/InvariantViolation 转译"
            );
            seen_admission_reject = true;
        } else if lag != ConflictLagReference::NoHistory {
            // S3：仅 T 历史（预约已释放、车已离开 T；此后静态机动预约照常
            // 可表达，拒绝只由 T 历史行触发）。
            assert_eq!(
                result,
                Err(SnapshotCaptureError::UnsupportedTransitionState(
                    UnsupportedTransitionState::TransitionLagHistory
                )),
                "tick {tick} T 清空历史须按 lag 行结构化拒绝"
            );
            seen_lag_reject = true;
        } else {
            result.expect("入 T 前状态可表达");
        }
    }
    assert!(
        seen_admission_reject,
        "场景必须覆盖持 T 预约/清空窗口（S1/S2）"
    );
    assert!(seen_lag_reject, "场景必须覆盖仅 T 历史（S3）");

    // 真实资源身份与历史：T 格携带真实 `ActualClear` 时间，拒绝不改写历史。
    let final_lag = lag_of(&world, t_address);
    let ConflictLagReference::ActualClear(cleared_at_ms) = final_lag else {
        panic!("车尾清空后 T 格应携带 ActualClear 历史，实际 {final_lag:?}");
    };
    assert!(cleared_at_ms <= world.time_ms(), "清空时间不得超前世界时钟");
    // 归属≠身份通道已切断：T 格无忠实持久 locator。
    assert_eq!(world.conflict_passage_locator(t_address), None);
    // 世界继续可运行：步进照常、拒绝结果稳定。
    world
        .state
        .prepare_waiting_step(0.1)
        .expect("prepare after refusal");
    world.step(TickInput::new(100)).expect("step after refusal");
    assert_eq!(lag_of(&world, t_address), final_lag, "拒绝不改写 T 历史");
    assert_eq!(
        world.capture_snapshot(),
        Err(SnapshotCaptureError::UnsupportedTransitionState(
            UnsupportedTransitionState::TransitionLagHistory
        )),
        "T 历史不消失，拒绝持续稳定"
    );
}

/// S4：T + 静态 Zone 混合历史——整入口拒绝（静态行不得单独放出），且两个
/// 真身份格的历史各自保真（旧通道把 T 行错绑到静态格或触发重复 locator）。
#[test]
fn s4_mixed_t_and_static_history_rejects_whole_entry_with_distinct_identity() {
    let (mut world, _route, _vehicle) = t_world_with_route_and_vehicle();
    let t_address = transition_addresses(&world)[0];
    let static_address = static_addresses(&world)
        .into_iter()
        .next()
        .expect("流-bearing fixture 含静态冲突格");
    write_lag(
        &mut world,
        t_address,
        ConflictLagReference::ActualClear(1_000),
    );
    write_lag(
        &mut world,
        static_address,
        ConflictLagReference::ActualClear(2_000),
    );

    assert_eq!(
        world.capture_snapshot(),
        Err(SnapshotCaptureError::UnsupportedTransitionState(
            UnsupportedTransitionState::TransitionLagHistory
        )),
        "混合历史整入口拒绝，不得只放行静态行"
    );
    assert_eq!(
        world.direct_state_digest(limits()),
        Err(SnapshotDigestError::UnsupportedTransitionState(
            UnsupportedTransitionState::TransitionLagHistory
        )),
        "摘要同口径（独立于 InvariantViolation 族）"
    );
    // 真实资源身份与历史逐项比对：两格各自保真。
    assert_eq!(
        lag_of(&world, t_address),
        ConflictLagReference::ActualClear(1_000)
    );
    assert_eq!(
        lag_of(&world, static_address),
        ConflictLagReference::ActualClear(2_000)
    );
    assert_eq!(world.conflict_passage_locator(t_address), None);
    assert!(world.conflict_passage_locator(static_address).is_some());
    // 世界继续可运行。
    world.state.prepare_waiting_step(0.1).expect("prepare");
    world.step(TickInput::new(100)).expect("step continues");
}

/// S5：多 T 历史——每个过渡资源格的身份与历史分别保真。旧通道把全部 T 格
/// 坍缩成同一 `(stream 0, 归属 zone)` 键（多 T 历史在恢复端必成重复 locator
/// 或错绑），本面以真身份逐格断言。世界级多 T 场景载体缺口（样本网络无策略
/// 声明、不可安装世界）登记 findings.md；世界级拒绝口径与 S4 同一代码路径
/// （任一 T 行即整入口拒绝），在此之上本用例钉多格身份不互覆。
#[test]
fn s5_multi_t_history_keeps_distinct_resource_identity() {
    use crate::kernel::conflict::ConflictArbiter;

    let revision = sample_revision();
    let mut arbiter = ConflictArbiter::install(&revision, 8).expect("install arbiter");
    let t_addresses: Vec<ConflictPassageAddress> = arbiter
        .read()
        .addresses()
        .filter(|address| matches!(address.resource(), ConflictResource::Transition(_)))
        .collect();
    assert!(
        t_addresses.len() >= 2,
        "多 T 场景需要至少两个过渡资源格（样本网络多段过渡链），实际 {}",
        t_addresses.len()
    );
    let (first, second) = (t_addresses[0], t_addresses[1]);
    arbiter
        .write()
        .restore_lag_reference(first, ConflictLagReference::ActualClear(1_000))
        .expect("first T history");
    arbiter
        .write()
        .restore_lag_reference(second, ConflictLagReference::CutoverFloor(2_000))
        .expect("second T history");

    // 真实资源身份与历史：两个 T 格互不覆盖、各自保真（旧通道的坍缩键会把
    // 两行合并成同一 locator——恢复端重复键拒绝或错绑）。
    assert_eq!(
        arbiter.read().lag_reference(first),
        Some(ConflictLagReference::ActualClear(1_000)),
        "第一过渡资源格历史保真"
    );
    assert_eq!(
        arbiter.read().lag_reference(second),
        Some(ConflictLagReference::CutoverFloor(2_000)),
        "第二过渡资源格历史保真（不得被坍缩键覆盖）"
    );
    let rows: Vec<(ConflictPassageAddress, ConflictLagReference)> =
        arbiter.read().persisted_lag_rows().collect();
    assert_eq!(rows.len(), 2, "两行历史都不得静默丢弃");
    assert_ne!(rows[0].0, rows[1].0, "两行归属不同的真实资源格");
}

/// D2 防御面：eligibility 的准入身份槽指向过渡出现记录（无真实 Gate）时，
/// 同一准入位点口径结构化拒绝（该位点在 committed 态按现行校验不可达，
/// 此处以构造态钉住统一口径）。
#[test]
fn d2_eligibility_on_transition_occurrence_rejects_at_admission_slot() {
    let (mut world, route, vehicle) = t_world_with_route_and_vehicle();
    let t_occurrence_index = {
        let compiled = world.state.compiled_route(route).expect("compiled route");
        compiled
            .conflicts
            .iter()
            .position(|occurrence| matches!(occurrence.resource, ConflictResource::Transition(_)))
            .expect("route carries a transition occurrence") as u32
    };
    let locator = world
        .conflict_passage_occurrence_locator(route, t_occurrence_index)
        .expect("transition occurrence locator");
    let capacity = usize::try_from(world.config().vehicle_capacity()).expect("capacity");
    world
        .state
        .committed
        .conflict_eligibility
        .resize(capacity, None);
    world.state.committed.conflict_eligibility[vehicle.index() as usize] =
        ConflictEligibilityState::update(None, locator, true, 0);
    assert_eq!(
        world.capture_snapshot(),
        Err(SnapshotCaptureError::UnsupportedTransitionState(
            UnsupportedTransitionState::AdmissionHopWithoutGate
        )),
        "eligibility 准入槽同口径归口 AdmissionHopWithoutGate（D1 裁定）"
    );
}

/// S6 控制组（零变化红线）：无 T 参与的静态冲突世界保存/摘要行为不变——
/// 捕获确定性、编码确定性、直算与捕获摘要逐字节一致。
#[test]
fn s6_static_conflict_persistence_control_zero_change() {
    let (world, _vehicle) =
        crate::admin::format_admission::tests::world_with_conflict_reservation();
    let first = world.capture_snapshot().expect("static capture");
    let second = world.capture_snapshot().expect("static capture again");
    assert_eq!(first, second);
    assert_eq!(
        crate::encode_lfrs(&first),
        crate::encode_lfrs(&second),
        "编码确定性"
    );
    assert_eq!(
        world.direct_state_digest(limits()).expect("direct digest"),
        deterministic_state_digest(&first).expect("captured digest"),
        "直算与捕获摘要逐字节一致（零变化域）"
    );
}

/// D3 回归（裁定 (a)）：含 T 格网络的跨修订迁移结构化失败、无部分状态残留。
/// 现状行为会给目标 T 格写入 `CutoverFloor` 并以归属对投影 lag 行（错误身份
/// 行，P1-4 同类），不得因「迁移今天能成功」保留。
#[test]
fn d3_cross_revision_migration_of_t_cell_network_fails_structurally_without_residue() {
    let revision = stream_bearing_revision();
    let mut world = install(revision.clone());
    assert!(!transition_addresses(&world).is_empty(), "fixture 必须含 T 格");
    let before = world
        .capture_snapshot()
        .expect("source without T state is expressible");
    let target = stream_bearing_revision();
    let target_origin = *target.canonical_origin();
    let rebinding = crate::admin::cutover_migration::CrossRevisionRebinding::build(
        world.state.binding.revision.identity(),
        target.identity(),
    )
    .expect("same semantic identities rebind");
    let error = match crate::admin::cutover_migration::migrate_structural_clone(
        &world.state,
        target,
        crate::admin::cutover::tests::transaction_tests::source_for(
            target_origin,
            "fixture://transition-migration-target",
        ),
        &rebinding,
    ) {
        Ok(_) => panic!("含 T 格网络跨修订迁移必须结构化失败"),
        Err(error) => error,
    };
    assert!(
        matches!(
            error,
            crate::CutoverError::UnsupportedTransitionState(
                UnsupportedTransitionState::TransitionLagHistory
            )
        ),
        "结构化失败且归口 lag 行错误族：{error:?}"
    );
    // 无部分状态残留：源世界状态逐项相等、照常可运行。
    let after = world
        .capture_snapshot()
        .expect("source capture after refused migration");
    assert_eq!(before, after, "被拒绝的迁移不得改动源世界");
    world
        .state
        .prepare_waiting_step(0.1)
        .expect("prepare after refused migration");
    world.step(TickInput::new(100)).expect("world continues");
}
