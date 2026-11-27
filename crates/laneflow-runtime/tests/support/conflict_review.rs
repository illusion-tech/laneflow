use super::*;

fn revision(options: ConflictPolicyFixture) -> Arc<SharedNetworkRevision> {
    compile_road_editing_revision(conflict_road_editing_module_with_shape_and_speed(
        2, false, true, false, 13.0, options,
    ))
}

fn routes(world: &mut TrafficWorld, revision: &SharedNetworkRevision) -> [RouteHandle; 2] {
    [0, 1].map(|index| {
        let stream = revision
            .conflict()
            .participant_stream(ParticipantStreamOrdinal::from_raw(index))
            .unwrap();
        let edges = revision
            .traffic()
            .maneuvers()
            .maneuver_path(stream.maneuver_path())
            .unwrap()
            .edges()
            .to_vec();
        world
            .register_route(RouteRegisterInput::new(edges))
            .unwrap()
    })
}

fn spawn(world: &mut TrafficWorld, route: RouteHandle) -> VehicleHandle {
    let boundary = calibration_gate(world, route);
    world
        .place_existing_active_vehicle(
            VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                0,
                boundary - 1,
                10_000,
            )
            .with_open_entrance(),
        )
        .unwrap()
}

#[test]
fn fresh_spawn_rejects_the_one_millimetre_red_approach() {
    let revision = revision(ConflictPolicyFixture {
        deny: true,
        ..Default::default()
    });
    let mut world = install_fixture(
        Arc::clone(&revision),
        WorldConfig::try_new(laneflow_runtime::WorldConfigInput {
            vehicle_capacity: 4,
            route_capacity: 4,
            route_edge_occurrence_capacity: 64,
            route_conflict_occurrence_capacity: 8,
            fixed_delta_time_ms: 100,
        })
        .expect("有效的交通配置"),
    )
    .unwrap();
    let [route, _] = routes(&mut world, &revision);
    let boundary = calibration_gate(&world, route);
    assert_eq!(
        world
            .spawn_vehicle(
                VehicleSpawnInput::new(
                    VehicleProfileOrdinal::from_raw(0),
                    route,
                    0,
                    boundary - 1,
                    10_000,
                )
                .with_open_entrance()
            )
            .unwrap_err(),
        SpawnError::StopConstraintUnsatisfiable
    );
    assert!(world.live_vehicles().is_empty());
}

#[test]
fn regulatory_denial_keeps_resource_evaluation_absent_and_gate_closed() {
    let revision = revision(ConflictPolicyFixture {
        deny: true,
        ..Default::default()
    });
    let mut world = install_fixture(
        Arc::clone(&revision),
        WorldConfig::try_new(laneflow_runtime::WorldConfigInput {
            vehicle_capacity: 4,
            route_capacity: 4,
            route_edge_occurrence_capacity: 64,
            route_conflict_occurrence_capacity: 8,
            fixed_delta_time_ms: 100,
        })
        .expect("有效的交通配置"),
    )
    .unwrap();
    let [route, _] = routes(&mut world, &revision);
    let vehicle = spawn(&mut world, route);
    world.step(TickInput::new(100)).unwrap();
    assert_eq!(
        calibration_outcome(&world, vehicle),
        laneflow_runtime::ConflictDecisionOutcome::NotEvaluated
    );
    assert_eq!(world.vehicle(vehicle).unwrap().route_edge_index(), 0);
    assert!(world.conflict_reservation(vehicle).is_none());
    assert!(world.latest_transition_events().is_empty());
}

#[test]
fn waiting_entry_cannot_share_conflict_coverage_in_a_compiled_network() {
    let module = conflict_road_editing_module_with_shape_and_speed(
        2,
        false,
        true,
        false,
        13.0,
        ConflictPolicyFixture {
            waiting: true,
            ..Default::default()
        },
    );
    let limits = CompileProfile::single_network_10k_stable_entities();
    let source = lfre::RoadEditingSourceWriter::new(&limits)
        .write(module)
        .unwrap();
    let input =
        lfre::RoadEditingModuleInput::try_new("runtime-conflict.lfre", source.as_bytes(), None)
            .unwrap();
    let mut unit = CompilationUnitBuilder::new(limits);
    unit.add_road_editing_module(input).unwrap();
    let error = Compiler::new()
        .compile(unit.build().unwrap())
        .err()
        .expect("overlap must fail compilation");
    assert!(error.diagnostics().iter().any(|diagnostic| {
        format!("{:?}", diagnostic.payload())
            .contains("participantStream.passages.waitingZoneOverlap")
    }));
}

