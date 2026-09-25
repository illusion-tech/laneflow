//! #285 有限跨层验收：正式复杂路口制品、资源生命周期、换根和逐拍重放。
//! 领域求解边界复用 Runtime 专项；这里检查 Session 消费链不丢失或误归属已提交事实。

#[allow(dead_code)]
#[path = "junction_debug_scene.rs"]
mod scene;

use std::{sync::Arc, time::Duration};

use bevy_app::App;
use bevy_ecs::{
    resource::Resource,
    schedule::IntoScheduleConfigs,
    system::{Res, ResMut},
};
use bevy_time::{TimePlugin, TimeUpdateStrategy};
use bevy_transform::components::Transform;
use laneflow_bevy::{
    LaneFlowCommittedPoseBatch, LaneFlowFixed, LaneFlowFixedSet,
    LaneFlowJunctionObservationContext, LaneFlowJunctionVehicleRow, LaneFlowPlugin,
    LaneFlowSession, LaneFlowTargetSpatial, despawn_vehicle,
};
use laneflow_runtime::{
    CapturedSnapshot, ConflictDecision, ConflictPassageOccurrenceLocator, ConflictPassageRange,
    CutoverPreflightLimits, RouteHandle, RouteRegisterInput, SnapshotRestoreLimits, TickInput,
    TrafficTransitionEvent, TrafficTransitionKind, TrafficWorld, VehicleHandle, VehicleSpawnInput,
    WaitingDecision, WaitingZoneMember, WaitingZoneSnapshot, deterministic_state_digest,
    encode_lfrs, restore_lfrs,
};
use laneflow_spatial::{FramePlacementToken, SpatialSession};

#[derive(Clone, Debug, Eq, PartialEq)]
struct Observation {
    context: LaneFlowJunctionObservationContext,
    vehicles: Vec<LaneFlowJunctionVehicleRow>,
    zones: Vec<WaitingZoneSnapshot>,
    members: Vec<WaitingZoneMember>,
    waiting: Vec<WaitingDecision>,
    conflict: Vec<ConflictDecision>,
    transitions: Vec<TrafficTransitionEvent>,
}

fn observation(session: &LaneFlowSession) -> Observation {
    let view = session.junction_observation();
    Observation {
        context: view.context(),
        vehicles: view.vehicles().collect(),
        zones: view.waiting_zones().collect(),
        members: view.waiting_zone_members().to_vec(),
        waiting: view.latest_waiting_decisions().to_vec(),
        conflict: view.latest_conflict_decisions().to_vec(),
        transitions: view.latest_transition_events().to_vec(),
    }
}

fn app() -> App {
    let mut app = App::new();
    app.add_plugins((TimePlugin, LaneFlowPlugin))
        .insert_resource(scene::build().unwrap().session)
        .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::ZERO));
    app.update();
    app
}

fn until(app: &mut App, condition: impl Fn(&LaneFlowSession) -> bool) {
    app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(
        16,
    )));
    for _ in 0..8_000 {
        if condition(app.world().resource::<LaneFlowSession>()) {
            return;
        }
        app.update();
        assert!(
            app.world()
                .resource::<LaneFlowSession>()
                .last_error()
                .is_none()
        );
    }
    panic!("reference scenario did not reach the required resource state");
}

fn digest(world: &laneflow_runtime::TrafficWorld) -> String {
    format!(
        "{:x}",
        deterministic_state_digest(&world.capture_snapshot().unwrap()).unwrap()
    )
}

