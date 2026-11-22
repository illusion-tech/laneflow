//! 受保护转向走廊黄灯放行后的清空核验（#1496，#1446 工作包 2）与样本诊断（#1498）。
//!
//! 判据见 `support/clearance.rs` 与 signalized-corridor-protected-turning §7.3、§8。
//! 常规测试运行判据负例与一个短程真实世界冒烟；完整规范矩阵为 `#[ignore]`，
//! 正式验收用 release 显式运行：
//!
//! ```text
//! cargo +1.99.1 test --release --locked -p laneflow-scenario \
//!     --test signalized_corridor_clearance -- --ignored --nocapture
//! ```
//!
//! 完整矩阵另把每个黄灯期跨越样本（含未闭合样本）的诊断事实与候选原因写到
//! `CARGO_TARGET_TMPDIR/clearance-diagnostics/<人口>-<seed>.tsv`，并在标准输出
//! 汇总候选原因的计数。诊断只附加输出，不参与通过判定；候选原因的判定规则见
//! `support/clearance.rs` 的 `analyze`。

#[path = "support/clearance.rs"]
mod clearance;
#[path = "support/corridor.rs"]
mod corridor;

use std::collections::HashMap;
use std::fmt::Write as _;

use clearance::{
    AnalysisParams, ClearanceCheck, Factor, Frame, Kinematics, Leader, PathOccurrence, Phase,
    RouteLayout, VehicleSample, Violation, YellowSample, analyze,
};
use corridor::{TICK_MS, corridor};
use laneflow_runtime::{RouteHandle, TickInput, TrafficWorld, VehicleHandle, VehicleStatus};
use laneflow_scenario::signalized_corridor::CorridorReplaceAttemptOutcome;
use laneflow_static_contract::{ManeuverPathOrdinal, SignalAspect, SignalGroupOrdinal};

/// 12-phase 程序周期 84 064 ms（走廊 §7.2）。
const CYCLE_TICKS: u32 = 5_254;
/// 规范人口（走廊 §8.1）。
const TARGETS: [usize; 3] = [50, 100, 200];
/// 规范压力 seed 集（走廊 §8.1），含默认 seed 0。
const SEEDS: [u64; 4] = [0, 7, 42, 101];
/// 每个组合运行的完整周期数：两个控制器的每个组都经历多次黄灯与交接。
const MATRIX_CYCLES: u32 = 4;
/// 本 profile 的黄灯 + 全红（走廊 §7.2：3 008 + 1 008 ms）。
const AUTHORED_YELLOW_PLUS_ALL_RED_MS: u64 = 4_016;

// ---------- 判据负例（合成时间线，不依赖 Runtime） ----------

/// 一个控制器两组：组 0 的路线 0 在下标 0→1 处穿过内部边 `[100, 110)`。
fn toy() -> ClearanceCheck<u32, u32> {
    let mut check = ClearanceCheck::new(&[0, 0]);
    check.add_layout(
        0,
        RouteLayout {
            prefix_mm: vec![0, 100, 110],
            occurrences: vec![PathOccurrence {
                path: 7,
                group: 0,
                from_index: 0,
                start_mm: 100,
                end_mm: 110,
            }],
        },
    );
    check
}

fn car(edge_index: u32, progress_mm: u32) -> VehicleSample<u32, u32> {
    moving(edge_index, progress_mm, 0, None)
}

fn moving(
    edge_index: u32,
    progress_mm: u32,
    speed_mm_s: u32,
    leader: Option<Leader>,
) -> VehicleSample<u32, u32> {
    VehicleSample {
        key: 1,
        route: 0,
        edge_index,
        progress_mm,
        length_mm: 4,
        speed_mm_s,
        limit_mm_s: 1_000,
        leader,
    }
}

const TOY_PARAMS: AnalysisParams = AnalysisParams {
    emergency_decel_mm_s2: 6_000,
    comfort_decel_mm_s2: 2_000,
    min_gap_mm: 2,
    time_headway_ms: 1_000,
};

fn step(
    check: &mut ClearanceCheck<u32, u32>,
    time_ms: u64,
    aspects: [SignalAspect; 2],
    vehicles: &[VehicleSample<u32, u32>],
) {
    check.observe(&Frame {
        time_ms,
        aspects: &aspects,
        vehicles,
    });
}

use SignalAspect::{Green, Red, Yellow};