#[test]
fn tail_clearance_and_route_completion_commit_in_the_same_tick() {
    let revision = revision(ConflictPolicyFixture {
        clearance: Some((2, 8.5)),
        ..Default::default()
    });
    let mut world = install_fixture(
        Arc::clone(&revision),
        WorldConfig::try_new(laneflow_runtime::WorldConfigInput {
            vehicle_capacity: 4,
            route_capacity: 4,
            route_edge_occurrence_capacity: 64,
            route_conflict_occurrence_capacity: 8,
            fixed_delta_time_ms: 100,
        })
        .expect("有效的交通配置"),
    )
    .unwrap();
    let [route, _] = routes(&mut world, &revision);
    let vehicle = spawn(&mut world, route);
    world.step(TickInput::new(100)).unwrap();
    assert!(world.conflict_reservation(vehicle).is_some());
    let decisions = world.latest_conflict_decisions().to_vec();
    let origin = LfcaOriginBinding::from_canonical_origin(*revision.canonical_origin());
    let descriptor = NetworkRevisionCutoverDescriptor::new(
        origin,
        origin,
        None,
        MigrationPolicyKind::SameRevisionRestore,
        world.world_binding(),
    );
    let _events = world
        .cutover_same_revision(
            Arc::clone(&revision),
            world.committed_source().clone(),
            &descriptor,
            &CutoverPreflightLimits::new(1_048_576),
        )
        .unwrap();
    assert_eq!(world.latest_conflict_decisions(), decisions);
    let restored = restore_lfrs(
        &encode_lfrs(&world.capture_snapshot().unwrap()),
        revision,
        world.committed_source().clone(),
        WorldConfig::try_new(laneflow_runtime::WorldConfigInput {
            vehicle_capacity: 4,
            route_capacity: 4,
            route_edge_occurrence_capacity: 64,
            route_conflict_occurrence_capacity: 8,
            fixed_delta_time_ms: 100,
        })
        .expect("有效的交通配置"),
        laneflow_runtime::ExecutionConfig::new(std::num::NonZeroU32::MIN),
        SnapshotRestoreLimits::new(1_048_576, 1_024),
    )
    .unwrap();
    let mut restored = restored.into_world();
    // #1272：路终/清空目标提前制动，完成时点后移；预算放宽到 160 拍。
    for _ in 0..160 {
        world
            .step(TickInput::new(100))
            .expect("tail clearance at RouteEnd must commit");
        restored
            .step(TickInput::new(100))
            .expect("restored tail clearance must commit");
        assert_eq!(
            deterministic_state_digest(&world.capture_snapshot().unwrap()).unwrap(),
            deterministic_state_digest(&restored.capture_snapshot().unwrap()).unwrap(),
        );
    }
    assert_eq!(
        world.vehicle(vehicle).unwrap().status(),
        VehicleStatus::Completed
    );
    assert!(world.conflict_reservation(vehicle).is_none());
}

#[test]
fn clearance_target_equal_to_next_gate_is_admitted() {
    let revision = revision(ConflictPolicyFixture {
        next_gate: true,
        clearance: Some((1, 8.5)),
        ..Default::default()
    });
    let mut world = install_fixture(
        Arc::clone(&revision),
        WorldConfig::try_new(laneflow_runtime::WorldConfigInput {
            vehicle_capacity: 4,
            route_capacity: 4,
            route_edge_occurrence_capacity: 64,
            route_conflict_occurrence_capacity: 8,
            fixed_delta_time_ms: 100,
        })
        .expect("有效的交通配置"),
    )
    .unwrap();
    let [route, _] = routes(&mut world, &revision);
    let vehicle = spawn(&mut world, route);
    world.step(TickInput::new(100)).unwrap();
    assert_eq!(
        calibration_outcome(&world, vehicle),
        laneflow_runtime::ConflictDecisionOutcome::Granted
    );
}

