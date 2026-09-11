//! `LaneFlowSession::junction_observation` 借用视图的行为测试（#285 复杂路口观测 G1 §3）。

#[path = "runtime_min_scene.rs"]
mod runtime_min_scene;

use std::{sync::Arc, time::Duration};

use bevy_app::App;
use bevy_ecs::{
    resource::Resource,
    schedule::IntoScheduleConfigs,
    system::{Res, ResMut},
};
use bevy_time::{TimePlugin, TimeUpdateStrategy};
use bevy_transform::TransformPlugin;
use laneflow_bevy::{
    LaneFlowFixed, LaneFlowFixedSet, LaneFlowJunctionObservationContext, LaneFlowPlugin,
    LaneFlowSession,
};
use laneflow_runtime::{EntityKind, WaitingZoneOrdinal, WorldGeneration};

#[test]
fn context_matches_world_getters_at_install() {
    let session = runtime_min_scene::session().expect("native example initialization");
    let world = session.world();
    let context = session.junction_observation().context();
    assert_eq!(context.world_id(), world.world_id());
    assert_eq!(context.world_generation(), world.world_generation());
    assert_eq!(context.world_generation(), WorldGeneration::INITIAL);
    assert_eq!(
        context.network_revision(),
        world.committed_source().network_revision()
    );
    assert_eq!(
        context.network_revision(),
        world.revision().network_revision()
    );
    assert_eq!(context.tick_index(), 0);
    assert_eq!(context.tick_index(), world.tick_index());
    assert_eq!(context.command_cursor(), world.command_cursor());
    assert_eq!(context.event_cursor(), 0);
    assert_eq!(context.event_cursor(), world.event_cursor());
}

#[test]
fn vehicles_follow_live_order_with_committed_state() {
    let session = runtime_min_scene::session().expect("native example initialization");
    let world = session.world();
    let rows: Vec<_> = session.junction_observation().vehicles().collect();
    assert_eq!(rows.len(), world.live_vehicles().len());
    for (row, &vehicle) in rows.iter().zip(world.live_vehicles()) {
        assert_eq!(row.vehicle(), vehicle);
        assert_eq!(
            row.state(),
            world.vehicle(vehicle).expect("live vehicle state")
        );
        assert_eq!(row.conflict_reservation(), None);
        assert_eq!(
            row.conflict_reservation(),
            world.conflict_reservation(vehicle)
        );
    }
}

#[test]
fn waiting_zones_cover_fixture_zones_in_ordinal_order() {
    let session = runtime_min_scene::session().expect("native example initialization");
    let world = session.world();
    let zone_count = world
        .traffic()
        .entity_counts()
        .count(EntityKind::WaitingZone);
    assert_eq!(zone_count, 1, "夹具恰有一个 WaitingZone（waiting-main）");
    let expected: Vec<_> = (0..zone_count)
        .map(WaitingZoneOrdinal::from_raw)
        .map(|zone| world.waiting_zone(zone).expect("fixture zone snapshot"))
        .collect();
    let zones: Vec<_> = session.junction_observation().waiting_zones().collect();
    assert_eq!(zones, expected);
    for (index, snapshot) in zones.iter().enumerate() {
        assert_eq!(snapshot.zone().index(), index, "按 zone ordinal 升序全覆盖");
    }
}

#[test]
fn borrowed_batches_match_world_getters() {
    let session = runtime_min_scene::session().expect("native example initialization");
    let world = session.world();
    let view = session.junction_observation();
    assert_eq!(view.waiting_zone_members(), world.waiting_zone_members());
    assert_eq!(
        view.latest_waiting_decisions(),
        world.latest_waiting_decisions()
    );
    assert_eq!(
        view.latest_conflict_decisions(),
        world.latest_conflict_decisions()
    );
    assert_eq!(
        view.latest_transition_events(),
        world.latest_transition_events()
    );
}

#[test]
fn route_gate_matches_world_direct_lookup() {
    let session = runtime_min_scene::session().expect("native example initialization");
    let world = session.world();
    let view = session.junction_observation();
    let route = world.live_routes().next().expect("fixture live route");
    let hop_count = u32::try_from(world.route_edges(route).expect("route edges").len())
        .expect("hop count fits u32");
    assert_eq!(hop_count, 3, "夹具路线为 entry/middle/exit 三边");
    let mut located = 0_u32;
    for hop in 0..hop_count {
        let direct = world.route_gate(route, hop);
        assert_eq!(view.route_gate(route, hop), direct);
        if direct.is_some() {
            located += 1;
        }
    }
    assert!(located >= 1, "夹具路线至少携带一道 Gate");
    assert_eq!(view.route_gate(route, hop_count), None);
}

