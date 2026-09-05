//! 唯一世界聚合与宿主会话；组合仿真内核和管理操作。

pub(crate) mod observation;
pub(crate) mod routing;
pub(crate) mod source;

/// 1-worker 交通世界。只克隆根 `Arc`，不复制静态 component。
/// 生命周期命令（路线、车辆、parking lifecycle 与原子 replace/despawn）只在两次
/// `step` 之间调用。
pub struct TrafficWorld {
    pub(crate) binding: crate::kernel::state::WorldBindingState,
    pub(crate) committed: crate::kernel::state::CommittedWorldState,
    pub(crate) derived: crate::kernel::state::DerivedIndexes,
    pub(crate) workspace: crate::kernel::state::TickWorkspace,
    pub(crate) admin: crate::admin::state::AdministrativeState,
}