#[test]
fn reserved_vehicle_cannot_cross_later_conflict_gate_without_authority() {
    let revision = revision(ConflictPolicyFixture {
        next_gate: true,
        clearance: Some((1, 8.5)),
        ..Default::default()
    });
    let mut world = install_fixture(
        Arc::clone(&revision),
        WorldConfig::try_new(laneflow_runtime::WorldConfigInput {
            vehicle_capacity: 4,
            route_capacity: 4,
            route_edge_occurrence_capacity: 64,
            route_conflict_occurrence_capacity: 8,
            fixed_delta_time_ms: 100,
        })
        .expect("有效的交通配置"),
    )
    .unwrap();
    let [route, _] = routes(&mut world, &revision);
    let vehicle = spawn(&mut world, route);
    let mut old_released_at_boundary = false;
    for _ in 0..80 {
        world.step(TickInput::new(100)).unwrap();
        let state = world.vehicle(vehicle).unwrap();
        if state.route_edge_index() == 1 && world.conflict_reservation(vehicle).is_none() {
            old_released_at_boundary = true;
        }
        if state.route_edge_index() > 1 {
            assert!(
                old_released_at_boundary,
                "old clearance and a fresh next-tick arbitration are required"
            );
            assert_eq!(
                world
                    .conflict_reservation(vehicle)
                    .unwrap()
                    .admission_gate_hop(),
                1,
                "the old reservation cannot authorize the later Gate"
            );
            return;
        }
    }
    panic!("later Gate was never reached");
}

#[test]
fn maneuver_completion_waits_for_tail_clearance() {
    let revision = revision(ConflictPolicyFixture {
        clearance: Some((2, 8.5)),
        ..Default::default()
    });
    let mut world = install_fixture(
        Arc::clone(&revision),
        WorldConfig::try_new(laneflow_runtime::WorldConfigInput {
            vehicle_capacity: 4,
            route_capacity: 4,
            route_edge_occurrence_capacity: 64,
            route_conflict_occurrence_capacity: 8,
            fixed_delta_time_ms: 100,
        })
        .expect("有效的交通配置"),
    )
    .unwrap();
    let [route, _] = routes(&mut world, &revision);
    let vehicle = spawn(&mut world, route);
    let mut completions = 0;
    let mut saw_clearing_after_exit = false;
    for _ in 0..80 {
        world.step(TickInput::new(100)).unwrap();
        saw_clearing_after_exit |= world.vehicle(vehicle).unwrap().route_edge_index() >= 2
            && world.conflict_reservation(vehicle).is_some();
        if world.latest_transition_events().iter().any(|event| {
            matches!(
                event.kind(),
                laneflow_runtime::TrafficTransitionKind::ManeuverTraversalCompleted { .. }
            )
        }) {
            assert!(
                world.conflict_reservation(vehicle).is_none(),
                "completion published before tail clearance"
            );
            completions += 1;
            let events = world.latest_transition_events();
            let release = events
                .iter()
                .position(|event| {
                    matches!(
                        event.kind(),
                        laneflow_runtime::TrafficTransitionKind::ReservationReleased { .. }
                    )
                })
                .unwrap();
            let completion = events
                .iter()
                .position(|event| {
                    matches!(
                        event.kind(),
                        laneflow_runtime::TrafficTransitionKind::ManeuverTraversalCompleted { .. }
                    )
                })
                .unwrap();
            assert!(release < completion);
            assert!(events[release].anchor().position() <= events[completion].anchor().position());
        }
    }
    assert!(saw_clearing_after_exit);
    assert_eq!(
        completions, 1,
        "completion must occur exactly once, after the last clear"
    );
}