#[test]
fn yellow_entry_clearing_within_yellow_plus_all_red_passes() {
    let mut check = toy();
    step(&mut check, 0, [Green, Red], &[car(0, 90)]);
    step(&mut check, 10, [Yellow, Red], &[car(0, 99)]);
    // 拍初黄灯，跨越边界 20。
    step(&mut check, 20, [Yellow, Red], &[car(1, 2)]);
    step(&mut check, 30, [Red, Red], &[car(2, 5)]);
    // 车尾 115 − 4 = 111 ≥ 110，t_c = 30 已在上一边界成立；40 转绿。
    step(&mut check, 40, [Red, Green], &[car(2, 9)]);
    assert!(check.violations.is_empty(), "{:?}", check.violations);
    assert_eq!(check.samples.len(), 1);
    let sample = &check.samples[0];
    assert_eq!((sample.t_y, sample.t_e, sample.t_c), (10, 20, Some(30)));
    assert_eq!(sample.budget_ms, Some(30));
    assert_eq!(check.handoffs, 1);
    assert_eq!(check.traversals[&7], 1);
}

#[test]
fn clearance_after_the_next_green_is_rejected() {
    // 等价于“全红过短”：车尾在下一组转绿之后才离开内部边。
    let mut check = toy();
    step(&mut check, 0, [Green, Red], &[car(0, 90)]);
    step(&mut check, 10, [Yellow, Red], &[car(0, 99)]);
    step(&mut check, 20, [Yellow, Red], &[car(1, 1)]);
    step(&mut check, 30, [Red, Green], &[car(1, 6)]);
    step(&mut check, 40, [Red, Green], &[car(2, 5)]);
    let late = check
        .violations
        .iter()
        .filter(|v| matches!(v, Violation::LateClearance(s) if s.elapsed_ms() == Some(30) && s.budget_ms == Some(20)))
        .count();
    let occupied = check
        .violations
        .iter()
        .filter(|v| {
            matches!(
                v,
                Violation::HandoffOccupied {
                    time_ms: 30,
                    green_group: 1,
                    previous_group: 0,
                    ..
                }
            )
        })
        .count();
    assert_eq!((late, occupied), (1, 1), "{:?}", check.violations);
}

#[test]
fn handoff_rejects_a_green_entry_tail_still_on_the_internal_edge() {
    // 绿灯期进入、车尾滞留内部边：不产生黄灯样本，但交接判据必须检出。
    let mut check = toy();
    step(&mut check, 0, [Green, Red], &[car(0, 99)]);
    step(&mut check, 10, [Green, Red], &[car(1, 3)]);
    step(&mut check, 20, [Yellow, Red], &[car(1, 3)]);
    step(&mut check, 30, [Red, Red], &[car(1, 3)]);
    step(&mut check, 40, [Red, Green], &[car(1, 3)]);
    assert!(check.samples.is_empty());
    assert!(
        matches!(
            check.violations.as_slice(),
            [Violation::HandoffOccupied {
                time_ms: 40,
                rear_mm: 99,
                front_mm: 103,
                ..
            }]
        ),
        "{:?}",
        check.violations
    );
}

#[test]
fn red_entry_is_rejected() {
    let mut check = toy();
    step(&mut check, 0, [Red, Green], &[car(0, 99)]);
    step(&mut check, 10, [Red, Green], &[car(1, 1)]);
    assert!(matches!(
        check.violations.as_slice(),
        [Violation::RedEntry {
            t_e: 10,
            path: 7,
            ..
        }]
    ));
}

#[test]
fn vehicle_lost_before_clearing_is_rejected() {
    let mut check = toy();
    step(&mut check, 0, [Green, Red], &[car(0, 90)]);
    step(&mut check, 10, [Yellow, Red], &[car(0, 99)]);
    step(&mut check, 20, [Yellow, Red], &[car(1, 1)]);
    step(&mut check, 30, [Red, Red], &[]);
    assert!(matches!(
        check.violations.as_slice(),
        [Violation::LostBeforeClear(_)]
    ));
}

#[test]
fn diagnostics_keep_the_actual_entry_position_and_the_yellow_onset_state() {
    // 跨越后前杠已进入内部边 3 mm：剩余清空距离按跨越后位置计算。
    let mut check = toy();
    step(&mut check, 0, [Green, Red], &[moving(0, 80, 1_000, None)]);
    step(&mut check, 10, [Yellow, Red], &[moving(0, 90, 1_000, None)]);
    step(&mut check, 20, [Yellow, Red], &[moving(0, 98, 1_000, None)]);
    step(&mut check, 30, [Yellow, Red], &[moving(1, 3, 1_000, None)]);
    step(&mut check, 40, [Red, Red], &[moving(2, 1, 1_000, None)]);
    let sample = &check.samples[0];
    assert_eq!(
        sample.at_yellow,
        Some(Kinematics {
            time_ms: 10,
            edge_index: 0,
            progress_mm: 90,
            front_mm: 90,
            speed_mm_s: 1_000,
        })
    );
    assert_eq!((sample.before.time_ms, sample.before.front_mm), (20, 98));
    assert_eq!((sample.after.time_ms, sample.after.front_mm), (30, 103));
    let phases: Vec<_> = sample
        .trace
        .iter()
        .map(|row| (row.time_ms, row.phase))
        .collect();
    assert_eq!(
        phases,
        [
            (10, Phase::Approach),
            (20, Phase::Approach),
            (30, Phase::Inside),
            (40, Phase::Inside),
        ]
    );
    // 停止线 100，黄灯起点前杠 90：d = 10，v²/(2b_e) = 1 000² / 12 000，向上取整为 84。
    let analysis = analyze(sample, &TOY_PARAMS);
    assert_eq!(analysis.distance_to_line_mm, Some(10));
    assert_eq!(analysis.stop_distance_mm, Some(84));
    assert_eq!(analysis.cannot_stop_at_yellow, Some(true));
    // 车尾离开内部边需前杠到 110 + 4 = 114，跨越后位于 103。
    assert_eq!(analysis.remaining_mm, 11);
}

