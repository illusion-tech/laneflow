# ADR 0021：城市模拟游戏交通基础的产品北极星

**状态**: Proposed（#291 G1 再修订输入）<br>
**日期**: 2026-07-29<br>
**适用范围**: LaneFlow 长期产品定位、城市模拟游戏上层边界、交通编排、路径规划、
路网修订、存档/回放、过载优雅降级（Graceful Overload Degradation）与中国特色
城市工作负载（Chinese-style City Workload）<br>
**扩展**: ADR 0001；本 ADR 不让 LaneFlow 自身拥有城市经济、市民出行需求或完整
交通工程仿真职责<br>

**关联文档**:

- 上游决策:
  - `0001-project-scope.md`
  - `0003-runtime-tick-and-determinism.md`
  - `0016-scenario-population-and-recycle-lifecycle-authority.md`
- 配套决策:
  - `0020-compiler-owned-static-network-and-static-image.md`
- 详细设计:
  - `../architecture.md`
  - `../design/network-compiler.md`
  - `../design/core-runtime-scalability-audit.md`
  - `../design/core-runtime-performance-baseline.md`
  - `../reference/glossary.md`
- GitHub:
  - #72
  - #215
  - #220
  - #291

## 术语规范

本 ADR 的中文术语和中文定义是权威事实，英文只作辅助理解。完整映射以
`../reference/glossary.md` 为单一事实源（Single Source of Truth，SSOT）；类型、
crate、字段、版本值、算法和协议常量等精确标识符保留原文。

## 背景

ADR 0001 正确地把城市经济模拟、市民出行需求模拟、专业交通工程仿真和完整
SUMO-like 系统排除在 LaneFlow 当前职责之外，但该“非目标”不能被解释为 LaneFlow
只面向园区或背景车流。

LaneFlow 的第一长期产品目标是为未来的中国特色城市模拟游戏提供交通基础。该目标
要求 LaneFlow 能够在游戏引擎内支撑大规模、确定性、可诊断、可存档、可修改路网的
交通执行，同时继续让人口、经济、土地利用、目的地、出发时刻和游戏规则由城市模拟
游戏拥有。

#291 的 Draft 方案已提出编译器拥有的静态路网、目标静态镜像和多世界共享边界；
#72/#215/#220 分别保存城市级规模、产品性能基线和未来并行运行时入口。若不先通过
G1 冻结产品北极星，后续容易发生两种相反漂移：

- 因“城市经济不是 LaneFlow 目标”而把城市级交通执行也错误降为非目标；
- 为了服务城市游戏而把人口经济、出行需求、路径选择和宿主游戏规则塞进交通运行时。

## 决策

### 1. 第一长期产品目标是城市模拟游戏交通基础

LaneFlow 的第一长期产品目标定义为：

> 面向中国特色城市模拟游戏的、可嵌入、确定性、可扩展的交通基础设施。

“交通基础设施”至少包含：

- 可重放的静态路网编制与编译；
- 可信、可共享、面向热路径的目标静态镜像；
- 各已实现交通执行域中的交通参与单元、动态通行定义（Dynamic Traversal
  Definition）、信号、冲突、停驻和动态交通状态的固定步进执行；
- 面向上层出行/游戏系统的显式命令、快照、事件和路径接入边界；
- 面向主流游戏引擎的 Adapter/Presentation 集成；
- 存档、回放、诊断、路网修订和规模演进所需的稳定身份与版本基础。

该目标不声明当前版本已经完成城市级产品认证。一万、十万和一百万的角色、硬件与
证据边界继续由 `core-runtime-performance-baseline.md` 裁决。

### 2. 城市游戏上层与 LaneFlow 权威分离

| 层级                                                         | 权威职责                                                                                     |
| ------------------------------------------------------------ | -------------------------------------------------------------------------------------------- |
| 城市模拟游戏层（City Simulation Game Layer）                 | 人口、经济、土地利用、建筑、工作/居住、物流任务、游戏规则                                    |
| 出行与交通编排层（Mobility and Traffic Orchestration Layer） | 出行需求、出发时刻、参与单元生成、目的地、人口生命周期和路线选择策略                         |
| 路径规划服务（Routing Service）                              | 从静态路网和已提交动态成本快照生成候选路径；不在参与单元固定步进内全图寻路                   |
| LaneFlow 编译器与静态镜像                                    | 静态标识、拓扑、几何、规则、索引、执行约束和目标布局                                         |
| LaneFlow 交通运行时（LaneFlow Traffic Runtime）              | 固定步进、已实现执行域的参与单元、动态通行定义、信号解释、冲突仲裁、停驻和每世界可变交通状态 |
| 引擎适配器与表现层（Engine Adapter and Presentation Layer）  | 宿主生命周期、实体、变换、动画、细节层次、用户界面和调试可视化                               |

