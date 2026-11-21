use std::num::NonZeroU32;

use laneflow_format::{FormatLimits, check_canonical_network_input};
use laneflow_runtime::{ExecutionConfig, TickInput};
use laneflow_static_contract::SignalAspect;
use laneflow_static_network::{
    SharedNetworkBuildLimits, SharedNetworkBuildOptions, SpatialBuildOption,
    build_shared_network_revision,
};
use laneflow_synth_generator::{Case, Identity, InitialState, Provenance, generate};

use crate::{
    Plan, cycle_ms,
    install::{
        Sample, route_position_offsets, validate_w_corridor_binding,
        validate_w_signal_reference_domain,
    },
    observe::motion_valid,
    signal_reference::{SignalSchedule, aspect},
};

fn bundle(case: Case, dt: u32) -> laneflow_synth_generator::Bundle {
    generate(
        Identity::new(10_000, case, dt).unwrap(),
        Provenance {
            source_commit: "a".repeat(40),
            generator_commit: "b".repeat(40),
        },
    )
    .unwrap()
}

#[test]
fn installed_worker_counts_preserve_initial_and_stepped_public_state() {
    let bundle = bundle(Case::W1, 16);
    let mut reference = Sample::new(&bundle, ExecutionConfig::new(NonZeroU32::MIN)).unwrap();
    let mut parallel = [2, 4, 8, 16].map(|workers| {
        let execution = ExecutionConfig::new(NonZeroU32::new(workers).unwrap());
        let sample = Sample::new(&bundle, execution).unwrap();
        assert_eq!(sample.world.execution_config(), execution);
        sample
    });
    for tick in 0..=8 {
        if tick != 0 {
            reference.world.step(TickInput::new(16)).unwrap();
            for sample in &mut parallel {
                sample.world.step(TickInput::new(16)).unwrap();
            }
        }
        let expected = reference.observe(tick, true).unwrap();
        let snapshot = reference.world.capture_snapshot().unwrap();
        let digest = laneflow_runtime::deterministic_state_digest(&snapshot).unwrap();
        for sample in &mut parallel {
            assert_eq!(sample.observe(tick, true).unwrap(), expected);
            assert_eq!(
                sample.world.latest_transition_events(),
                reference.world.latest_transition_events()
            );
            let snapshot = sample.world.capture_snapshot().unwrap();
            assert_eq!(
                laneflow_runtime::deterministic_state_digest(&snapshot).unwrap(),
                digest
            );
        }
    }
    assert!(Sample::new(&bundle, ExecutionConfig::new(NonZeroU32::new(17).unwrap())).is_err());
}

#[test]
fn frozen_signal_boundaries_and_offsets() {
    assert_eq!(cycle_ms(16), 58_032);
    assert_eq!(cycle_ms(33), 58_113);
    for (dt, ticks) in [
        (16, [1_875, 188, 63, 1_250, 188, 63]),
        (33, [910, 91, 31, 607, 91, 31]),
    ] {
        let expected = [
            [SignalAspect::Green, SignalAspect::Red],
            [SignalAspect::Yellow, SignalAspect::Red],
            [SignalAspect::Red, SignalAspect::Red],
            [SignalAspect::Red, SignalAspect::Green],
            [SignalAspect::Red, SignalAspect::Yellow],
            [SignalAspect::Red, SignalAspect::Red],
        ];
        let mut tick = 0;
        for (count, states) in ticks.into_iter().zip(expected) {
            for (route, expected) in states.into_iter().enumerate() {
                assert_eq!(
                    aspect(SignalSchedule::CellOffset, 0, route as u32, tick, dt),
                    expected
                );
                assert_eq!(
                    aspect(
                        SignalSchedule::CellOffset,
                        0,
                        route as u32,
                        tick + count - 1,
                        dt
                    ),
                    expected
                );
            }
            tick += count;
        }
        assert_eq!(
            aspect(SignalSchedule::CellOffset, 0, 0, tick, dt),
            SignalAspect::Green
        );
        assert_eq!(
            aspect(
                SignalSchedule::CellOffset,
                1,
                0,
                (30_000_u64.div_ceil(u64::from(dt)) * u64::from(dt) - 997).div_ceil(u64::from(dt)),
                dt
            ),
            SignalAspect::Yellow
        );
    }
}

