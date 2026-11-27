//! #1267 正式 LF-SYNTH-v2 输入上的候选准入三路径集成验收证据。
//!
//! 门控：`LANEFLOW_LF_SYNTH_INPUT_ROOT`（正式输入根目录）与
//! `LANEFLOW_ROUTING_EVIDENCE_IDENTITY`（身份目录名，如 `10000-W2-16-a`）。
//! 默认 `#[ignore]`，CI 只编译；实跑由 #1267/#1279 以显式环境变量驱动。
//!
//! 证据覆盖设计文档 §7 candidate registration 行与 §8 末条义务：
//! 成功（1/典型/长/重复边；成本过期后路线存活）、公开 API 可触达的
//! 失效与拒绝族（每族至少一条，逐条断言零路线槽、零边出现记录、零
//! 观测状态序号变化）、full/delta 精确账目，以及检查点后规范化重放的
//! 逐点摘要对拍。私有字段伪造类失败（绑定版本、会话修订、未来
//! tick/状态序号、非法时效窗）属合同测试面，由 `facade::routing`
//! 单测覆盖——真实宿主构造不出来，集成面不重复。

mod support {
    pub mod routing_evidence;
}

use laneflow_runtime::{
    AdmittedRouteRegisterError, AdmittedRouteRegisterInput, CandidateRouteError,
    CandidateRouteInput, CommittedTrafficObservationBatch, CostModelKey,
    DynamicCostSnapshotBinding, ObservationExportMode, ObservationSelection, ObservationSetBinding,
    RestoredSnapshot, RouteError, RouteHandle, RoutingAdmissionSession, StableRouteHandoffWindow,
    StateDigest, TickInput, TrafficWorld, VehicleSpawnInput, WorldConfig, WorldConfigInput,
    WorldId, WorldInstallInput, bind_observation_set, deterministic_state_digest, encode_lfrs,
    restore_lfrs,
};
use laneflow_static_contract::{LaneEdgeOrdinal, ParkingSpaceOrdinal, Sha256Digest, StableId128};
use laneflow_static_network::CanonicalNetworkOrigin;
use support::routing_evidence::{
    LFSYNTH_ROUTE_CAPACITY_HEADROOM, LfSynthFixture, ReceiverError, ReceiverLimits,
    build_lf_synth_fixture, dynamic_cost_snapshot_digest, lf_synth_cost_model,
    lf_synth_cost_payload, lf_synth_restore_limits, receive_cost_snapshot,
};

/// 拒绝零副作用的便宜账目：命令游标、观测状态序号、存活路线数与
/// 边出现记录总数。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Counters {
    command_cursor: u64,
    observation_state_sequence: u64,
    live_route_count: usize,
    live_occurrence_total: u64,
}

fn counters(world: &TrafficWorld) -> Counters {
    Counters {
        command_cursor: world.command_cursor(),
        observation_state_sequence: world.observation_state_sequence().get(),
        live_route_count: world.live_routes().count(),
        live_occurrence_total: world
            .live_routes()
            .map(|route| {
                u64::try_from(
                    world
                        .route_edges(route)
                        .map(<[LaneEdgeOrdinal]>::len)
                        .unwrap_or(0),
                )
                .expect("occurrence 计数在 u64 内")
            })
            .sum(),
    }
}

/// 由一次已提交观测批构造永久有效的成本绑定。
fn cost_binding_for(batch: &CommittedTrafficObservationBatch) -> DynamicCostSnapshotBinding {
    let observations = bind_observation_set(&[batch]).expect("观测集合绑定");
    cost_binding_with_validity(batch, observations, u64::MAX)
}

/// 同上，但 `valid_through_tick = 观测拍 + 1`（用于时效族探针）。
fn expiring_cost_binding_for(
    batch: &CommittedTrafficObservationBatch,
) -> DynamicCostSnapshotBinding {
    let observations = bind_observation_set(&[batch]).expect("观测集合绑定");
    cost_binding_with_validity(batch, observations, observations.observation_tick() + 1)
}