出行需求（Travel Demand）决定“谁在何时为何出发”；路径规划（Routing）决定“采用
哪条路径”；交通运行时决定“交通参与单元如何在所属执行域的静态规则和当前交通状态下
安全推进”。三者不得形成隐藏的双重权威。

交通运行时只导出已提交交通观测快照（Committed Traffic Observation Snapshot），
不拥有全局路径成本政策。路径规划/出行编排层结合静态路网、已提交交通观测、收费、
游戏政策和出行偏好构造动态成本快照（Dynamic Cost Snapshot），并拥有其版本与
成本模型；Traffic Runtime 只验证/注册候选动态通行定义并安全执行。当前道路机动车
执行域的具体投影是 `Route`。动态成本快照和候选动态通行定义必须绑定从
`TrustedStaticImage` 静态视图或已提交观测快照取得的路网修订标识、观测固定步进与
成本模型版本；Runtime 对修订不匹配的候选失败关闭，并继续验证候选稳定引用和拓扑，
不能以修订标识相等替代内容验证。过期容忍策略由 Routing G1 显式冻结。

“已提交交通观测快照”定义的是一致性时点，不要求每个固定步进复制全网。生产接入
必须允许按观测导出节奏（Observation Export Cadence）导出完整基线，并支持版本化
增量或分区选择；验证门禁必须分别量化观测导出、动态成本快照接收和候选通行定义
注册的条目数、字节、分配、墙钟耗时与对固定步进的干扰。

本 ADR 不冻结路径规划服务的 crate、算法或公共 API。后续 G1 可以选择独立
`laneflow-routing` 或宿主自有实现，但交通运行时的参与单元热路径不得执行全图
寻路。

### 3. 中国特色通过数据、政策和工作负载（Workload）表达

中国特色城市交通能力应通过可版本化的领域数据、法规政策、编制前端和代表性
工作负载表达，不在 SignalController、Adapter 或通用 fixed tick 中硬编码国家
分支。

优先覆盖的长期场景包括：

- 多阶段信号与左转待转区；
- 干路、支路、小区出口和不规则老城区的密集冲突；
- 早晚高峰的强方向不均衡；
- 公交、出租、停车、上下客和路侧摩擦；
- 后续非机动车、步行与混合交通；
- 施工、临时封闭、动态车道用途和玩家修改道路。

ADR 0018/0019 已冻结的法规来源沿袭、WaitingZone、ConflictZone 和车辆级通行权
边界继续有效。未来中国特色城市工作负载必须与现有合成基线和 LuST 补充基线并列，
不得静默替换既有 workload ID 或把尚未支持的参与者伪装为机动车。

### 4. 城市规模先区分人口、交通参与单元、执行域和表现

城市人口、乘客、出行需求、交通参与单元和表现实例属于不同权威层；不能默认一名
居民对应一个 LaneFlow 运行时实体。交通参与单元（Traffic Participant Unit）是
LaneFlow 中可独立保留身份的计数原子，例如当前一辆道路机动车是一个车辆运行单元；
未来骑行者与非机动车的组合、一个行人或一个轨道运营编组的计数原子，分别由对应
交通执行域（Traffic Execution Domain）的 G1 冻结。轨道编组不得按车厢或乘客重复
计算，乘客也不因乘坐交通工具就自动进入 Traffic Runtime 权威。

交通执行域把共享网络、运动/安全求解和生命周期契约的一类参与单元分组。道路机动车、
道路非机动车、步行和轨道交通是预期域，但不是本 ADR 冻结的 production enum。
`ParticipantClass` 只承担静态准入分类，与执行域正交；声明 `pedestrian` 或
`nonMotor` class 不代表对应运行时行为已经实现。

产品与性能声明必须按执行域 `d` 分别报告：

- 个体交通参与单元数 `N_individual[d]`；
- 活动交通参与单元数 `N_active[d]`；
- 意图更新参与单元数 `N_intent[d]`；
- 表现交通参与单元数 `N_presented[d]`；
- 聚合交通记录数 `N_aggregate_records[d]`；
- 聚合等价参与单元数 `N_aggregate_equivalent[d]`。

