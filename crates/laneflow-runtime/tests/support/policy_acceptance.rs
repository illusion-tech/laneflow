use super::*;

use laneflow_compiler::GateInterpretation;
use laneflow_runtime::{ConflictDecisionOutcome, ConflictNoGrantReason, TrafficTransitionKind};

fn boundary_signal_module(
    interpretation: GateInterpretation,
    stop_aspect: SignalAspect,
    speed: f64,
) -> lfre::RoadEditingSourceModule {
    conflict_road_editing_module_with_shape_and_speed(
        2,
        false,
        true,
        false,
        speed,
        ConflictPolicyFixture {
            right_turn_signal: Some(interpretation),
            signal_cycle_ms: Some([100, 300]),
            signal_stop_aspect: Some(stop_aspect),
            ..Default::default()
        },
    )
}

fn blocked_at_green(world: &mut TrafficWorld) -> VehicleHandle {
    world.step(TickInput::new(100)).unwrap();
    let [route, _] = right_turn_routes(world);
    world
        .spawn_vehicle(
            VehicleSpawnInput::new(VehicleProfileOrdinal::from_raw(0), route, 1, 10_501, 0)
                .with_open_entrance(),
        )
        .unwrap();
    at_gate(world, route)
}

fn eligibility_clocks(world: &TrafficWorld) -> Vec<u64> {
    let bytes = encode_lfrs(&world.capture_snapshot().unwrap());
    let wire = snapshot_wire::size_prefixed_root_as_runtime_snapshot(&bytes).unwrap();
    wire.vehicles()
        .iter()
        .filter_map(|vehicle| {
            vehicle
                .conflict_eligibility()
                .map(|value| value.first_eligible_tick())
        })
        .collect()
}

fn assert_snapshot_roundtrip(world: &TrafficWorld) {
    let snapshot = world.capture_snapshot().unwrap();
    let restored = restore_lfrs(
        &encode_lfrs(&snapshot),
        world.revision(),
        world.committed_source().clone(),
        world.config(),
        laneflow_runtime::ExecutionConfig::new(std::num::NonZeroU32::MIN),
        SnapshotRestoreLimits::new(1_048_576, 1_024),
    )
    .unwrap()
    .into_world();
    assert_eq!(
        deterministic_state_digest(&snapshot).unwrap(),
        deterministic_state_digest(&restored.capture_snapshot().unwrap()).unwrap()
    );
    assert_eq!(
        world.committed_signal_groups().as_slice(),
        restored.committed_signal_groups().as_slice()
    );
}

#[test]
fn signal_boundary_publishes_valid_eligibility_and_preserves_continuous_waiting() {
    for interpretation in [
        GateInterpretation::ProtectedGroup,
        GateInterpretation::PermissiveGroup,
    ] {
        for stop_aspect in [SignalAspect::Red, SignalAspect::Yellow] {
            let revision = compile_road_editing_revision(boundary_signal_module(
                interpretation,
                stop_aspect,
                13.0,
            ));
            let mut world = install_fixture(
                revision,
                WorldConfig::try_new(laneflow_runtime::WorldConfigInput {
                    vehicle_capacity: 4,
                    route_capacity: 4,
                    route_edge_occurrence_capacity: 64,
                    route_conflict_occurrence_capacity: 4,
                    fixed_delta_time_ms: 100,
                })
                .expect("有效的交通配置"),
            )
            .unwrap();
            let subject = blocked_at_green(&mut world);
            // #1272：下游阻塞车的清空时点后移，资格钟与决定拍随新轨迹
            // 重排（原 500 ms 清空 → 600 ms）。按性质核验，不钉具体拍号：
            // 阻塞期 NoGrant 在前、放行后边界停驶 NotEvaluated 在后；资格
            // 钟是锚点——连续等待期间稳定不重置，拒绝期清空，资格回归后
            // 以新锚点重启。
            let mut previous_clocks: Option<Vec<u64>> = None;
            let mut saw_no_grant = false;
            let mut blocked_before_release = false;
            let mut saw_not_evaluated = false;
            let mut saw_clocked = false;
            let mut last_anchored: Option<u64> = None;
            let mut restarted_checked: Option<bool> = None;
            let mut saw_cleared = false;
            let mut saw_restarted = false;
            for expected_time in [200, 300, 400, 500, 600, 700, 800, 900, 1_000, 1_100, 1_200] {
                world.step(TickInput::new(100)).unwrap();
                assert_eq!(world.time_ms(), expected_time);
                match calibration_outcome(&world, subject) {
                    ConflictDecisionOutcome::NoGrant(_) => {
                        saw_no_grant = true;
                        if !saw_not_evaluated {
                            blocked_before_release = true;
                        }
                    }
                    ConflictDecisionOutcome::NotEvaluated => {
                        saw_not_evaluated = true;
                    }
                    other => panic!("边界信号期不期待的决定 {other:?}"),
                }
                let clocks = eligibility_clocks(&world);
                if let Some(previous) = &previous_clocks
                    && !previous.is_empty()
                    && !clocks.is_empty()
                {
                    assert_eq!(
                        clocks, *previous,
                        "连续等待期间资格钟锚点稳定（不逐拍重置）"
                    );
                }
                if clocks.is_empty() {
                    if saw_clocked && !saw_cleared {
                        saw_cleared = true;
                        last_anchored = previous_clocks
                            .clone()
                            .unwrap_or_default()
                            .into_iter()
                            .max();
                    }
                } else {
                    if saw_cleared {
                        // R4 收紧：重启必须以新锚点——晚于清空前最后锚点，
                        // 复用旧 first_eligible_tick 不能通过。
                        if restarted_checked.is_none() {
                            restarted_checked = Some(
                                clocks
                                    .iter()
                                    .all(|clock| last_anchored.is_some_and(|last| *clock > last)),
                            );
                        }
                        saw_restarted = true;
                    }
                    saw_clocked = true;
                }
                previous_clocks = Some(clocks);
                assert_snapshot_roundtrip(&world);
            }
            assert!(saw_no_grant, "阻塞期出现 NoGrant");
            // 黄灯组合经逐拍困境区许可不再必然进入边界停驶态（#1272 语义的
            // 可观测变化）；其发布面由 advance_braking 锚覆盖。红灯组合保留
            // 完整「先阻塞后放行 + 边界停驶」秩序。
            if stop_aspect == SignalAspect::Red {
                assert!(
                    blocked_before_release,
                    "先阻塞后放行：首个 NotEvaluated 之前出现过 NoGrant"
                );
                assert!(saw_not_evaluated, "放行后边界停驶出现 NotEvaluated");
            }
            assert!(saw_clocked, "资格钟出现");
            assert!(saw_cleared, "拒绝期间资格钟清空");
            assert!(saw_restarted, "资格回归后以新锚点重启");
            assert!(
                restarted_checked.unwrap_or(false),
                "重启锚点必须晚于清空前最后锚点（不重置复用旧 first_eligible_tick）"
            );
        }
    }
}