#[test]
fn despawn_and_slot_reuse_do_not_turn_historical_grants_into_current_ownership() {
    for waiting in [true, false] {
        let mut app = app();
        until(&mut app, |session| {
            session.junction_observation().vehicles().any(|row| {
                if waiting {
                    row.state().waiting_membership().is_some()
                } else {
                    row.conflict_reservation().is_some()
                }
            })
        });
        let before = observation(app.world().resource::<LaneFlowSession>());
        let selected = before
            .vehicles
            .iter()
            .find(|row| {
                if waiting {
                    row.state().waiting_membership().is_some()
                } else {
                    row.conflict_reservation().is_some()
                }
            })
            .unwrap();
        let old = selected.vehicle();
        let route = selected.state().route();
        let edges = app
            .world()
            .resource::<LaneFlowSession>()
            .world()
            .route_edges(route)
            .unwrap()
            .to_vec();
        let gate_hop = (0..edges.len() as u32)
            .find(|&hop| {
                app.world()
                    .resource::<LaneFlowSession>()
                    .junction_observation()
                    .route_gate(route, hop)
                    .is_some()
            })
            .expect("selected route crosses a gate");
        let entity = app.world_mut().spawn(Transform::IDENTITY).id();
        app.world_mut()
            .resource_mut::<LaneFlowSession>()
            .bind_vehicle_entity(old, entity)
            .unwrap();
        let removed = despawn_vehicle(app.world_mut(), old).unwrap();
        assert_eq!(removed.entity, Some(entity));
        assert_eq!(removed.runtime.waiting_release.is_some(), waiting);
        assert_eq!(removed.runtime.conflict_release.is_some(), !waiting);
        let mut session = app.world_mut().resource_mut::<LaneFlowSession>();
        let after = observation(&session);
        assert_eq!(after.context.tick_index(), before.context.tick_index());
        assert!(after.context.command_cursor() > before.context.command_cursor());
        assert_eq!(after.waiting, before.waiting);
        assert_eq!(after.conflict, before.conflict);
        assert_eq!(after.transitions, before.transitions);
        assert!(after.vehicles.iter().all(|row| row.vehicle() != old));
        assert!(after.members.iter().all(|member| member.vehicle() != old));
        assert!(session.vehicle_entity(old).is_none());
        session.world_mut().remove_route(route).unwrap();
        let replacement = session
            .world_mut()
            .register_route(RouteRegisterInput::new(edges))
            .unwrap();
        assert_ne!(replacement, route);
        assert!(
            session
                .junction_observation()
                .route_gate(route, gate_hop)
                .is_none()
        );
        assert!(
            session
                .junction_observation()
                .route_gate(replacement, gate_hop)
                .is_some()
        );
        let new_vehicle = session
            .world_mut()
            .spawn_vehicle(
                VehicleSpawnInput::new(selected.state().profile(), replacement, 0, 6_500, 0)
                    .with_open_entrance(),
            )
            .unwrap();
        assert_ne!(new_vehicle, old);
        let current = session
            .junction_observation()
            .vehicles()
            .find(|row| row.vehicle() == new_vehicle)
            .unwrap();
        assert!(current.conflict_reservation().is_none());
        assert!(current.state().waiting_membership().is_none());
        let final_observation = observation(&session);
        assert_eq!(final_observation.waiting, before.waiting);
        assert_eq!(final_observation.conflict, before.conflict);
        assert_eq!(final_observation.transitions, before.transitions);
    }
}

/// 只在本参考场景使用：每条边序列唯一，且每条路线恰有一辆车。
/// 用逻辑内容关联保存侧身份，不从句柄值或槽位顺序推导快照 ID。
struct SnapshotIdentities {
    routes: Vec<(u64, RouteHandle)>,
    vehicles: Vec<(u64, VehicleHandle)>,
}

impl SnapshotIdentities {
    fn captured(world: &TrafficWorld, snapshot: &CapturedSnapshot) -> Self {
        let root = world.revision();
        let routes: Vec<_> = snapshot
            .routes()
            .iter()
            .map(|saved| {
                let matches: Vec<_> = world
                    .live_routes()
                    .filter(|&handle| {
                        let stable: Vec<_> = world
                            .route_edges(handle)
                            .unwrap()
                            .iter()
                            .map(|&edge| *root.identity().stable_id(edge).unwrap().as_untyped())
                            .collect();
                        stable == saved.edges()
                    })
                    .collect();
                assert_eq!(matches.len(), 1, "fixture route must be unique");
                (saved.snapshot_route_id(), matches[0])
            })
            .collect();
        let vehicles = snapshot
            .vehicles()
            .iter()
            .map(|saved| {
                let route = routes
                    .iter()
                    .find(|(id, _)| *id == saved.snapshot_route_id())
                    .unwrap()
                    .1;
                let matches: Vec<_> = world
                    .live_vehicles()
                    .iter()
                    .copied()
                    .filter(|&handle| world.vehicle(handle).unwrap().route() == route)
                    .collect();
                assert_eq!(matches.len(), 1, "fixture has one vehicle per route");
                (saved.snapshot_vehicle_id(), matches[0])
            })
            .collect();
        Self { routes, vehicles }
    }