#[test]
fn unresolved_samples_keep_their_diagnostics_with_unknown_clearance() {
    let mut check = toy();
    step(&mut check, 0, [Green, Red], &[moving(0, 95, 500, None)]);
    step(&mut check, 10, [Yellow, Red], &[moving(0, 99, 500, None)]);
    step(&mut check, 20, [Yellow, Red], &[moving(1, 2, 500, None)]);
    step(&mut check, 30, [Red, Red], &[moving(1, 5, 300, None)]);
    // 窗口在此结束：未清空，也未取得预算。
    assert_eq!(check.unresolved(), 1);
    let sample = &check.samples[0];
    assert!(sample.is_unresolved() && sample.t_c.is_none());
    assert_eq!(sample.trace.len(), 3);
    let analysis = analyze(sample, &TOY_PARAMS);
    assert_eq!(analysis.remaining_budget_ms, None);
    assert_eq!(analysis.remaining_mm, 12);
    // 10 ms 内 500 → 300 mm/s，没有前车，也没有前方停止线。
    assert_eq!(analysis.factors, [Factor::UnattributedDecel { ticks: 1 }]);
}

#[test]
fn analysis_reports_several_factors_together() {
    // 剩余预算不足以按限速清空，减速时前车又在跟驰范围内：两项同时成立。
    let close = Some(Leader {
        gap_mm: 3,
        speed_mm_s: 0,
    });
    let mut check = toy();
    step(&mut check, 0, [Green, Red], &[moving(0, 95, 600, None)]);
    step(&mut check, 10, [Yellow, Red], &[moving(0, 99, 600, None)]);
    step(&mut check, 20, [Yellow, Red], &[moving(1, 1, 600, close)]);
    step(&mut check, 30, [Red, Green], &[moving(1, 4, 200, close)]);
    step(&mut check, 40, [Red, Green], &[moving(2, 5, 200, None)]);
    let sample = &check.samples[0];
    assert!(sample.is_late());
    let analysis = analyze(sample, &TOY_PARAMS);
    // 预算 20 ms，跨越时已用 10 ms；剩余 114 − 101 = 13 mm，按限速 1 000 mm/s 需 13 ms。
    assert_eq!(analysis.remaining_budget_ms, Some(10));
    assert_eq!(analysis.remaining_mm, 13);
    assert_eq!(
        analysis.factors,
        [
            Factor::TimingInsufficient {
                required_ms: 13,
                remaining_ms: 10,
            },
            Factor::LeaderInRange {
                ticks: 1,
                min_gap_mm: 3,
            },
        ]
    );
}

#[test]
fn analysis_reports_approach_ticks_that_could_stop_but_did_not_brake() {
    // 黄灯起点距停止线 50 mm、10 mm/s：紧急与舒适减速度都能停住，下一边界却没有减速。
    let mut check = toy();
    step(&mut check, 0, [Green, Red], &[moving(0, 40, 10, None)]);
    step(&mut check, 10, [Yellow, Red], &[moving(0, 50, 10, None)]);
    step(
        &mut check,
        20,
        [Yellow, Red],
        &[moving(0, 99, 20_000, None)],
    );
    step(&mut check, 30, [Yellow, Red], &[moving(1, 1, 20_000, None)]);
    step(&mut check, 40, [Red, Green], &[moving(2, 5, 20_000, None)]);
    let analysis = analyze(&check.samples[0], &TOY_PARAMS);
    assert_eq!(
        analysis.factors,
        [
            Factor::TimingInsufficient {
                required_ms: 13,
                remaining_ms: 10,
            },
            // 第二行 20 000 mm/s、距线 1 mm：豁免判据自 t_y 后 10 ms 起成立。
            Factor::StoppableWithoutBraking {
                ticks: 1,
                comfort_feasible_at_yellow: true,
                exempt_from_ms: Some(10),
            },
        ]
    );
}