fn cost_binding_with_validity(
    batch: &CommittedTrafficObservationBatch,
    observations: ObservationSetBinding,
    valid_through_tick: u64,
) -> DynamicCostSnapshotBinding {
    let payload = lf_synth_cost_payload(batch);
    let model = lf_synth_cost_model();
    let placeholder = Sha256Digest::from_bytes([0x5a; 32]);
    let unsigned = DynamicCostSnapshotBinding::new(
        observations,
        model,
        valid_through_tick,
        batch.entry_count(),
        payload.len() as u64,
        placeholder,
    )
    .expect("成本绑定头");
    let digest = dynamic_cost_snapshot_digest(unsigned, &payload);
    DynamicCostSnapshotBinding::new(
        observations,
        model,
        valid_through_tick,
        batch.entry_count(),
        payload.len() as u64,
        digest,
    )
    .expect("成本绑定")
}

fn different_cost_model() -> CostModelKey {
    let base = lf_synth_cost_model();
    CostModelKey::new(base.model_id(), base.model_version() + 1)
}

#[test]
#[ignore = "manual formal-input integration evidence; 需显式 LF-SYNTH-v2 输入环境变量"]
fn routing_candidate_lf_synth_integration_evidence() {
    let mut fixture = build_lf_synth_fixture();
    let baseline = counters(&fixture.world);
    assert_eq!(baseline.live_route_count, fixture.state_route_count);
    assert_eq!(
        baseline.live_occurrence_total,
        fixture.state_route_edge_total
    );
    let typical_edges = fixture.typical_route.len();
    let long_edges = fixture.long_route.len();
    let mut success_probes = 0_usize;
    let mut failure_probes = 0_usize;

    // ---- 成功路径：1 / 典型 / 长（输入族拓扑为无环短链，重复边在
    // 合法路线中不可构造——证明边界记入结论；长轴扩展性由 §7.1 的
    // 4096 边 G2-LINEAR case 承担）----
    let one_input = fixture.candidate_one_input();
    let one = register_input(&mut fixture, one_input, 1, "candidate_1");
    success_probes += 1;
    fixture.world.remove_route(one).expect("移除 1 边候选");

    let typical_input = fixture.candidate_typical_input();
    let typical = register_input(
        &mut fixture,
        typical_input,
        typical_edges,
        "candidate_typical",
    );
    assert_registered_route(
        &fixture.world,
        typical,
        &fixture.typical_route,
        &fixture.typical_handoffs,
    );
    success_probes += 1;
    fixture.world.remove_route(typical).expect("移除典型候选");

    let long_input = fixture.candidate_long_input();
    let long = register_input(&mut fixture, long_input, long_edges, "candidate_long");
    assert_registered_route(
        &fixture.world,
        long,
        &fixture.long_route,
        &fixture.long_handoffs,
    );
    success_probes += 1;
    fixture.world.remove_route(long).expect("移除长候选");
    let after = counters(&fixture.world);
    assert_eq!(
        (after.live_route_count, after.live_occurrence_total),
        (baseline.live_route_count, baseline.live_occurrence_total),
        "成功探针全部清场后路线账目回到基线（命令游标按命令数推进，属正常）"
    );

    // ---- 失效与拒绝路径（不改世界状态的探针，前后快照摘要必须相等）----
    let digest_before = world_digest(&fixture.world);

    let empty_input = CandidateRouteInput::new(fixture.cost_binding, Vec::new());
    expect_rejection(
        &mut fixture,
        empty_input,
        CandidateRouteError::Route(RouteError::EmptySequence),
        "empty_sequence",
        &mut failure_probes,
    );

    let model_probe_input = fixture.candidate_one_input();
    let other_admission = fixture.world.open_routing_admission(different_cost_model());
    expect_rejection_with(
        &mut fixture,
        &other_admission,
        model_probe_input,
        CandidateRouteError::CostModelMismatch,
        "cost_model_mismatch",
        &mut failure_probes,
    );

    let (mut twin_world, twin_admission) = twin_world_with_admission(&fixture);
    let twin_probe_input = fixture.candidate_one_input();
    expect_rejection_on(
        &mut twin_world,
        &twin_admission,
        twin_probe_input,
        CandidateRouteError::CostWorldBindingMismatch,
        "cost_world_binding_mismatch",
        &mut failure_probes,
    );

    let unknown = StableId128::from_bytes([0xff; 16]);
    let unknown_input = CandidateRouteInput::new(fixture.cost_binding, vec![unknown]);
    expect_rejection(
        &mut fixture,
        unknown_input,
        CandidateRouteError::UnknownLaneEdge { stable_id: unknown },
        "unknown_lane_edge",
        &mut failure_probes,
    );

    let wrong_kind = fixture
        .world
        .revision()
        .identity()
        .stable_id(ParkingSpaceOrdinal::from_raw(0))
        .expect("车位稳定标识")
        .into_untyped();
    let wrong_kind_input = CandidateRouteInput::new(fixture.cost_binding, vec![wrong_kind]);
    expect_rejection(
        &mut fixture,
        wrong_kind_input,
        CandidateRouteError::UnknownLaneEdge {
            stable_id: wrong_kind,
        },
        "wrong_entity_kind",
        &mut failure_probes,
    );

    let disconnected_input = fixture.disconnected_candidate_input();
    expect_rejection(
        &mut fixture,
        disconnected_input,
        CandidateRouteError::Route(RouteError::Disconnected),
        "disconnected",
        &mut failure_probes,
    );

    if !fixture.long_handoffs.is_empty() {
        // 整串边仍在原修订内。去掉交接标记变成不连通的普通跳；把真窗口移到
        // 后一对出现则物理边界与边对不符。两条入口都要在提交之前拒绝。
        let mut shifted = fixture.long_handoffs.clone();
        shifted[0].source_index += 1;
        for (windows, error, label) in [
            (Vec::new(), RouteError::Disconnected, "missing_handoff"),
            (
                shifted,
                RouteError::InvalidHandoffWindow,
                "wrong_handoff_occurrence",
            ),
        ] {
            let input = fixture
                .candidate_long_input()
                .with_handoffs(windows.clone());
            expect_rejection(
                &mut fixture,
                input,
                CandidateRouteError::Route(error),
                label,
                &mut failure_probes,
            );
            let before = counters(&fixture.world);
            let command = admitted_input(
                *fixture.world.revision().canonical_origin(),
                &fixture.long_route,
                &windows,
            );
            assert_eq!(
                fixture.world.register_admitted_route(command),
                Err(AdmittedRouteRegisterError::Route(error)),
                "{label}: 重放命令必须精确拒绝"
            );
            assert_eq!(
                counters(&fixture.world),
                before,
                "{label}: 重放拒绝必须零副作用"
            );
            failure_probes += 1;
        }
    }

    assert_eq!(
        world_digest(&fixture.world),
        digest_before,
        "拒绝族探针不得改变世界快照摘要"
    );

    // ---- 容量边界：路线槽与边出现记录均按填满式 max/max+1 断言 ----
    let slot_expected = LFSYNTH_ROUTE_CAPACITY_HEADROOM as usize;
    let mut slots = Vec::new();
    for _ in 0..slot_expected {
        let input = fixture.candidate_one_input();
        slots.push(register_input(&mut fixture, input, 1, "route_slot_fill"));
        success_probes += 1;
    }
    assert_eq!(
        counters(&fixture.world).live_route_count,
        baseline.live_route_count + slot_expected,
        "1 边候选恰好填满路线槽余量 {LFSYNTH_ROUTE_CAPACITY_HEADROOM}"
    );
    failure_probes += 1;
    let before = counters(&fixture.world);
    let overflow_input = fixture.candidate_one_input();
    assert_eq!(
        fixture
            .world
            .register_candidate_route(&fixture.admission, overflow_input)
            .unwrap_err(),
        CandidateRouteError::Route(RouteError::CapacityExceeded),
        "路线槽 max+1 必须拒绝"
    );
    assert_eq!(counters(&fixture.world), before);
    let freed_slot = slots.pop().expect("已填槽位");
    fixture
        .world
        .remove_route(freed_slot)
        .expect("释放一个路线槽");
    let refill_input = fixture.candidate_one_input();
    let slot_refill = register_input(&mut fixture, refill_input, 1, "route_slot_refill");
    success_probes += 1;
    for route in slots {
        fixture.world.remove_route(route).expect("清场槽位候选");
    }
    fixture
        .world
        .remove_route(slot_refill)
        .expect("清场回填候选");
    let after = counters(&fixture.world);
    assert_eq!(
        (after.live_route_count, after.live_occurrence_total),
        (baseline.live_route_count, baseline.live_occurrence_total),
        "槽位探针清场后路线账目回到基线"
    );

    // 出现记录余量按「精确耗尽」填充：先典型候选后 1 边候选，直到余量
    // 为零；随后 max+1 拒绝、释放一个典型后 1 边回填，三段断言。
    let occurrence_headroom = fixture.occurrence_headroom;
    let mut filled = Vec::new();
    let mut consumed: u64 = 0;
    while occurrence_headroom - consumed >= typical_edges as u64 {
        let input = fixture.candidate_typical_input();
        filled.push(register_input(
            &mut fixture,
            input,
            typical_edges,
            "occurrence_fill_typical",
        ));
        consumed += typical_edges as u64;
        success_probes += 1;
    }
    while occurrence_headroom - consumed >= 1 {
        let input = fixture.candidate_one_input();
        filled.push(register_input(
            &mut fixture,
            input,
            1,
            "occurrence_fill_single",
        ));
        consumed += 1;
        success_probes += 1;
    }
    assert_eq!(
        consumed, occurrence_headroom,
        "填充必须精确耗尽出现记录余量"
    );
    assert_eq!(
        counters(&fixture.world).live_occurrence_total,
        baseline.live_occurrence_total + occurrence_headroom,
    );
    failure_probes += 1;
    let before = counters(&fixture.world);
    let occurrence_overflow_input = fixture.candidate_one_input();
    assert_eq!(
        fixture
            .world
            .register_candidate_route(&fixture.admission, occurrence_overflow_input)
            .unwrap_err(),
        CandidateRouteError::Route(RouteError::EdgeOccurrenceCapacityExceeded),
        "边出现记录 max+1 必须拒绝（即使只差一边）"
    );
    assert_eq!(counters(&fixture.world), before);
    // 累计释放 ≥ 2 边（典型候选 ≥ 2 边或两个 1 边），1 边候选必然可回填。
    let mut freed_edges: u64 = 0;
    while freed_edges < 2 {
        let route = filled.pop().expect("已填候选");
        freed_edges += fixture
            .world
            .route_edges(route)
            .map(<[_]>::len)
            .unwrap_or(0) as u64;
        fixture.world.remove_route(route).expect("清场填充候选");
    }
    let refill_input = fixture.candidate_one_input();
    let occurrence_refill = register_input(&mut fixture, refill_input, 1, "occurrence_refill");
    success_probes += 1;
    for route in filled {
        fixture.world.remove_route(route).expect("清场填充候选");
    }
    fixture
        .world
        .remove_route(occurrence_refill)
        .expect("清场出现记录探针");
    let after = counters(&fixture.world);
    assert_eq!(
        (after.live_route_count, after.live_occurrence_total),
        (baseline.live_route_count, baseline.live_occurrence_total),
        "容量探针全部清场后路线账目回到基线"
    );

    // ---- 接收端合同（宿主侧，不动世界状态）----
    let limits = fixture.receiver_limits();
    let entry_limited = ReceiverLimits {
        max_entry_count: limits.max_entry_count - 1,
        ..limits
    };
    expect_receiver_error(
        fixture.cost_binding,
        &fixture.cost_payload,
        entry_limited,
        ReceiverError::EntryLimitExceeded,
        "receiver_entry_limit_plus_one",
        &mut failure_probes,
    );
    let byte_limited = ReceiverLimits {
        max_exact_byte_length: limits.max_exact_byte_length - 1,
        ..limits
    };
    expect_receiver_error(
        fixture.cost_binding,
        &fixture.cost_payload,
        byte_limited,
        ReceiverError::ByteLimitExceeded,
        "receiver_byte_limit_plus_one",
        &mut failure_probes,
    );
    let mut truncated = fixture.cost_payload.clone();
    truncated.truncate(truncated.len() - 24);
    expect_receiver_error(
        fixture.cost_binding,
        &truncated,
        limits,
        ReceiverError::ExactByteLengthMismatch,
        "receiver_exact_bytes_mismatch",
        &mut failure_probes,
    );
    let mut tampered = fixture.cost_payload.clone();
    let last_index = tampered.len() - 1;
    tampered[last_index] ^= 0xff;
    expect_receiver_error(
        fixture.cost_binding,
        &tampered,
        limits,
        ReceiverError::SnapshotDigestMismatch,
        "receiver_digest_mismatch",
        &mut failure_probes,
    );

    // ---- 时效族：过期拒绝与已注册路线存活（会推进拍，放在容量探针后）----
    let fresh_full = fixture
        .world
        .export_observation(
            &mut fixture.observation_session,
            ObservationExportMode::Full,
        )
        .expect("时效探针前的全量观测");
    let expiring = expiring_cost_binding_for(&fresh_full);
    let expiring_input = CandidateRouteInput::new(expiring, fixture.typical_route.clone())
        .with_handoffs(fixture.typical_handoffs.clone());
    let survivor = register_input(
        &mut fixture,
        expiring_input,
        typical_edges,
        "expiring_candidate",
    );
    success_probes += 1;
    for _ in 0..2 {
        fixture.step();
    }
    assert_eq!(
        fixture.world.route_edges(survivor).map(<[_]>::len),
        Some(typical_edges),
        "成本过期后已注册路线必须存活"
    );
    failure_probes += 1;
    let before = counters(&fixture.world);
    let stale_input = CandidateRouteInput::new(expiring, fixture.one_route.clone());
    assert_eq!(
        fixture
            .world
            .register_candidate_route(&fixture.admission, stale_input)
            .unwrap_err(),
        CandidateRouteError::StaleDynamicCost,
        "越过有效期的绑定必须按 stale 拒绝"
    );
    assert_eq!(counters(&fixture.world), before);
    fixture.world.remove_route(survivor).expect("清场存活候选");

    // ---- full/delta 精确账目 ----
    fixture.step();
    let delta = fixture
        .world
        .export_observation(
            &mut fixture.observation_session,
            ObservationExportMode::Delta,
        )
        .expect("变化后增量导出");
    assert!(!delta.rows().is_empty(), "移动车辆负载下 delta 必有变化行");
    assert_eq!(delta.entry_count(), delta.rows().len() as u64);
    let delta_zero = fixture
        .world
        .export_observation(
            &mut fixture.observation_session,
            ObservationExportMode::Delta,
        )
        .expect("紧接零变化增量");
    assert_eq!(delta_zero.entry_count(), 0);
    let delta_payload = lf_synth_cost_payload(&delta);
    let delta_binding = cost_binding_for(&delta);
    let delta_limits = ReceiverLimits {
        max_entry_count: delta.entry_count(),
        max_exact_byte_length: delta_payload.len() as u64,
    };
    receive_cost_snapshot(delta_binding, &delta_payload, delta_limits)
        .expect("delta 批次派生的成本绑定通过接收端");
    let delta_input = CandidateRouteInput::new(delta_binding, fixture.one_route.clone());
    let delta_route = register_input(&mut fixture, delta_input, 1, "delta_binding_candidate");
    success_probes += 1;
    fixture
        .world
        .remove_route(delta_route)
        .expect("清场 delta 绑定候选");

    // ---- 检查点后规范化重放：逐点摘要对拍 ----
    let replay = checkpoint_replay(&mut fixture);
    success_probes += replay.host_side_replays;

    // ---- 汇总 ----
    println!(
        "routing-candidate-lfsynth-integration identity={} workload_id={} manifest_digest={:x} network_revision={:x} workload_manifest_digest={:x} state_digest={:x} selection_digest={:x} observation_set_digest={:x} cost_snapshot_digest={:x} edges={} state_routes={} state_route_edges={} vehicles={} active={} parked={} typical_edges={} long_edges={} long_handoffs={} success_probes={} failure_probes={} replay_points={} replay_divergence=none complete=true",
        fixture.identity_name,
        fixture.workload_id,
        fixture.manifest_digest,
        fixture.network_revision.as_digest(),
        fixture.workload_manifest_digest,
        fixture.state_digest,
        fixture.initial_full.selection_digest(),
        fixture.cost_binding.observation_set().digest(),
        fixture.cost_binding.snapshot_sha256(),
        fixture.edge_count,
        fixture.state_route_count,
        fixture.state_route_edge_total,
        fixture.vehicle_count,
        fixture.active_count,
        fixture.parked_count,
        typical_edges,
        long_edges,
        fixture.long_handoffs.len(),
        success_probes,
        failure_probes,
        replay.points,
    );
}

