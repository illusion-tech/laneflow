+++
schema_version = 1
id = "design-traffic-runtime-execution-config"
kind = "design"
status = "accepted"
revised_date = "2026-11-27"
summary = "定义交通世界的具名安装输入、执行资源与快照边界。"
scope = "世界安装、fresh restore、同/跨修订切换、执行资源与 LFRS 版本轴"
applicability = "#applicability"
issues = ["https://github.com/illusion-tech/laneflow/issues/220", "https://github.com/illusion-tech/laneflow/issues/1568"]
related = ["adr-0030", "design-traffic-runtime-parallel-execution", "design-traffic-runtime-revision-cutover", "design-traffic-runtime-snapshot"]
+++

# TrafficWorld 执行配置、资源与快照边界

<a id="applicability"></a>
## 适用边界与依据

设计入口： [#220][issue-220]、[#1568][issue-1568]
关联决策： [ADR 0030（Accepted）][adr-0030]
配套合同： [并行执行][parallel-execution]、
[快照][runtime-snapshot]、[修订切换][revision-cutover]

本文定义已接受的配置 API 与执行生命周期合同。当前配置已分离，LFRS 7 只保存
交通配置，安装和恢复支持 worker 1–16（#705 开放）。活动世界独占执行资源，私有
候选只持有交通状态和目标计划；P2 运动/前视预览、P3 已冻结只读输入上的候选字段
求值与 P5 逐车最终运动按 worker 数真实分发（#705/#706），协调器保序规范组装，
frontier 归约、P4 联合裁决、P5 资源转移与 P7 发布仍由协调器串行；城市性能认证
未完成（#707）。单次测量和进度由 GitHub
管理，配置可表达不证明性能预算已达成。

## 1. 配置职责

| 对象                                                   | 内容                                                         | 正本及持久化                                            |
| ------------------------------------------------------ | ------------------------------------------------------------ | ------------------------------------------------------- |
| 交通配置（Traffic Configuration）/ `WorldConfig`       | vehicle、route、edge/conflict occurrence 四项容量及 fixed dt | 影响准入或行为；继续写快照并进入逻辑摘要                |
| 执行配置（Execution Configuration）/ `ExecutionConfig` | 显式非零 worker 数                                           | 宿主安装/恢复输入；不进入快照、摘要、静态路网或交通身份 |
| 执行资源（Execution Resources）                        | 线程/调度队列等执行能力                                      | 活动世界独占，不结构克隆或持久化                        |
| 运行时执行计划（Runtime Execution Plan）               | 当前修订/工作集的任务布局、局部索引、依赖和所需缓冲          | 可重建，受根及工作集有效性约束，不持久化                |

worker 数是本世界一次同步操作中参与计算的线程数上限。1 使用融合路径，在调用
线程上计算，不创建辅助线程；N ≥ 2 时私有池有 N 个池线程承担全部计算，调用线程
在操作期间只阻塞等待、不参与计算（§3）。不要求任务数等于线程数，也不承诺宿主
进程或多个世界的总线程预算。

宿主显式提供配置，不自动按 CPU 数改变值，不静默降级。首版不公开分区数、地理
tile、亲和性、调度后端、热改线程数或共享执行器注入接口。缺少真实执行需求和
失败语义的资源限制不提前变成公开参数，更不能取代交通准入容量。

## 2. 公开 API

<a id="named-world-install"></a>
### 2.1 交通配置与安装输入

交通配置与世界安装各有一个具名参数对象，类型从 `laneflow_runtime` 根导出。
宿主可在取得共享修订前完成交通配置的构造。

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WorldConfigInput {
    pub vehicle_capacity: u32,
    pub route_capacity: u32,
    pub route_edge_occurrence_capacity: u64,
    pub route_conflict_occurrence_capacity: u64,
    pub fixed_delta_time_ms: u64,
}

impl WorldConfig {
    pub const fn try_new(input: WorldConfigInput) -> Result<Self, WorldConfigError>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
#[non_exhaustive]
pub enum WorldConfigError {
    #[error("fixed_delta_time_ms 必须落在 {min}..={max}，实际 {actual}")]
    DeltaOutOfRange { actual: u64, min: u64, max: u64 },
}

pub struct WorldInstallInput {
    pub revision: Arc<SharedNetworkRevision>,
    pub config: WorldConfig,
    pub execution: ExecutionConfig,
    pub source: CommittedNetworkSource,
    pub world_id: WorldId,
    pub policy_selection: WorldPolicySelection,
}

impl TrafficWorld {
    pub fn install(input: WorldInstallInput) -> Result<Self, InstallError>;
    pub const fn config(&self) -> WorldConfig;
    pub const fn execution_config(&self) -> ExecutionConfig;
}
```

无窗口宿主已取得修订、来源、世界身份和策略时，安装一个单线程世界的调用如下。
例中四项容量各不相同，便于核对宿主将每项预算交给了哪个字段。

```rust
let config = WorldConfig::try_new(WorldConfigInput {
    vehicle_capacity: 2,
    route_capacity: 3,
    route_edge_occurrence_capacity: 5,
    route_conflict_occurrence_capacity: 7,
    fixed_delta_time_ms: 100,
})?;
let world = TrafficWorld::install(WorldInstallInput {
    revision,
    config,
    execution: ExecutionConfig::new(std::num::NonZeroU32::MIN),
    source,
    world_id,
    policy_selection,
})?;
```

五项配置和六项安装输入均必填。两个 `*Input` 结构体均不加 `#[non_exhaustive]`，
使外部 crate 能按任意字段顺序写结构体字面量。漏写、重复或拼错字段均由编译器报告。
今后若新增必填项，宿主在编译时补上该项并显式决定其值。同类型变量仍可能赋给错误的
字段，实施验收须逐项读取实际安装值。

`WorldInstallInput` 手写 `core::fmt::Debug`。调试输出用 `revision.network_revision()`
标识修订，用 `Published` 或 `Editable` 标识来源，并列出世界身份、交通配置、执行配置
与策略选择。这样检查一次安装输入只需读取这些字段，无需展开整张共享路网或编辑数据。

Builder 也能为每个值命名。若以可选值暂存，漏项到 `build()` 才能报告。若以类型状态
记录已填字段，两组 Builder 分别需要五个和六个 setter，且只有字段齐全时才提供
`build` 或 `install`。此处所有字段都必填，具名对象用一次构造就能提供相同的字段
完整性检查。

`WorldConfig` 继续私有持有数值并提供五个 getter。`try_new` 仅检查固定步长的闭区间
`4..=1000` 毫秒，越界返回 `WorldConfigError::DeltaOutOfRange { actual, min, max }`。
该错误实现 `std::error::Error` 和 `Display`。外部宿主可匹配已知变体并读取三个整数，
用通配分支处理后续新增的配置错误。四项容量可分别取零，也可同时取零。容量表达宿主
允许的上限，预留是否成功取决于实际需求和平台。

`with_added_route_capacity(extra_routes, extra_edge_occurrences, extra_conflict_occurrences)`
沿用现行位置参数，仅用于自含测量世界在目录路线之外预留注册余量。三项增量分别饱和
相加，保留已经检查的步长。若常规宿主也需要增量配置，再评估具名化。

### 2.2 世界身份

`WorldId` 由 Runtime 定义，私有持有 `u64`，提供 `pub const fn new(raw: u64) -> Self`
与 `pub const fn get(self) -> u64`。派生 `Clone, Copy, Debug, Eq, PartialEq, Ord,
PartialOrd, Hash`，排序按内部整数值。实现 `core::fmt::Display` 并委托给内部 `u64`
的 `Display`，使 `CutoverError::TransactionWorldMismatch` 的 `{expected_world}` 原样显示
十进制数字。从整数构造统一显式调用 `WorldId::new`，不提供 `From<u64>`。
全部位模式有效，包括 0 和 `u64::MAX`。宿主分配身份，
同一宿主进程内，一个身份只属于一个逻辑世界。`WorldGeneration` 由 Runtime 维护，
普通安装和 fresh restore 从初代 0 开始。活动世界成功切换或原位恢复沿用身份并推进世代。
fresh restore 延续快照所指的逻辑世界，宿主须先使该身份的旧实例及其消费会话失效。

同一值通过 `TrafficWorld::world_id`、`WorldBinding`（含切换描述符）、
`ObservationStreamBinding`、`RoutingAdmissionSession`、`CapturedSnapshot` 以及
Bevy 的消费与路口观测上下文传递为 `WorldId`。`CutoverError::TransactionWorldMismatch`
的 `expected_world` 同样使用该类型。构造绑定时世界身份与世代有各自的
参数类型，命令及事件游标仍各按现行合同处理。仅在字节或外部整数边界显式取出或构造
标量。LFRS 的 `world_id:ulong` 和逻辑摘要中的小端八字节表示同一个宿主值。
切换描述符按身份值比对。观测与 Routing 摘要沿用世界身份的小端编码和原有排序规则。

Bevy 两个上下文的 `world_id()` 与现有的 `world_generation()` 一样直接返回 Runtime
类型。宿主需要构造世界身份或在签名中写出类型时，直接依赖 `laneflow-runtime` 并导入
`laneflow_runtime::WorldId`。`laneflow_bevy` 根不另行重导出。

### 2.3 执行配置与恢复入口

```rust
impl ExecutionConfig {
    pub const fn new(worker_count: std::num::NonZeroU32) -> Self;
    pub const fn worker_count(self) -> std::num::NonZeroU32;
}

pub fn restore_lfrs(
    bytes: &[u8],
    revision: Arc<SharedNetworkRevision>,
    source: CommittedNetworkSource,
    target_config: WorldConfig,
    execution: ExecutionConfig,
    limits: SnapshotRestoreLimits,
) -> Result<RestoredSnapshot, SnapshotRestoreError>;
```

本次仅将新世界的安装参数收拢为 `WorldInstallInput`。`restore_lfrs` 保留上述六个类型
各异的位置参数，宿主传入受检的目标配置，恢复过程从快照读出世界身份和策略（§4.2）。

0 在宿主构造 `NonZeroU32` 时被拒绝，串行可用 `NonZeroU32::MIN`。当前后端支持
worker 1–16，请求更大 worker 数须返回执行能力错误；类型可表达不代表任意数量
可执行。首版不增加猜测性 `Default`、旧构造器、弃用别名或转换入口。

`step(&mut self, TickInput) -> Result<StepOutcome, StepError>` 保持同步；返回前完成
本次所有任务的 join。生命周期命令、观测和 Routing 仍消费完整已提交世界，Adapter
不接收分区身份。宿主调用点须传执行配置，但不因此改变表现层数据协议。生命周期命令
走到重叠检查或占用索引重建点时，可以借执行资源并行备好派生占用索引与生成重叠
索引，再重新执行这条命令（[新生成第 8、9 节][vehicle-placement]）；
它只改派生索引，panic 同样使世界失效。

## 3. 私有所有权

引入执行资源时，必须以共同私有状态聚合复用现有五类所有者，活动 facade 另持
执行资源。候选拥有完整交通数据和目标计划，但不是可独立步进的活动世界：

```text
TrafficWorld
  state: WorldState
    binding / committed / derived / workspace / admin
  execution: WorldExecution
    config
    resources                 不可 Clone
    active_plan
    attempt_epoch / 任务登记

PreparedWorldState            私有；不提供公开 step
  state: WorldState
  prepared_plan
```

这些私有名称用于表达所有权，确切 Rust 方法归属须经真实借用核对。五类状态的正本
不改变，不用 `Option<Pool>` 构造缺资源的公开世界，也不靠全局线程池隐藏生命周期。
公共安装和恢复复用 `WorldState::prepare_traffic_state`。交通准备不接收执行配置，
不构造活动 facade；完整交通准入之后才建立计划和资源。恢复过程中不能启动线程。

首版计划包含目标修订绑定、独立计算的任务范围与所需缓冲，不预建 P4 资源组件图、
局部资源表或临时 grant 绑定表。P4 继续由协调器规范串行裁决。

计划包含目标修订的绑定及可重建布局；每拍工作集在取得当拍视图后建立，不能永久冻结
安装/恢复时的车辆顺序。计划不得保存跨安全边界的悬空借用。资源线程可存在于 step
之间，但不能持有上一操作的世界引用；候选准备默认同步，不新增与活动世界并发的
后台候选执行接口。

借用证明须覆盖实际共享的字段及构建模式。只读方法不自动使整个聚合满足 `Sync`；
诊断计数器、故障注入和测试专用内部可变性也须有明确归属。可以收窄任务视图，或
在任务局部记录、join 后汇总；不能用未证明的 `unsafe impl Sync` 绕过检查。初始化
清理、作用域内任务借用及持久线程执行任务是不同义务，分别通过不等于已证明完整
线程后端。

活动世界独占资源简化取消、drop 和寿命归属，代价是多世界分别付费。宿主共享执行器
需要单独冻结重入、饥饿、borrow lifetime、panic 和全局资源预算，不能只把私有池改为
一个公共 `Arc<Pool>` 参数。

私有后端使用 Rayon 的 `rayon-core` 独占线程池。N 配置（N ≥ 2）对应 N 个池线程，
每个保留 2 MiB 栈；调用线程把整次操作交给池后只等待，不另占计算核心，但仍保留
自身栈。曾经的 N−1 个辅助线程加调用线程算首块的形态，在整拍移入池内后会少一个
计算线程：#814 同机 A/B（`urban-100k`、持续十万 Active）中 8 worker 的拍时 p50
慢约 9%，16 worker 持平，因此按 N 个池线程定稿。
`in_place_scope` 的协调闭包在发起线程执行首块，其余任务使用互斥输出范围。
协调器分段并行按互斥输出块的边界二分 `join`，空闲线程窃取另一半；`join` 在两半
都完成后才返回，不留下脱离作用域的任务，也不等待用不上的线程。
整拍本身在一次 `broadcast` 内运行：0 号池线程执行拍体，其余线程在拍内自旋并经
`yield_now` 接手各并行段的任务，省去每段唤醒已睡线程的开销；代价是拍内串行空档
里辅助线程空转、总 CPU 上升。拍体返回或展开即置位结束标志，`broadcast` 等全部
线程退出后才返回。辅助线程若在等待已窃取的 `join` 子任务时才调起本拍的自旋作业，
该作业直接返回，不自旋到拍尾。
不使用全局池、`use_current_thread` 或脱离操作作用域的任务。LaneFlow 显式保留
每个 OS 线程的 `JoinHandle`：构建失败和正常析构均先关闭池，再 join 全部已启动
线程。Rayon 作用域 join 与 OS 线程退出 join 是两层不同的结算义务。

LaneFlow 持有的必需计划、任务范围、输出缓冲和线程登记表使用 checked 长度与
`try_reserve`。Rayon 内部 registry、工作队列及任务节点（含 `broadcast` 每次调用的固定大小
作业表）、标准库线程启动内部的
分配沿用进程级 OOM 语义，不能承诺均转换为可恢复错误；进程级失败不承诺世界
继续可用。可恢复预留错误与平台线程创建错误仍按 §4.3 返回，不能混为交通错误。

内存账本包括活动状态/计划/工作区、N 个池线程栈与队列、候选状态/计划/工作区、迁移
日志和退休状态。候选无第二套线程资源，但仍占内存。线程栈保留与提交字节分别计账。

## 4. 安装与 fresh restore

### 4.1 安装

宿主先构造有效的 `WorldConfig`。步长越界且来源错误时，调用方在取得配置前即收到
`WorldConfigError`。安装和恢复都以受检配置为输入。

安装依次检查来源修订、编辑来源的差异基线、信号相位与步长的整除和下限约束、
路权策略及 Conflict 仲裁状态，然后验证执行能力，预留计划和缓冲，初始化执行资源，
最后返回活动世界。例如，错误来源先于信号错误，`PhaseNotMultipleOfTick` 先于
超出后端能力的 worker 数。这些依赖修订或后端的检查各自在原阶段报告。

能力不支持时不创建线程；部分线程启动失败时，先取消并 join 已启动线程，再释放
私有状态并返回错误。资源初始化成功不授权发布尚未完成交通准入的对象。

现有安装包含不可恢复分配。新增计划/队列准备必须显式可失败，但配置拆分不自动
把整个安装变成可恢复 OOM；若改造既有分配，须独立说明错误面与顺序。

### 4.2 Fresh restore

恢复顺序为：

1. 调用方 wire 上限、framing、identifier、有界 FlatBuffers verifier。
2. 检查 format/runtime 版本、封闭表字段、静态及来源绑定、dt、容量、时钟与记录数。
3. 解码快照策略，复用共同交通安装原语建立私有状态。按现行顺序恢复路线、车辆和资源，
   在消费各记录时检查其封闭枚举，完成已提交状态与派生索引校验。
4. 验证执行能力，为最终恢复状态准备计划和缓冲，然后初始化执行资源。
5. 一次性返回完整 `RestoredSnapshot`；任一错误不返回半个世界。

调用方只为目标交通配置运行 `try_new`。快照保存的步长和容量仍是外部输入，由恢复
路径按上述顺序验证。合法目标步长为 4 ms 而快照保存 3 ms 时，来源绑定通过后应得到
`SnapshotRestoreError::FixedDeltaTimeMismatch`。损坏的车辆或路线记录与不支持的
worker 数并存时，先返回交通恢复错误。世界身份和路权策略由快照读入，宿主显式提供
目标修订、来源、交通配置、执行配置和恢复限额。

恢复直接调用共同交通准备原语。若先走公开安装入口，会在快照后半段仍可能失败时启动
资源，并按空世界建立计划。恢复路线时临时放开 Conflict occurrence 容量，重编译后再按
真实总量同时核对保存容量和目标容量。

`fixed_delta_time_ms` 必须精确匹配，四项目标语义容量都不能小于保存容量，即使实际
live state 放得下也拒绝缩容。放大容量可恢复，但会改变未来生命周期命令的成功条件，
因此精确回放对拍仍要求相同语义容量，摘要也继续包含它们。

执行错误放到完整交通恢复校验之后，是明确的新错误次序；不能声称与旧 worker 在
较早 install 阶段被拒绝的顺序完全相同。有效执行配置下，原有交通错误相对次序保持。
fresh restore 继承快照世界身份、建立新的本地世代/会话；不同执行配置不豁免同身份
旧活动世界和旧会话的失效要求。

### 4.3 错误类型

| 错误合同                                                                  | 意义                               | 返回位置                                                              |
| ------------------------------------------------------------------------- | ---------------------------------- | --------------------------------------------------------------------- |
| `ExecutionInitError::UnsupportedWorkerCount { requested, max_supported }` | 当前后端不能提供指定并行度         | `InstallError::ExecutionInit` / `SnapshotRestoreError::ExecutionInit` |
| `ExecutionInitError::ResourceReservationFailed`                           | 调度资源的必需预留失败             | 同上                                                                  |
| `ExecutionInitError::WorkerStartFailed`                                   | 平台线程创建失败，已创建资源先结算 | 同上；平台细节仅作诊断，不入逻辑摘要                                  |
| `ExecutionPlanError::SizeOverflow`                                        | 布局长度或索引计算溢出             | install / restore / cutover 各自的 `ExecutionPlan` 包装               |
| `ExecutionPlanError::ReservationFailed`                                   | 目标计划或其必需缓冲准备失败       | 同上                                                                  |

公开安装和恢复接受 worker 1–16（#705 已开放）；计划错误由安装、恢复及两种切换的
`ExecutionPlan` 包装。线程资源失败在私有原语中验证并结算。执行错误不冒充交通
容量、领域 StepError 或无关暂存错误。执行器 panic 的 join、世界失效及 drop 遵循
[并行执行 §4.3][parallel-execution-join]；初始化
错误的 Result 不构成运行中 panic 可恢复的承诺。

## 5. 修订切换与计划发布

`cutover_same_revision` 和 `prepare_cross_revision_cutover` 均不增加执行配置参数，
继承活动世界配置和资源。切换描述符不携带 worker，路网切换不承担热改并行度。

同修订换根保留原有认证、路线/资源重验证、事件与游标预检顺序，在首次根突变前
准备目标计划。即使 NetworkRevisionId 相同，也不能仅凭该值复用绑定旧内存布局的
计划；保留部分已证明可复用的索引不等于旧计划整体仍有效。目标计划与根、来源、
派生状态、世界世代和观测失效同界发布，资源身份保持。

跨修订 prepare 保留描述符、来源、策略、diff 校验及日志武装次序。结构克隆生成
私有候选状态，再准备初始目标计划；任一步失败按当前事务规则释放候选并解除本次
武装，活动交通状态和执行资源不变。

pump 追赶改变候选生命周期/路线/资源后须维护计划有效性；静态部分可复用，动态
工作集不可沿用过期结果。可标记失效并在需要前重建，但不能发布失效计划。

commit 在现有最终重放、重验证、摘要、事件与游标检查之后，完成最终计划核对与
必需补齐，再进入零分配发布段。当前发布是逐字段交换，执行资源保留在活动世界，
目标计划显式同界晋升，旧计划随退休状态释放。首个目标 tick 不能执行旧修订任务。

共同 `WorldState` 是所有权聚合，不授权整体交换其全部字段。现行切换只晋升通过
迁移验证的交通字段；管理日志、迁移 epoch、游标和历史批次分别按原有规则结算。
计划应描述实际发布后的根、世代和工作集，不能直接把候选准备时的世代当作最终
世代，也不能把候选完整管理状态覆盖到活动世界。新增最终计划准备仍必须位于
第一笔根突变之前，不能在真实 commit 成功后用可能失败的补建修复计划。

失败保留旧交通状态、根、配置、计划和执行资源；日志按现有失败结算规则解除，
不要求管理瞬态逐字节相同。发布/失败复用前均需 join，attempt epoch 与迁移事务
epoch、WorldGeneration 各自承担其原有寿命，不相互冒充。

计划准备和退休释放计入切换预算，最终补齐工作也计入静默窗口。不能因为存在提前
准备，就假设最终成本为零；投机大额预留失败不能伪装成实际需求失败。具体规则见
[并行执行][parallel-execution]的容量和完成前沿合同。

## 6. 格式与版本轴

| 轴                                   | 当前合同                                  | 原因                                                                                                                            |
| ------------------------------------ | ----------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------- |
| Rust API                             | 交通与执行配置分离，显式安装/恢复参数     | 1.0 前破坏性清扫，无兼容别名                                                                                                    |
| LFRS `format_version`                | 7                                         | 删除 worker，`fixed_delta_time_ms` 的 vtable 槽位移动；编辑来源绑定见 [`committed-road-editing-state.md`][committed-editing] §6 |
| schema / namespace                   | `runtime-snapshot/v7` / V7                | 只保留唯一当前 wire binding                                                                                                     |
| `RUNTIME_STATE_VERSION`              | 5                                         | 交通逻辑状态字段和含义不变                                                                                                      |
| `RUNTIME_STATE_DIGEST_VERSION`       | 8                                         | 规范原像已经排除 worker；算法为 BLAKE3-256（#1035），状态版本头及其余编码不变                                                   |
| 摘要 domain                          | `laneflow:runtime-state-digest:v1` 加 NUL | 摘要算法和规范化规则不变                                                                                                        |
| LFCA 六轴 / NetworkRevision 派生版本 | 各自现行值，不因执行配置升级              | 静态事实不增加执行分配                                                                                                          |
| cutover descriptor format            | 2                                         | 不修改持久化迁移描述符                                                                                                          |
| Observation / Routing 绑定           | 各自现行值，不因执行配置升级              | 不改变交通身份、游标或失效语义                                                                                                  |

#1568 迁移 Rust 调用与错误类型。四项容量、步长、宿主世界身份和策略保有既有值和编码，
因此沿用表中各格式及摘要版本。若后续修改交通状态或摘要原像，须重新判断相应版本。

`WorldConfigBinding` 不含 worker。绑定由钉版 flatc 生成，writer/reader、封闭字段数、
wire pin、xtask schema 路径与 Rust/C++/C# 检查保持一致。不手改生成 getter/slot，
不提供旧 schema 回退或旧字段转换。

仍使用 size-prefixed `LFRS`。当前 schema 的结构 verifier 先于版本检查，某些旧字节
可能先被判为 `InvalidFlatbuffer`，结构可通过者才得到版本错误；要求旧格式没有成功
路径，不强求所有旧字节都返回同一个错误。旧 header 伪装成 6 也不能被误读为合法
新配置；缺省字段、槽位错置和未知 vtable 字段都需实际字节测试。

## 7. 实施验收与评审边界

具名安装接口的实施按下列用例验收，G1 调用原型与既有主干回归作为对照基线。

1. 在独立宿主 crate 用两个公开字面量安装世界，使用 §2.1 的五个不同数值，逐字段
   读取并推进一个固定步。验证配置输入的复制与相等、安装输入的调试摘要、配置错误的
   通配匹配及切换错误中世界身份的十进制显示。漏写字段、裸整数、从整数调用 `.into()`、
   将 `WorldGeneration` 用作 `WorldId` 或穷举当前配置错误而省略通配分支均编译失败。
2. 在构造入口断言 3、1001 ms 的错误内容及 4、1000 ms 的成功值。分别验证合法零容量，
   包括四项全零的空世界。用有路线和车辆的既有场景验证各容量的准入语义。
3. 安装时组合错误来源、不兼容信号相位和超限 worker，分别固定来源错误、相位错误与
   合法交通下的执行错误，验证失败时没有活动世界或遗留线程。恢复时覆盖保存步长损坏、
   目标缩容、目标扩容、交通记录损坏并请求超限 worker，按 §4.2 检查首错与原子性。
4. 在同一主干基线及迁移后的实现分别执行相同的安装、命令与步进序列，比对提交结果、
   LFRS 字节及逻辑摘要。验证身份 0 与 `u64::MAX` 可往返、错误世界或世代的绑定
   被拒绝、fresh restore 和同／跨修订切换保有原身份及约定的世代。只改 worker 数时
   交通身份、快照字节及逻辑摘要相同。
5. 同次迁移 Runtime 的公开签名、正常安装与恢复调用点、无窗口示例、Bevy 会话、工具、
   测试及宿主接入指南。宿主从 `laneflow_runtime` 导入 `WorldId`，用同一类型接收
   `LaneFlowConsumptionContext::world_id()` 和 `LaneFlowJunctionObservationContext::world_id()`
   的返回值。恢复用例直接复用受检配置。

执行配置和资源生命周期的既有验收如下。

| 领域       | 必须验证                                                                                                                                                                                                         |
| ---------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| API/调用点 | Runtime 根导出、examples/adapter/tools/tests 显式配置；只有受支持执行配置可安装                                                                                                                                  |
| 格式       | 真实 v5/v6、未知 format/runtime、worker 缺省/非零、伪造 header、未知槽、合法 v6 完整恢复                                                                                                                         |
| 交通语义   | 四项容量、dt、策略和 Conflict/Waiting/parking 恢复；digest 7 固定原像字节保持                                                                                                                                    |
| 生命周期   | 部分资源初始化失败清理；drop/失败不遗留任务；fresh restore 无半成品                                                                                                                                              |
| 切换       | 两条路径的计划准备失败零发布、资源不复制、追赶后计划新鲜度与首拍目标绑定                                                                                                                                         |
| 并行       | worker 1–16 可安装/恢复；P2 预览、P3 候选字段与 P5 最终运动真实多 worker 分发（#705/#706），协调器保序规范消费，frontier/P4/资源转移/P7 串行；合法配置下状态/事件/逻辑首错/retry 等价；旧 epoch、join 与序号配对 |
| 成本       | 活动/候选/退休计划、线程栈和队列峰值；最终补齐与静默窗口成本                                                                                                                                                     |

已接受的边界是世界独占资源、候选复用原资源和首版独立计算并行。配置拆分与
容器迁移不代替资源/计划生命周期、持久线程调度和真实多 worker 验收。私有候选/
活动世界借用原型只能支持所有权判断；生产状态聚合、执行后端与城市性能仍须由
各自实现和验证完成。P4 组件计划及临时序号绑定不属于首版验收。

[issue-220]: https://github.com/illusion-tech/laneflow/issues/220
[issue-1568]: https://github.com/illusion-tech/laneflow/issues/1568
[adr-0030]: ../adr/0030-single-world-parallel-execution.md
[parallel-execution]: traffic-runtime-parallel-execution.md
[runtime-snapshot]: traffic-runtime-snapshot.md
[revision-cutover]: traffic-runtime-revision-cutover.md
[vehicle-placement]: traffic-runtime-vehicle-placement.md
[parallel-execution-join]: traffic-runtime-parallel-execution.md#43-join取消与失败清理
[committed-editing]: committed-road-editing-state.md
