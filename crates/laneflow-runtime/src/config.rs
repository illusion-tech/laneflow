/// 每世界安装配置。同一 world 运行中不得改变 `fixed_delta_time_ms`。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WorldConfig {
    vehicle_capacity: u32,
    route_capacity: u32,
    route_edge_occurrence_capacity: u64,
    worker_count: u32,
    fixed_delta_time_ms: u64,
}

impl WorldConfig {
    /// 创建配置。合法性在 `install` 检查，构造器本身不失败。
    #[must_use]
    pub const fn new(
        vehicle_capacity: u32,
        route_capacity: u32,
        route_edge_occurrence_capacity: u64,
        worker_count: u32,
        fixed_delta_time_ms: u64,
    ) -> Self {
        Self {
            vehicle_capacity,
            route_capacity,
            route_edge_occurrence_capacity,
            worker_count,
            fixed_delta_time_ms,
        }
    }

    #[must_use]
    pub const fn vehicle_capacity(self) -> u32 {
        self.vehicle_capacity
    }

    #[must_use]
    pub const fn route_capacity(self) -> u32 {
        self.route_capacity
    }

    /// 全部存活动态路线 `edges.len()` 的总和；重复边按 occurrence 计数。
    #[must_use]
    pub const fn route_edge_occurrence_capacity(self) -> u64 {
        self.route_edge_occurrence_capacity
    }

    #[must_use]
    pub const fn worker_count(self) -> u32 {
        self.worker_count
    }

    #[must_use]
    pub const fn fixed_delta_time_ms(self) -> u64 {
        self.fixed_delta_time_ms
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
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StepOutcome {
    tick_index: u64,
    time_ms: u64,
}

impl StepOutcome {
    pub(crate) const fn new(tick_index: u64, time_ms: u64) -> Self {
        Self {
            tick_index,
            time_ms,
        }
    }

    /// 成功步进后的已提交 `tick_index`。
    #[must_use]
    pub const fn tick_index(self) -> u64 {
        self.tick_index
    }

    /// 成功步进后的已提交 `time_ms`。
    #[must_use]
    pub const fn time_ms(self) -> u64 {
        self.time_ms
    }
}