fn register_input(
    fixture: &mut LfSynthFixture,
    input: CandidateRouteInput,
    expected_edges: usize,
    label: &str,
) -> RouteHandle {
    let handle = fixture
        .world
        .register_candidate_route(&fixture.admission, input)
        .unwrap_or_else(|error| panic!("{label}: 成功路径不得失败：{error:?}"));
    assert_eq!(
        fixture.world.route_edges(handle).map(<[_]>::len),
        Some(expected_edges),
        "{label}: 注册后的边数必须精确"
    );
    handle
}

fn assert_registered_route(
    world: &TrafficWorld,
    handle: RouteHandle,
    expected_edges: &[StableId128],
    expected_windows: &[StableRouteHandoffWindow],
) {
    let revision = world.revision();
    let edges = world.route_edges(handle).expect("路线句柄有效");
    let ids = revision.identity();
    assert!(
        edges
            .iter()
            .map(|&edge| ids.stable_id(edge).unwrap().into_untyped())
            .eq(expected_edges.iter().copied()),
        "每个路线出现的稳定边身份与顺序必须一致"
    );
    let windows = world
        .route_handoff_windows(handle)
        .expect("已登记路线的交接窗");
    assert_eq!(windows.len(), expected_windows.len());
    for (&(source, binding), expected) in windows.iter().zip(expected_windows) {
        assert_eq!(
            StableRouteHandoffWindow::from_binding(&revision, source, binding).as_ref(),
            Some(expected),
            "物理边界、左右成员、源出现和毫米站位必须一致"
        );
        let index = source as usize;
        assert_eq!(binding.window().edges, [edges[index], edges[index + 1]]);
    }
}