#[test]
fn w_reference_domain_rejects_drifted_reference_facts() {
    // 参考判定的适用条件必须显式核对：冻结事实通过；受检 profile 的派生
    // 紧急减速度漂移（不再是 6000）时拒绝——四舍五入凑巧相等不等于原始
    // 值精确，生成器演进后不得静默套用旧假设。
    let bundle = bundle(Case::W1, 16);
    let input = check_canonical_network_input(
        bundle.files()["network.lfca"].as_slice(),
        FormatLimits::HARD,
    )
    .unwrap();
    let revision = build_shared_network_revision(
        input,
        SharedNetworkBuildOptions::new(
            SpatialBuildOption::RetainAvailable,
            SharedNetworkBuildLimits::new(2 * 1_024 * 1_024 * 1_024, 2 * 1_024 * 1_024 * 1_024),
        ),
    )
    .unwrap();
    let state: InitialState = serde_json::from_slice(&bundle.files()["state.json"]).unwrap();
    let route_offsets = route_position_offsets(&state, &revision).unwrap();
    assert!(validate_w_signal_reference_domain(&revision, &state, &route_offsets, 6_000).is_ok());
    for drifted in [0_u64, 6_001, 9_000] {
        assert!(
            validate_w_signal_reference_domain(&revision, &state, &route_offsets, drifted).is_err(),
            "drifted emergency decel {drifted} must reject"
        );
    }
}

#[test]
fn w_corridor_binding_rejects_non_grid_or_shared_edges() {
    // 快路径绑定前提（设计 §5.2）须显式证明，不得仅凭 rank/25 推断空间
    // 隔离：冻结 W 输入通过；网格边长破坏、路线内别名、跨路线共享物理边
    // 均拒绝。
    let bundle = bundle(Case::W1, 16);
    let input = check_canonical_network_input(
        bundle.files()["network.lfca"].as_slice(),
        FormatLimits::HARD,
    )
    .unwrap();
    let revision = build_shared_network_revision(
        input,
        SharedNetworkBuildOptions::new(
            SpatialBuildOption::RetainAvailable,
            SharedNetworkBuildLimits::new(2 * 1_024 * 1_024 * 1_024, 2 * 1_024 * 1_024 * 1_024),
        ),
    )
    .unwrap();
    let state: InitialState = serde_json::from_slice(&bundle.files()["state.json"]).unwrap();
    let route_offsets = route_position_offsets(&state, &revision).unwrap();
    assert!(validate_w_corridor_binding(&state, &route_offsets).is_ok());

    let mut grid = route_offsets.clone();
    grid[0][2] = 2 * 5_130_000 - 1;
    assert!(
        validate_w_corridor_binding(&state, &grid).is_err(),
        "非 5 130 m 网格的出口边必须拒绝快路径"
    );

    let mut aliased: InitialState = serde_json::from_slice(&bundle.files()["state.json"]).unwrap();
    aliased.routes[0].edge_ids[1] = aliased.routes[0].edge_ids[0].clone();
    assert!(
        validate_w_corridor_binding(&aliased, &route_offsets).is_err(),
        "路线内两条边别名必须拒绝快路径"
    );

    let mut shared: InitialState = serde_json::from_slice(&bundle.files()["state.json"]).unwrap();
    shared.routes[1].edge_ids[0] = shared.routes[0].edge_ids[0].clone();
    assert!(
        validate_w_corridor_binding(&shared, &route_offsets).is_err(),
        "跨路线共享物理边必须拒绝快路径"
    );
}

#[test]
fn motion_guard_rejects_backwards_overspeed_and_impossible_acceleration() {
    assert!(motion_valid(1_000, 1_176, 10, 11, 1_400, 16));
    assert!(!motion_valid(1_000, 999, 10, 10, 1_400, 16));
    assert!(!motion_valid(0, 222_402, 13_900, 13_900, 1_400, 16));
    assert!(!motion_valid(0, 0, 13_900, 13_901, 1_400, 16));
    assert!(!motion_valid(0, 0, 0, 100, 1_400, 16));
    // #1159：整数硬停止投影清速度；emergency_decel 不是已提交速度差的绝对上限。
    assert!(motion_valid(0, 0, 1_000, 0, 1_400, 16));
}

