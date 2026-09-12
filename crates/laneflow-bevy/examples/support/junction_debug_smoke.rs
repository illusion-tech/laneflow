//! `junction_debug` 示例的无窗口 smoke（#285 阶段二 §4 硬不变量）。
//!
//! headless App + ManualDuration；overlay 开/关两跑同输入，对拍
//! `deterministic_state_digest` 与每拍观测摘要（transition 事件 kind 序列 +
//! 决策批次计数）。测试 target 不能依赖 optional bevy facade，因此以最小镜像
//! 系统复现 overlay 的 Session 访问面：只读 `junction_observation` /
//! `committed_signal_groups` / 封闭 pose 提取 + 写独立 ECS 实体。

#[path = "junction_debug_scene.rs"]
mod junction_debug_scene;

use std::{collections::BTreeSet, time::Duration};

use bevy_app::App;
use bevy_ecs::{
    component::Component,
    entity::Entity,
    query::With,
    resource::Resource,
    schedule::IntoScheduleConfigs,
    system::{Commands, Local, Query, Res, ResMut},
};
use bevy_time::{TimePlugin, TimeUpdateStrategy};
use bevy_transform::{TransformPlugin, components::Transform};
use laneflow_bevy::{
    LaneFlowCommittedPoseBatch, LaneFlowFixed, LaneFlowFixedSet, LaneFlowPlugin, LaneFlowSession,
};
use laneflow_runtime::{VehicleHandle, WorldPolicySelection, deterministic_state_digest};
use laneflow_spatial::FramePlacementToken;

/// 跨帧复用的提取缓冲（adapter-api §6 稳定容量合同）。
#[derive(Resource, Default)]
struct PoseBuffer(LaneFlowCommittedPoseBatch);

#[derive(Resource)]
struct SpawnedPlan(Box<[junction_debug_scene::SpawnedVehicle]>);

#[derive(Resource)]
struct ProxyMap(std::collections::HashMap<VehicleHandle, Entity>);

/// overlay 开/关开关；两跑唯一差异输入。
#[derive(Resource)]
struct OverlaySwitch(bool);

#[derive(Component)]
struct OverlayMarker;

/// 镜像 overlay 面板的数据源（每拍决策聚合 + 信号状态）。
#[derive(Resource, Default)]
struct MirrorPanel {
    content: String,
    decision_ticks: u64,
}

/// 每拍观测摘要：两跑对拍的 transition/决策证据。
#[derive(Resource, Default)]
struct TickObservations(Vec<TickObservation>);

#[derive(Clone, Debug, Eq, PartialEq)]
struct TickObservation {
    tick: u64,
    waiting_decisions: usize,
    conflict_decisions: usize,
    waiting_outcomes: Vec<String>,
    conflict_outcomes: Vec<String>,
    transitions: Vec<String>,
}

fn setup_proxies(
    mut commands: Commands,
    plan: Res<SpawnedPlan>,
    mut session: ResMut<LaneFlowSession>,
) {
    let mut map = std::collections::HashMap::with_capacity(plan.0.len());
    for spawned in plan.0.iter() {
        let entity = commands.spawn(Transform::IDENTITY).id();
        session
            .bind_vehicle_entity(spawned.vehicle, entity)
            .expect("bind spawned vehicle");
        map.insert(spawned.vehicle, entity);
    }
    commands.insert_resource(ProxyMap(map));
}

fn sync_proxies(
    mut session: ResMut<LaneFlowSession>,
    mut poses: ResMut<PoseBuffer>,
    proxies: Res<ProxyMap>,
    mut transforms: Query<&mut Transform>,
) {
    session
        .extract_committed_pose_batch(FramePlacementToken::new(1), &mut poses.0)
        .expect("extract");
    for (vehicle, entity) in proxies.0.iter() {
        let Some(index) = poses.0.vehicles().iter().position(|v| v == vehicle) else {
            continue;
        };
        let Some(record) = poses.0.batch().records().get(index) else {
            continue;
        };
        if let Ok(mut transform) = transforms.get_mut(*entity) {
            let position = record.pose().position();
            *transform = Transform::from_xyz(position.x(), position.y(), position.z());
        }
    }
}

/// 两跑都运行的每拍观测收集（测试 oracle，只读 Session）。
fn collect_tick_observations(
    session: Res<LaneFlowSession>,
    mut observed: ResMut<TickObservations>,
) {
    let view = session.junction_observation();
    let waiting_outcomes: Vec<String> = view
        .latest_waiting_decisions()
        .iter()
        .map(|decision| format!("{:?}", decision.outcome()))
        .collect();
    let conflict_outcomes: Vec<String> = view
        .latest_conflict_decisions()
        .iter()
        .map(|decision| format!("{:?}", decision.outcome()))
        .collect();
    let transitions: Vec<String> = view
        .latest_transition_events()
        .iter()
        .map(|event| format!("{:?}", event.kind()))
        .collect();
    observed.0.push(TickObservation {
        tick: view.context().tick_index(),
        waiting_decisions: waiting_outcomes.len(),
        conflict_decisions: conflict_outcomes.len(),
        waiting_outcomes,
        conflict_outcomes,
        transitions,
    });
}