// ---------- 前车：按车身占用片段查询 ----------

/// 共同进口边 0 长 100，之后分流：右转 `0 → 1 → 3`，直行 `0 → 2 → 4`。
const SPLIT_LENGTHS: [u32; 5] = [100, 10, 10, 20, 20];
const RIGHT: [usize; 3] = [0, 1, 3];
const THROUGH: [usize; 3] = [0, 2, 4];

fn split_occupancy(right_progress_mm: u32) -> Occupancy<char> {
    let mut occupancy = Occupancy::new();
    // 前车已右转进入边 1；后方直行车与更后面的车仍在共同进口边上。
    add_body(
        &mut occupancy,
        &SPLIT_LENGTHS,
        &RIGHT,
        1,
        right_progress_mm,
        5,
        700,
        'r',
    );
    add_body(&mut occupancy, &SPLIT_LENGTHS, &THROUGH, 0, 90, 5, 900, 't');
    add_body(&mut occupancy, &SPLIT_LENGTHS, &THROUGH, 0, 50, 5, 900, 'b');
    occupancy
}

#[test]
fn leader_sees_a_diverging_tail_still_on_the_shared_approach() {
    // 右转车前杠在边 1 的 2 mm 处，车长 5：边 0 上仍有 [97, 100] 的车尾。
    let occupancy = split_occupancy(2);
    assert_eq!(
        leader(&occupancy, &SPLIT_LENGTHS, &THROUGH, 0, 90, 't'),
        Some(Leader {
            gap_mm: 7,
            speed_mm_s: 700,
        })
    );
    // 后方车辆看到的是直行车，自身与身后车辆不计入。
    assert_eq!(
        leader(&occupancy, &SPLIT_LENGTHS, &THROUGH, 0, 50, 'b'),
        Some(Leader {
            gap_mm: 35,
            speed_mm_s: 900,
        })
    );
}

#[test]
fn leader_no_longer_sees_a_diverging_tail_once_it_has_cleared() {
    // 右转车前杠到边 1 的 6 mm 处：车尾 1 mm，已完全离开共同进口边。
    let occupancy = split_occupancy(6);
    assert_eq!(
        leader(&occupancy, &SPLIT_LENGTHS, &THROUGH, 0, 90, 't'),
        None
    );
}

// ---------- 真实走廊世界 ----------

/// 内部边 → (路径序号, 组序号)，以及组 → 控制器。
struct Topology {
    path_of_triple: HashMap<[u32; 3], (u32, usize)>,
    group_controllers: Vec<usize>,
    lane_lengths: Vec<u32>,
    speed_limits: Vec<u32>,
    params: AnalysisParams,
}

fn topology(world: &TrafficWorld) -> Topology {
    let revision = world.revision();
    let traffic = revision.traffic();
    let relations = traffic.relations();
    let maneuvers = traffic.maneuvers();
    let mut path_of_triple = HashMap::new();
    for raw in 0..maneuvers.maneuver_path_count() {
        let path = maneuvers
            .maneuver_path(ManeuverPathOrdinal::from_raw(raw))
            .expect("path");
        let edges = path.edges();
        assert_eq!(edges.len(), 3, "走廊 path 恰为三边（§8.4）");
        let gate = relations
            .maneuver_gate(path.maneuver_gates()[0])
            .expect("gate");
        let group = gate
            .signal_group()
            .expect("受保护走廊每个 Gate 都属于信号组");
        let triple = [edges[0].raw(), edges[1].raw(), edges[2].raw()];
        assert!(
            path_of_triple
                .insert(triple, (raw, group.index()))
                .is_none(),
            "path 三边唯一"
        );
    }
    let groups = world.committed_signal_groups().as_slice().len();
    let group_controllers = (0..groups)
        .map(|raw| {
            relations
                .signal_group(SignalGroupOrdinal::from_raw(
                    u32::try_from(raw).expect("fits"),
                ))
                .expect("group")
                .controller()
                .index()
        })
        .collect();
    // 走廊只有乘用车一种 profile（走廊 §8.1）；分析参数取自其静态数据。
    let profiles: Vec<_> = world
        .live_vehicles()
        .iter()
        .map(|&handle| world.vehicle(handle).expect("live").profile())
        .collect();
    let profile = profiles[0];
    assert!(
        profiles.iter().all(|p| *p == profile),
        "走廊只有一种 profile"
    );
    let view = relations.vehicle_profile(profile).expect("profile");
    let mm = |value: f32| {
        // profile 参数是有限正数，量化为整数毫米单位。
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "有限正数"
        )]
        let rounded = (value * 1_000.0).round() as u32;
        rounded
    };
    Topology {
        path_of_triple,
        group_controllers,
        lane_lengths: traffic.lane_lengths_millimetres().to_vec(),
        speed_limits: traffic.lane_speed_limits_millimetres_per_second().to_vec(),
        params: AnalysisParams {
            emergency_decel_mm_s2: mm(view.emergency_decel()),
            comfort_decel_mm_s2: mm(view.comfort_decel()),
            min_gap_mm: view.min_gap_mm(),
            time_headway_ms: mm(view.time_headway()),
        },
    }
}