#[test]
fn canonical_initialization_and_guard_negative_controls() {
    let bundle = bundle(Case::W1, 16);
    let plan = Plan::read(&bundle).unwrap();
    assert_eq!(
        (plan.warm_up_ticks, plan.observation_ticks),
        (14_508, 29_016)
    );
    let mut sample = Sample::new(&bundle, ExecutionConfig::new(NonZeroU32::MIN)).unwrap();
    assert_eq!(
        sample.observe(0, true).unwrap().to_hex().as_str(),
        "ebb35b54634106fdc227b7b3f86712232ad4e30c75512e556aa538771cf73a7e"
    );
    for tick in 1..=8 {
        let outcome = sample.world.step(TickInput::new(16)).unwrap();
        sample
            .observe(tick, outcome.parking_arrivals().is_empty())
            .unwrap();
    }
    assert!(sample.observe(8, false).is_err());
    assert!(sample.observe(9, true).is_err());
    let cursor = sample.cursor;
    sample.cursor += 1;
    assert!(sample.observe(8, true).is_err());
    sample.cursor = cursor;
    sample.groups.swap(0, 1);
    assert!(sample.observe(8, true).is_err());
    sample.groups.swap(0, 1);
    sample.handles.swap(0, 1);
    assert!(sample.observe(8, true).is_err());
    sample.handles.swap(0, 1);
    sample.bindings[62] = None;
    assert!(sample.observe(8, true).is_err());
}

#[test]
fn invalid_input_and_existing_output_are_preserved() {
    let bundle = bundle(Case::W3, 33);
    assert!(Plan::read(&bundle).is_ok());
    let root = tempfile::tempdir().unwrap();
    let input = root.path().join("input");
    let output = root.path().join("output");
    bundle.write_new(&input).unwrap();
    std::fs::write(input.join("input-plan.json"), b"{}").unwrap();
    assert!(
        crate::run(
            &input,
            &"c".repeat(40),
            ExecutionConfig::new(NonZeroU32::MIN),
            &output
        )
        .is_err()
    );
    let before = std::fs::read(output.join("report.json")).unwrap();
    let report: serde_json::Value = serde_json::from_slice(&before).unwrap();
    assert_eq!(report["complete"], false);
    assert!(
        crate::run(
            &input,
            &"c".repeat(40),
            ExecutionConfig::new(NonZeroU32::MIN),
            &output
        )
        .is_err()
    );
    assert_eq!(std::fs::read(output.join("report.json")).unwrap(), before);
}

#[test]
fn w2_is_fully_active_and_shortened_plan_is_rejected() {
    let bundle = bundle(Case::W2, 33);
    let plan = Plan::read(&bundle).unwrap();
    assert_eq!(
        (plan.warm_up_ticks, plan.observation_ticks),
        (7_044, 14_088)
    );
    let mut sample = Sample::new(&bundle, ExecutionConfig::new(NonZeroU32::MIN)).unwrap();
    sample.observe(0, true).unwrap();
    assert!(
        sample
            .initial
            .iter()
            .all(|state| state.status() == laneflow_runtime::VehicleStatus::Active)
    );
    for tick in 1..=8 {
        sample.world.step(TickInput::new(33)).unwrap();
        sample.observe(tick, true).unwrap();
    }
    let root = tempfile::tempdir().unwrap();
    let input = root.path().join("input");
    bundle.write_new(&input).unwrap();
    let mut plan: serde_json::Value =
        serde_json::from_slice(&bundle.files()["input-plan.json"]).unwrap();
    plan["observation_ticks"] = 1.into();
    let mut bytes = serde_json::to_vec(&plan).unwrap();
    bytes.push(b'\n');
    std::fs::write(input.join("input-plan.json"), bytes).unwrap();
    let output = root.path().join("output");
    assert!(
        crate::run(
            &input,
            &"c".repeat(40),
            ExecutionConfig::new(NonZeroU32::MIN),
            &output
        )
        .is_err()
    );
    assert!(!output.join("ticks.bin").exists());
}