/// overlay 打开的镜像：每拍消费 junction_observation 写面板资源。
fn mirror_overlay_observe(
    switch: Res<OverlaySwitch>,
    session: Res<LaneFlowSession>,
    plan: Res<SpawnedPlan>,
    mut panel: ResMut<MirrorPanel>,
) {
    if !switch.0 {
        panel.content.clear();
        return;
    }
    let view = session.junction_observation();
    // 按车聚合：每拍对整个决策批次建一次索引（观测合同 §3）。
    let mut by_vehicle: std::collections::HashMap<VehicleHandle, usize> =
        std::collections::HashMap::new();
    for decision in view.latest_waiting_decisions() {
        *by_vehicle.entry(decision.vehicle()).or_default() += 1;
    }
    for decision in view.latest_conflict_decisions() {
        *by_vehicle.entry(decision.vehicle()).or_default() += 1;
    }
    if !by_vehicle.is_empty() {
        panel.decision_ticks += 1;
    }
    let selected = session.world().live_vehicles().first().copied();
    let role = selected.and_then(|vehicle| {
        plan.0
            .iter()
            .find(|spawned| spawned.vehicle == vehicle)
            .map(|spawned| spawned.role.tag())
    });
    panel.content = format!(
        "tick={} selected={selected:?} role={} signals={}",
        view.context().tick_index(),
        role.unwrap_or("none"),
        session.world().committed_signal_groups().as_slice().len()
    );
}

/// overlay 打开的镜像：静态标记实体按世代维护（写独立 ECS 实体）。
fn mirror_overlay_static(
    switch: Res<OverlaySwitch>,
    session: Res<LaneFlowSession>,
    mut commands: Commands,
    mut cache: Local<Option<u64>>,
    markers: Query<Entity, With<OverlayMarker>>,
) {
    let generation = session.world().world_generation().get();
    if !switch.0 {
        if cache.is_some() {
            for entity in &markers {
                commands.entity(entity).despawn();
            }
            *cache = None;
        }
        return;
    }
    if *cache == Some(generation) {
        return;
    }
    for entity in &markers {
        commands.entity(entity).despawn();
    }
    *cache = Some(generation);
    let zone_count = session.junction_observation().waiting_zones().count();
    for _ in 0..zone_count {
        commands.spawn((Transform::IDENTITY, OverlayMarker));
    }
}

struct RunOutcome {
    digest: String,
    observations: Vec<TickObservation>,
    decision_ticks: u64,
    panel_content: String,
    first_transform: [f32; 3],
    last_transform: [f32; 3],
    steps: u32,
    marker_count: usize,
}

/// 每帧 8 个 fixed step（8 × 16 ms）；约 3_300 tick 后车辆到达机动门，
/// 决策批次与 transition 事件积累完成。
const FRAME_DELTA_MS: u64 = 128;
const FRAMES: usize = 700;

fn proxy_translation(app: &App, proxy: Entity) -> [f32; 3] {
    let transform = app.world().get::<Transform>(proxy).expect("transform");
    [
        transform.translation.x,
        transform.translation.y,
        transform.translation.z,
    ]
}

fn run(overlay: bool) -> RunOutcome {
    let scene = junction_debug_scene::build().expect("native example initialization");
    let mut app = App::new();
    app.add_plugins((TimePlugin, TransformPlugin, LaneFlowPlugin));
    app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(
        FRAME_DELTA_MS,
    )));
    app.insert_resource(SpawnedPlan(scene.spawned));
    app.insert_resource(OverlaySwitch(overlay));
    app.insert_resource(scene.session);
    app.init_resource::<PoseBuffer>();
    app.init_resource::<TickObservations>();
    app.init_resource::<MirrorPanel>();
    app.add_systems(bevy_app::Startup, setup_proxies);
    app.add_systems(bevy_app::Update, (sync_proxies, mirror_overlay_static));
    app.add_systems(
        LaneFlowFixed,
        (
            collect_tick_observations.in_set(LaneFlowFixedSet::Observe),
            mirror_overlay_observe.in_set(LaneFlowFixedSet::Observe),
        ),
    );

    app.update();
    let proxy = {
        let map = app.world().resource::<ProxyMap>();
        *map.0.values().next().expect("one proxy")
    };
    let first_transform = proxy_translation(&app, proxy);
    for _ in 0..FRAMES {
        app.update();
    }
    let last_transform = proxy_translation(&app, proxy);
    let digest = {
        let session = app.world().resource::<LaneFlowSession>();
        let snapshot = session.world().capture_snapshot().expect("snapshot");
        format!(
            "{:x}",
            deterministic_state_digest(&snapshot).expect("digest")
        )
    };
    let steps = app
        .world()
        .resource::<LaneFlowSession>()
        .frame_report()
        .steps_run();
    let marker_count = {
        let mut query = app.world_mut().query::<&OverlayMarker>();
        query.iter(app.world()).count()
    };
    RunOutcome {
        digest,
        observations: app.world().resource::<TickObservations>().0.clone(),
        decision_ticks: app.world().resource::<MirrorPanel>().decision_ticks,
        panel_content: app.world().resource::<MirrorPanel>().content.clone(),
        first_transform,
        last_transform,
        steps,
        marker_count,
    }
}