聚合记录数衡量实际计算和内存工作量，聚合等价参与单元数衡量保真度覆盖；两者不得
互相代替。没有逐域分解的“城市交通参与者总数”不能作为性能比较、产品通过或容量
承诺。当前 Core 与 `LF-SYNTH-v1`、LuST 等既有 workload 只提供车辆特化证据，必须
显式标注为道路机动车执行域，不能据此宣称非机动车、行人或轨道交通能力。

城市模拟游戏目标不等于“每个居民始终以一个完整微观参与单元进入每个 tick”。默认
研究顺序继续是：

1. 单线程精确、数据导向和批处理；
2. 保持个体身份的单机并行；
3. 通过明确保真度契约（Fidelity Contract）的降频微观；
4. 只有产品证据证明上述路径不足时，才进入聚合/精确迁移
   （Aggregate/Exact Migration）。

多世界（Multi-world）共享静态镜像是测试、回放、参数探索和并行场景的重要轴，但
不能替代单个大型城市世界的运行时扩展。

### 5. 静态表示为不可变路网修订，而非永不变化的城市

“静态路网”表示一个已编译路网修订（Network Revision）在其生命周期内不可变，
不表示城市道路从游戏开始到结束永不改变。

路网修订以路网修订标识（Network Revision ID）`NetworkRevisionId` 认证：它是
independent validator 从 portable artifact 的目标无关规范路网语义载荷（Canonical
Network Semantic Payload）独立重算的版本化不透明摘要，不是调用方编号，也不等同
于完整 artifact/image bytes digest。
Validation receipt、`StaticImageDescriptor` 与切换描述符必须分别绑定该标识；
Runtime 当前修订只来自 `TrustedStaticImage`。相同语义的不同 target/profile image
共享修订，布局或工具 provenance 变化不制造伪修订。

玩家或工具修改权威来源模块后：

1. 增量编译器生成新的可移植规范制品、目标静态镜像、源映射和语义差异；
2. 新修订独立完成验证与信任绑定；
3. 运行中的世界只在显式安全边界执行镜像切换事务（Image Cutover Transaction）；
4. 未变化实体通过旧/新可信镜像的稳定身份索引（Static Identity Index）重建映射；
   该索引只复核身份对应，不能证明相同稳定身份的语义兼容；语义差异只有在外部切换
   描述符绑定并经独立验证后才能驱动迁移，缺失证据时不能采用仅凭索引的回退；
5. 被删除、重接或语义改变的网络元素上的交通参与单元、动态通行定义、停驻、预约
   和控制器状态按版本化迁移策略处理；无法证明完整迁移时切换失败关闭。

镜像不得原地修改。默认在线切换按准备（Prepare）→增量追赶（Delta Catch-up）→
静默提交（Quiescent Commit）→回收（Retire）分阶段：

1. 旧世界在准备期间继续固定步进；Runtime 在基准提交边界捕获只读状态，在后台完成
   新镜像验证、分配、稳定身份映射和结构性迁移。候选不得独立模拟未来固定步进、
   接收新游戏命令或发出事件；迁移策略要求的终止/重映射等切换事件只能形成未提交
   候选批次；
2. 旧世界的每次原子提交把规范的已提交动态状态变化、生命周期变化和命令/事件游标
   写入有界迁移增量日志。候选只按受信任迁移策略重解释这条已提交变更流，不重新
   执行输入命令，也不发布第二份行为结果或重复旧世界事件；
3. 候选落后量进入提交预算后，Runtime 在下一固定步进读取输入之前短暂静默旧世界，
   排空日志尾，并证明候选状态与切换事件批次等价于“把确定性迁移函数应用到旧世界
   最新已提交状态”的结果；复核修订、稳定身份、全部动态引用、命令/事件游标和状态
   摘要后，把镜像/状态绑定与规范排序的切换事件批次作为同一原子提交只发布一次；
4. 旧镜像在全部借用快照、姿态批次和适配器 token 退出后回收。

无法追上、日志溢出、迁移失败或预算超限时放弃候选，不发布任何切换事件，世界继续
绑定旧修订且不暴露半迁移状态。运行时世界在任一固定步进内只绑定一个可信路网修订。
宿主只有在显式维护暂停模式（Paused Maintenance Mode）中才可暂停整个准备期；该
模式的完整停顿必须独立预算，不能算作在线静默提交停顿。

短期施工、事故封闭、动态车道用途或法规时窗若不改变静态身份/拓扑，应由显式
运行时覆盖层（Runtime Overlay）或命令承担；结构性道路编辑进入新路网修订。
两者的划分必须由后续领域 G1 冻结，不能由 Adapter 或编辑器私自选择。

