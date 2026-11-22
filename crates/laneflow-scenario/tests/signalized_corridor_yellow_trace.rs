//! 走廊黄灯接近阶段的停止决策只读诊断（#1500）。
//!
//! 在受保护转向走廊中重放 #1498 的选定样本，用 Runtime 的 `motion-trace` 诊断
//! 记录每拍的停止目标查询、门决策与停止归约数值，逐拍输出：拍初到停止线距离、
//! 舒适与紧急停车余量 `h_c = d − v²/(2b_c)`、`h_e = d − v²/(2b_e)`、门是否拒绝与
//! 豁免、P5 实际路径（列式重新求值或复用 P2 预览）、两档可行域与归约后速度，
//! 以及已提交的速度、进度与余数。只在 `motion-trace` feature 下编译：
//!
//! ```text
//! cargo +1.99.1 test --locked -p laneflow-scenario --features motion-trace \
//!     --test signalized_corridor_yellow_trace -- --nocapture
//! ```
//!
//! 逐拍表格同时写到 `CARGO_TARGET_TMPDIR/yellow-trace/<样本>.tsv`，不入仓。

#[path = "support/corridor.rs"]
mod corridor;

use std::fmt::Write as _;

use corridor::{TICK_MS, corridor};
use laneflow_runtime::motion_trace::{self, MotionPath, TraceDistance, TraceEvent, TraceMotion};
use laneflow_runtime::{
    StepOutcome, TickInput, TrafficTransitionEvent, TrafficWorld, VehicleHandle, VehicleState,
    WaitingDecision,
};
use laneflow_scenario::signalized_corridor::{
    CorridorPopulationController, CorridorReplaceAttemptOutcome,
};
use laneflow_static_contract::SignalAspect;

/// #1498 诊断中的样本身份（`9ce3c4d8` 的完整矩阵）。
struct Sample {
    name: &'static str,
    target: usize,
    seed: u64,
    /// 车辆句柄的 Debug 形式。
    vehicle: &'static str,
    /// 本组黄灯起点（ms）。
    t_y: u64,
    /// fromEdge 在路线中的下标；跨越即前杠离开该边。
    from_index: u32,
}

const SAMPLES: [Sample; 3] = [
    // 黄灯起点距线 10.3 m、4.9 m/s：舒适减速度即可停住。
    Sample {
        name: "S1",
        target: 50,
        seed: 7,
        vehicle: "VehicleHandle { index: 4, generation: 0 }",
        t_y: 52_064,
        from_index: 0,
    },
    // 黄灯起点距线 3.0 m、5.86 m/s：紧急停车距离 2.86 m，刚好可停。
    Sample {
        name: "S2",
        target: 100,
        seed: 0,
        vehicle: "VehicleHandle { index: 81, generation: 0 }",
        t_y: 38_048,
        from_index: 0,
    },
    // 黄灯起点距线 11.6 m、12.5 m/s：紧急停车距离 13.1 m，起点即不可停（对照）。
    Sample {
        name: "S3",
        target: 100,
        seed: 42,
        vehicle: "VehicleHandle { index: 54, generation: 0 }",
        t_y: 2_016,
        from_index: 0,
    },
];

/// 跨越后再记录的拍数。
const AFTER_CROSSING_TICKS: u32 = 4;
/// 记录窗口上限：黄灯 + 全红。
const WINDOW_MS: u64 = 4_016;

/// 一拍提交后的可观测结果，用于比较开启与关闭记录。
#[derive(Debug, PartialEq)]
struct Committed {
    outcome: StepOutcome,
    vehicles: Vec<VehicleState>,
    aspects: Vec<SignalAspect>,
    transitions: Vec<TrafficTransitionEvent>,
    waiting: Vec<WaitingDecision>,
}

struct Replay {
    vehicle: VehicleHandle,
    /// 记录窗口内每拍（拍号 = 拍初时间）之后被追踪车辆的状态。
    states: Vec<(u64, VehicleState)>,
    /// 记录窗口起点时被追踪车辆的状态。
    initial: VehicleState,
    committed: Vec<Committed>,
    events: Vec<TraceEvent>,
}

