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
            assert_eq!(
                state.route_edge_index, 0,
                "红灯不越门（dt={delta_ms}）"
            );
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

/// 有前车红灯（静止 leader，30 m 入口边）：跟车轴由 IIDM 平滑减速；主体
/// 减速不超过紧急界、末端清零拍属裁定保留的 leader 几何投影例外。
#[test]
fn red_light_with_stopped_leader_brakes_smoothly() {
    let revision = revision("runtime-fixture-policy", |module| {
        // 与 add_signal_corridor 同构的 30 m 入口边走廊（停车线在入口边末）。
        super::placement_admission_tests::add_edge(module, "entry", 30.0, 10.0, Some("middle"));
        super::placement_admission_tests::add_edge(module, "middle", 8.0, 10.0, Some("exit"));
        super::placement_admission_tests::add_edge(module, "exit", 12.0, 10.0, None);
        add_signal_corridor_gates(module, SignalAspect::Red);
    });
    let mut world = install_dt(revision, 100);
    let route = route_of(&mut world);
    // 前车停在停止线前 5 m（进度 25_000）；后车 8 m/s 从边起点接近，
    // 初始间隙 20.5 m 足够 IIDM 平滑减速。
    let leader = spawn(&mut world, route, 0, 25_000, 0).expect("leader spawn");
    let follower = spawn(&mut world, route, 0, 0, 8_000).expect("follower spawn");
    let mut previous = 8_000_u32;
    let mut max_drop = 0_u32;
    let mut stopped = false;
    for _ in 0..400 {
        world
            .step(crate::TickInput::new(100))
            .expect("follower step");
        let state = world.vehicle(follower).expect("follower");
        let leader_front = world.vehicle(leader).expect("leader").progress_mm;
        // 前车在低速窗内可向停止线蠕行（裁定窗口语义），以实际车位比对。
        assert!(
            state.progress_mm + 4_500 + 2_000 <= leader_front,
            "不侵入前车 min_gap：{} + 6500 <= {leader_front}",
            state.progress_mm
        );
        // 末端清零拍（该拍速度归零）属裁定保留的 leader 几何投影例外，
        // 不计入主体减速界。
        if state.speed_mm_s > 0 {
            max_drop = max_drop.max(previous.saturating_sub(state.speed_mm_s));
        }
        previous = state.speed_mm_s;
        if state.speed_mm_s == 0 {
            stopped = true;
            break;
        }
    }
    assert!(stopped, "follower 停稳在前车后方");
    let bound = (4.0 * 100.0) as u32 + 2;
    assert!(
        max_drop <= bound,
        "主体减速 {max_drop} ≤ 紧急界 {bound}"
    );
    let leader_state = world.vehicle(leader).expect("leader");
    assert_eq!(leader_state.route_edge_index, 0, "前车不越过停止线");
    assert!(leader_state.progress_mm <= 30_000, "前车停在入口边内");
}

/// `add_signal_corridor` 的门/信号部分（边由调用方先建）。
fn add_signal_corridor_gates(
    module: &mut laneflow_compiler::SyntheticModuleBuilder,
    aspect: SignalAspect,
) {
    use laneflow_compiler::{
        JunctionInput, JunctionReference, LaneEdgeReference, ManeuverGateInput,
        ManeuverPathInput, ManeuverPathReference, MovementInput, MovementReference,
        SignalControlInput, SignalControllerInput, SignalGroupInput, SignalGroupReference,
        SignalGroupStateInput, SignalPhaseInput, StopLineInput, StopLineReference,
    };
    let states = [SignalGroupStateInput {
        signal_group: SignalGroupReference::local("group-entry"),
        aspect,
    }];
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
            phases: &[SignalPhaseInput {
                signal_phase_key: "phase-shown",
                duration_ms: 10_000,
                states: &states,
            }],
        })
        .expect("controller");
    crate::test_policy::add_gate_policy(
        module,
        "fixture-policy",
        &[("gate-entry", laneflow_compiler::GateInterpretation::ProtectedGroup)],
    );
}