fn admitted_input(
    origin: CanonicalNetworkOrigin,
    edges: &[StableId128],
    windows: &[StableRouteHandoffWindow],
) -> AdmittedRouteRegisterInput {
    AdmittedRouteRegisterInput::new(
        origin.network_revision(),
        origin
            .static_contract_versions()
            .network_revision_derivation_version(),
        edges.to_vec(),
    )
    .with_handoffs(windows.to_vec())
}

fn expect_rejection(
    fixture: &mut LfSynthFixture,
    input: CandidateRouteInput,
    expected: CandidateRouteError,
    label: &str,
    failure_probes: &mut usize,
) {
    let before = counters(&fixture.world);
    let actual = fixture
        .world
        .register_candidate_route(&fixture.admission, input)
        .unwrap_err();
    assert_eq!(actual, expected, "{label}: 失败族必须精确归类");
    assert_eq!(
        counters(&fixture.world),
        before,
        "{label}: 拒绝必须零副作用"
    );
    *failure_probes += 1;
}

fn expect_rejection_with(
    fixture: &mut LfSynthFixture,
    admission: &RoutingAdmissionSession,
    input: CandidateRouteInput,
    expected: CandidateRouteError,
    label: &str,
    failure_probes: &mut usize,
) {
    let before = counters(&fixture.world);
    let actual = fixture
        .world
        .register_candidate_route(admission, input)
        .unwrap_err();
    assert_eq!(actual, expected, "{label}: 失败族必须精确归类");
    assert_eq!(
        counters(&fixture.world),
        before,
        "{label}: 拒绝必须零副作用"
    );
    *failure_probes += 1;
}