fn layout(world: &TrafficWorld, topology: &Topology, route: RouteHandle) -> RouteLayout {
    let edges = world.route_edges(route).expect("route edges");
    let mut prefix_mm = Vec::with_capacity(edges.len() + 1);
    let mut at = 0_u64;
    for edge in edges {
        prefix_mm.push(at);
        at += u64::from(topology.lane_lengths[edge.index()]);
    }
    prefix_mm.push(at);
    let occurrences = edges
        .windows(3)
        .enumerate()
        .filter_map(|(index, window)| {
            let triple = [window[0].raw(), window[1].raw(), window[2].raw()];
            topology
                .path_of_triple
                .get(&triple)
                .map(|&(path, group)| PathOccurrence {
                    path,
                    group,
                    from_index: u32::try_from(index).expect("fits"),
                    start_mm: prefix_mm[index + 1],
                    end_mm: prefix_mm[index + 2],
                })
        })
        .collect();
    RouteLayout {
        prefix_mm,
        occurrences,
    }
}

fn frame_vehicles(
    world: &TrafficWorld,
    topology: &Topology,
    check: &mut ClearanceCheck<VehicleHandle, RouteHandle>,
    out: &mut Vec<VehicleSample<VehicleHandle, RouteHandle>>,
) {
    out.clear();
    let mut occupancy = Occupancy::new();
    let mut active = Vec::new();
    for &handle in world.live_vehicles() {
        let state = world.vehicle(handle).expect("live");
        if state.status() != VehicleStatus::Active {
            continue;
        }
        let edges = edge_indices(world, state.route());
        add_body(
            &mut occupancy,
            &topology.lane_lengths,
            &edges,
            state.route_edge_index() as usize,
            state.progress_mm(),
            state.length_mm(),
            state.speed_mm_s(),
            handle,
        );
        active.push((handle, state, edges));
    }
    for (handle, state, edges) in active {
        if !check.has_layout(state.route()) {
            check.add_layout(state.route(), layout(world, topology, state.route()));
        }
        let index = state.route_edge_index() as usize;
        out.push(VehicleSample {
            key: handle,
            route: state.route(),
            edge_index: state.route_edge_index(),
            progress_mm: state.progress_mm(),
            length_mm: state.length_mm(),
            speed_mm_s: state.speed_mm_s(),
            limit_mm_s: topology.speed_limits[edges[index]],
            leader: leader(
                &occupancy,
                &topology.lane_lengths,
                &edges,
                index,
                state.progress_mm(),
                handle,
            ),
        });
    }
}

fn edge_indices(world: &TrafficWorld, route: RouteHandle) -> Vec<usize> {
    world
        .route_edges(route)
        .expect("route edges")
        .iter()
        .map(|edge| edge.index())
        .collect()
}

/// 一段车身占用：车道边内的区间 `[start_mm, end_mm]`。
#[derive(Clone, Copy, Debug)]
struct Segment<K> {
    start_mm: u32,
    end_mm: u32,
    speed_mm_s: u32,
    key: K,
}

/// 车道边序号 → 其上的车身占用片段。
type Occupancy<K> = HashMap<usize, Vec<Segment<K>>>;

/// 按车辆自己的路线从前杠向后回溯，把整段车身分到各条车道边上；
/// 分流或汇入处车尾仍压在上一条边时，那条边上也有一段占用。
#[expect(clippy::too_many_arguments, reason = "一辆车的公开状态逐项传入")]
fn add_body<K: Copy>(
    occupancy: &mut Occupancy<K>,
    lane_lengths: &[u32],
    route_edges: &[usize],
    index: usize,
    progress_mm: u32,
    length_mm: u32,
    speed_mm_s: u32,
    key: K,
) {
    let mut remaining = length_mm;
    let mut end = progress_mm;
    let mut at = index;
    loop {
        let start = end.saturating_sub(remaining);
        occupancy.entry(route_edges[at]).or_default().push(Segment {
            start_mm: start,
            end_mm: end,
            speed_mm_s,
            key,
        });
        remaining -= end - start;
        if remaining == 0 || at == 0 {
            break;
        }
        at -= 1;
        end = lane_lengths[route_edges[at]];
    }
}