### 6. 每世界唯一性、存档和回放独立于共享镜像

共享静态镜像不得包含或复制：

- world/session identity 或 nonce；
- world seed、伪随机数生成器状态或随机流 cursor；
- 交通参与单元、控制器时钟、预约、占用和动态通行定义；
- 存档时间、游戏规则状态或宿主 Entity；
- 工作线程/分区分配（Worker/Partition Assignment）与动态负载状态。

运行时快照（Runtime Snapshot）是独立版本化制品。精确恢复至少绑定：

- `canonicalArtifactDigest`
- `canonicalArtifactByteLength`
- `originStaticImageDigest`
- `originStaticImageByteLength`
- `runtimeSnapshotVersion`
- 交通运行时版本和约束版本
- `networkRevisionDerivationVersion + networkRevision`
- world identity、tick/time、输入命令序列游标
- 仅在后续 G1 显式授予 Traffic Runtime 随机权威时，才包含运行时自有随机流状态
- 全部每世界可变交通状态

`canonicalArtifactDigest + canonicalArtifactByteLength` 与路网修订是同修订恢复
的静态语义权威；`originStaticImageDigest + originStaticImageByteLength` 记录创建
快照时的精确 target/profile image，供审计和同镜像快速恢复。恢复可以改用绑定相同
规范制品、路网修订和 identity/constraint versions 的另一可信 image，但必须通过其
生产必需的 `StaticIdentityIndex` 重建全部稳定静态引用；否则失败关闭。
快照中的修订 token 只能从当前 `TrustedStaticImage` descriptor 复制，恢复时也只与
候选可信 descriptor 比较；Save Manifest、调用方参数或 image header 不能覆盖它。

跨路网修订恢复必须显式执行快照迁移，并使用旧/新 `StaticIdentityIndex` 与受信任
语义差异；不能把旧 dense ordinal 直接解释为新镜像实体。参与迁移的语义差异必须
由外部 `NetworkRevisionCutoverDescriptor` 绑定 base/target 路网修订标识、制品与
镜像各自摘要/精确长度、`semanticDiffDigest + semanticDiffByteLength`、migration
policy version 和 `validationReceiptDigest + validationReceiptByteLength`；裸
compiler diff 只能用于诊断。

动态通行定义、交通参与单元和其他运行时实体必须使用快照局部标识
（Snapshot-local Identity）保存引用关系；当前动态 Route 或未来各执行域的等价
通行定义同时保存可重建的稳定静态实体引用/规范定义。原进程的
运行时句柄（Runtime Handle）、槽位（Slot）、代次（Generation）、分区或工作线程
分配均不得持久化为恢复后的身份。

运行时执行计划在加载快照后依据当前硬件与负载重建；快照可以保留诊断性调度统计，
但不得要求恢复原工作线程/分区布局才能得到相同精确结果。

ADR 0016 的 caller-owned seed 权威继续有效：人口、出行、Routing 和游戏规则使用的
seed/随机流状态保存在城市游戏或出行编排层的存档中。上层存档清单（Save
Manifest）应原子绑定这些状态、输入命令序列与 LaneFlow 运行时快照摘要；Traffic
Runtime 不为方便存档而新增隐藏随机数。

回放工具链以显式输入命令序列（Input Command Sequence）、周期检查点
（Checkpoint）和确定性状态摘要为基础。调试构建应能借助冷标识/源映射生成失同步诊断制品
（Desynchronization Diagnostic Artifact），定位首个分歧 tick、phase、实体和资源
组件；具体摘要算法、采样频率和发布构建裁剪由后续运行时 G1 冻结。

### 7. 过载优雅降级（Graceful Overload Degradation）不得静默改变交通语义

宿主可以暂停、快进、慢放或统一降低模拟时间推进速度，但不得因帧预算不足而静默：

- 丢 fixed tick 或中间事件；
- 让不同分区读取不同逻辑时点；
- 由 Presentation LOD 改变交通安全或通行权；
- 让工作线程数、任务完成顺序或壁钟时间改变已提交状态。

降低交通保真度（Traffic Fidelity）、引入多频率或聚合（Aggregate）层级必须满足
独立保真度契约（Fidelity Contract）和 G1/ADR；它不能伪装成 Adapter 侧的性能
开关。

## 与既有 ADR 的关系

- 本 ADR **扩展但不取代** ADR 0001：LaneFlow 仍不拥有城市经济、市民出行需求和
  专业交通工程仿真；新增的是城市模拟游戏交通基础这一第一长期产品目标。
