//! Bevy 0.19 Plugin 与 LaneFlow 专用 schedule。

use bevy_app::{App, First, MainScheduleOrder, Plugin};
use bevy_ecs::{
    prelude::resource_exists,
    schedule::{IntoScheduleConfigs, Schedule, ScheduleLabel, SingleThreadedExecutor, SystemSet},
    system::{Res, ResMut},
    world::World,
};
use bevy_time::Time;

use crate::LaneFlowSession;

/// 每个 Bevy outer frame 运行一次的 LaneFlow 驱动 schedule。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, ScheduleLabel)]
pub struct LaneFlowOuterFrame;

/// `LaneFlowOuterFrame` 内的专属阶段集合。
///
/// `Administration` 每 outer frame 必运行，与 accumulator/fixed step 无关：
/// 维护暂停式切换等行政操作放在这里在结构上零步进可达（#534 G1 冻结；
/// `LaneFlowFixed` 整个 schedule 受 `can_step()` 门控，零步进帧连
/// `Lifecycle` 都不运行，行政操作不得依赖 fixed step 的存在）。
///
/// 不变量：**所有公开 set 在无 `LaneFlowSession` 态安全**——两个阶段都以
/// `resource_exists` 条件门控，宿主系统在 Session 未插入或暂时移除的帧被
/// 跳过而非 panic；核心循环的守卫只保护自身，公开 set 的安全性必须由
/// schedule 配置保证。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, SystemSet)]
pub enum LaneFlowOuterFrameSet {
    /// 行政边界：宿主驱动维护暂停式切换、显式 Spatial 重绑等行政操作；
    /// 位于本帧步进与位姿采集之前。无 Session 时整体跳过。
    Administration,
    /// 本 crate 的 outer-frame 驱动（accumulator、fixed 循环、帧报告），
    /// 也向宿主开放挂载。无 Session 时整体跳过。
    Drive,
}

/// 根据 Session accumulator 运行零次或多次的 LaneFlow fixed schedule。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, ScheduleLabel)]
pub struct LaneFlowFixed;

/// 每个 LaneFlow fixed step 内稳定执行的公共阶段。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, SystemSet)]
pub enum LaneFlowFixedSet {
    /// 调用方通过 `LaneFlowSession::world_mut` 提交生命周期命令的 fixed-step 边界。
    Lifecycle,
    /// Adapter 推进一次 TrafficWorld fixed tick。
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
        outer_frame.configure_sets(
            (
                LaneFlowOuterFrameSet::Administration,
                LaneFlowOuterFrameSet::Drive,
            )
                .run_if(resource_exists::<LaneFlowSession>)
                .chain(),
        );
        outer_frame.add_systems(run_outer_frame.in_set(LaneFlowOuterFrameSet::Drive));

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
        fixed.add_systems(step_world.in_set(LaneFlowFixedSet::Step));

        app.add_schedule(outer_frame).add_schedule(fixed);
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

fn step_world(mut session: ResMut<'_, LaneFlowSession>) {
    session.step_world();
}

fn session_has_no_error(session: Res<'_, LaneFlowSession>) -> bool {
    session.last_error().is_none()
}
