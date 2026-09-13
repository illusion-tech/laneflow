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
    ConflictDecision, CutoverPreflightLimits, RouteRegisterInput, SnapshotRestoreLimits, TickInput,
    TrafficTransitionEvent, VehicleSpawnInput, WaitingDecision, WaitingZoneMember,
    WaitingZoneSnapshot, deterministic_state_digest, encode_lfrs, restore_lfrs,
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
            .spawn_vehicle(VehicleSpawnInput::new(
                selected.state().profile(),
                replacement,
                0,
                6_500,
                0,
            ))
            .unwrap();
        assert_ne!(new_vehicle, old);
        let current = session
            .junction_observation()
            .vehicles()
            .find(|row| row.vehicle() == new_vehicle)
            .unwrap();
        assert!(current.conflict_reservation().is_none());
        assert!(current.state().waiting_membership().is_none());
        assert_eq!(observation(&session).conflict, before.conflict);
    }
}

#[test]
fn held_resources_survive_snapshot_restore_and_replay_exactly() {
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
        let session = app.world().resource::<LaneFlowSession>();
        let world = session.world();
        let bytes = encode_lfrs(&world.capture_snapshot().unwrap());
        let restore = || {
            restore_lfrs(
                &bytes,
                world.revision(),
                world.committed_source().clone(),
                world.config(),
                SnapshotRestoreLimits::new(16 * 1_024 * 1_024, 4 * 1_024),
            )
            .unwrap()
            .into_world()
        };
        let mut first = restore();
        assert_eq!(digest(&first), digest(world));
        for row in session.junction_observation().vehicles() {
            assert_eq!(first.vehicle(row.vehicle()), Some(row.state()));
            assert_eq!(
                first.conflict_reservation(row.vehicle()),
                row.conflict_reservation()
            );
        }
        for _ in 0..2_000 {
            first.step(TickInput::new(16)).unwrap();
            app.update();
            let second = app.world().resource::<LaneFlowSession>().world();
            assert_eq!(
                first.latest_transition_events(),
                second.latest_transition_events()
            );
            assert_eq!(
                first.latest_waiting_decisions(),
                second.latest_waiting_decisions()
            );
            assert_eq!(
                first.latest_conflict_decisions(),
                second.latest_conflict_decisions()
            );
        }
        assert_eq!(
            digest(&first),
            digest(app.world().resource::<LaneFlowSession>().world())
        );
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