#[test]
fn signal_boundary_journal_replays_normalized_eligibility_during_cutover() {
    let module = |speed| {
        boundary_signal_module(GateInterpretation::ProtectedGroup, SignalAspect::Red, speed)
    };
    let (base, target, diff, binding) = compile_conflict_cutover_pair(module(13.0), module(13.1));
    let mut world = install_fixture(
        Arc::clone(&base),
        WorldConfig::try_new(laneflow_runtime::WorldConfigInput {
            vehicle_capacity: 4,
            route_capacity: 4,
            route_edge_occurrence_capacity: 64,
            route_conflict_occurrence_capacity: 4,
            fixed_delta_time_ms: 100,
        })
        .expect("有效的交通配置"),
    )
    .unwrap();
    blocked_at_green(&mut world);
    world.step(TickInput::new(100)).unwrap();
    assert_eq!(eligibility_clocks(&world), vec![2]);
    let descriptor = NetworkRevisionCutoverDescriptor::new(
        LfcaOriginBinding::from_canonical_origin(*base.canonical_origin()),
        LfcaOriginBinding::from_canonical_origin(*target.canonical_origin()),
        Some(binding),
        MigrationPolicyKind::CrossRevisionDirect,
        world.world_binding(),
    );
    let transaction = world
        .prepare_cross_revision_cutover(
            Arc::clone(&target),
            published_source(&target, "fixture://signal-boundary-cutover"),
            &descriptor,
            &diff,
            &CutoverPreflightLimits::new(1_048_576),
            &CutoverTransactionLimits::default(),
        )
        .unwrap();
    world.step(TickInput::new(100)).unwrap();
    world.step(TickInput::new(100)).unwrap();
    assert_eq!(world.time_ms(), 400);
    assert!(eligibility_clocks(&world).is_empty());
    let commit = transaction.commit(&mut world).unwrap();
    assert_eq!(commit.events.as_slice().len(), 1);
    assert_eq!(world.time_ms(), 400);
    assert!(eligibility_clocks(&world).is_empty());
    assert_snapshot_roundtrip(&world);
    world.step(TickInput::new(100)).unwrap();
}

#[test]
fn resource_free_gate_emits_decision_for_red_and_green() {
    for green in [false, true] {
        let revision =
            compile_road_editing_revision(conflict_road_editing_module_with_shape_and_speed(
                2,
                false,
                false,
                false,
                13.0,
                ConflictPolicyFixture {
                    right_turn_signal: Some(GateInterpretation::ProtectedGroup),
                    signal_cycle_ms: Some([100, 10_000]),
                    ..Default::default()
                },
            ));
        let mut world = install_fixture(
            revision,
            WorldConfig::try_new(laneflow_runtime::WorldConfigInput {
                vehicle_capacity: 4,
                route_capacity: 4,
                route_edge_occurrence_capacity: 64,
                route_conflict_occurrence_capacity: 4,
                fixed_delta_time_ms: 100,
            })
            .expect("有效的交通配置"),
        )
        .unwrap();
        if green {
            world.step(TickInput::new(100)).unwrap();
        }
        let [route, _] = right_turn_routes(&mut world);
        let subject = at_gate(&mut world, route);
        world.step(TickInput::new(100)).unwrap();
        assert_eq!(
            calibration_outcome(&world, subject),
            if green {
                ConflictDecisionOutcome::NotRequired
            } else {
                ConflictDecisionOutcome::NotEvaluated
            }
        );
        assert_eq!(
            world.vehicle(subject).unwrap().route_edge_index(),
            u32::from(green)
        );
        assert!(world.conflict_reservation(subject).is_none());
    }
}

