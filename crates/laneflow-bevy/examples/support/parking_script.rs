//! 固定车辆脚本：只读已提交状态与 Tick，运行在 `LaneFlowFixedSet::Lifecycle`。
use bevy_ecs::{
    resource::Resource,
    world::{Mut, World},
};
use laneflow_bevy::{LaneFlowSession, despawn_vehicle};
use laneflow_runtime::{
    LeaveParkingTarget, ParkingCommandOutcome, ParkingError, ParkingTarget, ReserveParkingTarget,
    RouteHandle, VehicleHandle, VirtualEntryAnchorSelector, VirtualExitAnchorSelector,
};

use super::{
    parking_host as host,
    parking_scene::{Cast, OCCURRENCES_PER_LAP, Plan, Role},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Phase {
    /// 没有停车计划，或路线上已没有下一个入口。
    Drive,
    /// 下一拍发起预留。
    Reserve,
    /// 已预留，驶向入口。
    Approach,
    /// 阻挡车：已到达但保持预留，等虚拟车首次离场被拒。
    Hold,
    /// 已停入，停留结束后离场或移除。
    Parked,
    Removed,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Actor {
    pub role: Role,
    pub vehicle: VehicleHandle,
    pub phase: Phase,
    plan: Plan,
    entry_occurrence: u32,
    dwell_left: u32,
    early_park_pending: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Rejection {
    pub tick: u64,
    pub role: Role,
    pub command: &'static str,
    pub error: ParkingError,
}

#[derive(Resource)]
pub(super) struct Script {
    pub actors: Vec<Actor>,
    pub last_rejection: Option<Rejection>,
    route: RouteHandle,
    route_occurrences: u32,
    dwell_ticks: u32,
    blocker_released: bool,
}

impl Script {
    pub fn new(cast: &[Cast], route: RouteHandle, route_occurrences: u32, dwell: u32) -> Self {
        let actors = cast
            .iter()
            .map(|cast| {
                let entry_occurrence = match cast.plan {
                    Plan::None => 0,
                    Plan::Space {
                        entry_occurrence, ..
                    }
                    | Plan::Pool {
                        entry_occurrence, ..
                    } => entry_occurrence,
                };
                Actor {
                    role: cast.role,
                    vehicle: cast.vehicle,
                    phase: if cast.plan == Plan::None {
                        Phase::Drive
                    } else {
                        Phase::Reserve
                    },
                    plan: cast.plan,
                    entry_occurrence,
                    dwell_left: 0,
                    early_park_pending: cast.role == Role::Virtual,
                }
            })
            .collect();
        Self {
            actors,
            last_rejection: None,
            route,
            route_occurrences,
            dwell_ticks: dwell,
            blocker_released: false,
        }
    }

    fn blocker_holding(&self) -> bool {
        self.actors
            .iter()
            .any(|actor| actor.role == Role::Blocker && actor.phase == Phase::Hold)
    }
}

fn parking_target(plan: Plan) -> Result<ParkingTarget, String> {
    match plan {
        Plan::None => Err("该车辆没有停车计划".into()),
        Plan::Space { space, .. } => Ok(ParkingTarget::ExplicitSpace(space)),
        Plan::Pool { facility, .. } => Ok(ParkingTarget::VirtualPool(facility)),
    }
}

fn reserve_input(plan: Plan, entry_occurrence: u32) -> Result<ReserveParkingTarget, String> {
    match plan {
        Plan::None => Err("该车辆没有停车计划".into()),
        Plan::Space { space, .. } => Ok(ReserveParkingTarget::ExplicitSpace {
            space,
            entry_route_occurrence: entry_occurrence,
        }),
        Plan::Pool { facility, .. } => Ok(ReserveParkingTarget::VirtualPool {
            facility,
            entry_anchor: VirtualEntryAnchorSelector::from_raw(0),
            entry_route_occurrence: entry_occurrence,
        }),
    }
}

/// 显式泊位在同一出现记录离场；虚拟池的出口在入口的下一条边。
fn exit_occurrence(plan: Plan, entry_occurrence: u32) -> u32 {
    match plan {
        Plan::Pool { .. } => entry_occurrence + 1,
        Plan::None | Plan::Space { .. } => entry_occurrence,
    }
}

fn leave_input(
    plan: Plan,
    route: RouteHandle,
    occurrence: u32,
) -> Result<LeaveParkingTarget, String> {
    match plan {
        Plan::None => Err("该车辆没有停车计划".into()),
        Plan::Space { space, .. } => Ok(LeaveParkingTarget::ExplicitSpace {
            space,
            route,
            exit_route_occurrence: occurrence,
        }),
        Plan::Pool { facility, .. } => Ok(LeaveParkingTarget::VirtualPool {
            facility,
            route,
            exit_anchor: VirtualExitAnchorSelector::from_raw(0),
            exit_route_occurrence: occurrence,
        }),
    }
}

/// 离场位置之后、相位相同的第一个入口出现记录。
fn next_entry(entry_occurrence: u32, exit: u32) -> u32 {
    let phase = entry_occurrence % OCCURRENCES_PER_LAP;
    let mut next = exit + 1;
    while next % OCCURRENCES_PER_LAP != phase {
        next += 1;
    }
    next
}

pub(super) fn lifecycle(world: &mut World) {
    let result = world.resource_scope(|world, mut script: Mut<Script>| {
        for slot in 0..script.actors.len() {
            advance(world, &mut script, slot)?;
        }
        Ok::<(), String>(())
    });
    if let Err(error) = result {
        host::fail(world, error);
    }
}

fn advance(world: &mut World, script: &mut Script, slot: usize) -> Result<(), String> {
    let actor = script.actors[slot];
    match actor.phase {
        Phase::Drive | Phase::Removed => Ok(()),
        Phase::Reserve => reserve(world, script, slot),
        Phase::Approach => approach(world, script, slot),
        Phase::Hold if script.blocker_released => park(world, script, slot),
        Phase::Hold => Ok(()),
        Phase::Parked if actor.dwell_left > 0 => {
            script.actors[slot].dwell_left -= 1;
            Ok(())
        }
        Phase::Parked if actor.role == Role::Removal => remove(world, script, slot),
        Phase::Parked => leave(world, script, slot),
    }
}

fn reserve(world: &mut World, script: &mut Script, slot: usize) -> Result<(), String> {
    let actor = script.actors[slot];
    let input = reserve_input(actor.plan, actor.entry_occurrence)?;
    let mut session = world.resource_mut::<LaneFlowSession>();
    match session.world_mut().reserve_parking(actor.vehicle, input) {
        Ok(ParkingCommandOutcome::Committed(_)) => {
            script.actors[slot].phase = Phase::Approach;
            Ok(())
        }
        other => Err(format!("{:?} 预留结果不符：{other:?}", actor.role)),
    }
}

fn approach(world: &mut World, script: &mut Script, slot: usize) -> Result<(), String> {
    let actor = script.actors[slot];
    let target = parking_target(actor.plan)?;
    if actor.early_park_pending {
        script.actors[slot].early_park_pending = false;
        return early_park(world, script, slot, target);
    }
    let session = world.resource::<LaneFlowSession>();
    if !session.world().parking_arrived(actor.vehicle, target) {
        return Ok(());
    }
    if actor.role == Role::Blocker && !script.blocker_released {
        script.actors[slot].phase = Phase::Hold;
        return Ok(());
    }
    park(world, script, slot)
}

/// 到达前发起一次停入：只接受 `NotArrived`，绑定、来源和表现都不变。
fn early_park(
    world: &mut World,
    script: &mut Script,
    slot: usize,
    target: ParkingTarget,
) -> Result<(), String> {
    let actor = script.actors[slot];
    let mut session = world.resource_mut::<LaneFlowSession>();
    let tick = session.world().tick_index();
    match session.world_mut().park_vehicle(actor.vehicle, target) {
        Err(error @ ParkingError::NotArrived) => {
            script.last_rejection = Some(Rejection {
                tick,
                role: actor.role,
                command: "park",
                error,
            });
            Ok(())
        }
        other => Err(format!("过早停入结果不符：{other:?}")),
    }
}

fn park(world: &mut World, script: &mut Script, slot: usize) -> Result<(), String> {
    let actor = script.actors[slot];
    let target = parking_target(actor.plan)?;
    let mut session = world.resource_mut::<LaneFlowSession>();
    match session.world_mut().park_vehicle(actor.vehicle, target) {
        Ok(ParkingCommandOutcome::Committed(_)) => {
            let dwell = script.dwell_ticks;
            let actor = &mut script.actors[slot];
            actor.phase = Phase::Parked;
            actor.dwell_left = dwell;
            Ok(())
        }
        other => Err(format!("{:?} 停入结果不符：{other:?}", actor.role)),
    }
}

fn leave(world: &mut World, script: &mut Script, slot: usize) -> Result<(), String> {
    let actor = script.actors[slot];
    // 虚拟车首次离场必须等阻挡车就位，否则这条失败路径不会发生。
    if actor.role == Role::Virtual && !script.blocker_released && !script.blocker_holding() {
        return Ok(());
    }
    let exit = exit_occurrence(actor.plan, actor.entry_occurrence);
    let input = leave_input(actor.plan, script.route, exit)?;
    let mut session = world.resource_mut::<LaneFlowSession>();
    let tick = session.world().tick_index();
    match session.world_mut().leave_parking(actor.vehicle, input) {
        Ok(_) => {
            let next = next_entry(actor.entry_occurrence, exit);
            let route_occurrences = script.route_occurrences;
            let actor = &mut script.actors[slot];
            if next < route_occurrences {
                actor.entry_occurrence = next;
                actor.phase = Phase::Reserve;
            } else {
                actor.phase = Phase::Drive;
            }
            Ok(())
        }
        Err(
            error @ (ParkingError::LeavePhysicalOverlap { .. }
            | ParkingError::LeaveUnsafeFollower { .. }),
        ) => {
            script.last_rejection = Some(Rejection {
                tick,
                role: actor.role,
                command: "leave",
                error,
            });
            if actor.role == Role::Virtual {
                script.blocker_released = true;
            }
            Ok(())
        }
        Err(error) => Err(format!("{:?} 离场结果不符：{error:?}", actor.role)),
    }
}

/// 只有 typed 移除成功才销毁 Entity；缺席多少帧都不走到这里。
fn remove(world: &mut World, script: &mut Script, slot: usize) -> Result<(), String> {
    let vehicle = script.actors[slot].vehicle;
    let record = despawn_vehicle(world, vehicle).map_err(|error| format!("{error:?}"))?;
    let entity = record.entity.ok_or("移除车没有绑定实体")?;
    world.despawn(entity);
    script.actors[slot].phase = Phase::Removed;
    Ok(())
}