fn expect_rejection_on(
    world: &mut TrafficWorld,
    admission: &RoutingAdmissionSession,
    input: CandidateRouteInput,
    expected: CandidateRouteError,
    label: &str,
    failure_probes: &mut usize,
) {
    let before = (
        world.command_cursor(),
        world.observation_state_sequence().get(),
        world.live_routes().count(),
    );
    let actual = world
        .register_candidate_route(admission, input)
        .unwrap_err();
    assert_eq!(actual, expected, "{label}: 失败族必须精确归类");
    assert_eq!(
        (
            world.command_cursor(),
            world.observation_state_sequence().get(),
            world.live_routes().count(),
        ),
        before,
        "{label}: 拒绝必须零副作用"
    );
    *failure_probes += 1;
}

fn expect_receiver_error(
    binding: DynamicCostSnapshotBinding,
    payload: &[u8],
    limits: ReceiverLimits,
    expected: ReceiverError,
    label: &str,
    failure_probes: &mut usize,
) {
    let actual = receive_cost_snapshot(binding, payload, limits).unwrap_err();
    assert_eq!(actual, expected, "{label}: 接收端失败必须精确归类");
    *failure_probes += 1;
}

/// 同一修订上的最小孪生世界：跨世界绑定失配探针（真实多世界宿主可
/// 触达）。路线槽与出现记录只留探针所需。
fn twin_world_with_admission(fixture: &LfSynthFixture) -> (TrafficWorld, RoutingAdmissionSession) {
    let world = TrafficWorld::install(WorldInstallInput {
        revision: std::sync::Arc::clone(&fixture.world.revision()),
        config: WorldConfig::try_new(WorldConfigInput {
            vehicle_capacity: 0,
            route_capacity: 1,
            route_edge_occurrence_capacity: 4_096,
            route_conflict_occurrence_capacity: 0,
            fixed_delta_time_ms: fixture.fixed_delta_ms,
        })
        .expect("有效的交通配置"),
        execution: laneflow_runtime::ExecutionConfig::new(std::num::NonZeroU32::MIN),
        source: fixture.world.committed_source().clone(),
        world_id: WorldId::new(1_268),
        policy_selection: laneflow_runtime::WorldPolicySelection::Pinned(fixture.policy),
    })
    .expect("安装孪生世界");
    let admission = world.open_routing_admission(lf_synth_cost_model());
    (world, admission)
}