#[test]
fn resource_free_gate_keeps_following_resource_gate_in_the_same_tick() {
    for blocked in [false, true] {
        let revision =
            compile_road_editing_revision(conflict_road_editing_module_with_shape_and_speed(
                2,
                false,
                true,
                false,
                13.0,
                ConflictPolicyFixture {
                    conflict_after_release: true,
                    ..Default::default()
                },
            ));
        let mut world = install_fixture(
            revision,
            WorldConfig::try_new(laneflow_runtime::WorldConfigInput {
                vehicle_capacity: 4,
                route_capacity: 4,
                route_edge_occurrence_capacity: 64,
                route_conflict_occurrence_capacity: 4,
                fixed_delta_time_ms: 1_000,
            })
            .expect("有效的交通配置"),
        )
        .unwrap();
        let [route, conflicting_route] = right_turn_routes(&mut world);
        if blocked {
            world
                .spawn_vehicle(
                    VehicleSpawnInput::new(
                        VehicleProfileOrdinal::from_raw(0),
                        conflicting_route,
                        1,
                        13_000,
                        10_000,
                    )
                    .with_open_entrance(),
                )
                .unwrap();
        }
        let subject = world
            .place_existing_active_vehicle(
                VehicleSpawnInput::new(
                    VehicleProfileOrdinal::from_raw(0),
                    route,
                    0,
                    calibration_gate(&world, route),
                    13_000,
                )
                .with_open_entrance(),
            )
            .unwrap();
        // #1272：13 m/s 起步先为停止/清空目标减速，后续资源门的决定出现在
        // 车辆接近后的拍上——逐拍推进直到该决定出现或预算耗尽。
        let expected_hop1 = if blocked {
            ConflictDecisionOutcome::NoGrant(ConflictNoGrantReason::ConflictOccupied)
        } else {
            ConflictDecisionOutcome::Granted
        };
        // 车辆完成路线后 latest 批次轮换，不再保留资源决定——改核验：
        // 完成前曾出现（或保有）期望决定，且 !blocked 时的预约曾建立。
        let mut decided = false;
        let mut saw_hop0_not_required = false;
        let mut saw_hop1_expected = false;
        let mut reserved_once = false;
        let mut never_held_while_blocked = true;
        for _ in 0..12 {
            world.step(TickInput::new(1_000)).unwrap();
            let decisions: Vec<_> = world
                .latest_conflict_decisions()
                .iter()
                .filter(|decision| decision.vehicle() == subject)
                .map(|decision| (decision.anchor().hop(), decision.outcome()))
                .collect();
            // #1272 减速后两跳决定落在不同拍（原同拍）；预算内先后出现即算。
            if decisions.contains(&(0, ConflictDecisionOutcome::NotRequired)) {
                saw_hop0_not_required = true;
            }
            if decisions.contains(&(1, expected_hop1)) {
                saw_hop1_expected = true;
            }
            decided = saw_hop0_not_required && saw_hop1_expected;
            if world.conflict_reservation(subject).is_some() {
                if blocked {
                    never_held_while_blocked = false;
                }
                reserved_once = true;
            }
            if decided && (blocked || reserved_once) {
                break;
            }
        }
        // #1272：13 m/s 起步为停止/清空目标减速，后续资源门决定出现在后续
        // 拍；未阻塞路径曾建立并按 resource_free_release 释放（完成后查询为
        // None），阻塞路径自始无预约。预约状态在完成/释放后均为 None——以
        // 「从未违规持有」为验收：blocked 自始 None，未阻塞曾 Some。
        assert!(
            decided,
            "12 拍预算内先后出现两跳期望决定（blocked={blocked}）"
        );
        if blocked {
            assert!(never_held_while_blocked, "阻塞路径自始不持有预约");
        } else {
            assert!(
                reserved_once,
                "unblocked subject must hold a reservation at some tick"
            );
        }
    }
}

/// F2' 复审修锚（合同 §3.1.1）：完全空闲的 Conflict 声明不产生停止目标——
/// 「尚未申请且无已知阻塞的入口不因『没有 grant』自行制造停止目标」。
/// 同几何 A/B：`include_conflict` 关/开，单车 13 m 入口边、10 m/s、100 ms，
/// 首拍提交必须逐字段一致（旧实现把空闲资源当停止目标提前紧急制动：
/// 9802 vs 10097 mm/s）。
#[test]
fn idle_conflict_entry_is_a_position_barrier_not_a_stop_target() {
    let mut outcomes = Vec::new();
    for include_conflict in [false, true] {
        let revision =
            compile_road_editing_revision(conflict_road_editing_module_with_shape_and_speed(
                2,
                false,
                include_conflict,
                false,
                13.0,
                ConflictPolicyFixture::default(),
            ));
        let mut world = install_fixture(
            revision,
            WorldConfig::try_new(laneflow_runtime::WorldConfigInput {
                vehicle_capacity: 4,
                route_capacity: 4,
                route_edge_occurrence_capacity: 64,
                route_conflict_occurrence_capacity: 4,
                fixed_delta_time_ms: 100,
            })
            .expect("有效的交通配置"),
        )
        .unwrap();
        let [route, _] = right_turn_routes(&mut world);
        let subject = world
            .place_existing_active_vehicle(
                VehicleSpawnInput::new(VehicleProfileOrdinal::from_raw(0), route, 0, 0, 10_000)
                    .with_open_entrance(),
            )
            .unwrap();
        world.step(TickInput::new(100)).unwrap();
        let state = world.vehicle(subject).unwrap();
        outcomes.push((
            state.route_edge_index(),
            state.progress_mm(),
            state.speed_mm_s(),
            state.carry_um(),
        ));
    }
    assert_eq!(
        outcomes[0], outcomes[1],
        "空闲 Conflict 声明与无声明同几何同拍逐字段一致"
    );
    assert!(
        outcomes[0].2 > 9_000,
        "首拍不因空闲资源提前紧急制动：{:?}",
        outcomes[0]
    );
}