/// 沿本车路线在当前边与其后两条边上找最近的车身占用（排除本车，任何路线的车辆都计入）。
fn leader<K: Copy + Eq>(
    occupancy: &Occupancy<K>,
    lane_lengths: &[u32],
    route_edges: &[usize],
    index: usize,
    progress_mm: u32,
    own: K,
) -> Option<Leader> {
    // 坐标以本车当前边起点为原点。
    let mut offset = 0_i64;
    for (step, &edge) in route_edges.iter().enumerate().skip(index).take(3) {
        let nearest = occupancy
            .get(&edge)
            .into_iter()
            .flatten()
            .filter(|segment| segment.key != own)
            .filter(|segment| step != index || segment.end_mm > progress_mm)
            .min_by_key(|segment| segment.start_mm);
        if let Some(segment) = nearest {
            let rear = offset + i64::from(segment.start_mm);
            let gap = (rear - i64::from(progress_mm)).max(0);
            return Some(Leader {
                gap_mm: u64::try_from(gap).expect("non-negative"),
                speed_mm_s: segment.speed_mm_s,
            });
        }
        offset += i64::from(lane_lengths[edge]);
    }
    None
}

struct Report {
    target: usize,
    seed: u64,
    ticks: u32,
    samples: Vec<YellowSample<VehicleHandle, RouteHandle>>,
    params: AnalysisParams,
    unresolved: usize,
    handoffs: u64,
    max_elapsed_ms: u64,
    min_budget_ms: u64,
    max_budget_ms: u64,
    traversals: HashMap<u32, u64>,
    violations: Vec<Violation<VehicleHandle, RouteHandle>>,
}

fn run(target: usize, seed: u64, ticks: u32) -> Report {
    let (mut world, mut controller) = corridor(target, seed);
    let topology = topology(&world);
    let mut check = ClearanceCheck::new(&topology.group_controllers);
    let mut vehicles = Vec::new();
    let mut aspects = Vec::new();
    let observe = |world: &TrafficWorld,
                   check: &mut ClearanceCheck<VehicleHandle, RouteHandle>,
                   vehicles: &mut Vec<VehicleSample<VehicleHandle, RouteHandle>>,
                   aspects: &mut Vec<SignalAspect>| {
        aspects.clear();
        aspects.extend(
            world
                .committed_signal_groups()
                .as_slice()
                .iter()
                .map(|(_, aspect)| *aspect),
        );
        frame_vehicles(world, &topology, check, vehicles);
        check.observe(&Frame {
            time_ms: world.time_ms(),
            aspects,
            vehicles,
        });
    };
    observe(&world, &mut check, &mut vehicles, &mut aspects);
    for _ in 0..ticks {
        let revision = world.revision().network_revision();
        controller
            .apply_pending(revision, world.policy_selection(), |old, input| {
                CorridorReplaceAttemptOutcome::from_replace(
                    world.replace_completed_vehicle(old, input),
                )
            })
            .expect("apply");
        world.step(TickInput::new(TICK_MS)).expect("step");
        controller.consume_world(&world).expect("consume");
        observe(&world, &mut check, &mut vehicles, &mut aspects);
    }
    let resolved = check
        .samples
        .iter()
        .filter(|s| s.t_c.is_some() && s.budget_ms.is_some());
    let max_elapsed_ms = resolved
        .clone()
        .filter_map(|s| s.elapsed_ms())
        .max()
        .unwrap_or(0);
    let budgets = resolved.filter_map(|s| s.budget_ms);
    let min_budget_ms = budgets.clone().min().unwrap_or(0);
    let max_budget_ms = budgets.max().unwrap_or(0);
    Report {
        target,
        seed,
        ticks,
        unresolved: check.unresolved(),
        params: topology.params,
        handoffs: check.handoffs,
        max_elapsed_ms,
        min_budget_ms,
        max_budget_ms,
        traversals: check.traversals,
        violations: check.violations,
        samples: check.samples,
    }
}

#[test]
fn one_cycle_on_the_real_corridor_exercises_both_criteria() {
    // 冒烟只证明判据在真实世界上取得样本：清空合同的通过与否由完整矩阵判定
    // （走廊 §8.2：失败时不放宽断言）。红灯跨越不依赖配时，任何规模都必须为零。
    let report = run(100, 0, CYCLE_TICKS);
    assert!(!report.samples.is_empty(), "一个周期内应出现黄灯期跨越");
    // 两个控制器各 4 次交接。
    assert!(report.handoffs >= 8, "handoffs={}", report.handoffs);
    assert!(
        !report
            .violations
            .iter()
            .any(|v| matches!(v, Violation::RedEntry { .. })),
        "{:#?}",
        report.violations
    );
}

