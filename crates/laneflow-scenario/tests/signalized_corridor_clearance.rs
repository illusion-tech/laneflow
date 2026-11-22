//! 受保护转向走廊黄灯放行后的清空核验（#1496，#1446 工作包 2）。
//!
//! 判据见 `support/clearance.rs` 与 signalized-corridor-protected-turning §7.3、§8。
//! 常规测试运行判据负例与一个短程真实世界冒烟；完整规范矩阵为 `#[ignore]`，
//! 正式验收用 release 显式运行：
//!
//! ```text
//! cargo +1.99.1 test --release --locked -p laneflow-scenario \
//!     --test signalized_corridor_clearance -- --ignored --nocapture
//! ```

#[path = "support/clearance.rs"]
mod clearance;

use std::collections::HashMap;
use std::sync::Arc;

use clearance::{ClearanceCheck, Frame, PathOccurrence, RouteLayout, VehicleSample, Violation};
use laneflow_format::{FormatLimits, check_canonical_network_input};
use laneflow_runtime::{
    RouteHandle, TickInput, TrafficWorld, VehicleHandle, VehicleStatus, WorldConfig,
};
use laneflow_scenario::signalized_corridor::{
    CorridorCatalog, CorridorPopulationConfig, CorridorPopulationController,
    CorridorPopulationPrepare, CorridorReplaceAttemptOutcome, PASSENGER_CAR_PROFILE_KEY, bind,
};
use laneflow_static_contract::{ManeuverPathOrdinal, SignalAspect, SignalGroupOrdinal};
use laneflow_static_network::{
    SharedNetworkBuildLimits, SharedNetworkBuildOptions, SharedNetworkRevision, SpatialBuildOption,
    build_shared_network_revision,
};

const CORRIDOR_LFCA: &[u8] = include_bytes!("../../../examples/data/v0.2-signalized-corridor.lfca");
const CORRIDOR_CATALOG: &str =
    include_str!("../../../examples/data/v0.2-signalized-corridor.catalog.toml");
const TICK_MS: u64 = 16;
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
    VehicleSample {
        key: 1,
        route: 0,
        edge_index,
        progress_mm,
        length_mm: 4,
    }
}

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

// ---------- 真实走廊世界 ----------

fn install(revision: Arc<SharedNetworkRevision>, target: usize) -> TrafficWorld {
    let selection = laneflow_runtime::WorldPolicySelection::Pinned(laneflow_runtime::PolicyPin {
        policy: laneflow_static_contract::RightOfWayPolicySetId::from_untyped(
            laneflow_compiler::derive_canonical_stable_id_v1(
                laneflow_static_contract::EntityKind::RightOfWayPolicySet,
                "laneflow/signalized-corridor",
                "protected-entry",
            )
            .expect("policy identity"),
        ),
    });
    let origin = *revision.canonical_origin();
    TrafficWorld::install(
        Arc::clone(&revision),
        WorldConfig::new(
            u32::try_from(target).expect("fits"),
            28,
            1_024,
            1_024,
            TICK_MS,
        ),
        laneflow_runtime::ExecutionConfig::new(std::num::NonZeroU32::MIN),
        laneflow_runtime::CommittedNetworkSource::Published {
            reference: laneflow_runtime::PublishedLfcaReference::new(
                "fixture://in-process",
                origin.canonical_artifact_digest(),
                origin.canonical_artifact_byte_length(),
                origin.network_revision(),
            )
            .expect("reference"),
        },
        0,
        selection,
    )
    .expect("install")
}

fn corridor(target: usize, seed: u64) -> (TrafficWorld, CorridorPopulationController) {
    let input = check_canonical_network_input(CORRIDOR_LFCA, FormatLimits::HARD).expect("LFCA");
    let revision = build_shared_network_revision(
        input,
        SharedNetworkBuildOptions::new(
            SpatialBuildOption::RetainAvailable,
            SharedNetworkBuildLimits::new(64 * 1_024 * 1_024, 16 * 1_024 * 1_024),
        ),
    )
    .expect("revision");
    let catalog: CorridorCatalog = toml::from_str(CORRIDOR_CATALOG).expect("catalog");
    let bound = bind(&catalog, revision.as_ref()).expect("bind");
    let profile = *bound
        .profiles
        .get(PASSENGER_CAR_PROFILE_KEY)
        .expect("passenger-car");
    let config = CorridorPopulationConfig::try_new(target, seed).expect("config");
    let mut prepared =
        CorridorPopulationPrepare::prepare(config, bound, revision.as_ref(), profile)
            .expect("prepare");
    let mut world = install(Arc::clone(&revision), target);
    let (vehicles, routes) = prepared
        .spawn_initial_vehicles(&mut world)
        .expect("initial population");
    let controller = prepared.bind(&mut world, &vehicles, &routes).expect("bind");
    (world, controller)
}

/// 内部边 → (路径序号, 组序号)，以及组 → 控制器。
struct Topology {
    path_of_triple: HashMap<[u32; 3], (u32, usize)>,
    group_controllers: Vec<usize>,
    lane_lengths: Vec<u32>,
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
    Topology {
        path_of_triple,
        group_controllers,
        lane_lengths: traffic.lane_lengths_millimetres().to_vec(),
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
    for &handle in world.live_vehicles() {
        let state = world.vehicle(handle).expect("live");
        if state.status() != VehicleStatus::Active {
            continue;
        }
        if !check.has_layout(state.route()) {
            check.add_layout(state.route(), layout(world, topology, state.route()));
        }
        out.push(VehicleSample {
            key: handle,
            route: state.route(),
            edge_index: state.route_edge_index(),
            progress_mm: state.progress_mm(),
            length_mm: state.length_mm(),
        });
    }
}

struct Report {
    target: usize,
    seed: u64,
    ticks: u32,
    samples: usize,
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
        samples: check.samples.len(),
        unresolved: check.unresolved(),
        handoffs: check.handoffs,
        max_elapsed_ms,
        min_budget_ms,
        max_budget_ms,
        traversals: check.traversals,
        violations: check.violations,
    }
}

#[test]
fn one_cycle_on_the_real_corridor_exercises_both_criteria() {
    // 冒烟只证明判据在真实世界上取得样本：清空合同的通过与否由完整矩阵判定
    // （走廊 §8.2：失败时不放宽断言）。红灯跨越不依赖配时，任何规模都必须为零。
    let report = run(100, 0, CYCLE_TICKS);
    assert!(report.samples > 0, "一个周期内应出现黄灯期跨越");
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
                report.samples,
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
            if report.samples > report.unresolved {
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
    assert!(failures.is_empty(), "{failures:#?}");
}
