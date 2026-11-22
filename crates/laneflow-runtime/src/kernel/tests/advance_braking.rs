//! #1272 提前制动行为锚：红灯提前减速且减速度不随步长放大、黄灯困境区
//! 逐拍许可的临界两侧。fixture 复用 placement_admission 的信号走廊（profile 0：
//! comfort 2.0 / emergency 4.0 m/s²，入口边 10 m，停止线在入口边末）。

use super::placement_admission_tests::{add_signal_corridor, install_dt, revision, spawn};
use laneflow_static_contract::SignalAspect;

fn red_world(delta_ms: u64) -> crate::TrafficWorld {
    let revision = revision("runtime-fixture-policy", |module| {
        add_signal_corridor(module, SignalAspect::Red);
    });
    install_dt(revision, delta_ms)
}

fn aspect_world(aspect: SignalAspect, delta_ms: u64) -> crate::TrafficWorld {
    let revision = revision("runtime-fixture-policy", |module| {
        add_signal_corridor(module, aspect);
    });
    install_dt(revision, delta_ms)
}

fn route_of(world: &mut crate::TrafficWorld) -> crate::RouteHandle {
    super::placement_admission_tests::register_named(
        world,
        "runtime-fixture-policy",
        &["entry", "middle", "exit"],
    )
}

/// 无前车红灯：减速在到线前开始、单拍已提交速度差不超过紧急减速度（含量化
/// 容差），且该界不随步长缩短而放大——这正是 #1272 立项时 100/200/500 m/s²
/// 伪影的回归锚。车辆停在停止线前、不越过门。
#[test]
fn red_light_brakes_in_advance_and_decel_is_step_size_independent() {
    for delta_ms in [20_u64, 50, 100] {
        let mut world = red_world(delta_ms);
        let route = route_of(&mut world);
        // 8 m/s，紧急刹停 8 m < 10 m 入口边，准入放行；舒适刹停 16 m 放不进，
        // 全程处于紧急域之下的正常停车前提内。
        let vehicle = spawn(&mut world, route, 0, 0, 8_000).expect("spawn");
        let mut previous = 8_000_u32;
        let mut max_drop = 0_u32;
        let mut slowed_before_line = false;
        let mut stopped = false;
        for _ in 0..400 {
            world
                .step(crate::TickInput::new(delta_ms))
                .expect("red light step");
            let state = world.vehicle(vehicle).expect("vehicle");
            assert_eq!(state.route_edge_index, 0, "红灯不越门（dt={delta_ms}）");
            assert!(state.progress_mm <= 10_000, "不越过停止线（dt={delta_ms}）");
            let drop = previous.saturating_sub(state.speed_mm_s);
            max_drop = max_drop.max(drop);
            if drop > 0 && state.progress_mm < 9_000 {
                slowed_before_line = true;
            }
            previous = state.speed_mm_s;
            if state.speed_mm_s == 0 {
                stopped = true;
                break;
            }
        }
        assert!(slowed_before_line, "减速在到线前开始（dt={delta_ms}）");
        assert!(stopped, "车辆停稳（dt={delta_ms}）");
        // 紧急 4.0 m/s² + 量化容差 2 mm/s；旧伪影为 v/Δt（20 ms 时 400 m/s²）。
        let bound = (4.0 * delta_ms as f32) as u32 + 2;
        assert!(
            max_drop <= bound,
            "单拍速度差 {max_drop} ≤ 紧急界 {bound}（dt={delta_ms}）"
        );
    }
}

/// 黄灯困境区：绿灯巡航中在紧急刹停距离内切黄灯，当拍按许可跨越，世界继续
/// 合法步进；车辆不会因红灯式的硬拒绝被钉在停止线上。
#[test]
fn yellow_dilemma_crosses_when_unstoppable() {
    let mut world = aspect_world(SignalAspect::Green, 100);
    let route = route_of(&mut world);
    let vehicle = spawn(&mut world, route, 0, 0, 8_000).expect("spawn");
    // 前进到距停止线约 2 m（8 m/s 紧急刹停 8 m，2 m 放不进 → 困境区）。
    loop {
        world.step(crate::TickInput::new(100)).expect("green step");
        let state = world.vehicle(vehicle).expect("vehicle");
        assert_eq!(state.route_edge_index, 0, "绿灯期可自由前进");
        if state.progress_mm >= 8_000 {
            break;
        }
    }
    let mut crossed = false;
    for _ in 0..10 {
        // 程序单相位恒绿；每拍前把已提交灯色改写为黄（车辆读拍初快照），
        // 模拟黄灯持续。
        world
            .state
            .committed
            .signal_aspects
            .fill(SignalAspect::Yellow);
        world
            .step(crate::TickInput::new(100))
            .expect("yellow dilemma step");
        let state = world.vehicle(vehicle).expect("vehicle");
        if state.route_edge_index > 0 {
            crossed = true;
            break;
        }
    }
    assert!(crossed, "困境区内的黄灯门当拍许可跨越");
}

