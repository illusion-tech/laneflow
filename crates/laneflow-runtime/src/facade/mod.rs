//! 唯一世界聚合与宿主会话；组合仿真内核和管理操作。

pub(crate) mod observation;
/// 宿主 Routing 与 Traffic Runtime 的纯契约边界。
pub(crate) mod routing;
/// 已提交路网来源：活动聚合的来源指名。
pub(crate) mod source;

/// 1-worker 交通世界。只克隆根 `Arc`，不复制静态 component。
/// 生命周期命令（路线、车辆、parking lifecycle 与原子 replace/despawn）只在两次
/// `step` 之间调用。
/// 执行 panic 会使世界永久失效；后续世界查询、交通与管理操作均 panic，宿主应销毁
/// 并从合法来源重新构建。析构会等待所有世界独占辅助线程退出。
pub struct TrafficWorld {
    pub(crate) state: crate::kernel::state::WorldState,
    pub(crate) execution: crate::kernel::execution::WorldExecution,
}
