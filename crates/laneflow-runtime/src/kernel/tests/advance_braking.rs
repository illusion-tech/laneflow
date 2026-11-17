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