#[test]
fn fresh_grant_still_stops_at_the_following_gate_in_the_same_tick() {
    let revision = revision(ConflictPolicyFixture {
        next_gate: true,
        clearance: Some((1, 8.5)),
        ..Default::default()
    });
    let mut world = install_fixture(
        Arc::clone(&revision),
        WorldConfig::try_new(laneflow_runtime::WorldConfigInput {
            vehicle_capacity: 4,
            route_capacity: 4,
            route_edge_occurrence_capacity: 64,
            route_conflict_occurrence_capacity: 8,
            fixed_delta_time_ms: 1_000,
        })
        .expect("有效的交通配置"),
    )
    .unwrap();
    let [route, _] = routes(&mut world, &revision);
    let boundary = calibration_gate(&world, route);
    // #1272：首拍预览按停止目标级联减速（紧急域内），13 m/s 到下一门约
    // 13 m、紧急刹停 9.4 m 放得下——准入成功，不再因「本拍够得到即急停」
    // 拒绝。停不住的更近摆放仍由 can_stop_before 拒绝（见上方 1 mm 用例）。
    let vehicle = world
        .spawn_vehicle(
            VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                0,
                boundary,
                13_000,
            )
            .with_open_entrance(),
        )
        .expect("stoppable before the following gate within the emergency envelope");
    world.step(TickInput::new(1_000)).unwrap();
    let state = world.vehicle(vehicle).unwrap();
    assert_eq!(state.route_edge_index(), 1);
    // #1272 紧急域制动 + F2' 复审修：下一门未授权且无已知阻塞，只是
    // 位置屏障（合同 §3.1.1），不再作为停止目标提前紧急制动——落点随
    // 驻留上界后移到 11_824 mm；「未过门、门线前驻留、预约保留」的性质
    // 断言全部保留（下方）。
    assert_eq!(state.progress_mm(), 11_824);
    assert!(state.speed_mm_s() > 0 || state.progress_mm() < 13_000);
    // #1272：车辆在下一门前 2 m 处以紧急域上界速度驻留（耗尽衰减），
    // 尚未过门——没有旧尾清空、没有 GateCrossed，预约随裁决生命周期保留。
    // 「同拍过首门后停在下一门前」的新语义由本断言固化；跨门发生在放行拍。
    assert!(
        world.conflict_reservation(vehicle).is_some(),
        "reservation holds while braking ahead of the next gate"
    );
    assert_eq!(
        world
            .latest_transition_events()
            .iter()
            .filter(|event| matches!(
                event.kind(),
                laneflow_runtime::TrafficTransitionKind::GateCrossed { .. }
            ))
            .count(),
        1
    );
}

#[test]
fn same_tick_acquire_enter_clear_release_and_complete_survive_empty_endpoints() {
    use laneflow_runtime::TrafficTransitionKind as Kind;
    let revision = revision(ConflictPolicyFixture::default());
    let mut world = install_fixture(
        Arc::clone(&revision),
        WorldConfig::try_new(laneflow_runtime::WorldConfigInput {
            vehicle_capacity: 4,
            route_capacity: 4,
            route_edge_occurrence_capacity: 64,
            route_conflict_occurrence_capacity: 8,
            fixed_delta_time_ms: 1_000,
        })
        .expect("有效的交通配置"),
    )
    .unwrap();
    let [route, _] = routes(&mut world, &revision);
    let boundary = calibration_gate(&world, route);
    let vehicle = world
        .spawn_vehicle(
            VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                0,
                boundary,
                13_000,
            )
            .with_open_entrance(),
        )
        .unwrap();
    assert!(world.conflict_reservation(vehicle).is_none());
    // #1272：13 m/s 起步在同拍内先为路终/清空目标减速，完整获取-进入-清空-
    // 释放-完成生命周期跨多拍收束；按事件序核验（顺序不变，时点后移）。
    let mut kinds: Vec<_> = Vec::new();
    for _ in 0..12 {
        assert!(world.conflict_reservation(vehicle).is_none());
        world.step(TickInput::new(1_000)).unwrap();
        let batch: Vec<_> = world
            .latest_transition_events()
            .iter()
            .map(|event| event.kind())
            .filter(|kind| !matches!(kind, Kind::GateCrossed { .. }))
            .collect();
        for kind in batch {
            if !kinds.contains(&kind) {
                kinds.push(kind);
            }
        }
        if kinds.len() == 5
            && matches!(
                kinds.as_slice(),
                [
                    Kind::ReservationAcquired { .. },
                    Kind::ConflictEntered { .. },
                    Kind::ConflictCleared { .. },
                    Kind::ReservationReleased { .. },
                    Kind::ManeuverTraversalCompleted { .. }
                ]
            )
        {
            break;
        }
    }
    assert!(
        matches!(
            kinds.as_slice(),
            [
                Kind::ReservationAcquired { .. },
                Kind::ConflictEntered { .. },
                Kind::ConflictCleared { .. },
                Kind::ReservationReleased { .. },
                Kind::ManeuverTraversalCompleted { .. }
            ]
        ),
        "{kinds:?}"
    );
}