/// F2' 复审修锚：owner 持有共同 Conflict 的预约后，subject 从另一条路线
/// 10 m/s 接近（初始门距 ≈ 13 m，紧急 4.5 m/s² 刹停 11.1 m 放得进）。
/// 已知阻塞在制动窗内即形成停止目标——首拍即减速；未取得预约期间全程
/// 保持可停（v²/(2·b_e) ≤ 剩余门距，不再出现评审反例「一拍内从可停变
/// 不可停、空间差额 748.7 mm」）、不越门，最终停在门线前等待或等来预约。
#[test]
fn held_conflict_brakes_in_advance_and_never_becomes_unstoppable() {
    let revision =
        compile_road_editing_revision(conflict_road_editing_module_with_shape_and_speed(
            2,
            false,
            true,
            false,
            13.0,
            ConflictPolicyFixture::default(),
        ));
    let mut world = install_fixture(
        revision,
        WorldConfig::try_new(laneflow_runtime::WorldConfigInput {
            vehicle_capacity: 4,
            route_capacity: 4,
            route_edge_occurrence_capacity: 64,
            route_conflict_occurrence_capacity: 4,
            fixed_delta_time_ms: 100,
        })
        .expect("有效的交通配置"),
    )
    .unwrap();
    let [route, conflicting] = right_turn_routes(&mut world);
    // owner：沿主路线公开 spawn + step，直至持有共同 Conflict 的预约。
    let owner = world
        .spawn_vehicle(
            VehicleSpawnInput::new(VehicleProfileOrdinal::from_raw(0), route, 0, 0, 10_000)
                .with_open_entrance(),
        )
        .unwrap();
    let mut owner_holds = false;
    for _ in 0..40 {
        world.step(TickInput::new(100)).unwrap();
        if world.conflict_reservation(owner).is_some() {
            owner_holds = true;
            break;
        }
    }
    assert!(owner_holds, "owner 必须取得共同 Conflict 的预约");
    let gate_line = calibration_gate(&world, conflicting);
    let subject = world
        .place_existing_active_vehicle(
            VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                conflicting,
                0,
                0,
                10_000,
            )
            .with_open_entrance(),
        )
        .unwrap();
    // 紧急减速度按 fixture profile 实读（R4 收紧：本 fixture car profile
    // 为 4.0 m/s²，完整停车距离（毫米）= v² / (2·b_e·1000)）。
    let emergency = world
        .traffic()
        .relations()
        .vehicle_profile(VehicleProfileOrdinal::from_raw(0))
        .unwrap()
        .emergency_decel();
    let twice_emergency_mm = (2.0 * emergency * 1_000.0) as u64;
    let mut slowed_on_first_tick = false;
    let mut ended_waiting_or_granted = false;
    for _ in 0..400 {
        let before = world.vehicle(subject).unwrap();
        // 阻塞事实仍成立（owner 仍持有）期间才要求可停不变量；源事实
        // 释放后停止目标随之失效（合同 §3.1.1），车辆按 IIDM 恢复。
        let owner_holds = world.conflict_reservation(owner).is_some();
        world.step(TickInput::new(100)).unwrap();
        let after = world.vehicle(subject).unwrap();
        if world.conflict_reservation(subject).is_none() {
            assert_eq!(after.route_edge_index(), 0, "未授权期间不越门");
            assert!(after.progress_mm() <= gate_line, "不越过准入门线");
            if owner_holds {
                // 已知阻塞仍成立期间停止目标持续参与速度归约：拍初速度的
                // 紧急停车距离必须放进拍初剩余距离（评审反例「一拍内从
                // 可停变不可停、空间差额 748.7 mm」不得复现）。
                let stopping_mm = u64::from(before.speed_mm_s()) * u64::from(before.speed_mm_s())
                    / twice_emergency_mm;
                assert!(
                    stopping_mm <= u64::from(gate_line - before.progress_mm()),
                    "阻塞期间可停不变量：拍初 v²/(2·b_e)={stopping_mm} mm ≤ 拍初剩余 {} mm",
                    gate_line - before.progress_mm()
                );
            }
        }
        if !slowed_on_first_tick {
            assert!(
                after.speed_mm_s() < before.speed_mm_s(),
                "已知阻塞在制动窗内：首拍即减速（{} → {}）",
                before.speed_mm_s(),
                after.speed_mm_s()
            );
            slowed_on_first_tick = true;
        }
        if after.speed_mm_s() == 0 || world.conflict_reservation(subject).is_some() {
            ended_waiting_or_granted = true;
            break;
        }
    }
    assert!(slowed_on_first_tick);
    assert!(ended_waiting_or_granted, "subject 停在门线前等待或等来预约");
}

#[test]
fn resource_free_decisions_follow_final_motion_after_new_or_held_reservation() {
    for speed in [6_500, 13_000] {
        let revision =
            compile_road_editing_revision(conflict_road_editing_module_with_shape_and_speed(
                2,
                false,
                true,
                false,
                13.0,
                ConflictPolicyFixture {
                    resource_free_release: true,
                    ..Default::default()
                },
            ));
        let mut world = install_fixture(
            revision,
            WorldConfig::try_new(laneflow_runtime::WorldConfigInput {
                vehicle_capacity: 4,
                route_capacity: 4,
                route_edge_occurrence_capacity: 64,
                route_conflict_occurrence_capacity: 4,
                fixed_delta_time_ms: 1_000,
            })
            .expect("有效的交通配置"),
        )
        .unwrap();
        let [route, _] = right_turn_routes(&mut world);
        let subject = world
            .spawn_vehicle(
                VehicleSpawnInput::new(
                    VehicleProfileOrdinal::from_raw(0),
                    route,
                    0,
                    calibration_gate(&world, route),
                    speed,
                )
                .with_open_entrance(),
            )
            .unwrap();
        world.step(TickInput::new(1_000)).unwrap();
        if speed == 6_500 {
            assert!(world.conflict_reservation(subject).is_some());
            assert_eq!(world.vehicle(subject).unwrap().route_edge_index(), 1);
            world.step(TickInput::new(1_000)).unwrap();
        }
        // #1272：13 m/s 起步在多拍内为停止/路终目标减速，hop-2 的到达与
        // 资源释放决定跨拍收束——逐拍推进直到到达或预算耗尽。
        let mut reached = false;
        for _ in 0..12 {
            let decided = world.latest_conflict_decisions().iter().any(|decision| {
                decision.vehicle() == subject
                    && decision.anchor().hop() == 1
                    && decision.outcome() == ConflictDecisionOutcome::NotRequired
            });
            let at_hop2 = world.vehicle(subject).unwrap().route_edge_index() == 2;
            if decided && at_hop2 {
                reached = true;
                break;
            }
            world.step(TickInput::new(1_000)).unwrap();
        }
        assert!(
            reached,
            "initial speed {speed}; state {:?}; decisions {:?}",
            world.vehicle(subject),
            world.latest_conflict_decisions()
        );
    }
}