fn step(world: &mut TrafficWorld, controller: &mut CorridorPopulationController) -> StepOutcome {
    let revision = world.revision().network_revision();
    controller
        .apply_pending(revision, world.policy_selection(), |old, input| {
            CorridorReplaceAttemptOutcome::from_replace(world.replace_completed_vehicle(old, input))
        })
        .expect("apply");
    let outcome = world.step(TickInput::new(TICK_MS)).expect("step");
    controller.consume_world(world).expect("consume");
    outcome
}

fn find(world: &TrafficWorld, name: &str) -> VehicleHandle {
    world
        .live_vehicles()
        .iter()
        .copied()
        .find(|handle| format!("{handle:?}") == name)
        .unwrap_or_else(|| panic!("{name} 不在世界中"))
}

fn replay(sample: &Sample, trace: bool) -> Replay {
    let (mut world, mut controller) = corridor(sample.target, sample.seed);
    while world.time_ms() < sample.t_y {
        step(&mut world, &mut controller);
    }
    assert_eq!(world.time_ms(), sample.t_y, "黄灯起点对齐 16 ms 拍");
    let vehicle = find(&world, sample.vehicle);
    let initial = world.vehicle(vehicle).expect("live");
    assert_eq!(
        initial.route_edge_index(),
        sample.from_index,
        "黄灯起点仍在 fromEdge"
    );
    if trace {
        motion_trace::start(&[vehicle]);
    }
    let mut states = Vec::new();
    let mut committed = Vec::new();
    let mut after = None;
    while world.time_ms() < sample.t_y + WINDOW_MS && after != Some(0) {
        let tick = world.time_ms();
        if trace {
            motion_trace::set_tick(tick);
        }
        let outcome = step(&mut world, &mut controller);
        let state = world.vehicle(vehicle).expect("live");
        states.push((tick, state));
        committed.push(Committed {
            outcome,
            vehicles: world
                .live_vehicles()
                .iter()
                .map(|&handle| world.vehicle(handle).expect("live"))
                .collect(),
            aspects: world
                .committed_signal_groups()
                .as_slice()
                .iter()
                .map(|(_, aspect)| *aspect)
                .collect(),
            transitions: world.latest_transition_events().to_vec(),
            waiting: world.latest_waiting_decisions().to_vec(),
        });
        after = match after {
            None if state.route_edge_index() > sample.from_index => Some(AFTER_CROSSING_TICKS),
            Some(left) => Some(left - 1),
            None => None,
        };
    }
    let events = if trace {
        motion_trace::finish()
    } else {
        Vec::new()
    };
    Replay {
        vehicle,
        states,
        initial,
        committed,
        events,
    }
}

fn tick_of(event: &TraceEvent) -> u64 {
    match event {
        TraceEvent::Stops { tick, .. }
        | TraceEvent::Gate { tick, .. }
        | TraceEvent::GateBeyondWindow { tick, .. }
        | TraceEvent::SpeedDrop { tick, .. }
        | TraceEvent::Motion { tick, .. } => *tick,
    }
}

/// 一拍的解释：P5 的实际路径与它所用的停止归约数值。
struct Explained<'a> {
    path: MotionPath,
    motion: Option<&'a TraceMotion>,
    gate: Option<&'a TraceEvent>,
    beyond_window: bool,
    speed_drop: Option<f32>,
}