fn world_with_crossing_far_gate_owner(
    internal_m: f64,
) -> (TrafficWorld, RouteHandle, VehicleHandle) {
    let revision = revision(ConflictPolicyFixture {
        next_gate: true,
        clearance: Some((1, 8.5)),
        close_internal_m: internal_m,
        ..Default::default()
    });
    let mut world = install_fixture(
        Arc::clone(&revision),
        WorldConfig::try_new(laneflow_runtime::WorldConfigInput {
            vehicle_capacity: 4,
            route_capacity: 4,
            route_edge_occurrence_capacity: 64,
            route_conflict_occurrence_capacity: 8,
            fixed_delta_time_ms: 100,
        })
        .expect("有效的交通配置"),
    )
    .unwrap();
    let [route, crossing] = routes(&mut world, &revision);
    let crossing_boundary = calibration_gate(&world, crossing);
    let owner = world
        .spawn_vehicle(
            VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                crossing,
                0,
                crossing_boundary,
                13_000,
            )
            .with_open_entrance(),
        )
        .unwrap();
    for _ in 0..160 {
        world.step(TickInput::new(100)).unwrap();
        if let Some(reservation) = world.conflict_reservation(owner)
            && reservation.passage_range().admission_gate_hop() == 1
        {
            break;
        }
    }
    let held = world
        .conflict_reservation(owner)
        .expect("交叉流必须持有 B")
        .passage_range();
    assert_eq!(held.admission_gate_hop(), 1);
    let occupied = world
        .conflict_passage_occurrence_locator(crossing, held.first_conflict_occurrence_index())
        .unwrap()
        .address();
    let far = world
        .conflict_passage_occurrence_locator(route, 1)
        .unwrap()
        .address();
    assert_eq!(occupied.static_zone().unwrap(), far.static_zone().unwrap());
    assert_ne!(occupied.stream(), far.stream());
    (world, route, owner)
}

/// R4-2 复审修锚：近处空闲入口只是位置屏障，不遮挡制动窗内更远的已知
/// 阻塞——owner 持有后门（门 1）预约时，subject 在前门（门 0，空闲未
/// 授权）之外即开始减速；阻塞期间保持紧急可停（评审反例：旧实现的单一
/// 最近入口附带布尔，远门持有被近门空闲遮挡，一拍内从可停变不可停）。
#[test]
fn idle_near_gate_does_not_shadow_a_held_far_gate_stop_target() {
    let (mut world, route, owner) = world_with_crossing_far_gate_owner(13.0);
    let boundary = calibration_gate(&world, route);
    let far_line = boundary
        + world.traffic().lane_lengths_millimetres()[world.route_edges(route).unwrap()[1].index()];
    // subject 距门 0 约 12.3 m、10 m/s（门 1 在更远处被持有）。
    let subject = world
        .place_existing_active_vehicle(
            VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                0,
                boundary - 12_300,
                10_000,
            )
            .with_open_entrance(),
        )
        .unwrap();
    let mut slowed_on_first_tick = false;
    for _ in 0..200 {
        let before = world.vehicle(subject).unwrap();
        let owner_holds = world
            .conflict_reservation(owner)
            .is_some_and(|reservation| reservation.passage_range().admission_gate_hop() >= 1);
        world.step(TickInput::new(100)).unwrap();
        let after = world.vehicle(subject).unwrap();
        if !slowed_on_first_tick {
            assert!(
                after.speed_mm_s() < before.speed_mm_s(),
                "远门持有不被近门空闲遮挡：首拍即减速（{} → {}）",
                before.speed_mm_s(),
                after.speed_mm_s()
            );
            slowed_on_first_tick = true;
        }
        if owner_holds {
            // 阻塞期间紧急可停不变量：拍初速度对门 1 线（被持有的远门）的
            // 拍初剩余距离（评审反例对远门断言；近门空闲不要求可停）。
            let stopping = u64::from(before.speed_mm_s()) * u64::from(before.speed_mm_s()) / 8_000;
            let remaining = u64::from(far_line - before.progress_mm());
            assert!(
                stopping <= remaining,
                "阻塞期间对远门保持可停：需 {stopping} mm ≤ 剩余 {remaining} mm"
            );
        }
        if world.conflict_reservation(subject).is_some() {
            break;
        }
    }
    assert!(slowed_on_first_tick);
}