#[test]
fn rejected_resource_gate_does_not_report_unreached_resource_free_gate() {
    let revision =
        compile_road_editing_revision(conflict_road_editing_module_with_shape_and_speed(
            2,
            false,
            true,
            false,
            13.0,
            ConflictPolicyFixture {
                resource_free_release: true,
                ..Default::default()
            },
        ));
    let mut world = install_fixture(
        revision,
        WorldConfig::try_new(laneflow_runtime::WorldConfigInput {
            vehicle_capacity: 4,
            route_capacity: 4,
            route_edge_occurrence_capacity: 64,
            route_conflict_occurrence_capacity: 4,
            fixed_delta_time_ms: 1_000,
        })
        .expect("有效的交通配置"),
    )
    .unwrap();
    let [route, conflicting_route] = right_turn_routes(&mut world);
    at_gate(&mut world, conflicting_route);
    let subject = world
        .place_existing_active_vehicle(
            VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                0,
                calibration_gate(&world, route),
                13_000,
            )
            .with_open_entrance(),
        )
        .unwrap();
    world.step(TickInput::new(1_000)).unwrap();
    let decisions: Vec<_> = world
        .latest_conflict_decisions()
        .iter()
        .filter(|decision| decision.vehicle() == subject)
        .map(|decision| (decision.anchor().hop(), decision.outcome()))
        .collect();
    assert_eq!(
        decisions,
        vec![(
            0,
            ConflictDecisionOutcome::NoGrant(ConflictNoGrantReason::ConflictOccupied)
        )]
    );
}

#[test]
fn waiting_capacity_denial_is_observable_without_claiming_following_conflict() {
    let revision =
        compile_road_editing_revision(conflict_road_editing_module_with_shape_and_speed(
            2,
            false,
            true,
            false,
            13.0,
            ConflictPolicyFixture {
                waiting: true,
                waiting_on_north_only: true,
                conflict_after_release: true,
                include_long_vehicle: true,
                ..Default::default()
            },
        ));
    let long = calibration_profile(&revision, "long-vehicle");
    let short = calibration_profile(&revision, "car");
    let mut world = install_fixture(
        revision,
        WorldConfig::try_new(laneflow_runtime::WorldConfigInput {
            vehicle_capacity: 4,
            route_capacity: 4,
            route_edge_occurrence_capacity: 64,
            route_conflict_occurrence_capacity: 4,
            fixed_delta_time_ms: 100,
        })
        .expect("有效的交通配置"),
    )
    .unwrap();
    let [east, north] = right_turn_routes(&mut world);
    let leader = world
        .spawn_vehicle(
            VehicleSpawnInput::new(short, north, 0, calibration_gate(&world, north), 10_000)
                .with_open_entrance(),
        )
        .unwrap();
    for _ in 0..20 {
        world.step(TickInput::new(100)).unwrap();
        if world.vehicle(leader).unwrap().route_edge_index() == 1
            && world.vehicle(leader).unwrap().progress_mm() >= 8_000
        {
            break;
        }
    }
    assert!(
        world
            .vehicle(leader)
            .unwrap()
            .waiting_membership()
            .is_some()
    );
    assert!(world.conflict_reservation(leader).is_none());
    let waiting = world
        .place_existing_active_vehicle(
            VehicleSpawnInput::new(long, north, 0, calibration_gate(&world, north), 10_000)
                .with_open_entrance(),
        )
        .unwrap();
    let ready = world
        .spawn_vehicle(VehicleSpawnInput::new(short, east, 1, 13_000, 10_000).with_open_entrance())
        .unwrap();
    for tick in 0..3 {
        world.step(TickInput::new(100)).unwrap();
        assert_eq!(
            calibration_outcome(&world, waiting),
            ConflictDecisionOutcome::NoGrant(ConflictNoGrantReason::WaitingCapacity)
        );
        assert!(world.conflict_reservation(waiting).is_none());
        if tick == 0 {
            assert_eq!(
                calibration_outcome(&world, ready),
                ConflictDecisionOutcome::Granted
            );
        }
    }
}

fn right_turn_revision(
    interpretation: GateInterpretation,
    deny: bool,
) -> Arc<SharedNetworkRevision> {
    compile_road_editing_revision(conflict_road_editing_module_with_shape_and_speed(
        2,
        false,
        true,
        false,
        13.0,
        ConflictPolicyFixture {
            yielding: true,
            right_turn_signal: Some(interpretation),
            deny,
            ..Default::default()
        },
    ))
}

fn right_turn_routes(world: &mut TrafficWorld) -> [RouteHandle; 2] {
    [
        ["east-entry", "east-internal", "west-exit"],
        ["north-entry", "north-internal", "south-exit"],
    ]
    .map(|edges| register_conflict_route(world, &edges))
}