/// 按裁定的逐拍困境区语义：舒适域带宽按构造只有一拍舒适减速
/// （`[v−b_c·Δt, v)`），巡航中的黄灯车随 IIDM 加速骑到紧急界、沿紧急
/// 减速度下行，进入困境区半径（`v²/(2·b_e) > d`）后逐拍经许可过门。
/// 该用例固化此可观测后果：全程单拍已提交速度差不超过紧急界、世界合法
/// 步进、门被跨越。真正的「停车」由红灯锚（停止目标 + 到位衰减）覆盖。
#[test]
fn yellow_boundary_ride_stays_within_emergency_bounds_and_crosses() {
    let mut world = aspect_world(SignalAspect::Yellow, 100);
    let route = route_of(&mut world);
    let vehicle = spawn(&mut world, route, 0, 0, 6_000).expect("spawn");
    let mut previous = 6_000_u32;
    let mut max_drop = 0_u32;
    let mut crossed = false;
    for _ in 0..100 {
        world
            .step(crate::TickInput::new(100))
            .expect("yellow boundary ride step");
        let state = world.vehicle(vehicle).expect("vehicle");
        max_drop = max_drop.max(previous.saturating_sub(state.speed_mm_s));
        previous = state.speed_mm_s;
        if state.route_edge_index > 0 {
            crossed = true;
            break;
        }
    }
    assert!(crossed, "困境区半径内逐拍许可跨越");
    let bound = (4.0 * 100.0) as u32 + 2;
    assert!(
        max_drop <= bound,
        "骑紧急界全程单拍速度差 {max_drop} ≤ 紧急界 {bound}"
    );
}

/// #1489 / vehicle-following §10.2：静止前车后、起始跟车可行的 follower 逐拍保持
/// 可行并停在保留最小间隙处；包括最后停稳拍在内，单拍降幅不超过
/// `b_e·Δt + ε_v`（ε_v = 1 mm/s），不依赖几何投影清零。覆盖多种步长、初速与
/// 前车位置（profile 0：emergency 4.0 m/s²，min_gap 2 m，车长 4.5 m）。
#[test]
fn stopped_leader_follower_stops_within_emergency_bound() {
    for delta_ms in [20_u64, 50, 100] {
        for (leader_progress, follower_speed) in [
            (25_000_u32, 8_000_u32),
            (15_000, 8_000),
            (25_000, 4_000),
            (12_000, 5_000),
        ] {
            let revision = revision("runtime-fixture-policy", |module| {
                // 与 add_signal_corridor 同构的 30 m 入口边走廊（停车线在入口边末）。
                super::placement_admission_tests::add_edge(
                    module,
                    "entry",
                    30.0,
                    10.0,
                    Some("middle"),
                );
                super::placement_admission_tests::add_edge(
                    module,
                    "middle",
                    8.0,
                    10.0,
                    Some("exit"),
                );
                super::placement_admission_tests::add_edge(module, "exit", 12.0, 10.0, None);
                add_signal_corridor_gates(module, SignalAspect::Red);
            });
            let mut world = install_dt(revision, delta_ms);
            let route = route_of(&mut world);
            let leader = spawn(&mut world, route, 0, leader_progress, 0).expect("leader spawn");
            let follower = spawn(&mut world, route, 0, 0, follower_speed).expect("follower spawn");
            let case = format!("dt={delta_ms} leader={leader_progress} v={follower_speed}");
            let mut previous = follower_speed;
            let mut max_drop = 0_u32;
            let mut stopped = false;
            for _ in 0..2_000 {
                world
                    .step(crate::TickInput::new(delta_ms))
                    .expect("follower step");
                let state = world.vehicle(follower).expect("follower");
                let leader_front = world.vehicle(leader).expect("leader").progress_mm;
                assert!(
                    state.progress_mm + 4_500 + 2_000 <= leader_front,
                    "不侵入前车 min_gap（{case}）：{} + 6500 <= {leader_front}",
                    state.progress_mm
                );
                max_drop = max_drop.max(previous.saturating_sub(state.speed_mm_s));
                previous = state.speed_mm_s;
                if state.speed_mm_s == 0 {
                    stopped = true;
                    break;
                }
            }
            assert!(stopped, "follower 停稳在前车后方（{case}）");
            let bound = (4.0 * delta_ms as f64).ceil() as u32 + 1;
            assert!(
                max_drop <= bound,
                "单拍降幅 {max_drop} ≤ b_e·Δt + ε_v = {bound}（{case}）"
            );
        }
    }
}

