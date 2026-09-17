//! #705 生产阈值以上的多 worker 整步等价对照（唯一在生产阈值下验证
//! 「多 worker 整步等价」的集成证据）。
//!
//! multi-gate 场景 1_280 活动车辆（高出生产分发阈值 1_024 的保守余量，
//! 补员保持稳态），worker 1（融合）与 2/4（真实分发，自然跨过阈值，无
//! cfg(test) 强制入口——集成测试链接的库不带 lib 私有旋钮）逐拍比较
//! digest、最新 Waiting/Conflict 决策与统一事件批次、tick/时间游标。
//! 每拍定步前断言 Active 不低于生产阈值，证明多 worker 臂确实走在分发路径。

mod multi_gate_scene;

use laneflow_runtime::{StepOutcome, TickInput, TrafficWorld, deterministic_state_digest};

const VEHICLES: usize = 1_280;
const PRODUCTION_DISPATCH_MIN_ACTIVE: usize = 1_024;
const WARMUP_TICKS: usize = 8;
const MEASURED_TICKS: usize = 48;

fn tick_record(world: &TrafficWorld, outcome: &StepOutcome) -> String {
    let snapshot = world.capture_snapshot().expect("capture");
    format!(
        "{:?}|{:?}|{:?}|{:?}|{:?}",
        deterministic_state_digest(&snapshot).expect("digest"),
        world.latest_waiting_decisions(),
        world.latest_conflict_decisions(),
        world.latest_transition_events(),
        (
            world.tick_index(),
            world.time_ms(),
            outcome.tick_index(),
            outcome.time_ms(),
            outcome.parking_arrivals()
        ),
    )
}

fn active_count(world: &TrafficWorld) -> usize {
    world
        .live_vehicles()
        .iter()
        .filter(|handle| {
            world
                .vehicle(**handle)
                .is_some_and(|state| state.status() == laneflow_runtime::VehicleStatus::Active)
        })
        .count()
}

fn run_arm(
    workers: u32,
    revision: &std::sync::Arc<laneflow_static_network::SharedNetworkRevision>,
) -> Vec<String> {
    let mut world = multi_gate_scene::install(revision, workers, 705_900);
    let routes = multi_gate_scene::routes_of(&world);
    let boundaries = multi_gate_scene::route_boundaries(&world, &routes);
    for _ in 0..WARMUP_TICKS {
        multi_gate_scene::replenish(&mut world, &routes, &boundaries);
        multi_gate_scene::step(&mut world);
    }
    let mut records = Vec::with_capacity(MEASURED_TICKS);
    for _ in 0..MEASURED_TICKS {
        multi_gate_scene::replenish(&mut world, &routes, &boundaries);
        let active = active_count(&world);
        assert!(
            active >= PRODUCTION_DISPATCH_MIN_ACTIVE,
            "workers={workers} arm must stay above the production dispatch threshold, got {active}"
        );
        let outcome = world
            .step(TickInput::new(multi_gate_scene::DELTA_MS))
            .unwrap();
        records.push(tick_record(&world, &outcome));
    }
    records
}

#[test]
fn over_production_threshold_workers_match_fused_reference() {
    let revision = multi_gate_scene::build_revision(VEHICLES);
    let reference = run_arm(1, &revision);
    for workers in [2_u32, 4] {
        assert_eq!(
            run_arm(workers, &revision),
            reference,
            "workers={workers} diverged from the fused reference above the production threshold"
        );
    }
}