/// W4 正式 run 入口（`burst::run`）的会话接线端到端：压缩时序计划
///（与恢复 E2E 同法，冻结输入字节不动）走完整生产路径，回执携带验证
/// 回执——规格身份、真实见证计数（过门与 Green 探针各 100）、逐命令
/// 触发计数与零 Blocked；报告级检查点携带验证材料。
#[test]
fn w4_full_run_report_carries_verification_receipt() {
    let bundle = bundle(Case::W4, 16);
    let mut plan = Plan::read(&bundle).unwrap();
    let old_warm = plan.warm_up_ticks;
    plan.warm_up_ticks = 1;
    for batch in &mut plan.scheduled {
        let boundary = batch["boundary"].as_u64().unwrap();
        if boundary > 0 {
            batch["boundary"] = serde_json::json!(boundary + 1 - old_warm);
        }
    }
    let root = tempfile::tempdir().unwrap();
    let output = root.path().join("output");
    std::fs::create_dir(&output).unwrap();
    let report = crate::burst::run(
        &bundle,
        &plan,
        &"c".repeat(40),
        ExecutionConfig::new(NonZeroU32::MIN),
        &output,
    )
    .unwrap();
    assert_eq!(report["complete"], true);
    let receipt = &report["verification_receipt"];
    assert_eq!(receipt["spec"], "lf-synth-w4-burst-semantic-run");
    assert_eq!(receipt["successful_steps"], 3);
    assert_eq!(receipt["blocked_attempts"], 0);
    assert_eq!(receipt["witnesses"]["per-active-crossing"], 100);
    assert_eq!(receipt["witnesses"]["w4-green-probe"], 100);
    assert_eq!(receipt["trigger_counts"]["w-command-authority"], 600);
    let checkpoints = report["checkpoints"].as_array().unwrap();
    assert!(
        checkpoints
            .iter()
            .all(|point| point["verification"]["spec_name"] == "lf-synth-w4-burst-semantic-run"),
        "报告级检查点必须携带验证材料"
    );
}

#[test]
fn w_session_wiring_counts_rules_and_can_finish_positively() {
    // W1/W2 正式 run 的会话接线：真实 Sample、真实 Runtime step 与真实
    // observer。运动/身份、车身分离、走廊顺序三条每轮规则逐拍登记触发
    // （初始观测轮与每个成功步进轮各一次）；信号与门事件在短窗内零过门，
    // 如实记零不冒充覆盖。会话以小窗口驱动到成功终结：每轮义务闭合、
    // 必需见证在档（短窗内无真实过门，见证按受信构造入口模型直接登记，
    // 语义版本与规格身份随回执核对）——正向 finish 产生回执。
    use laneflow_traffic_verification::session::FinishRejected;
    use laneflow_traffic_verification::spec::RuleId;

    let bundle = bundle(Case::W1, 16);
    let plan = Plan::read(&bundle).unwrap();
    let full_total = plan.warm_up_ticks + plan.observation_ticks;
    let mut sample = Sample::new(&bundle, ExecutionConfig::new(NonZeroU32::MIN)).unwrap();
    // 正向路径用小窗口规约（同一正式构造入口，仅窗口参数不同）。
    let window = 3_u64;
    sample.session = Some(crate::verification::start_session(window).unwrap());
    sample.input_identity = crate::verification::input_identity(&bundle);
    sample
        .session
        .as_mut()
        .unwrap()
        .confirm_initial_boundary(0)
        .unwrap();
    sample.observe(0, true).unwrap();
    // 初始观测轮：三条每轮规则已各登记一次。
    {
        let session = sample.session.as_ref().unwrap();
        assert_eq!(session.trigger_count(RuleId("w-motion-identity")), 1);
        assert_eq!(session.trigger_count(RuleId("body-separation")), 1);
        assert_eq!(session.trigger_count(RuleId("w-lane-order")), 1);
        assert_eq!(session.trigger_count(RuleId("signal-permit")), 0);
        assert_eq!(session.trigger_count(RuleId("gate-events")), 0);
    }
    sample
        .session
        .as_mut()
        .unwrap()
        .note_witness(laneflow_traffic_verification::spec::WitnessId(
            "per-active-crossing",
        ))
        .unwrap();
    for tick in 1..=window {
        {
            let session = sample.session.as_mut().unwrap();
            session.declare_commands(0).unwrap();
            session.advance_to_pre_tick().unwrap();
            session.begin_step().unwrap();
        }
        sample.world.step(TickInput::new(16)).unwrap();
        sample.observe(tick, true).unwrap();
        sample
            .session
            .as_mut()
            .unwrap()
            .note_successful_step(tick)
            .unwrap();
    }
    {
        let session = sample.session.as_ref().unwrap();
        // 初始轮 + 3 个成功步进轮 = 4 次。
        assert_eq!(session.trigger_count(RuleId("w-motion-identity")), 4);
        assert_eq!(session.trigger_count(RuleId("body-separation")), 4);
        assert_eq!(session.trigger_count(RuleId("w-lane-order")), 4);
    }
    let receipt = sample
        .session
        .as_mut()
        .unwrap()
        .finish()
        .expect("每轮义务闭合+见证在档的小窗口必须成功终结");
    assert_eq!(receipt.successful_steps(), window);
    assert_eq!(receipt.spec_name(), "lf-synth-w-semantic-run");
    assert_eq!(receipt.semantic_version(), crate::SEMANTIC_CONTRACT);
    // 终结后重复 finish 拒绝。
    assert_eq!(
        sample.session.as_mut().unwrap().finish(),
        Err(FinishRejected::DoubleFinish)
    );
    // 正式窗口的短运行：完整窗口未满，finish 以窗口不完整拒绝。
    let mut full_sample = Sample::new(&bundle, ExecutionConfig::new(NonZeroU32::MIN)).unwrap();
    full_sample.session = Some(crate::verification::start_session(full_total).unwrap());
    full_sample.input_identity = crate::verification::input_identity(&bundle);
    full_sample
        .session
        .as_mut()
        .unwrap()
        .confirm_initial_boundary(0)
        .unwrap();
    full_sample.observe(0, true).unwrap();
    for tick in 1..=8 {
        {
            let session = full_sample.session.as_mut().unwrap();
            session.declare_commands(0).unwrap();
            session.advance_to_pre_tick().unwrap();
            session.begin_step().unwrap();
        }
        full_sample.world.step(TickInput::new(16)).unwrap();
        full_sample.observe(tick, true).unwrap();
        full_sample
            .session
            .as_mut()
            .unwrap()
            .note_successful_step(tick)
            .unwrap();
    }
    assert!(matches!(
        full_sample.session.as_mut().unwrap().finish(),
        Err(FinishRejected::WindowIncomplete { .. })
    ));
    // 终结是消费式一次性：被拒后不得补记重试（§3.3/§6）。
    assert_eq!(
        full_sample.session.as_mut().unwrap().finish(),
        Err(FinishRejected::DoubleFinish)
    );
    assert!(matches!(
        full_sample
            .session
            .as_mut()
            .unwrap()
            .note_check(RuleId("body-separation")),
        Err(laneflow_traffic_verification::session::ProtocolError::Finished)
    ));
}