fn explain(events: &[TraceEvent], tick: u64) -> Explained<'_> {
    let of_tick: Vec<_> = events.iter().filter(|e| tick_of(e) == tick).collect();
    let motion_of = |wanted: MotionPath| {
        of_tick.iter().find_map(|event| match event {
            TraceEvent::Motion { path, motion, .. } if *path == wanted => Some(motion.as_ref()),
            _ => None,
        })
    };
    let (path, motion) = if motion_of(MotionPath::PreviewReused).is_some() {
        (
            MotionPath::PreviewReused,
            motion_of(MotionPath::Scalar).flatten(),
        )
    } else {
        (
            MotionPath::Columnar,
            motion_of(MotionPath::Columnar).flatten(),
        )
    };
    // 同拍可能查询多次（P2 与 P5）；取最后一次逐门解释。
    let gate = of_tick
        .iter()
        .rev()
        .find(|event| matches!(event, TraceEvent::Gate { .. }))
        .copied();
    let beyond_window = gate.is_none()
        && of_tick.iter().any(|event| {
            matches!(
                event,
                TraceEvent::GateBeyondWindow { .. }
                    | TraceEvent::Stops {
                        signal_skipped: true,
                        ..
                    }
            )
        });
    let numeric = match path {
        MotionPath::PreviewReused => MotionPath::Scalar,
        other => other,
    };
    let speed_drop = of_tick.iter().rev().find_map(|event| match event {
        TraceEvent::SpeedDrop {
            path, speed_m_s, ..
        } if *path == numeric => Some(*speed_m_s),
        _ => None,
    });
    Explained {
        path,
        motion,
        gate,
        beyond_window,
        speed_drop,
    }
}

fn margin_mm(distance_mm: u32, speed_mm_s: u32, decel: f32) -> f64 {
    let v = f64::from(speed_mm_s) / 1_000.0;
    f64::from(distance_mm) - v * v / (2.0 * f64::from(decel)) * 1_000.0
}

fn opt(value: Option<f32>) -> String {
    value.map_or_else(|| "none".to_owned(), |v| format!("{v:.4}"))
}

/// 逐拍表格；列含义见文件头注释。
fn table(sample: &Sample, replay: &Replay) -> String {
    let mut out = String::new();
    writeln!(
        out,
        "# {} 人口 {} seed {} 车辆 {} t_y {}",
        sample.name, sample.target, sample.seed, sample.vehicle, sample.t_y
    )
    .expect("write");
    writeln!(
        out,
        "tick\tedge\tprogress\tspeed\tcarry\td_mm\th_c_mm\th_e_mm\tgate\tp5\tstop_m\t\
         limit\thard_room\tleader_gap\tleader_speed\t\
         proposal\tfloor\tsafe_cap\tcandidate\tcomfort\temergency\treduced\tspeed_drop\t\
         next_speed\tnext_progress\tnext_carry\tnext_edge"
    )
    .expect("write");
    let mut previous = replay.initial;
    for &(tick, next) in &replay.states {
        let explained = explain(&replay.events, tick);
        let (gate, distance) = match explained.gate {
            Some(TraceEvent::Gate {
                distance,
                restrictive,
                dilemma_exempt,
                ..
            }) => (
                format!(
                    "{}{}",
                    if *restrictive { "deny" } else { "allow" },
                    if *dilemma_exempt { "+exempt" } else { "" }
                ),
                match distance {
                    TraceDistance::Finite(mm) => Some(*mm),
                    TraceDistance::Beyond => None,
                },
            ),
            _ if explained.beyond_window => ("beyond-window".to_owned(), None),
            _ => ("none".to_owned(), None),
        };
        let (h_c, h_e) = match (distance, explained.motion) {
            (Some(d), Some(motion)) => (
                format!(
                    "{:.0}",
                    margin_mm(d, previous.speed_mm_s(), motion.comfort_decel)
                ),
                format!(
                    "{:.0}",
                    margin_mm(d, previous.speed_mm_s(), motion.emergency_decel)
                ),
            ),
            _ => ("-".to_owned(), "-".to_owned()),
        };
        let numbers = explained.motion.map_or_else(
            || "-\t-\t-\t-\t-\t-\t-\t-\t-\t-\t-\t-".to_owned(),
            |m| {
                format!(
                    "{:.3}\t{}\t{}\t{}\t{}\t{:.4}\t{:.4}\t{}\t{:.4}\t{}\t{}\t{:.4}",
                    m.stop_m,
                    m.current_limit_mm_s,
                    m.hard_room_mm,
                    m.leader_gap_mm
                        .map_or_else(|| "none".to_owned(), |gap| gap.to_string()),
                    m.leader_speed_mm_s,
                    m.proposal_speed,
                    m.floor,
                    opt(m.safe_cap),
                    m.candidate,
                    opt(m.comfort),
                    opt(m.emergency),
                    m.reduced
                )
            },
        );
        writeln!(
            out,
            "{tick}\t{}\t{}\t{}\t{}\t{}\t{h_c}\t{h_e}\t{gate}\t{:?}\t{numbers}\t{}\t{}\t{}\t{}\t{}",
            previous.route_edge_index(),
            previous.progress_mm(),
            previous.speed_mm_s(),
            previous.carry_um(),
            distance.map_or_else(|| "-".to_owned(), |d| d.to_string()),
            explained.path,
            opt(explained.speed_drop),
            next.speed_mm_s(),
            next.progress_mm(),
            next.carry_um(),
            next.route_edge_index(),
        )
        .expect("write");
        previous = next;
    }
    out
}