fn register_conflict_route(world: &mut TrafficWorld, keys: &[&str]) -> RouteHandle {
    let revision = world.revision();
    let edges: Vec<_> = keys
        .iter()
        .map(|key| {
            let id =
                derive_canonical_stable_id_v1(EntityKind::LaneEdge, "city/runtime-conflict", key)
                    .unwrap();
            revision
                .identity()
                .ordinal(LaneEdgeId::from_untyped(id))
                .unwrap()
        })
        .collect();
    world
        .register_route(RouteRegisterInput::new(edges))
        .unwrap()
}

fn at_gate(world: &mut TrafficWorld, route: RouteHandle) -> VehicleHandle {
    world
        .place_existing_active_vehicle(
            VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                0,
                calibration_gate(world, route),
                10_000,
            )
            .with_open_entrance(),
        )
        .unwrap()
}

#[test]
fn formal_right_turn_red_uses_compiled_policy_and_still_yields() {
    for (interpretation, deny, priority_approach, expected) in [
        (
            GateInterpretation::CnCircularRightTurn,
            false,
            true,
            ConflictDecisionOutcome::NoGrant(ConflictNoGrantReason::LeadGap),
        ),
        (
            GateInterpretation::CnCircularRightTurn,
            false,
            false,
            ConflictDecisionOutcome::Granted,
        ),
        (
            GateInterpretation::DirectionalRightPermissive,
            false,
            false,
            ConflictDecisionOutcome::NotEvaluated,
        ),
        (
            GateInterpretation::CnCircularRightTurn,
            true,
            false,
            ConflictDecisionOutcome::NotEvaluated,
        ),
    ] {
        let revision = right_turn_revision(interpretation, deny);
        let mut world = install_fixture(
            revision,
            WorldConfig::try_new(laneflow_runtime::WorldConfigInput {
                vehicle_capacity: 4,
                route_capacity: 4,
                route_edge_occurrence_capacity: 64,
                route_conflict_occurrence_capacity: 4,
                fixed_delta_time_ms: 100,
            })
            .expect("有效的交通配置"),
        )
        .unwrap();
        let [right_turn, priority_route] = right_turn_routes(&mut world);
        let subject = at_gate(&mut world, right_turn);
        if priority_approach {
            // 已经在路上的优先车。新生成会因右转车在门前停不住而拒绝。
            world
                .place_existing_active_vehicle(
                    VehicleSpawnInput::new(
                        VehicleProfileOrdinal::from_raw(0),
                        priority_route,
                        0,
                        calibration_gate(&world, priority_route) - 2_000,
                        10_000,
                    )
                    .with_open_entrance(),
                )
                .unwrap();
        }
        let signals = world.committed_signal_groups();
        assert_eq!(signals.as_slice().len(), 1);
        assert_eq!(signals.as_slice()[0].1, SignalAspect::Red);
        world.step(TickInput::new(100)).unwrap();
        assert_eq!(calibration_outcome(&world, subject), expected);
        let granted = expected == ConflictDecisionOutcome::Granted;
        assert_eq!(world.conflict_reservation(subject).is_some(), granted);
        assert_eq!(
            world.vehicle(subject).unwrap().route_edge_index(),
            u32::from(granted)
        );
        if !granted {
            assert!(
                world
                    .latest_transition_events()
                    .iter()
                    .all(|event| event.vehicle() != subject)
            );
        }
    }
}

fn alternating_signal_world(interpretation: GateInterpretation) -> TrafficWorld {
    let revision =
        compile_road_editing_revision(conflict_road_editing_module_with_shape_and_speed(
            2,
            false,
            true,
            false,
            13.0,
            ConflictPolicyFixture {
                yielding: true,
                right_turn_signal: Some(interpretation),
                signal_cycle_ms: Some([100, 10_000]),
                ..Default::default()
            },
        ));
    install_fixture(
        revision,
        WorldConfig::try_new(laneflow_runtime::WorldConfigInput {
            vehicle_capacity: 4,
            route_capacity: 4,
            route_edge_occurrence_capacity: 64,
            route_conflict_occurrence_capacity: 4,
            fixed_delta_time_ms: 100,
        })
        .expect("有效的交通配置"),
    )
    .unwrap()
}

#[test]
fn protected_green_skips_red_approach_gap_but_permissive_keeps_it() {
    for (interpretation, expected) in [
        (
            GateInterpretation::ProtectedGroup,
            ConflictDecisionOutcome::Granted,
        ),
        (
            GateInterpretation::DirectionalRightProtected,
            ConflictDecisionOutcome::Granted,
        ),
        (
            GateInterpretation::PermissiveGroup,
            ConflictDecisionOutcome::NoGrant(ConflictNoGrantReason::LeadGap),
        ),
    ] {
        let mut world = alternating_signal_world(interpretation);
        world.step(TickInput::new(100)).unwrap();
        let [subject_route, priority_route] = right_turn_routes(&mut world);
        let subject = at_gate(&mut world, subject_route);
        let target = world
            .place_existing_active_vehicle(
                VehicleSpawnInput::new(
                    VehicleProfileOrdinal::from_raw(0),
                    priority_route,
                    0,
                    calibration_gate(&world, priority_route) - 2_000,
                    10_000,
                )
                .with_open_entrance(),
            )
            .unwrap();
        world.step(TickInput::new(100)).unwrap();
        assert_eq!(
            calibration_outcome(&world, subject),
            expected,
            "{interpretation:?}"
        );
        let granted = expected == ConflictDecisionOutcome::Granted;
        assert_eq!(world.conflict_reservation(subject).is_some(), granted);
        assert_eq!(
            world.vehicle(subject).unwrap().route_edge_index(),
            u32::from(granted)
        );
        assert!(world.conflict_reservation(target).is_none());
        assert_eq!(world.vehicle(target).unwrap().route_edge_index(), 0);
    }
}