/// a′ 取证（#1457 关单裁决点）的记录探针：按完成序记录相位结束与步进
/// 发起拍，用于断言真实步进失败后无终结相位、失败拍步进未完成。
#[derive(Default)]
struct PhaseRecorder {
    ended: Vec<crate::verification_cost::Phase>,
    steps: Vec<u64>,
}

impl crate::verification_cost::Probe for PhaseRecorder {
    type Mark = std::time::Instant;

    fn start(&mut self) -> Self::Mark {
        std::time::Instant::now()
    }

    fn end(&mut self, phase: crate::verification_cost::Phase, _mark: Self::Mark) {
        self.ended.push(phase);
    }

    fn runtime_step_tick(&mut self, tick: u64) {
        self.steps.push(tick);
    }
}

/// 真实 Runtime 步进错误与会话组合的 fail-closed 负例（W1/W2 驱动）：
/// 冻结的 16 ms bundle 输入不改，仅在内存把计划步长改为 33；安装仍按
/// InitialState 构建 16 ms 世界（install.rs world_config），驱动提交
/// 33 ms TickInput，会话已进入步进阶段（declare/advance/begin 完成），
/// 真实 `TrafficWorld::step` 在步进预检返回 DeltaMismatch。断言：错误
/// 来自目标步进并携带期望/实际步长；初始边界后没有成功拍轨迹（轨迹只
/// 有 tick 0 记录）；失败拍步进区间未完成、其后无观察与终结相位。
/// 证明边界：这是步进前置拒绝，不证明事务中途失败回滚，也不证明
/// note_expected_failure 后的同输入重试与命令不重放（原子性 retry 的
/// 证明范围另列）；外层 incomplete 报告行为由错误包装器既有测试核验。
#[test]
fn w1_real_step_delta_mismatch_fails_closed() {
    let bundle = bundle(Case::W1, 16);
    let mut plan = Plan::read(&bundle).unwrap();
    plan.identity.fixed_delta_ms = 33;
    let root = tempfile::tempdir().unwrap();
    let output = root.path().join("output");
    std::fs::create_dir(&output).unwrap();
    let mut probe = PhaseRecorder::default();
    let error = crate::run_plan(
        &bundle,
        &plan,
        &"c".repeat(40),
        ExecutionConfig::new(NonZeroU32::MIN),
        &output,
        &mut probe,
    )
    .unwrap_err();
    let message = error.to_string();
    assert!(message.contains("step 1"), "错误须来自目标步进：{message}");
    assert!(message.contains("DeltaMismatch"), "{message}");
    assert!(
        message.contains("expected_delta_time_ms: 16"),
        "须携带世界配置步长：{message}"
    );
    assert!(
        message.contains("actual_delta_time_ms: 33"),
        "须携带本次输入步长：{message}"
    );
    // 轨迹只有格式头与初始边界（tick 0）记录；失败拍与其后拍不写入。
    // 每条记录 = 8 字节拍号 + 32 字节 blake3 摘要。
    let trace = std::fs::read(output.join("ticks.bin")).unwrap();
    let header = b"lf-synth-public-ticks-1\0";
    assert_eq!(trace.len(), header.len() + 8 + 32, "只有 tick 0 记录");
    assert_eq!(
        u64::from_le_bytes(trace[header.len()..header.len() + 8].try_into().unwrap()),
        0,
        "唯一记录须是初始边界"
    );
    // 会话已进入步进阶段（Session 相位完成）；步进区间发起但未完成；
    // 失败拍之后无观察、无终结。
    use crate::verification_cost::Phase;
    assert!(
        probe.ended.contains(&Phase::Session),
        "会话须已进入步进阶段：{:?}",
        probe.ended
    );
    // 失败步进的计量区间本身完整结束（measured 在错误路径也收区间），
    // 但其后不得再有任何相位：无失败拍的观察、无终结。
    assert_eq!(
        probe.ended.last(),
        Some(&Phase::RuntimeStep),
        "失败步进须是最后一个相位：{:?}",
        probe.ended
    );
    assert!(
        !probe.ended.contains(&Phase::Finish),
        "真实步进失败后不得执行终结相位"
    );
    assert_eq!(probe.steps, vec![1], "只发起过一次步进");
}