    fn route(&self, handle: RouteHandle) -> u64 {
        self.routes
            .iter()
            .find(|(_, value)| *value == handle)
            .unwrap()
            .0
    }

    fn vehicle(&self, handle: VehicleHandle) -> u64 {
        self.vehicles
            .iter()
            .find(|(_, value)| *value == handle)
            .unwrap()
            .0
    }

    fn passage(&self, value: ConflictPassageOccurrenceLocator) -> String {
        format!(
            "{:?}",
            (
                self.route(value.route()),
                value.maneuver_occurrence_index(),
                value.admission_gate_hop(),
                value.conflict_occurrence_index(),
                value.address(),
                value.stable_locator()
            )
        )
    }

    fn range(&self, value: ConflictPassageRange) -> (u64, u32, u32, u32, u32) {
        (
            self.route(value.route()),
            value.maneuver_occurrence_index(),
            value.admission_gate_hop(),
            value.first_conflict_occurrence_index(),
            value.passage_count(),
        )
    }

    fn kind(&self, value: TrafficTransitionKind) -> String {
        match value {
            TrafficTransitionKind::ReservationAcquired { passage_range } => {
                format!("ReservationAcquired {:?}", self.range(passage_range))
            }
            TrafficTransitionKind::ReservationReleased { passage_range } => {
                format!("ReservationReleased {:?}", self.range(passage_range))
            }
            TrafficTransitionKind::ConflictEntered { passage } => {
                format!("ConflictEntered {}", self.passage(passage))
            }
            TrafficTransitionKind::ConflictCleared { passage } => {
                format!("ConflictCleared {}", self.passage(passage))
            }
            TrafficTransitionKind::ProjectionApplied { .. }
            | TrafficTransitionKind::GateCrossed { .. }
            | TrafficTransitionKind::WaitingLeft { .. }
            | TrafficTransitionKind::WaitingEntered { .. }
            | TrafficTransitionKind::ManeuverTraversalCompleted { .. } => format!("{value:?}"),
        }
    }

    /// 格式化的全部字段均已消除进程句柄；嵌套 passage/range 也必须经过映射。
    fn outputs(&self, world: &TrafficWorld) -> [Vec<String>; 3] {
        [
            world
                .latest_waiting_decisions()
                .iter()
                .map(|value| {
                    let anchor = value.anchor();
                    format!(
                        "{:?}",
                        (
                            self.vehicle(value.vehicle()),
                            value.vehicle_update_sequence(),
                            value.zone(),
                            self.route(anchor.route()),
                            anchor.maneuver_occurrence_index(),
                            anchor.hop(),
                            value.outcome()
                        )
                    )
                })
                .collect(),
            world
                .latest_conflict_decisions()
                .iter()
                .map(|value| {
                    let anchor = value.anchor();
                    format!(
                        "{:?}",
                        (
                            self.vehicle(value.vehicle()),
                            value.vehicle_update_sequence(),
                            self.route(anchor.route()),
                            anchor.maneuver_occurrence_index(),
                            anchor.hop(),
                            value.passage().map(|passage| self.passage(passage)),
                            value.outcome()
                        )
                    )
                })
                .collect(),
            world
                .latest_transition_events()
                .iter()
                .map(|value| {
                    let anchor = value.anchor();
                    format!(
                        "{:?}",
                        (
                            value.tick(),
                            self.vehicle(value.vehicle()),
                            value.vehicle_update_sequence(),
                            self.route(anchor.route()),
                            anchor.maneuver_occurrence_index(),
                            anchor.hop(),
                            anchor.position(),
                            self.kind(value.kind())
                        )
                    )
                })
                .collect(),
        ]
    }
}