- 本 ADR **扩展但不取代** ADR 0003：当前确定性范围不变；工作线程/分区数无关和
  跨平台位级确定性仍由后续并行/数值 ADR 冻结。
- 本 ADR **复用** ADR 0016 的 caller-owned population/seed 权威；城市游戏可以
  替换 reference scenario policy，交通运行时不隐藏人口模型或随机数。
- 本 ADR 与 ADR 0020 配套：ADR 0020 冻结静态编译和镜像边界，本 ADR 冻结这些
  能力服务的产品目标、动态路网修订和上层职责。

## 后果

正向后果：

- “不是城市经济模拟器”不再被误读为“不服务城市级交通”；
- 编译器、运行时、路径规划、城市游戏和 Adapter 之间的长期权威边界清晰；
- 不可变镜像可以同时服务性能、玩家改路、存档、回放和不可信 Mod 内容；
- 中国特色交通通过正式数据/政策/工作负载演进，不污染通用运行时；
- 多世界与单世界扩展各自拥有明确价值，不互相替代。

成本与风险：

- 路网修订切换、运行时快照、路径规划、并行执行和中国特色城市工作负载都需要后续
  独立 G1 与实现 Issue；
- 镜像切换准备期同时保留旧/新镜像与候选世界状态，会产生可量化的峰值内存；迁移
  日志可能增长、后台追赶可能干扰正常固定步进，密集改路时也可能持续无法追上；
  后继门禁必须冻结后台预算、日志上限、最大追赶落后量、在线静默提交停顿、失败
  放弃、切换事件批次“放弃零发布/提交恰一次”、延迟回收与显式维护暂停的完整停顿
  预算；
- 城市游戏存档必须量化运行时快照保存/加载的制品大小、墙钟耗时、主线程停顿、后台
  固定步进干扰和峰值内存，不能只验证功能等价；交通观测完整/增量导出和动态成本
  快照接收/候选注册也必须量化边界成本；
- 稳定标识与语义差异将参与运行时迁移，错误边界必须比只做离线治理更严格；
- 全部支持存档或玩家改路的生产镜像都要保留共享冷身份索引；后继 Gate 必须量化其
  retained memory、按需映射和双向 lookup 成本；
- 城市游戏上层和交通编排层若缺少正式接口，仍可能通过 Adapter 或 scenario policy
  形成隐藏耦合；
- 不能再只用背景车流示例或多世界吞吐量证明城市游戏产品目标。

## 被拒绝的替代方案

### 把城市经济和出行需求纳入交通运行时

这会让交通安全、人口经济和游戏规则形成一个无法独立测试、复用或嵌入的单体；
拒绝。

### 因当前 Milestone 只覆盖局部场景而取消城市级北极星

当前交付范围与长期第一产品目标是不同时间尺度。局部走廊仍是正确的渐进验证路径，
但不能反向定义终态产品；拒绝。

### 把道路修改实现为静态镜像原地修改（In-place Mutation）

这会破坏共享、摘要、信任、零拷贝视图和确定性；采用不可变修订加事务切换。

### 把最终分区（Partition）、工作线程（Worker）和随机种子烘焙进共享镜像

它会让同一地图随硬件/世界实例产生错误耦合，并复制唯一性状态；共享镜像只保存
静态事实、执行约束和可重建性能提示。

### 用多世界集合（Ensemble）吞吐代替单个城市世界扩展

两者服务不同产品路径。多世界不能证明一个大型城市世界的屏障（Barrier）、边界
邻域（Halo）、内存和动态负载均衡；拒绝。

## #291 G1 再修订接受条件

1. README、architecture、roadmap、Agent Skills 和双语术语表明确记录城市模拟游戏
   交通基础的第一长期产品目标；
2. ADR 0020 与 `network-compiler.md` 冻结静态执行约束、分区规划提示和每世界
   运行时执行计划的职责分离；
3. 不可变路网修订、在线准备/增量追赶/静默提交的镜像切换事务、运行时快照、
   每世界唯一性和路径规划边界进入长期设计；
4. 固定边界邻域（Halo）、额外一 tick 边界延迟、全局单线程归约器（Reducer）、
   最终分区（Partition）烘焙和过早数值方案均未被写成生产既定事实；
5. 后继并行、路网修订/存档、路径规划和中国特色城市工作负载由独立 G1/Issue
   承载；
6. 本地一致性验证通过，并取得当前 exact head 的外部 clean review。
