//! 仿真内核：运行状态、固定步进与交通规则；不消费制品或快照 wire。

pub(crate) mod config;
/// 冲突区裁决核心：冲突候选、间隙证明与组合资源的单写者语义原语。
pub(crate) mod conflict;
/// 固定步进内的 Conflict 编排：候选求值、组合仲裁与 tick 局部 grant。
pub(crate) mod conflict_tick;
/// 按物理边与实际后车间距索引下游占用的 AVL 区间树。
pub(crate) mod downstream_index;
/// 运行时公开错误类型（安装、路线、生成、替换与步进等失败）。
pub(crate) mod error;
/// 代际感知的世界句柄（路线与车辆）。
pub(crate) mod handle;
/// 宿主输入命令的规范化记录（路线注册、车辆生成等）。
pub(crate) mod input;
/// 固定步进跟车求解使用的车道边占用索引。
pub(crate) mod occupancy;
/// 停车预约、绑定与虚拟容量的运行时状态。
pub(crate) mod parking;
/// 串行步进阶段的借用能力视图；准备计算不取得 `&mut TrafficWorld`。
pub(crate) mod phase;
/// 显式世界策略绑定与步长派生表。
pub(crate) mod policy;
/// 已提交 pose 与信号批次的权威来源。
pub(crate) mod pose;
/// 道路准入（生成）的物理边重叠候选索引。
pub(crate) mod spawn_overlap;
/// 固定步进与管理操作共享的五类私有状态所有者。
pub(crate) mod state;
/// 编译路线、路线/车辆槽位与占用区间行走等运行时表。
pub(crate) mod tables;
/// 固定步进主循环：预检、占用重建、准备与提交。
pub(crate) mod tick;
/// 已验证的 Waiting/Conflict 转移到唯一公开事件批次的投影。
pub(crate) mod transitions;
/// 已提交毫米与 IIDM 瞬时 SI 之间的换算。
pub(crate) mod units;
/// 已提交车辆生命周期状态与替换记录。
pub(crate) mod vehicle;
/// 等待区运行时状态与准入裁决。
pub(crate) mod waiting;
/// Waiting 容量视图、反向依赖阈值与候选图事务。
pub(crate) mod waiting_dependencies;
/// 本 tick 实际 Waiting 依赖的增量无环图。
pub(crate) mod waiting_graph;
/// `TrafficWorld` 安装、查询与生命周期命令的实现。
pub(crate) mod world;

#[cfg(test)]
#[path = "tests/spawn_overlap.rs"]
mod spawn_overlap_tests;

/// 测试构建中的批次阶段墙钟剖析；无生产 feature、API 或状态字段。
#[cfg(test)]
pub(crate) mod performance_profile;

/// #216 测试专用 exact 路径归因和有限候选；不进入生产构建。
#[cfg(test)]
#[path = "tests/exact_path.rs"]
pub(crate) mod exact_path_research;