fn world_digest(world: &TrafficWorld) -> StateDigest {
    let snapshot = world.capture_snapshot().expect("快照");
    deterministic_state_digest(&snapshot).expect("状态摘要")
}

fn capture_point(world: &TrafficWorld) -> (u64, u64, StateDigest) {
    let snapshot = world.capture_snapshot().expect("后缀快照");
    (
        snapshot.command_cursor(),
        snapshot.tick(),
        deterministic_state_digest(&snapshot).expect("后缀摘要"),
    )
}

struct ReplayOutcome {
    points: usize,
    host_side_replays: usize,
}

/// 把已登记长候选的稳定内容录成宿主命令。恢复检查点后，两侧依次
/// 注册已准入路线、spawn、推进三拍，再用各自的新成本绑定注册候选。
fn checkpoint_replay(fixture: &mut LfSynthFixture) -> ReplayOutcome {
    let checkpoint_input = fixture.candidate_long_input();
    let checkpoint_route = fixture
        .world
        .register_candidate_route(&fixture.admission, checkpoint_input)
        .expect("检查点前注册候选");
    assert_registered_route(
        &fixture.world,
        checkpoint_route,
        &fixture.long_route,
        &fixture.long_handoffs,
    );
    let snapshot = fixture.world.capture_snapshot().expect("检查点快照");
    let captured = fixture
        .world
        .live_routes()
        .zip(snapshot.routes())
        .find_map(|(route, captured)| (route == checkpoint_route).then_some(captured))
        .expect("检查点内的长候选");
    assert_eq!(captured.edges(), fixture.long_route);
    assert_eq!(captured.handoff_windows(), fixture.long_handoffs);

    let restored: RestoredSnapshot = restore_lfrs(
        &encode_lfrs(&snapshot),
        fixture.world.revision(),
        fixture.world.committed_source().clone(),
        fixture.world.config(),
        laneflow_runtime::ExecutionConfig::new(std::num::NonZeroU32::MIN),
        lf_synth_restore_limits(),
    )
    .expect("恢复检查点");
    assert_eq!(
        restored.route_mappings().len(),
        fixture.state_route_count + 1
    );
    assert_eq!(restored.vehicle_mappings().len(), fixture.vehicle_count);
    let restored_route = restored
        .route_handle(captured.snapshot_route_id())
        .expect("快照局部 ID 的新句柄");
    let mut replayed = restored.into_world();
    assert_registered_route(
        &replayed,
        restored_route,
        &fixture.long_route,
        &fixture.long_handoffs,
    );
    assert_eq!(
        world_digest(&replayed),
        deterministic_state_digest(&snapshot).unwrap()
    );

    // 记录已准入的路线及修订，无须保留成本来源。每次执行返回各自的新句柄。
    let replay_command = |world: &mut TrafficWorld| {
        let command = admitted_input(
            snapshot.origin(),
            captured.edges(),
            captured.handoff_windows(),
        );
        let route = world
            .register_admitted_route(command)
            .expect("重放规范化已准入命令");
        assert_registered_route(world, route, captured.edges(), captured.handoff_windows());
        route
    };
    let original_handle = replay_command(&mut fixture.world);
    let replayed_handle = replay_command(&mut replayed);
    let mut points = 0;
    let mut compare = |expected: &TrafficWorld, actual: &TrafficWorld| {
        points += 1;
        assert_eq!(
            capture_point(actual),
            capture_point(expected),
            "重放后缀第 {points} 个核对点"
        );
    };
    compare(&fixture.world, &replayed);

    for (world, route) in [
        (&mut fixture.world, original_handle),
        (&mut replayed, replayed_handle),
    ] {
        let spawn = VehicleSpawnInput::new(fixture.profile_ordinal, route, 0, 1_000, 0)
            .with_open_entrance();
        world.spawn_vehicle(spawn).expect("后缀 spawn");
    }
    compare(&fixture.world, &replayed);
    for _ in 0..3 {
        assert!(
            fixture.step_requests().is_empty(),
            "后缀三拍内，正式宿主输入仅推进活动横移"
        );
        fixture.step();
        replayed
            .step(TickInput::new(fixture.fixed_delta_ms))
            .expect("重放同一固定步进");
        compare(&fixture.world, &replayed);
    }

    let (edges, windows) = if fixture.long_handoffs.is_empty() {
        (&fixture.one_route, &[][..])
    } else {
        (&fixture.long_route, fixture.long_handoffs.as_slice())
    };
    for world in [&mut fixture.world, &mut replayed] {
        let mut session = world
            .open_observation_export(ObservationSelection::AllLaneEdges)
            .expect("后缀观测会话");
        let full = world
            .export_observation(&mut session, ObservationExportMode::Full)
            .expect("后缀全量观测");
        let admission = world.open_routing_admission(lf_synth_cost_model());
        let input = CandidateRouteInput::new(cost_binding_for(&full), edges.clone())
            .with_handoffs(windows.to_vec());
        let route = world
            .register_candidate_route(&admission, input)
            .expect("后缀候选注册");
        assert_registered_route(world, route, edges, windows);
    }
    compare(&fixture.world, &replayed);
    ReplayOutcome {
        points,
        host_side_replays: 2,
    }
}