#[test]
fn scene_spawns_fixed_plan_on_pinned_policy() {
    let scene = junction_debug_scene::build().expect("native example initialization");
    assert!(matches!(
        scene.session.world().policy_selection(),
        WorldPolicySelection::Pinned(_)
    ));
    assert_eq!(scene.session.world().live_vehicles().len(), 4);
    assert_eq!(scene.spawned.len(), 4);
    let mut slots = BTreeSet::new();
    for spawned in scene.spawned.iter() {
        assert!(slots.insert((spawned.route_id, spawned.slot_id)));
    }
}

#[test]
fn overlay_toggle_preserves_runtime_digest_and_event_summary() {
    let with_overlay = run(true);
    let without_overlay = run(false);
    assert!(
        with_overlay.steps > 0 && without_overlay.steps > 0,
        "LaneFlowFixed schedule must step TrafficWorld"
    );
    assert_eq!(
        with_overlay.observations.len(),
        without_overlay.observations.len(),
        "两跑步进拍数必须一致"
    );
    assert!(
        with_overlay.observations.len() > 1_000,
        "积累拍数不足：{}",
        with_overlay.observations.len()
    );
    // 硬不变量（§4）：相同命令输入下 overlay 开/关的 Runtime 状态与事件摘要一致。
    assert_eq!(
        with_overlay.digest, without_overlay.digest,
        "overlay 开/关两跑 deterministic_state_digest 必须一致"
    );
    assert_eq!(
        with_overlay.observations, without_overlay.observations,
        "overlay 开/关两跑每拍 transition/决策摘要必须一致"
    );
    // overlay 镜像真正运行并写入了独立实体与面板。
    assert!(with_overlay.marker_count > 0, "overlay 静态标记实体缺失");
    assert!(
        !with_overlay.panel_content.is_empty(),
        "overlay 面板数据源必须非空"
    );
    // 车辆 Transform 随推进移动。
    assert_ne!(
        with_overlay.first_transform, with_overlay.last_transform,
        "proxy Transform must change after runtime steps"
    );
    // 足够拍数后决策批次非空（面板数据源）。
    assert!(
        with_overlay.decision_ticks > 0,
        "决策批次在 {} 拍内必须至少一拍非空",
        with_overlay.observations.len()
    );
    let transitions: usize = with_overlay
        .observations
        .iter()
        .map(|observation| observation.transitions.len())
        .sum();
    assert!(transitions > 0, "transition 事件摘要必须至少一条");
}

#[test]
fn fixed_plan_vehicles_reach_gates_and_evaluate_decisions() {
    // 许可左转车是 spawn 计划第三辆：到达许可门后先 NoGrant、后在间隙中通过；
    // 直行车在主干道绿灯窗内过门；保护左转车进待转区等待放行。
    let outcome = run(true);
    let gate_crossed = outcome
        .observations
        .iter()
        .flat_map(|observation| observation.transitions.iter())
        .filter(|kind| kind.starts_with("GateCrossed"))
        .count();
    let conflict_decisions: usize = outcome
        .observations
        .iter()
        .map(|observation| observation.conflict_decisions)
        .sum();
    let waiting_decisions: usize = outcome
        .observations
        .iter()
        .map(|observation| observation.waiting_decisions)
        .sum();
    // 先 NoGrant 后通过：许可门评估先出现 NoGrant，之后出现 Granted。
    let no_grant_ticks: Vec<u64> = outcome
        .observations
        .iter()
        .filter(|observation| {
            observation
                .conflict_outcomes
                .iter()
                .any(|outcome| outcome.starts_with("NoGrant"))
        })
        .map(|observation| observation.tick)
        .collect();
    let granted_ticks: Vec<u64> = outcome
        .observations
        .iter()
        .filter(|observation| {
            observation
                .conflict_outcomes
                .iter()
                .any(|outcome| outcome == "Granted")
        })
        .map(|observation| observation.tick)
        .collect();
    assert!(
        gate_crossed > 0,
        "车辆必须在观察窗内跨过至少一道机动门（GateCrossed）"
    );
    assert!(
        conflict_decisions > 0,
        "许可左转车必须在其许可门处产生冲突决策评估"
    );
    assert!(
        waiting_decisions > 0,
        "保护左转车必须在待转区门处产生 Waiting 决策评估"
    );
    assert!(
        !no_grant_ticks.is_empty() && !granted_ticks.is_empty(),
        "许可左转必须同时经历 NoGrant 评估与 Granted（no_grant={no_grant_ticks:?} granted={granted_ticks:?}）"
    );
    assert!(
        no_grant_ticks
            .iter()
            .any(|no_grant| granted_ticks.iter().any(|granted| no_grant < granted)),
        "NoGrant 评估必须先于 Granted 出现（no_grant={no_grant_ticks:?} granted={granted_ticks:?}）"
    );
}
