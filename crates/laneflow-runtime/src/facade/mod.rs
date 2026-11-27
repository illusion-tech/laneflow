//! 唯一世界聚合与宿主会话；组合仿真内核和管理操作。

use std::{fmt, sync::Arc};

use laneflow_static_network::SharedNetworkRevision;

use crate::{CommittedNetworkSource, ExecutionConfig, WorldConfig, WorldId, WorldPolicySelection};

/// 已提交道路编辑状态与编辑来源差异基线。
pub(crate) mod editable_source;
pub(crate) mod observation;
/// 宿主 Routing 与 Traffic Runtime 的纯契约边界。
pub(crate) mod routing;
/// 已提交路网来源：活动聚合的来源指名。
pub(crate) mod source;

/// 安装新世界所需的完整输入。来源和策略在绑定共享修订时校验。
///
/// 策略选择须显式给出，即使该修订允许 [`WorldPolicySelection::NotRequired`]。
///
/// ```compile_fail,E0063
/// # use laneflow_runtime::*;
/// # use laneflow_static_network::SharedNetworkRevision;
/// # use std::sync::Arc;
/// # fn inputs(revision: Arc<SharedNetworkRevision>, config: WorldConfig,
/// #     execution: ExecutionConfig, source: CommittedNetworkSource, world_id: WorldId) {
/// let _ = WorldInstallInput { revision, config, execution, source, world_id };
/// # }
/// ```
pub struct WorldInstallInput {
    /// 待安装的共享静态路网修订。
    pub revision: Arc<SharedNetworkRevision>,
    /// 宿主的交通容量和受检固定步长。
    pub config: WorldConfig,
    /// 宿主为本世界指定的执行配置。
    pub execution: ExecutionConfig,
    /// 与共享修订对应的已提交来源。
    pub source: CommittedNetworkSource,
    /// 宿主分配的世界身份。
    pub world_id: WorldId,
    /// 该修订的路权策略选择。
    pub policy_selection: WorldPolicySelection,
}

impl fmt::Debug for WorldInstallInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let source = match self.source {
            CommittedNetworkSource::Published { .. } => "Published",
            CommittedNetworkSource::Editable { .. } => "Editable",
        };
        f.debug_struct("WorldInstallInput")
            .field("network_revision", &self.revision.network_revision())
            .field("config", &self.config)
            .field("execution", &self.execution)
            .field("source", &format_args!("{source}"))
            .field("world_id", &self.world_id)
            .field("policy_selection", &self.policy_selection)
            .finish()
    }
}

/// 交通世界。只克隆根 `Arc`，不复制静态 component。
///
/// 安装时显式传入 [`crate::ExecutionConfig`]。worker 数为 1 时在调用线程计算；
/// 2–16 时由该世界独占的池线程计算，调用线程只等待。worker 数不进入快照或交通身份，
/// 也不改变已提交交通结果。超出后端能力的请求在完整交通准备后拒绝。
/// 生命周期命令（路线、车辆、parking lifecycle 与原子 replace/despawn）只在两次
/// `step` 之间调用。
/// 执行 panic 会使世界永久失效；后续世界查询、交通与管理操作均 panic，宿主应销毁
/// 并从合法来源重新构建。析构会等待所有世界独占辅助线程退出。
pub struct TrafficWorld {
    pub(crate) state: crate::kernel::state::WorldState,
    pub(crate) execution: crate::kernel::execution::WorldExecution,
}