#[test]
fn protected_green_waits_for_reservation_and_occupancy_but_skips_clear_lag() {
    for (interpretation, expected) in [
        (
            GateInterpretation::ProtectedGroup,
            ConflictDecisionOutcome::Granted,
        ),
        (
            GateInterpretation::DirectionalRightProtected,
            ConflictDecisionOutcome::Granted,
        ),
        (
            GateInterpretation::PermissiveGroup,
            ConflictDecisionOutcome::NoGrant(ConflictNoGrantReason::LagGap),
        ),
    ] {
        let mut world = alternating_signal_world(interpretation);
        let [subject_route, priority_route] = right_turn_routes(&mut world);
        let target = at_gate(&mut world, priority_route);
        world.step(TickInput::new(100)).unwrap();
        assert!(world.conflict_reservation(target).is_some());
        assert!(
            world.latest_transition_events().iter().all(|event| {
                !matches!(event.kind(), TrafficTransitionKind::ConflictEntered { .. })
            }),
            "the first green tick reserves the passage before physical entry"
        );
        let subject = at_gate(&mut world, subject_route);
        let mut entered = false;
        let mut rejected_while_occupied = false;
        for _ in 0..30 {
            world.step(TickInput::new(100)).unwrap();
            assert_eq!(
                calibration_outcome(&world, subject),
                ConflictDecisionOutcome::NoGrant(ConflictNoGrantReason::ConflictOccupied),
                "{interpretation:?} must preserve reservation and occupancy exclusion",
            );
            assert!(world.conflict_reservation(subject).is_none());
            rejected_while_occupied |= entered;
            entered |= world.latest_transition_events().iter().any(|event| {
                event.vehicle() == target
                    && matches!(event.kind(), TrafficTransitionKind::ConflictEntered { .. })
            });
            if world.conflict_reservation(target).is_none() {
                break;
            }
        }
        assert!(entered && rejected_while_occupied);
        assert!(world.conflict_reservation(target).is_none());
        assert!(world.latest_transition_events().iter().any(|event| {
            event.vehicle() == target
                && matches!(event.kind(), TrafficTransitionKind::ConflictCleared { .. })
        }));
        let bytes = encode_lfrs(&world.capture_snapshot().unwrap());
        let snapshot = snapshot_wire::size_prefixed_root_as_runtime_snapshot(&bytes).unwrap();
        assert!(snapshot.conflict_lag_states().iter().any(|row| {
            row.reference_kind() == snapshot_wire::ConflictLagReferenceKind::ActualClear
                && row.reference_time_ms() == world.time_ms()
        }));
        world.step(TickInput::new(100)).unwrap();
        assert_eq!(
            calibration_outcome(&world, subject),
            expected,
            "{interpretation:?}"
        );
        assert_eq!(
            world.conflict_reservation(subject).is_some(),
            expected == ConflictDecisionOutcome::Granted,
        );
    }
}

#[test]
fn protected_green_still_requires_downstream_storage() {
    for interpretation in [
        GateInterpretation::ProtectedGroup,
        GateInterpretation::DirectionalRightProtected,
    ] {
        let mut world = alternating_signal_world(interpretation);
        world.step(TickInput::new(100)).unwrap();
        let [route, _] = right_turn_routes(&mut world);
        world
            .spawn_vehicle(
                VehicleSpawnInput::new(VehicleProfileOrdinal::from_raw(0), route, 1, 10_501, 0)
                    .with_open_entrance(),
            )
            .unwrap();
        let subject = at_gate(&mut world, route);
        world.step(TickInput::new(100)).unwrap();
        assert_eq!(
            calibration_outcome(&world, subject),
            ConflictDecisionOutcome::NoGrant(ConflictNoGrantReason::DownstreamStorageBoundary),
            "{interpretation:?}",
        );
        assert!(world.conflict_reservation(subject).is_none());
        assert_eq!(world.vehicle(subject).unwrap().route_edge_index(), 0);
        assert!(
            world
                .latest_transition_events()
                .iter()
                .all(|event| event.vehicle() != subject)
        );
    }
}

fn added_conflict_floor_world(old_vehicle: bool) -> TrafficWorld {
    let module = |include_conflict| {
        conflict_road_editing_module_with_shape_and_speed(
            2,
            false,
            include_conflict,
            false,
            13.0,
            ConflictPolicyFixture {
                yielding: true,
                ..Default::default()
            },
        )
    };
    let (base, target, diff, binding) = compile_conflict_cutover_pair(module(false), module(true));
    let mut world = install_fixture(
        Arc::clone(&base),
        WorldConfig::try_new(laneflow_runtime::WorldConfigInput {
            vehicle_capacity: 4,
            route_capacity: 4,
            route_edge_occurrence_capacity: 64,
            route_conflict_occurrence_capacity: 4,
            fixed_delta_time_ms: 4,
        })
        .expect("有效的交通配置"),
    )
    .unwrap();
    if old_vehicle {
        let route =
            register_conflict_route(&mut world, &["north-entry", "north-internal", "south-exit"]);
        let old = world
            .spawn_vehicle(
                VehicleSpawnInput::new(
                    VehicleProfileOrdinal::from_raw(0),
                    route,
                    1,
                    10_000,
                    10_000,
                )
                .with_open_entrance(),
            )
            .unwrap();
        world.step(TickInput::new(4)).unwrap();
        let state = world.vehicle(old).unwrap();
        assert!(
            state.progress_mm() - 4_500 > 5_501,
            "old tail cleared the future passage"
        );
        world.despawn_vehicle(old).unwrap();
    }
    let descriptor = NetworkRevisionCutoverDescriptor::new(
        LfcaOriginBinding::from_canonical_origin(*base.canonical_origin()),
        LfcaOriginBinding::from_canonical_origin(*target.canonical_origin()),
        Some(binding),
        MigrationPolicyKind::CrossRevisionDirect,
        world.world_binding(),
    );
    let transaction = world
        .prepare_cross_revision_cutover(
            Arc::clone(&target),
            published_source(&target, "fixture://policy-acceptance-floor"),
            &descriptor,
            &diff,
            &CutoverPreflightLimits::new(1_048_576),
            &CutoverTransactionLimits::default(),
        )
        .unwrap();
    if old_vehicle {
        for _ in 0..25 {
            world.step(TickInput::new(4)).unwrap();
        }
    }
    let commit = transaction.commit(&mut world).unwrap();
    assert_eq!(
        commit.events.as_slice().len(),
        1,
        "only the cutover event is published"
    );
    assert!(
        world.latest_transition_events().is_empty(),
        "no fabricated clear event"
    );
    assert_eq!(world.time_ms(), if old_vehicle { 104 } else { 0 });
    world
}