#[test]
#[ignore = "完整规范矩阵；正式验收以 release 显式运行"]
fn canonical_matrix_clears_every_yellow_entry_and_handoff() {
    let mut traversals: HashMap<u32, u64> = HashMap::new();
    let mut failures = Vec::new();
    let mut totals = (Tally::default(), Tally::default());
    println!(
        "target seed ticks yellow_samples unresolved handoffs max_tc_minus_ty_ms budget_ms \
         late_clearance handoff_occupied red_entry lost"
    );
    for target in TARGETS {
        for seed in SEEDS {
            let report = run(target, seed, MATRIX_CYCLES * CYCLE_TICKS);
            let count = |f: fn(&Violation<VehicleHandle, RouteHandle>) -> bool| {
                report.violations.iter().filter(|v| f(v)).count()
            };
            println!(
                "{} {} {} {} {} {} {} {}..{} {} {} {} {}",
                report.target,
                report.seed,
                report.ticks,
                report.samples.len(),
                report.unresolved,
                report.handoffs,
                report.max_elapsed_ms,
                report.min_budget_ms,
                report.max_budget_ms,
                count(|v| matches!(v, Violation::LateClearance(_))),
                count(|v| matches!(v, Violation::HandoffOccupied { .. })),
                count(|v| matches!(v, Violation::RedEntry { .. })),
                count(|v| matches!(v, Violation::LostBeforeClear(_))),
            );
            for (path, count) in &report.traversals {
                *traversals.entry(*path).or_default() += count;
            }
            write_diagnostics(&report);
            let late = tally(&report, YellowSample::is_late);
            let unresolved = tally(&report, YellowSample::is_unresolved);
            for (status, counts) in [("late", &late), ("unresolved", &unresolved)] {
                println!(
                    "factors {} {} {status} {counts}",
                    report.target, report.seed
                );
            }
            totals.0.add(&late);
            totals.1.add(&unresolved);
            if report.samples.len() > report.unresolved {
                assert_eq!(
                    (report.min_budget_ms, report.max_budget_ms),
                    (
                        AUTHORED_YELLOW_PLUS_ALL_RED_MS,
                        AUTHORED_YELLOW_PLUS_ALL_RED_MS
                    ),
                    "观测到的 Y + R 与走廊 §7.2 不符"
                );
            }
            if !report.violations.is_empty() {
                failures.push((target, seed, report.violations));
            }
        }
    }
    let mut per_path: Vec<_> = traversals.into_iter().collect();
    per_path.sort_unstable();
    println!("path traversals: {per_path:?}");
    println!("factors total late {}", totals.0);
    println!("factors total unresolved {}", totals.1);
    assert!(failures.is_empty(), "{failures:#?}");
}

// ---------- 诊断输出 ----------

type Sample = YellowSample<VehicleHandle, RouteHandle>;

/// 一组样本的候选原因计数；一个样本可以同时计入多项。
#[derive(Default)]
struct Tally {
    samples: usize,
    timing: usize,
    slow_entry: usize,
    limit_reduction: usize,
    leader_in_range: usize,
    stop_line_ahead: usize,
    unattributed_decel: usize,
    stoppable_without_braking: usize,
    comfort_feasible: usize,
    undetermined: usize,
    cannot_stop: usize,
    can_stop: usize,
    onset_unknown: usize,
}

impl Tally {
    fn add(&mut self, other: &Self) {
        self.samples += other.samples;
        self.timing += other.timing;
        self.slow_entry += other.slow_entry;
        self.limit_reduction += other.limit_reduction;
        self.leader_in_range += other.leader_in_range;
        self.stop_line_ahead += other.stop_line_ahead;
        self.unattributed_decel += other.unattributed_decel;
        self.stoppable_without_braking += other.stoppable_without_braking;
        self.comfort_feasible += other.comfort_feasible;
        self.undetermined += other.undetermined;
        self.cannot_stop += other.cannot_stop;
        self.can_stop += other.can_stop;
        self.onset_unknown += other.onset_unknown;
    }
}

impl std::fmt::Display for Tally {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "samples={} timing_insufficient={} slow_entry={} limit_reduction={} \
             leader_in_range={} stop_line_ahead={} unattributed_decel={} \
             stoppable_without_braking={}(comfort_feasible_at_yellow={}) undetermined={} \
             at_yellow_cannot_stop/can_stop/unknown={}/{}/{}",
            self.samples,
            self.timing,
            self.slow_entry,
            self.limit_reduction,
            self.leader_in_range,
            self.stop_line_ahead,
            self.unattributed_decel,
            self.stoppable_without_braking,
            self.comfort_feasible,
            self.undetermined,
            self.cannot_stop,
            self.can_stop,
            self.onset_unknown,
        )
    }
}