/// `add_signal_corridor` 的门/信号部分（边由调用方先建）。
fn add_signal_corridor_gates(
    module: &mut laneflow_compiler::SyntheticModuleBuilder,
    aspect: SignalAspect,
) {
    add_signal_corridor_program(module, &[(aspect, 10_000)]);
}

/// 同 [`add_signal_corridor_gates`]，控制器程序按给定 (灯色, 时长) 序列循环；
/// F1' 复审修锚需要「黄灯 10 s → 绿灯」两相位程序。
fn add_signal_corridor_program(
    module: &mut laneflow_compiler::SyntheticModuleBuilder,
    phases: &[(SignalAspect, u64)],
) {
    use laneflow_compiler::{
        JunctionInput, JunctionReference, LaneEdgeReference, ManeuverGateInput, ManeuverPathInput,
        ManeuverPathReference, MovementInput, MovementReference, SignalControlInput,
        SignalControllerInput, SignalGroupInput, SignalGroupReference, SignalGroupStateInput,
        SignalPhaseInput, StopLineInput, StopLineReference,
    };
    const PHASE_KEYS: [&str; 2] = ["phase-first", "phase-second"];
    let states: Vec<SignalGroupStateInput> = phases
        .iter()
        .map(|(aspect, _)| SignalGroupStateInput {
            signal_group: SignalGroupReference::local("group-entry"),
            aspect: *aspect,
        })
        .collect();
    let phase_inputs: Vec<SignalPhaseInput> = phases
        .iter()
        .enumerate()
        .map(|(index, (_, duration_ms))| SignalPhaseInput {
            signal_phase_key: PHASE_KEYS[index],
            duration_ms: *duration_ms,
            states: &states[index..index + 1],
        })
        .collect();
    let groups = [SignalGroupReference::local("group-entry")];
    module
        .add_junction(JunctionInput {
            junction_key: "junction-main",
        })
        .expect("junction")
        .add_movement(MovementInput {
            turn_direction: None,
            movement_key: "movement-through",
            junction: JunctionReference::local("junction-main"),
            directed_entry_approach_key: "approach-westbound",
            directed_exit_approach_key: "approach-eastbound",
        })
        .expect("movement")
        .add_maneuver_path(ManeuverPathInput {
            maneuver_path_key: "path-main",
            movement: MovementReference::local("movement-through"),
            entry_edge: LaneEdgeReference::local("entry"),
            internal_edges: &[LaneEdgeReference::local("middle")],
            exit_edge: LaneEdgeReference::local("exit"),
        })
        .expect("path")
        .add_stop_line(StopLineInput {
            stop_line_key: "stop-entry",
            lane_edge: LaneEdgeReference::local("entry"),
        })
        .expect("stop")
        .add_signal_group(SignalGroupInput {
            signal_group_key: "group-entry",
        })
        .expect("group")
        .add_maneuver_gate(ManeuverGateInput {
            maneuver_gate_key: "gate-entry",
            maneuver_path: ManeuverPathReference::local("path-main"),
            transition_index: 0,
            stop_line: StopLineReference::local("stop-entry"),
            signal_control: SignalControlInput::Group(SignalGroupReference::local("group-entry")),
        })
        .expect("gate")
        .add_signal_controller(SignalControllerInput {
            signal_controller_key: "controller-main",
            offset_ms: 0,
            signal_groups: &groups,
            phases: &phase_inputs,
        })
        .expect("controller");
    crate::test_policy::add_gate_policy(
        module,
        "fixture-policy",
        &[(
            "gate-entry",
            laneflow_compiler::GateInterpretation::ProtectedGroup,
        )],
    );
}