#[test]
fn headless_session_exposes_domain_observation() {
    // 无 Spatial 配对的 headless Session；领域观察不依赖 Spatial 配对。
    let world = runtime_min_scene::world().expect("fixture world");
    let session = LaneFlowSession::new(world, None, runtime_min_scene::session_config())
        .expect("headless session");
    let world = session.world();
    let view = session.junction_observation();
    assert!(Arc::ptr_eq(&view.revision(), &world.revision()));
    assert_eq!(view.context().tick_index(), world.tick_index());
    assert_eq!(view.vehicles().count(), world.live_vehicles().len());
    assert_eq!(
        view.latest_transition_events(),
        world.latest_transition_events()
    );
}

/// 每拍 Observe 阶段收集的视图上下文。
#[derive(Resource, Default)]
struct ObservedContexts(Vec<LaneFlowJunctionObservationContext>);

fn collect_observation(session: Res<LaneFlowSession>, mut observed: ResMut<ObservedContexts>) {
    let view = session.junction_observation();
    let world = session.world();
    let context = view.context();
    assert_eq!(context.tick_index(), world.tick_index());
    assert_eq!(context.command_cursor(), world.command_cursor());
    assert_eq!(context.event_cursor(), world.event_cursor());
    assert_eq!(view.waiting_zone_members(), world.waiting_zone_members());
    assert_eq!(
        view.latest_waiting_decisions(),
        world.latest_waiting_decisions()
    );
    assert_eq!(
        view.latest_conflict_decisions(),
        world.latest_conflict_decisions()
    );
    assert_eq!(
        view.latest_transition_events(),
        world.latest_transition_events()
    );
    let rows: Vec<_> = view.vehicles().collect();
    assert_eq!(rows.len(), world.live_vehicles().len());
    for (row, &vehicle) in rows.iter().zip(world.live_vehicles()) {
        assert_eq!(row.vehicle(), vehicle);
        assert_eq!(
            row.state(),
            world.vehicle(vehicle).expect("live vehicle state")
        );
        assert_eq!(
            row.conflict_reservation(),
            world.conflict_reservation(vehicle)
        );
    }
    observed.0.push(context);
}

#[test]
fn observe_set_collects_every_fixed_tick() {
    let session = runtime_min_scene::session().expect("native example initialization");
    let mut app = App::new();
    app.add_plugins((TimePlugin, TransformPlugin, LaneFlowPlugin));
    app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(
        100,
    )));
    app.insert_resource(session);
    app.init_resource::<ObservedContexts>();
    app.add_systems(
        LaneFlowFixed,
        collect_observation.in_set(LaneFlowFixedSet::Observe),
    );
    // ManualDuration 下首帧只初始化时钟（delta 为零），之后每帧恰好推进一拍。
    let mut stepped = 0_u32;
    for _ in 0..4 {
        app.update();
        stepped += app
            .world()
            .resource::<LaneFlowSession>()
            .frame_report()
            .steps_run();
    }
    let app_world = app.world();
    let world = app_world.resource::<LaneFlowSession>().world();
    let observed = &app_world.resource::<ObservedContexts>().0;
    assert!(stepped >= 3, "四帧至少推进三拍");
    assert_eq!(
        observed.len(),
        usize::try_from(stepped).expect("step count fits usize"),
        "每个 successful fixed tick 恰被 Observe 收集一次"
    );
    let ticks: Vec<u64> = observed
        .iter()
        .map(|context| context.tick_index())
        .collect();
    let expected: Vec<u64> = (1..=u64::from(stepped)).collect();
    assert_eq!(ticks, expected, "Observe 阶段自 tick 1 起逐拍连续收集");
    for context in observed {
        assert_eq!(context.world_generation(), WorldGeneration::INITIAL);
        assert_eq!(
            context.network_revision(),
            world.committed_source().network_revision()
        );
        assert_eq!(context.command_cursor(), world.command_cursor());
    }
}