fn tally(report: &Report, select: fn(&Sample) -> bool) -> Tally {
    let mut tally = Tally::default();
    for sample in report.samples.iter().filter(|s| select(s)) {
        let analysis = analyze(sample, &report.params);
        tally.samples += 1;
        match analysis.cannot_stop_at_yellow {
            Some(true) => tally.cannot_stop += 1,
            Some(false) => tally.can_stop += 1,
            None => tally.onset_unknown += 1,
        }
        if analysis.factors.is_empty() {
            tally.undetermined += 1;
        }
        for factor in &analysis.factors {
            match factor {
                Factor::TimingInsufficient { .. } => tally.timing += 1,
                Factor::SlowEntry { .. } => tally.slow_entry += 1,
                Factor::LimitReduction { .. } => tally.limit_reduction += 1,
                Factor::LeaderInRange { .. } => tally.leader_in_range += 1,
                Factor::StopLineAhead { .. } => tally.stop_line_ahead += 1,
                Factor::UnattributedDecel { .. } => tally.unattributed_decel += 1,
                Factor::StoppableWithoutBraking {
                    comfort_feasible_at_yellow,
                    ..
                } => {
                    tally.stoppable_without_braking += 1;
                    if *comfort_feasible_at_yellow {
                        tally.comfort_feasible += 1;
                    }
                }
            }
        }
    }
    tally
}

fn known<T: std::fmt::Display>(value: Option<T>) -> String {
    value.map_or_else(|| "unknown".to_owned(), |value| value.to_string())
}

fn kinematics(k: Kinematics) -> String {
    format!(
        "{}/{}/{}/{}/{}",
        k.time_ms, k.edge_index, k.progress_mm, k.front_mm, k.speed_mm_s
    )
}

/// 每个黄灯期跨越样本（含未闭合样本）的诊断事实写到 `CARGO_TARGET_TMPDIR`，不入仓。
fn write_diagnostics(report: &Report) {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("clearance-diagnostics");
    std::fs::create_dir_all(&dir).expect("diagnostics dir");
    let mut out = String::new();
    writeln!(
        out,
        "# 运动状态 = time_ms/edge_index/progress_mm/front_mm/speed_mm_s；\
         轨迹列 = time_ms phase edge_index front_mm rear_mm speed_mm_s limit_mm_s \
         leader_gap_mm leader_speed_mm_s next_stop_mm next_stop_aspect"
    )
    .expect("write");
    for sample in &report.samples {
        let analysis = analyze(sample, &report.params);
        let status = if sample.is_unresolved() {
            "unresolved"
        } else if sample.is_late() {
            "late"
        } else {
            "cleared"
        };
        let factors = if analysis.factors.is_empty() {
            "undetermined".to_owned()
        } else {
            format!("{:?}", analysis.factors)
        };
        writeln!(
            out,
            "sample\tvehicle={:?}\troute={:?}\tpath={}\tgroup={}\tstatus={status}\tt_y={}\t\
             t_e={}\tt_c={}\tbudget_ms={}\tat_yellow={}\tdistance_to_line_mm={}\t\
             stop_distance_mm={}\tcannot_stop_at_yellow={}\tbefore={}\tafter={}\t\
             remaining_mm={}\tremaining_budget_ms={}\tfactors={factors}",
            sample.key,
            sample.route,
            sample.path,
            sample.group,
            sample.t_y,
            sample.t_e,
            known(sample.t_c),
            known(sample.budget_ms),
            known(sample.at_yellow.map(kinematics)),
            known(analysis.distance_to_line_mm),
            known(analysis.stop_distance_mm),
            known(analysis.cannot_stop_at_yellow),
            kinematics(sample.before),
            kinematics(sample.after),
            analysis.remaining_mm,
            known(analysis.remaining_budget_ms),
        )
        .expect("write");
        for row in &sample.trace {
            writeln!(
                out,
                "{}\t{:?}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
                row.time_ms,
                row.phase,
                row.edge_index,
                row.front_mm,
                row.rear_mm,
                row.speed_mm_s,
                row.limit_mm_s,
                known(row.leader.map(|l| l.gap_mm)),
                known(row.leader.map(|l| l.speed_mm_s)),
                known(row.next_stop.map(|s| s.distance_mm)),
                row.next_stop
                    .map_or_else(|| "none".to_owned(), |s| format!("{:?}", s.aspect)),
            )
            .expect("write");
        }
    }
    std::fs::write(
        dir.join(format!("{}-{}.tsv", report.target, report.seed)),
        out,
    )
    .expect("write diagnostics");
}
