//! #1500 只读运动诊断：记录选定车辆每拍的停止目标查询、门决策与运动求值。
//!
//! 仅在 `motion-trace` feature 下编译，供诊断入口使用，不属于稳定 API。记录器
//! 只读取计算过程中已有的值；未启动或车辆不在目标集合中时不做任何事。打点处
//! 不改变任何计算结果，开启与关闭记录时的已提交状态一致。
//!
//! 运动求值中的两档可行域由与 Runtime 相同的公开数值原语重算
//! （`raw_proposal`、`safe_speed`、`max_next_speed_for_decel`），列式路径不在
//! 运动内核的标量或向量通道内打点。

use std::cell::RefCell;

use crate::VehicleHandle;

/// 到停止位置的距离；`Beyond` 表示超出可表示的有限范围。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TraceDistance {
    /// 有限距离（毫米）。
    Finite(u32),
    /// 超出有限范围。
    Beyond,
}

impl From<laneflow_static_network::BoundedDistance> for TraceDistance {
    fn from(value: laneflow_static_network::BoundedDistance) -> Self {
        match value {
            laneflow_static_network::BoundedDistance::Finite(mm) => Self::Finite(mm),
            laneflow_static_network::BoundedDistance::BeyondFinite => Self::Beyond,
        }
    }
}

/// 产生运动求值的路径。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MotionPath {
    /// 逐车标量求值（P2 完整预览）。
    Scalar,
    /// P5 列式重新求值。
    Columnar,
    /// P5 直接复用本拍 P2 的完整预览，不再求值。
    PreviewReused,
}

/// 求值时的拍初运动状态。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TraceStart {
    /// 路线边下标。
    pub route_edge_index: u32,
    /// 边内进度（毫米）。
    pub progress_mm: u32,
    /// 速度（毫米每秒）。
    pub speed_mm_s: u32,
    /// 行程微米余数。
    pub carry_um: u16,
}

/// 一次运动求值中与停止归约有关的数值（米、米每秒）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TraceMotion {
    /// 拍初运动状态。
    pub start: TraceStart,
    /// 期望速度（毫米每秒）。
    pub desired_mm_s: u32,
    /// 当前边限速（毫米每秒）。
    pub current_limit_mm_s: u32,
    /// 舒适减速度（米每二次方秒）。
    pub comfort_decel: f32,
    /// 紧急减速度（米每二次方秒）。
    pub emergency_decel: f32,
    /// 前车车距（毫米）；无前车为 `None`。
    pub leader_gap_mm: Option<i64>,
    /// 前车已提交速度（毫米每秒）。
    pub leader_speed_mm_s: u32,
    /// 停止目标距离（信号、无信号拒绝门、停车与路终取近者）；无目标为 `+∞`。
    pub stop_m: f32,
    /// 本拍硬空间（毫米）。
    pub hard_room_mm: u32,
    /// IIDM 原始提案速度。
    pub proposal_speed: f32,
    /// 紧急减速度下界 `max(v − b_e·Δt, 0)`。
    pub floor: f32,
    /// 跟车安全上界；无前车为 `None`。
    pub safe_cap: Option<f32>,
    /// 停止目标前的候选速度 `max(min(提案, 安全上界), 下界)`。
    pub candidate: f32,
    /// 诊断是否重算了停止目标的两档可行域（目标有限、为正且候选为正）。这是重算
    /// 的条件，不是实际分支日志：列式内核另以 `stop_solve_participates` 跳过制动
    /// 窗外的求解（窗外求解不改变归约结果），因此本字段为真不代表内核执行了求根。
    pub stop_recomputed: bool,
    /// 舒适档的最大可行终速；`None` 表示无解。
    pub comfort: Option<f32>,
    /// 紧急档的最大可行终速；`None` 表示无解。
    pub emergency: Option<f32>,
    /// 停止归约后的速度。
    pub reduced: f32,
}

