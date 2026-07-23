//! Bevy 0.19 Plugin 与 LaneFlow 专用 schedule。

use bevy_app::{App, First, MainScheduleOrder, Plugin, PostUpdate};
use bevy_ecs::{
    schedule::{IntoScheduleConfigs, Schedule, ScheduleLabel, SingleThreadedExecutor, SystemSet},
    system::{Res, ResMut},
    world::World,
};
use bevy_time::Time;
use bevy_transform::TransformSystems;

use crate::{LaneFlowSession, presentation::sync_lane_flow_transforms};

/// 每个 Bevy outer frame 运行一次的 LaneFlow 驱动 schedule。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, ScheduleLabel)]
pub struct LaneFlowOuterFrame;

/// 根据 Session accumulator 运行零次或多次的 LaneFlow fixed schedule。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, ScheduleLabel)]
pub struct LaneFlowFixed;

/// 每个 LaneFlow fixed step 内稳定执行的公共阶段。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, SystemSet)]
pub enum LaneFlowFixedSet {
    /// 调用方提交 vehicle lifecycle command 的 fixed-step boundary。
    Lifecycle,
    /// Adapter 推进一次 LaneFlow Core fixed tick。
    Step,
    /// 调用方读取本次 fixed step committed 结果。
    Observe,
}

/// 安装 LaneFlow 专用 outer-frame/fixed schedule 的 Bevy Plugin。
#[derive(Clone, Copy, Debug, Default)]
pub struct LaneFlowPlugin;

impl Plugin for LaneFlowPlugin {
    fn build(&self, app: &mut App) {
        let mut outer_frame = Schedule::new(LaneFlowOuterFrame);
        outer_frame.set_executor(SingleThreadedExecutor::new());
        outer_frame.add_systems(run_outer_frame);

        let mut fixed = Schedule::new(LaneFlowFixed);
        fixed.set_executor(SingleThreadedExecutor::new());
        fixed.configure_sets(
            (
                LaneFlowFixedSet::Lifecycle,
                LaneFlowFixedSet::Step.run_if(session_has_no_error),
                LaneFlowFixedSet::Observe.run_if(session_has_no_error),
            )
                .chain(),
        );
        fixed.add_systems(step_core.in_set(LaneFlowFixedSet::Step));

        app.add_schedule(outer_frame).add_schedule(fixed);
        app.add_systems(
            PostUpdate,
            sync_lane_flow_transforms.before(TransformSystems::Propagate),
        );
        app.world_mut()
            .resource_mut::<MainScheduleOrder>()
            .insert_after(First, LaneFlowOuterFrame);
    }
}

fn run_outer_frame(world: &mut World) {
    if !world.contains_resource::<LaneFlowSession>() {
        return;
    }

    let Some(frame_delta) = world.get_resource::<Time>().map(Time::delta) else {
        world
            .resource_mut::<LaneFlowSession>()
            .record_missing_time();
        return;
    };

    let (frame_ready, max_catch_up_steps) = {
        let mut session = world.resource_mut::<LaneFlowSession>();
        let frame_ready = session.begin_outer_frame(frame_delta);
        let max_catch_up_steps = session.config().max_catch_up_steps().get();
        (frame_ready, max_catch_up_steps)
    };

    if frame_ready {
        world.schedule_scope(LaneFlowFixed, |world, schedule| {
            for _ in 0..max_catch_up_steps {
                if !world.resource::<LaneFlowSession>().can_step() {
                    break;
                }
                schedule.run(world);
            }
        });
    }

    world.resource_mut::<LaneFlowSession>().finish_outer_frame();
}

fn step_core(mut session: ResMut<'_, LaneFlowSession>) {
    session.step_core();
}

fn session_has_no_error(session: Res<'_, LaneFlowSession>) -> bool {
    session.last_error().is_none()
}
