use std::num::NonZeroU32;

/// 每世界交通配置；容量和固定步长进入快照与逻辑摘要，执行并行度另行指定。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WorldConfig {
    vehicle_capacity: u32,
    route_capacity: u32,
    route_edge_occurrence_capacity: u64,
    route_conflict_occurrence_capacity: u64,
    fixed_delta_time_ms: u64,
}

impl WorldConfig {
    /// 创建配置。合法性在 `install` 检查，构造器本身不失败。
    #[must_use]
    pub const fn new(
        vehicle_capacity: u32,
        route_capacity: u32,
        route_edge_occurrence_capacity: u64,
        route_conflict_occurrence_capacity: u64,
        fixed_delta_time_ms: u64,
    ) -> Self {
        Self {
            vehicle_capacity,
            route_capacity,
            route_edge_occurrence_capacity,
            route_conflict_occurrence_capacity,
            fixed_delta_time_ms,
        }
    }

    /// 车辆槽位容量上限。
    #[must_use]
    pub const fn vehicle_capacity(self) -> u32 {
        self.vehicle_capacity
    }

    /// 路线槽位容量上限。
    #[must_use]
    pub const fn route_capacity(self) -> u32 {
        self.route_capacity
    }

    /// 全部存活动态路线 `edges.len()` 的总和；重复边按 occurrence 计数。
    #[must_use]
    pub const fn route_edge_occurrence_capacity(self) -> u64 {
        self.route_edge_occurrence_capacity
    }

    /// 全部存活路线 `conflicts.len()` 的总和；重复 passage occurrence 逐项计数。
    #[must_use]
    pub const fn route_conflict_occurrence_capacity(self) -> u64 {
        self.route_conflict_occurrence_capacity
    }

    /// 固定步长（毫秒）。
    #[must_use]
    pub const fn fixed_delta_time_ms(self) -> u64 {
        self.fixed_delta_time_ms
    }
}

/// 宿主显式指定的执行配置，不进入交通快照、摘要或共享静态路网。
///
/// 线程数包含调用线程；1 不创建辅助线程。构造器不验证当前后端的能力，
/// 不支持的数量由安装或完整交通恢复后的执行校验拒绝。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExecutionConfig {
    worker_count: NonZeroU32,
}

impl ExecutionConfig {
    /// 指定一次同步操作参与计算的线程数上限。
    #[must_use]
    pub const fn new(worker_count: NonZeroU32) -> Self {
        Self { worker_count }
    }

    /// 包含调用线程的非零线程数上限。
    #[must_use]
    pub const fn worker_count(self) -> NonZeroU32 {
        self.worker_count
    }

    pub(crate) fn validate_supported(self) -> Result<(), crate::ExecutionInitError> {
        if self.worker_count.get() != 1 {
            return Err(crate::ExecutionInitError::UnsupportedWorkerCount {
                requested: self.worker_count.get(),
                max_supported: 1,
            });
        }
        Ok(())
    }
}

/// 单次步进输入。`delta_time_ms` 必须等于 world 的固定步长。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TickInput {
    /// 本次请求的固定步长。
    pub delta_time_ms: u64,
}

impl TickInput {
    /// 创建步进输入。
    #[must_use]
    pub const fn new(delta_time_ms: u64) -> Self {
        Self { delta_time_ms }
    }
}

/// 成功步进后的可观察时间。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StepOutcome {
    tick_index: u64,
    time_ms: u64,
    parking_arrivals: Vec<crate::ParkingArrivalObservation>,
}

impl StepOutcome {
    /// 构造成功步进结果；仅供 world 提交路径使用。
    pub(crate) const fn new(
        tick_index: u64,
        time_ms: u64,
        parking_arrivals: Vec<crate::ParkingArrivalObservation>,
    ) -> Self {
        Self {
            tick_index,
            time_ms,
            parking_arrivals,
        }
    }

    /// 成功步进后的已提交 `tick_index`。
    #[must_use]
    pub const fn tick_index(&self) -> u64 {
        self.tick_index
    }

    /// 成功步进后的已提交 `time_ms`。
    #[must_use]
    pub const fn time_ms(&self) -> u64 {
        self.time_ms
    }

    /// 本拍从未到达变为 exact committed arrival 的稳定 live-order observation。
    #[must_use]
    pub fn parking_arrivals(&self) -> &[crate::ParkingArrivalObservation] {
        &self.parking_arrivals
    }
}