/// R6-1 复审修锚：申请预览必须纳入**其他**资源的拍初停止目标（合同
/// §3.1.1——申请预览仅排除本次待申请资源自身的停止目标）。评审反例：
/// subject 距近处 Conflict A 1 m、距被 owner 持有的远门 B 14 m、10 m/s；
/// 预览按无约束行程 ~1_004 mm 判定可达 A 而申请，正式运动纳入 B 的停止
/// 目标后只走 ~990 mm——grant 未消费却占住 staged claim，本可通行的
/// competitor 被 NoGrant(ConflictOccupied) 拒绝。修复后 subject 的申请
/// 预览同口径制动，本拍不申请 A，competitor 照常 Granted。
#[test]
fn application_preview_respects_other_resources_stop_targets() {
    // 内部边长 24 m，B 在 subject 前方 25 m 的制动窗内。交叉流占住 B，
    // 使它成为持续的停止目标，同时把同流跟入与 IIDM 前车制动排除在本算例外。
    let (mut world, route, _) = world_with_crossing_far_gate_owner(24.0);
    let boundary = calibration_gate(&world, route);
    // subject 距 A 1 m、距 B 25 m、10 m/s（评审反例参数同构）。
    let subject = world
        .place_existing_active_vehicle(
            VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                0,
                boundary - 1_000,
                10_000,
            )
            .with_open_entrance(),
        )
        .unwrap();
    world.step(TickInput::new(100)).unwrap();
    let after = world.vehicle(subject).unwrap();
    // B 的停止目标在制动窗内：正式运动本拍到不了 A。
    assert!(
        after.route_edge_index() == 0 && after.progress_mm() < boundary,
        "subject 本拍实际到不了 A（B 的停止目标参与制动）: boundary={boundary}, after={after:?}"
    );
    // 申请预览同口径制动（合同 §3.1.1）：本拍到不了 A 就不得申请——
    // 申请+授权而未消费正是评审反例中挤占本可通行车辆的 staged claim。
    assert!(
        world
            .latest_conflict_decisions()
            .iter()
            .all(|decision| decision.vehicle() != subject),
        "预览到不了 A：subject 本拍不得产生 A 的申请决定"
    );
    assert!(
        world.conflict_reservation(subject).is_none(),
        "subject 不得占用 A 的 staged claim"
    );
}

/// R5-2 复审修锚：空闲且可接纳的 Waiting 区不是停止目标——同几何 A/B
/// （无 Waiting 声明 vs 有），首拍提交必须逐字段一致（评审反例：旧口径
/// 对空区也计入候选车前间隙，5 m 区放 4.5 m 车被误判 6.5 m 需求而提前
/// 减速 9802 vs 10097）。
#[test]
fn admissible_empty_waiting_zone_does_not_brake() {
    let mut outcomes = Vec::new();
    // R6 收紧：对照组不得声明 Waiting（waiting||late_waiting 都会建区），
    // 用 (false,false) 作真无声明对照，(true,true) 保持原几何。
    for (waiting, late_waiting) in [(false, false), (true, true)] {
        let revision = revision(ConflictPolicyFixture {
            waiting,
            late_waiting,
            ..Default::default()
        });
        let mut world = install_fixture(
            Arc::clone(&revision),
            WorldConfig::try_new(laneflow_runtime::WorldConfigInput {
                vehicle_capacity: 4,
                route_capacity: 4,
                route_edge_occurrence_capacity: 64,
                route_conflict_occurrence_capacity: 8,
                fixed_delta_time_ms: 100,
            })
            .expect("有效的交通配置"),
        )
        .unwrap();
        let [route, _] = routes(&mut world, &revision);
        let boundary = calibration_gate(&world, route);
        let subject = world
            .place_existing_active_vehicle(
                VehicleSpawnInput::new(
                    VehicleProfileOrdinal::from_raw(0),
                    route,
                    0,
                    boundary - 13_000,
                    10_000,
                )
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
        "空闲可接纳的 Waiting 声明与无声明同几何同拍逐字段一致"
    );
}