#[test]
fn yellow_approach_samples_are_explained_tick_by_tick() {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("yellow-trace");
    std::fs::create_dir_all(&dir).expect("trace dir");
    for sample in &SAMPLES {
        let replay = replay(sample, true);
        assert!(
            replay
                .states
                .iter()
                .any(|(_, state)| state.route_edge_index() > sample.from_index),
            "{} 在记录窗口内跨越",
            sample.name
        );
        for &(tick, _) in &replay.states {
            let explained = explain(&replay.events, tick);
            // 每一拍都能指出 P5 的路径，且有对应的停止归约数值。
            assert!(
                explained.motion.is_some(),
                "{} 拍 {tick} 缺少 {:?} 路径的求值记录：{:#?}",
                sample.name,
                explained.path,
                replay
                    .events
                    .iter()
                    .filter(|e| tick_of(e) == tick)
                    .collect::<Vec<_>>()
            );
        }
        assert!(
            replay.events.iter().all(|event| match event {
                TraceEvent::Stops { vehicle, .. }
                | TraceEvent::Gate { vehicle, .. }
                | TraceEvent::GateBeyondWindow { vehicle, .. }
                | TraceEvent::SpeedDrop { vehicle, .. }
                | TraceEvent::Motion { vehicle, .. } => *vehicle == replay.vehicle,
            }),
            "只记录被追踪车辆"
        );
        let table = table(sample, &replay);
        println!("{table}");
        std::fs::write(dir.join(format!("{}.tsv", sample.name)), table).expect("write trace");
    }
}

#[test]
fn tracing_does_not_change_committed_state() {
    for sample in &SAMPLES {
        let traced = replay(sample, true);
        let plain = replay(sample, false);
        assert!(!traced.events.is_empty(), "{} 开启时有记录", sample.name);
        assert_eq!(
            traced.committed.len(),
            plain.committed.len(),
            "{} 记录窗口长度",
            sample.name
        );
        for (index, (with, without)) in traced.committed.iter().zip(&plain.committed).enumerate() {
            assert_eq!(
                with, without,
                "{} 窗口内第 {index} 拍的已提交结果因记录而改变",
                sample.name
            );
        }
    }
}

#[test]
fn preview_reuse_ticks_are_recorded_with_their_preview() {
    let mut reused = 0;
    for sample in &SAMPLES {
        let replay = replay(sample, true);
        for &(tick, _) in &replay.states {
            let explained = explain(&replay.events, tick);
            if explained.path == MotionPath::PreviewReused {
                // 复用拍的数值来自同拍 P2 预览的标量求值记录。
                assert!(
                    explained.motion.is_some(),
                    "{} 拍 {tick} 复用预览但缺少预览求值记录",
                    sample.name
                );
                reused += 1;
            }
        }
    }
    assert!(reused > 0, "选定样本中应出现 P5 复用 P2 预览的拍");
}