#[test]
fn new_conflict_floor_survives_restore_and_enforces_496_500_ms_from_commit() {
    for old_vehicle in [false, true] {
        let mut original = added_conflict_floor_world(old_vehicle);
        let floor = original.time_ms();
        let bytes = encode_lfrs(&original.capture_snapshot().unwrap());
        let wire = snapshot_wire::size_prefixed_root_as_runtime_snapshot(&bytes).unwrap();
        assert_eq!(
            wire.conflict_lag_states().len(),
            original.conflict_passage_cell_count()
        );
        for row in wire.conflict_lag_states() {
            assert_eq!(
                row.reference_kind(),
                snapshot_wire::ConflictLagReferenceKind::CutoverFloor
            );
            assert_eq!(row.reference_time_ms(), floor);
        }
        let mut restored = restore_lfrs(
            &bytes,
            original.revision(),
            original.committed_source().clone(),
            original.config(),
            laneflow_runtime::ExecutionConfig::new(std::num::NonZeroU32::MIN),
            SnapshotRestoreLimits::new(1_048_576, 1_024),
        )
        .unwrap()
        .into_world();
        for world in [&mut original, &mut restored] {
            let route =
                register_conflict_route(world, &["east-entry", "east-internal", "west-exit"]);
            let subject = at_gate(world, route);
            // The supported minimum fixed tick is 4 ms. Exact 499/500 ms is
            // covered by the shared gap primitive; this formal path reaches 496/500.
            for elapsed in (0..500).step_by(4) {
                assert_eq!(world.time_ms(), floor + elapsed);
                world.step(TickInput::new(4)).unwrap();
                assert_eq!(
                    calibration_outcome(world, subject),
                    ConflictDecisionOutcome::NoGrant(ConflictNoGrantReason::LagGap),
                    "new cell must reject through 496 ms after commit"
                );
                assert!(world.conflict_reservation(subject).is_none());
                assert!(world.latest_transition_events().is_empty());
            }
            world.step(TickInput::new(4)).unwrap();
            assert_eq!(
                calibration_outcome(world, subject),
                ConflictDecisionOutcome::Granted
            );
        }
        assert_eq!(
            deterministic_state_digest(&original.capture_snapshot().unwrap()).unwrap(),
            deterministic_state_digest(&restored.capture_snapshot().unwrap()).unwrap(),
        );
    }
}

/// R4-4 复审修锚：P4 本拍明确 `NoGrant(LeadGap)` 而 cells 仍空闲时，该门
/// 合入 P5 停止目标——subject 以舒适解保持在停止线前（评审反例：旧实现
/// 把明确拒绝当纯位置屏障，到线清零 next_progress=门线、speed=0；按
/// `L=0` 目标舒适域存在正常解，最大可行终速约 73 mm/s、行程约 13.7 mm）。
#[test]
fn explicit_leadgap_denial_brakes_to_a_stop_target_before_the_line() {
    let revision = right_turn_revision(GateInterpretation::CnCircularRightTurn, false);
    let mut world = install_fixture(
        revision,
        WorldConfig::try_new(laneflow_runtime::WorldConfigInput {
            vehicle_capacity: 4,
            route_capacity: 4,
            route_edge_occurrence_capacity: 64,
            route_conflict_occurrence_capacity: 4,
            fixed_delta_time_ms: 100,
        })
        .expect("有效的交通配置"),
    )
    .unwrap();
    let [right_turn, priority_route] = right_turn_routes(&mut world);
    let gate = calibration_gate(&world, right_turn);
    let subject = world
        .place_existing_active_vehicle(
            VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                right_turn,
                0,
                gate - 15,
                200,
            )
            .with_open_entrance(),
        )
        .unwrap();
    world
        .place_existing_active_vehicle(
            VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                priority_route,
                0,
                calibration_gate(&world, priority_route) - 2_000,
                10_000,
            )
            .with_open_entrance(),
        )
        .unwrap();
    world.step(TickInput::new(100)).unwrap();
    assert_eq!(
        calibration_outcome(&world, subject),
        ConflictDecisionOutcome::NoGrant(ConflictNoGrantReason::LeadGap)
    );
    assert!(
        world.conflict_reservation(subject).is_none(),
        "cells 仍空闲（拒绝来自让行判据，非占用）"
    );
    let state = world.vehicle(subject).unwrap();
    assert!(
        state.progress_mm() < gate,
        "明确拒绝作为停止目标：不到线清零（progress {} < 门线 {}）",
        state.progress_mm(),
        gate
    );
    assert!(
        state.speed_mm_s() > 0,
        "舒适域解保留线前速度（{} mm/s），不再一拍归零",
        state.speed_mm_s()
    );
}