#[test]
fn held_resources_survive_snapshot_restore_and_replay_exactly() {
    for waiting in [true, false] {
        let mut app = app();
        // 先制造真实代际变化，确保本测试不能碰巧依赖恢复前后的原始句柄相等。
        {
            let session = app.world().resource::<LaneFlowSession>();
            let old = session.world().live_vehicles()[0];
            let state = session.world().vehicle(old).unwrap();
            despawn_vehicle(app.world_mut(), old).unwrap();
            let mut session = app.world_mut().resource_mut::<LaneFlowSession>();
            let new = session
                .world_mut()
                .spawn_vehicle(
                    VehicleSpawnInput::new(
                        state.profile(),
                        state.route(),
                        state.route_edge_index(),
                        state.progress_mm(),
                        state.speed_mm_s(),
                    )
                    .with_open_entrance(),
                )
                .unwrap();
            assert_ne!(new, old);
        }
        until(&mut app, |session| {
            session.junction_observation().vehicles().any(|row| {
                if waiting {
                    row.state().waiting_membership().is_some()
                } else {
                    row.conflict_reservation().is_some()
                }
            })
        });
        let session = app.world().resource::<LaneFlowSession>();
        let world = session.world();
        let snapshot = world.capture_snapshot().unwrap();
        let identities = SnapshotIdentities::captured(world, &snapshot);
        let bytes = encode_lfrs(&snapshot);
        let root = world.revision();
        let source = world.committed_source().clone();
        let config = world.config();
        let initial_digest = digest(world);
        let mut expected = Vec::new();
        for _ in 0..2_000 {
            app.update();
            let session = app.world().resource::<LaneFlowSession>();
            assert!(session.last_error().is_none());
            expected.push(identities.outputs(session.world()));
        }
        let final_digest = digest(app.world().resource::<LaneFlowSession>().world());
        // fresh restore 接管相同 world_id 前，必须销毁原 world/session。
        drop(app);
        let restored = restore_lfrs(
            &bytes,
            root,
            source,
            config,
            laneflow_runtime::ExecutionConfig::new(std::num::NonZeroU32::MIN),
            SnapshotRestoreLimits::new(16 * 1_024 * 1_024, 4 * 1_024),
        )
        .unwrap();
        let restored_identities = SnapshotIdentities {
            routes: restored.route_mappings().to_vec(),
            vehicles: restored.vehicle_mappings().to_vec(),
        };
        assert_ne!(identities.vehicles, restored_identities.vehicles);
        assert_eq!(digest(restored.world()), initial_digest);
        let mut first = restored.into_world();
        for outputs in expected {
            first.step(TickInput::new(16)).unwrap();
            assert_eq!(restored_identities.outputs(&first), outputs);
        }
        assert_eq!(digest(&first), final_digest);
        let before = digest(&first);
        let transitions = first.latest_transition_events().to_vec();
        let waiting_decisions = first.latest_waiting_decisions().to_vec();
        let conflict_decisions = first.latest_conflict_decisions().to_vec();
        assert!(first.step(TickInput::new(32)).is_err());
        assert_eq!(digest(&first), before);
        assert_eq!(first.latest_transition_events(), transitions);
        assert_eq!(first.latest_waiting_decisions(), waiting_decisions);
        assert_eq!(first.latest_conflict_decisions(), conflict_decisions);
    }
}