/// 同款 fail-closed 负例（W3/W4 burst 驱动）：会话在安装期绑定并完成
/// 拍前阶段（advance/begin）后，真实 step 以 DeltaMismatch 拒绝；错误
/// 传播中止运行，无终结相位、失败拍不写入轨迹。
#[test]
fn w4_burst_real_step_delta_mismatch_fails_closed() {
    let bundle = bundle(Case::W4, 16);
    let mut plan = Plan::read(&bundle).unwrap();
    plan.identity.fixed_delta_ms = 33;
    let root = tempfile::tempdir().unwrap();
    let output = root.path().join("output");
    std::fs::create_dir(&output).unwrap();
    let mut probe = PhaseRecorder::default();
    let error = crate::burst::run_with_probe(
        &bundle,
        &plan,
        &"c".repeat(40),
        ExecutionConfig::new(NonZeroU32::MIN),
        &output,
        &mut probe,
    )
    .unwrap_err();
    let message = error.to_string();
    assert!(message.contains("step 1"), "错误须来自目标步进：{message}");
    assert!(message.contains("DeltaMismatch"), "{message}");
    assert!(
        message.contains("expected_delta_time_ms: 16")
            && message.contains("actual_delta_time_ms: 33"),
        "{message}"
    );
    let trace = std::fs::read(output.join("ticks.bin")).unwrap();
    let header = b"lf-synth-burst-ticks-1\0";
    assert_eq!(trace.len(), header.len() + 8 + 32, "只有 tick 0 记录");
    use crate::verification_cost::Phase;
    assert!(
        probe.ended.contains(&Phase::Session),
        "burst 会话须已完成拍前阶段：{:?}",
        probe.ended
    );
    assert_eq!(
        probe.ended.last(),
        Some(&Phase::RuntimeStep),
        "失败步进须是最后一个相位：{:?}",
        probe.ended
    );
    assert!(
        !probe.ended.contains(&Phase::Finish),
        "真实步进失败后不得执行终结相位"
    );
    assert_eq!(probe.steps, vec![1], "只发起过一次步进");
}