/// 一条诊断记录；`tick` 为调用方以 [`set_tick`] 标记的拍号。
#[derive(Clone, Debug, PartialEq)]
pub enum TraceEvent {
    /// 停止目标查询的汇总。
    Stops {
        /// 拍号。
        tick: u64,
        /// 车辆。
        vehicle: VehicleHandle,
        /// 制动前视窗（毫米，未取整）。
        window_mm: Option<f64>,
        /// 下一受控门已在窗外，未逐门解释。
        signal_skipped: bool,
        /// 信号停止距离；无限制门为 `None`。
        signal: Option<TraceDistance>,
        /// 无信号拒绝门的停止距离。
        gate: Option<TraceDistance>,
        /// 最终采用的停止目标（不含路终）。
        movement: Option<TraceDistance>,
    },
    /// 信号停止查询逐门解释的一扇门。
    Gate {
        /// 拍号。
        tick: u64,
        /// 车辆。
        vehicle: VehicleHandle,
        /// 门所在的路线 hop。
        hop: u32,
        /// 拍初到该门停止线的距离。
        distance: TraceDistance,
        /// 基础门决策是否为拒绝并停车。
        restrictive: bool,
        /// 是否满足黄灯困境区豁免。
        dilemma_exempt: bool,
    },
    /// 信号停止查询遇到窗外的门，停止解释。
    GateBeyondWindow {
        /// 拍号。
        tick: u64,
        /// 车辆。
        vehicle: VehicleHandle,
        /// 门所在的路线 hop。
        hop: u32,
    },
    /// 前方限速下降约束之后的速度（米每秒，量化前）：标量路径每次求值都记录；
    /// 列式路径只在该行进入限速下降子批时记录。
    SpeedDrop {
        /// 拍号。
        tick: u64,
        /// 车辆。
        vehicle: VehicleHandle,
        /// 求值路径。
        path: MotionPath,
        /// 限速下降约束之后的速度。
        speed_m_s: f32,
    },
    /// 一次运动求值；`PreviewReused` 只标记路径，数值见同拍的 `Scalar` 记录。
    Motion {
        /// 拍号。
        tick: u64,
        /// 车辆。
        vehicle: VehicleHandle,
        /// 求值路径。
        path: MotionPath,
        /// 停止归约数值；`PreviewReused` 为 `None`。
        motion: Option<TraceMotion>,
    },
}

struct Recorder {
    tick: u64,
    targets: Vec<VehicleHandle>,
    events: Vec<TraceEvent>,
}

thread_local! {
    static RECORDER: RefCell<Option<Recorder>> = const { RefCell::new(None) };
}

/// 在当前线程开始记录 `targets` 中的车辆，丢弃之前的记录。
///
/// 记录器是线程局部的：只记录在调用线程上执行的求值，世界须使用单 worker
/// 的调用方执行资源。不同线程上的世界互不影响。
pub fn start(targets: &[VehicleHandle]) {
    RECORDER.with_borrow_mut(|recorder| {
        *recorder = Some(Recorder {
            tick: 0,
            targets: targets.to_vec(),
            events: Vec::new(),
        });
    });
}

/// 标记之后记录所属的拍号。
pub fn set_tick(tick: u64) {
    RECORDER.with_borrow_mut(|recorder| {
        if let Some(recorder) = recorder.as_mut() {
            recorder.tick = tick;
        }
    });
}

/// 停止记录并取回当前线程的全部记录。
pub fn finish() -> Vec<TraceEvent> {
    RECORDER
        .with_borrow_mut(Option::take)
        .map(|recorder| recorder.events)
        .unwrap_or_default()
}

/// 车辆在当前线程的目标集合中时追加一条记录；`event` 只在需要时求值。
pub(crate) fn record(vehicle: VehicleHandle, event: impl FnOnce(u64) -> TraceEvent) {
    RECORDER.with_borrow_mut(|recorder| {
        if let Some(recorder) = recorder
            .as_mut()
            .filter(|recorder| recorder.targets.contains(&vehicle))
        {
            let event = event(recorder.tick);
            recorder.events.push(event);
        }
    });
}