#[test]
fn same_revision_root_replacement_preserves_resources_and_rejects_mismatched_spatial() {
    for waiting in [true, false] {
        let mut app = app();
        until(&mut app, |session| {
            session.junction_observation().vehicles().any(|row| {
                if waiting {
                    row.state().waiting_membership().is_some()
                } else {
                    row.conflict_reservation().is_some()
                }
            })
        });
        let mut session = app.world_mut().resource_mut::<LaneFlowSession>();
        let before = observation(&session);
        let before_digest = digest(session.world());
        let mut poses = LaneFlowCommittedPoseBatch::new();
        session
            .extract_committed_pose_batch(FramePlacementToken::new(1), &mut poses)
            .unwrap();
        let target = scene::build().unwrap().session.world().revision();
        let other = scene::build().unwrap().session.world().revision();
        assert!(!Arc::ptr_eq(&target, &other));
        let source = session.world().committed_source().clone();
        let limits = CutoverPreflightLimits::new(4 * 1_024 * 1_024);
        assert!(
            session
                .same_revision_restore(
                    Arc::clone(&target),
                    source.clone(),
                    LaneFlowTargetSpatial::Rebind(SpatialSession::bind(other).unwrap().unwrap()),
                    &limits,
                )
                .is_err()
        );
        assert_eq!(observation(&session), before);
        assert_eq!(digest(session.world()), before_digest);
        assert!(session.consumption_context_is_current(poses.context()));
        let record = session
            .same_revision_restore(
                Arc::clone(&target),
                source,
                LaneFlowTargetSpatial::Rebind(
                    SpatialSession::bind(Arc::clone(&target)).unwrap().unwrap(),
                ),
                &limits,
            )
            .unwrap();
        assert!(!record.events().is_empty());
        let after = observation(&session);
        assert_eq!(after.context.tick_index(), before.context.tick_index());
        assert_eq!(
            after.context.world_generation().get(),
            before.context.world_generation().get() + 1
        );
        assert_eq!(after.vehicles, before.vehicles);
        assert_eq!(after.zones, before.zones);
        assert_eq!(after.members, before.members);
        assert_eq!(after.waiting, before.waiting);
        assert_eq!(after.conflict, before.conflict);
        assert_eq!(after.transitions, before.transitions);
        assert!(Arc::ptr_eq(
            &session.junction_observation().revision(),
            &target
        ));
        assert!(!session.consumption_context_is_current(poses.context()));
        let before_positions: Vec<_> = poses
            .batch()
            .records()
            .iter()
            .map(|row| row.pose())
            .collect();
        session
            .extract_committed_pose_batch(FramePlacementToken::new(1), &mut poses)
            .unwrap();
        let after_positions: Vec<_> = poses
            .batch()
            .records()
            .iter()
            .map(|row| row.pose())
            .collect();
        assert_eq!(before_positions, after_positions);
    }
}

#[derive(Resource, Default)]
struct Trace(Vec<Observation>);

fn observe_tick(session: Res<LaneFlowSession>, mut trace: ResMut<Trace>) {
    trace.0.push(observation(&session));
}

fn replay(chunks: &[u64]) -> (String, Vec<Observation>) {
    let mut app = app();
    app.init_resource::<Trace>().add_systems(
        LaneFlowFixed,
        observe_tick.in_set(LaneFlowFixedSet::Observe),
    );
    let mut frame = 0;
    loop {
        assert!(frame < 22_400, "replay exceeded the bounded frame budget");
        let tick = app
            .world()
            .resource::<LaneFlowSession>()
            .world()
            .tick_index();
        if tick == 5_600 {
            break;
        }
        let ticks = chunks[frame % chunks.len()].min(5_600 - tick);
        app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(
            ticks * 16,
        )));
        app.update();
        assert!(
            app.world()
                .resource::<LaneFlowSession>()
                .last_error()
                .is_none()
        );
        frame += 1;
    }
    let trace = std::mem::take(&mut app.world_mut().resource_mut::<Trace>().0);
    assert_eq!(trace.len(), 5_600);
    for (index, row) in trace.iter().enumerate() {
        assert_eq!(row.context.tick_index(), index as u64 + 1);
    }
    (
        digest(app.world().resource::<LaneFlowSession>().world()),
        trace,
    )
}

#[test]
fn zero_single_and_catch_up_frames_preserve_every_tick_of_domain_evidence() {
    let (plain_digest, plain) = replay(&[1]);
    let (catch_up_digest, catch_up) = replay(&[0, 1, 2, 8]);
    assert!(plain.iter().any(|row| !row.waiting.is_empty()));
    assert!(plain.iter().any(|row| !row.conflict.is_empty()));
    assert!(plain.iter().any(|row| !row.transitions.is_empty()));
    assert_eq!(plain_digest, catch_up_digest);
    assert_eq!(plain, catch_up);
}