/// F1' 复审修生产路径锚：黄灯 10 s → 绿灯程序下，拍初可停（6²/(2·4) =
/// 4.5 m ＜ 10 m）的车辆，其到达下界保持生产运动学值（≈1 416 ms，不晚于
/// 实际合法跨门 1 500 ms），不被抬到放行 10 000 ms、也不出 3 000 ms 时窗；
/// 同几何纯红（红 → 绿直切）保持仍正常抬升出窗——差异只来自区间含黄灯。
#[test]
fn yellow_program_keeps_frontier_arrival_lower_bound_conservative() {
    fn two_phase_world(
        first: SignalAspect,
        second: SignalAspect,
    ) -> (crate::TrafficWorld, crate::VehicleHandle) {
        let network = revision("runtime-fixture-policy", |module| {
            super::placement_admission_tests::add_edge(module, "entry", 10.0, 10.0, Some("middle"));
            super::placement_admission_tests::add_edge(module, "middle", 8.0, 10.0, Some("exit"));
            super::placement_admission_tests::add_edge(module, "exit", 12.0, 10.0, None);
            add_signal_corridor_program(module, &[(first, 10_000), (second, 10_000)]);
        });
        let mut world = install_dt(network, 100);
        let route = route_of(&mut world);
        let vehicle = spawn(&mut world, route, 0, 0, 6_000).expect("spawn");
        (world, vehicle)
    }
    let (world, vehicle) = two_phase_world(SignalAspect::Yellow, SignalAspect::Green);
    let state = world.vehicle(vehicle).expect("vehicle");
    let view = world.state.read_view();
    let prepared = crate::kernel::entry_frontier::PreparedSignalApproach::new(view, &state, 4.0);
    let eta = crate::kernel::conflict::PreparedApproachEta::new(0, 6_000, 1.5, 3_000)
        .expect("finite eta inputs");
    let kinematic = eta.lower_bound(10_000);
    let crate::ApproachEstimate::Finite(bound_ms) = kinematic else {
        panic!("kinematic lower bound must be finite");
    };
    assert!(
        bound_ms <= 1_500,
        "运动学下界 {bound_ms} ms 不晚于实际合法跨门 1 500 ms"
    );
    assert_eq!(
        prepared.apply(kinematic, 10_000, 3_000),
        kinematic,
        "含黄灯的保持弃证，保留运动学下界"
    );

    let (world, vehicle) = two_phase_world(SignalAspect::Red, SignalAspect::Green);
    let state = world.vehicle(vehicle).expect("vehicle");
    let view = world.state.read_view();
    let prepared = crate::kernel::entry_frontier::PreparedSignalApproach::new(view, &state, 4.0);
    assert_eq!(
        prepared.apply(kinematic, 10_000, 3_000),
        crate::ApproachEstimate::OutsideHorizon,
        "同几何纯红保持（红 → 绿直切）仍抬升出窗"
    );
}

/// F5' 复审修锚：黄灯困境区豁免的门对 Waiting 非入口决定发布
/// `NotRequired`（与 Conflict 发布同源的拍初有效决定），不再被纯灯态误报
/// `NotEvaluated`；纯红灯无可豁免仍发布 `NotEvaluated`。
#[test]
fn waiting_non_entry_outcome_consumes_effective_yellow_decision() {
    let mut world = aspect_world(SignalAspect::Green, 100);
    let route = route_of(&mut world);
    let vehicle = spawn(&mut world, route, 0, 0, 8_000).expect("spawn");
    // 前进到困境区（8 m/s 紧急刹停 8 m，距停止线约 2 m 放不进）。
    loop {
        world.step(crate::TickInput::new(100)).expect("green step");
        if world.vehicle(vehicle).unwrap().progress_mm >= 8_000 {
            break;
        }
    }
    world
        .state
        .committed
        .signal_aspects
        .fill(SignalAspect::Yellow);
    let state = world.vehicle(vehicle).unwrap();
    let (gate, distance) = {
        let view = world.state.read_view();
        let compiled = view.compiled_route(state.route).expect("compiled");
        let gate = compiled.hop_gate[0].expect("gate");
        let position = crate::kernel::vehicle_store::MotionPosition::from(&state);
        (
            gate,
            crate::kernel::conflict_tick::gate_hop_line_distance(compiled, &position, 0),
        )
    };
    world
        .state
        .committed
        .signal_aspects
        .fill(SignalAspect::Yellow);
    {
        let view = world.state.read_view();
        assert!(
            distance.is_some_and(|mm| {
                view.gate_yellow_dilemma_exempt(gate, state.profile, state.speed_mm_s, mm)
            }),
            "拍初确处困境区（速度 {}、距线 {:?}）",
            state.speed_mm_s,
            distance
        );
        assert_eq!(
            crate::kernel::waiting::non_entry_gate_outcome(
                view,
                gate,
                state.profile,
                state.speed_mm_s,
                distance,
            ),
            crate::WaitingDecisionOutcome::NotRequired,
            "困境区豁免的门发布 NotRequired"
        );
    }
    world.state.committed.signal_aspects.fill(SignalAspect::Red);
    {
        let view = world.state.read_view();
        assert_eq!(
            crate::kernel::waiting::non_entry_gate_outcome(
                view,
                gate,
                state.profile,
                state.speed_mm_s,
                distance,
            ),
            crate::WaitingDecisionOutcome::NotEvaluated,
            "纯红灯无可豁免仍发布 NotEvaluated"
        );
    }
}
