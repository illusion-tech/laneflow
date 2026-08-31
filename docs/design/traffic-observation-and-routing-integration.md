# 交通观测与 Routing 接入

**文档状态**: Accepted（#303 G1 Pass）<br>
**最后更新**: 2026-08-31（#541 parking lifecycle 接缝）<br>
**适用范围**: 已提交交通观测的 full/delta/partition 导出、动态成本绑定、候选路线注册、过期语义、#302 切换/快照交互与独立性能门禁<br>
**关联文档**:
[`../adr/0020-compiler-owned-static-network-and-static-image.md`](../adr/0020-compiler-owned-static-network-and-static-image.md)、
[`../adr/0021-city-simulation-game-traffic-foundation.md`](../adr/0021-city-simulation-game-traffic-foundation.md)、
[`../adr/0029-retire-precompiled-static-route.md`](../adr/0029-retire-precompiled-static-route.md)、
[`traffic-runtime-shared-consumption.md`](traffic-runtime-shared-consumption.md)、
[`traffic-runtime-revision-cutover.md`](traffic-runtime-revision-cutover.md)、
[`traffic-runtime-snapshot.md`](traffic-runtime-snapshot.md)

本文是 #303 已接受的 G1 实现输入。Rust 名称、错误枚举与具体容器布局由 G2 落定；
G2 不得静默改变本文的权威分层、绑定集、时点、失败关闭
和预算口径，发现需要改变时必须返回 G1。

1.0 前的“冻结”只表示当前证据下供实现消费的唯一权威，不是不可推翻的铁律，也不是
对外兼容承诺。实现证据、可复现的真实工作负载、产品现实或 #302/#303 接缝若证明本文
错误、过度设计或走了弯路，应先修改本 design/相关 ADR，并在新的 exact head 重新取得
G1；不得为了维护旧文字堆兼容层、双轨或错误实现。

## 1. 产品选择与权威边界

v1 选择**宿主自有 Routing 实现 + LaneFlow 纯契约边界**：

- 不新增 reference `laneflow-routing` 算法 crate；当前仓库没有已经验证的全图算法、
  OD/交通分配产品范围、远程服务部署需求或对应 SLA，不能凭接口想象制造新产品。
- 不新增 LaneFlow 持久化或发布格式。观测批次和候选输入是同进程版本化类型；宿主若
  把 Routing 放到进程外，自行拥有传输格式、认证、重试和兼容，不能把宿主 wire
  宣称为 LaneFlow 制品。LFCA/LFSM/LFSD/LFCP 与 Runtime Snapshot 的格式集合不变。
- `laneflow-runtime` 新增两类边界：从当前已提交状态按请求导出观测；把带完整来源
  绑定的候选边稳定标识序列降低到现有唯一 `register_route` 编译器。
- 动态成本快照及其成本条目载荷由 Routing/出行编排拥有。Runtime 不接收、不保存、
  不解释成本条目；只接收由候选复制的成本快照绑定，以执行修订/时点/过期核对。
- Spatial、Engine Adapter 与 scenario catalog 不拥有观测聚合、成本模型或路线选择。
  Adapter 可以薄转发生命周期命令，但不得在 ECS schedule、位姿提取或表现层隐藏寻路。

四层命令/快照/事件权威如下：

| 层级                        | 拥有                                               | 不拥有                              |
| --------------------------- | -------------------------------------------------- | ----------------------------------- |
| 城市游戏层                  | 经济、建筑、人口、收费与游戏规则                   | Traffic tick、候选路线内容验证      |
| 出行与交通编排层            | 出行需求、出发时刻、目的地、路线选择、调用方随机流 | 静态路网或车辆推进权威              |
| Routing service（宿主实现） | 观测消费、成本模型、动态成本快照、候选边序列       | 参与单元 fixed tick、Runtime 路线表 |
| Traffic Runtime             | 已提交观测事实、候选内容验证/注册、路线与车辆执行  | 全图寻路、成本政策、需求与选择策略  |

`register_route` 仍是唯一出现项编译器：场景 catalog 可继续把同修订共享根边序号
直接交给它；Routing 候选入口只是在它之前增加稳定标识解析和来源绑定校验，不得
复制第二套连通、机动、门、等待区或准入编译器。

## 2. 已提交交通观测

### 2.1 一致性时点与 v1 行形状

导出只在两次 `step` 之间读取一个精确的已提交边界。它不得观察 snapshot(T) 到 T+D
之间的 `next_states`、部分占用重建或中间信号值。导出成功不推进 `tick_index`、
`time_ms`、命令游标、事件游标或下述观测状态序号。

`tick_index` 不是已提交状态的完整版本：`spawn_vehicle`、`replace_completed_vehicle`、
`park_vehicle` / `leave_parking` / `despawn_vehicle` 等生命周期命令可在同一 tick 内改变
观测事实。因此每个活动世界还
维护一个只在当前世界世代/观测 stream 内单调递增的
`ObservationStateSequence(u64)` / `observationStateSequence`：成功 `step` 以及每个会改变 v1 观测行的成功生命周期
提交各推进一次；失败、只读查询、导出和不影响 v1 行的路线表变更不推进。安装、
恢复或成功切换建立新 stream 时从该 stream 的初始值重新开始，旧 stream 的序号
不得跨世界世代比较。初值为 `0`，推进使用 checked `+1`；耗尽时本应改变观测行的
命令/step 失败且已提交世界不变，不得用饱和、回绕或仅看 tick 代替严格单调语义。

v1 每条物理 `LaneEdge` 行冻结为：

| 字段                       | 语义                                                                                         |
| -------------------------- | -------------------------------------------------------------------------------------------- |
| `laneEdgeStableId`         | 当前共享修订中 `EntityKind::LaneEdge` 的 `StableId128`                                       |
| `frontVehicleCount`        | 前保险杠当前落在该边上的 `Active` 车辆数；Parked/Completed 不计                              |
| `occupiedLengthMm`         | 所有 `Active` 车身在该物理边上已提交半开占用区间的并集长度；跨边片段分别计，同边重叠不重复计 |
| `frontSpeedSumMmPerSecond` | 上述前保险杠归属车辆的已提交 `speed_mm_s` 之和                                               |

前保险杠的边归属跟随车辆权威路线 occurrence，而不是另做几何猜测：当前
`route_edge_index` 指向哪条边，`frontVehicleCount` 与 `frontSpeedSumMmPerSecond`
就归哪条边。hop 被拒且 `progress_mm == edge_length` 时仍归旧边；hop 成功后
`route_edge_index` 已切到下一边且 `progress_mm == 0`，归新边；Active 车辆到达最后
一边端点后，在生命周期明确把它变为 Completed 前仍归最后一边。

三项数值分别用 checked `u32/u64/u64` 累加；溢出整批失败且不推进导出 session。
Routing 用边长与这些整数自行派生密度、平均速度或成本，不把浮点除法、舍入或成本
政策倒灌进 Runtime。当前 `TrafficWorld` 没有动态封闭/overlay 权威，因此 v1 不伪造
`closed` 行；#237 或后继若生产化 runtime overlay，须先回到 G1 扩展观测行。

三项聚合必须从同一个 `observationStateSequence` 的已提交 `VehicleState`、路线和
占用区间求值。当前步进求解用的私有 `OccupancyIndex` 在 snapshot(T) 上重建后才提交
T+D 状态，不能直接当作 T+D 观测来源；G2 可以重算，或重构成带精确状态序号且经
oracle 证明一致的共享聚合，但不能混用当前车辆状态与前一提交边界的占用缓存。

完整基线对所选边**逐边出行，包含全零行**，按 `laneEdgeStableId` 升序规范排列。
增量也按同序排列；值从非零变零时必须显式发全零行，不使用删除或 tombstone 语义。

每批头至少携带：封闭 `bindingVersion`、#302 世界身份/世界世代组成的
`ObservationStreamBinding`、
`NetworkRevisionId/networkRevisionDerivationVersion`、`selectionDigest`、full/delta kind、
base/current delivery sequence、tick 与
`observationStateSequence`、精确 `entryCount`。
批次另报告 `logicalBytes`（`size_of::<CommittedTrafficObservationBatch>() +
rows.len() * size_of::<CommittedTrafficObservationRow>()`）和 `retainedBytes`（同式把
`len` 换成实际 `capacity`）；session 对自身结构和 selection / ordinal map / baseline
三组缓冲按同一规则报告。它们是当前实现版本的精确资源观测，不是跨语言编码或稳定
ABI；所有乘加 checked，不能把 logical rows 冒充 retained capacity。

### 2.2 分区选择

观测分区是调用方声明的逻辑选择，不是 `RuntimeExecutionPlan` 的 worker/partition：

- `AllLaneEdges`；或
- 当前修订下按 `StableId128` 升序、无重复、非空的显式 LaneEdge 集合。

创建导出 session 时，Runtime 用 `SharedIdentityIndex` 一次性解析并验证选择，计算带
域分隔版本的 SHA-256 `selectionDigest`。G2 摘要输入精确为
`"laneflow:runtime-observation-selection:v1\0" || entryCount:u64-le ||
laneEdgeStableId[0..N]:raw-16-bytes`；`AllLaneEdges` 与内容完全相同的显式选择得到相同
摘要，不把选择表达方式伪装成内容差异。未知标识、错误实体 kind、重复或不规范排序
失败关闭，不留下 session。选择在 session 生命周期内不可变；调用方要换分区必须
新建 session 并先取完整基线。禁止把执行计划分区编号、dense ordinal 或几何包围盒
解释规则写进该契约。

### 2.3 导出 session、full 与 delta

语义 API（不承诺最终 Rust 拼写）：

```rust
TrafficWorld::open_observation_export(
    selection: ObservationSelection,
) -> Result<ObservationExportSession, ObservationError>;

TrafficWorld::export_observation(
    session: &mut ObservationExportSession,
    mode: FullOrDelta,
) -> Result<CommittedTrafficObservationBatch, ObservationError>;
```

session 是调用方持有、Runtime 签发且字段私有、无公共构造器的
`ObservationExportSession`，绑定：#302 同一世界身份/世代、当前
`NetworkRevisionId` / `networkRevisionDerivationVersion`、`selectionDigest`、观测
stream 身份、上一成功 delivery sequence / `tick` / `observationStateSequence` 与
上一行值。它不进入世界确定性状态。

G2 不另设会与共同世代漂移的 stream 计数器：每个活动世界世代只有一个观测 stream，
字段私有的 `ObservationStreamBinding { world_id: u64, world_generation:
WorldGeneration }` 就是唯一 stream 身份。`ObservationExportMode::{Full, Delta}` 进入同一
`export_observation` 入口；新 session 首次 delta、旧世代/revision session、delivery
sequence 耗尽、聚合/资源算术溢出和分配失败全部失败关闭且不推进 session。

- 新 session 第一次只允许 `Full`。成功批次 `sequence = 0`，并成为下一增量的
  精确基线；full 的 base 字段必须缺失，不用零值伪造 base。
- `Delta` 只与该 session 上一次**成功**批次比较；批次头携带
  `(baseSequence, baseTick, baseObservationStateSequence, sequence, tick,
  observationStateSequence)`。消费者只能严格连续应用，缺批、重排、重复或 base
  不匹配必须丢弃局部结果并重新取 full。
- 调用方可随时显式 `Full` 重置基线；新的 full 递增 sequence，不复用旧基线身份。
- 同一已提交 tick 可重复导出：若中间没有观测相关提交，连续 delta 可以是零行，
  `observationStateSequence` 不变但 delivery sequence 仍唯一递增；若同 tick 内发生
  生命周期提交，则状态序号必须递增并按新状态计算变化行。批次
  `(tick, observationStateSequence)` 不得早于 base。
- 输出分配/容量、checked 算术或任何行构造失败时，不返回部分批次、不更新 session。

观测导出节奏完全由宿主调用决定。Runtime 不按墙钟自动调度，不为“以后可能导出”
在每个 tick 维护全网副本或变更日志。full/delta 都在显式导出调用内从一个已提交状态
求值；delta 减少跨边界条目与字节，不承诺免除当前状态扫描。多个消费者各持独立
session，不能共享并竞争一个隐式全局游标。

## 3. 动态成本快照

动态成本快照是 Routing service 拥有的不可变对象。LaneFlow 冻结其候选来源绑定，
不冻结成本条目 payload、算法或宿主 wire。每份快照至少提供：

| 字段                                                     | 契约                                                                                                                |
| -------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------- |
| `bindingVersion`                                         | 本绑定封闭版本；未知值拒绝                                                                                          |
| `worldIdentity` / `worldGeneration`                      | 与观测 session 相同的 #302 `worldBinding` 世界身份及活动聚合世代；不复制其基线命令/事件游标                         |
| `networkRevisionId` / `networkRevisionDerivationVersion` | 从所消费观测原样复制                                                                                                |
| `observationTick` / `observationStateSequence`           | 形成成本的共同已提交 tick 与该 tick 内精确观测状态序号；多分区输入必须同世界、同世代、同修订、同 tick、同状态序号   |
| `observationSetDigest`                                   | 按 stream/delivery sequence/selection digest 规范排列后形成的观测输入集合摘要；不能把不同状态序号的分区拼成一份成本 |
| `costModelId` / `costModelVersion`                       | 宿主拥有的不透明模型身份与封闭版本，只做精确相等比较                                                                |
| `validThroughTick`                                       | 最后允许候选注册的已提交 tick（含）；必须 `>= observationTick`                                                      |
| `entryCount`                                             | Routing 接收并验证的精确成本条目数                                                                                  |
| `exactByteLength` / `snapshotSha256`                     | 宿主成本 payload 的精确字节数；摘要以域分隔版本前缀覆盖除自身外的上述绑定字段及 exact payload                       |

G2 将宿主不透明成本模型键落定为
`CostModelKey { modelId: Sha256Digest, modelVersion: u32 }`；`modelId` 是宿主所选模型
身份的定长摘要值，不证明算法来源或可信性。形成成本前，宿主通过
`bind_observation_set` 把一个或多个实际成功的观测批次降低为字段私有的
`ObservationSetBinding`。该入口逐项拒绝不同 stream、修订/派生版本、tick 或状态序号，
再按 `(worldId, worldGeneration, deliverySequence, selectionDigest)` 升序规范化；完全相同
的输入绑定重复出现也拒绝。`observationSetDigest` 的精确输入为：

```text
"laneflow:runtime-observation-set:v1\0"
|| inputCount:u64-le
|| repeated(
     worldId:u64-le
     || worldGeneration:u64-le
     || deliverySequence:u64-le
     || selectionDigest:32 raw bytes
   )
```

`snapshotSha256` 的 G2 精确输入为：

```text
"laneflow:dynamic-cost-snapshot:v1\0"
|| bindingVersion:u16-le
|| worldId:u64-le
|| worldGeneration:u64-le
|| networkRevisionId:32 raw bytes
|| networkRevisionDerivationVersion:u16-le
|| observationTick:u64-le
|| observationStateSequence:u64-le
|| observationSetDigest:32 raw bytes
|| costModelId:32 raw bytes
|| costModelVersion:u32-le
|| validThroughTick:u64-le
|| entryCount:u64-le
|| exactByteLength:u64-le
|| exact payload bytes
```

这些摘要规则是同进程 contract 的规范输入，不新增 LaneFlow wire 或发布格式。

过期策略只使用 fixed tick：候选在 `currentTick < observationTick` 时是“来自未来”，
在 `currentTick > validThroughTick` 时过期；两者都失败关闭。在同一活动世界世代/
观测 stream 内，无论两个 tick 是否相等，只要当前 `observationStateSequence <` 绑定值，
该组合就不可能已被导出，必须按“来自未来或损坏来源”拒绝。状态序号证明成本的
多分区输入来自同一精确状态，不另发明 `validThroughStateSequence`；形成成本后的
同 tick 生命周期提交不会绕过或缩短宿主显式选择的 fixed-tick 有效窗。没有墙钟、
默认宽限、自动刷新或“版本较新即可”语义。`validThroughTick` 由成本模型/出行编排
在构造快照时显式决定并被摘要绑定，Runtime 不暗设产品政策。

`snapshotSha256` 证明候选引用的是哪份绑定 + payload，不授予信任或迁移权限。Routing
receiver 必须先核对调用方容量上限与 exact bytes，再解析/分配/哈希，并验证实际
条目数；Runtime 不重复解析成本 payload。调用方本就拥有路线选择权，伪造成本只能
产出一个仍须通过 Runtime 完整内容验证的候选，不能绕过静态安全规则。
候选是否确由该 payload 算出、候选边是否被成本条目覆盖，由 Routing receiver 在
产出候选前证明；Runtime 不复刻成本解析/算法来做第二预言机。

## 4. 候选路线注册

Routing 候选入口的语义输入为：

```rust
struct CandidateRouteInput {
    cost_snapshot: DynamicCostSnapshotBinding,
    lane_edges: Box<[StableId128]>,
}

TrafficWorld::open_routing_admission(
    cost_model: CostModelKey,
) -> RoutingAdmissionSession;

TrafficWorld::register_candidate_route(
    admission: &RoutingAdmissionSession,
    input: CandidateRouteInput,
) -> Result<RouteHandle, CandidateRouteError>;
```

G2 将 `open_routing_admission` 落为不可失败：它只复制当前世界/世代/修订与定长模型键，
不分配、不解析，也不在 Runtime 留隐式 session 状态；为不存在的失败原因保留 `Result`
只会形成空壳错误合同。

`RoutingAdmissionSession` 是 Runtime 签发、调用方持有的不可伪造能力，绑定当前
世界身份/世代、修订和调用方显式选择的 `(costModelId, costModelVersion)`；它不保存成本
payload，也不进入 Runtime Snapshot。模型升级必须开新 session，不存在“较新版本兼容”。

Runtime 按下列顺序失败关闭，任一失败不占路线槽、不留下 compiled occurrence：

1. O(1) 预检空序列、候选边数和 checked 大小；在解析/compiled 分配前按下文统一
   `route_edge_occurrence_capacity` 核对本次边出现项数。候选入口可以提早返回同一错误，
   但最终权威检查必须在 direct/candidate/cutover/restore/replay 共用的注册/编译路径
   内完成。
2. 核对 `bindingVersion`、世界身份/世代、修订标识/派生版本、观测
   tick/state sequence/set digest、条目数/bytes/digest 字段完整且自洽；成本模型
   身份/版本必须与 admission session 精确相等。
3. 按 §3 比较当前已提交 tick 与 `[observationTick, validThroughTick]`；在同一活动
   世界世代/stream 内，无论 tick 是否相等，都拒绝大于当前值的
   `observationStateSequence`。
4. 用当前根 `SharedIdentityIndex` 把每个 `LaneEdge StableId128` 解析成当前 dense ordinal；
   未知标识或错误 kind 拒绝。修订相等不能跳过这一步。
5. 把解析后的有序序列交给现有唯一 `compile_route` / `register_route` 路径，重做连通、
   机动 occurrence、门/等待区、端点与溢出不变量。

重复边允许，语义与 ADR 0029 的 `register_route` / snapshot 路线序列一致。候选成本
payload 不进路线表。注册成功后，候选成为普通每世界 `Route`：成本过期不会让已注册
路线或行驶中车辆突然失效；未使用路线由调用方显式 `remove_route`。新的 spawn 仍按
现行 `(ParticipantClass, Route)` 做后缀准入，不把成本模型当安全准入。

同修订场景 catalog 的 `register_route(LaneEdgeOrdinal...)` 继续存在，服务已绑定同一
根的本地数据；它不能接收 `StableId128`、revision/tick/model 自报字段，也不能被
包装成隐藏 Routing。跨观测/成本边界的候选必须走本节严格入口；两类入口最终消费
同一条路线容量和编译合同。

### 4.1 统一路线边出现项容量

当前唯一 `compile_route` 会按输入长度物化边序列、后缀距离、分段坐标、hop、下一
受控转换等多组 O(n) 热表。候选入口若独有边数上限，direct `register_route`、恢复或
回放就能绕过同一资源合同；物理 LaneEdge 数也不能作为上限，因为路线允许重复边。

#303 G1 已接受合同因此给 `WorldConfig` 增加语义容量
`route_edge_occurrence_capacity`：它统计本世界全部**存活动态路线**有序边序列的
`edges.len()` 总和，每个重复 occurrence 都计一次。`register_route` 的唯一共享路径在
任何 compiled 分配前，以 checked 算术核对「当前已占用 + 本次输入」；direct、
candidate、cutover target 重绑、snapshot restore 与 replay 一视同仁。`remove_route`
成功后释放对应计数；注册失败、移除失败和切换放弃都不改变计数。恢复时快照路线总
occurrence 超过恢复配置则整次恢复失败关闭。

该容量不是共享根物理边数，不是单条路线的产品长度政策，也不恢复已删除静态路线
出现项的 `1920` 历史预算。单条路线只要不超过本世界剩余 occurrence 容量，仍可合法
重复边；G2 必须在同一共享路径实现 max/max+1、checked 溢出与分配失败零部分提交，
并以具名 Routing 工作负载登记配置值和 retained memory 证据。

`route_edge_occurrence_capacity` 的语义保持为边 occurrence，不能拿它隐式支付一条
边可能展开出的多个冲突通行段。#283 的
[`traffic-runtime-conflict-occurrence.md`](traffic-runtime-conflict-occurrence.md) 另设
`route_conflict_occurrence_capacity`；两个计数由同一注册提交点原子维护。

**G2 首切片实现决定**：`WorldConfig::new` 在 `route_capacity` 之后要求调用方显式提供
`u64 route_edge_occurrence_capacity`，不设产品默认值；活动总量计数器同为 `u64`。
空序列、路线槽容量、边 occurrence 容量依次预检；后两者分别返回
`RouteError::CapacityExceeded` 与 `RouteError::EdgeOccurrenceCapacityExceeded`，checked
加法不可表示与 max+1 共享后一个错误。仓库既有测试/证据夹具迁移时显式使用 `1_024`
只表示该 fixture 的配置 provenance，不是产品默认值或单条路线限制。G2 第二
切片已将 compiled-route 的所有已知长度热表改为 `try_reserve_exact` 预留；任一预留失败
统一返回 `RouteError::AllocationFailed`，并由确定性失败注入覆盖「首次预留」与「部分热表
已物化」两个边界；两者都不提交路线槽、活动路线数、边 occurrence 计数或空闲表。
同修订 cutover 对 target 根重编译路线时复用同一 `compile_route`；其热表预留失败映射为
`CutoverError::StagingAllocFailed`，并以确定性失败注入验证旧根、来源、路线、车辆与
占用保持可继续步进的原子放弃状态。

## 5. 与 #302 切换、快照和回放的接缝

- **切换准备期**：对外观测只来自仍活动的旧聚合。候选世界不可见，不导出“新修订
  预览观测”。在旧世界成功注册的候选路线已经成为路线生命周期变更，必须进入 #302
  迁移增量日志并在 target 根重绑/重验证；失败则整个切换失败关闭。
- **切换提交**：成功原子切换后，世界世代递增、观测 stream 与其
  `observationStateSequence` 同界重建；旧观测/admission session、未注册候选及旧修订
  成本绑定全部 stale。调用方须在新修订开新 session 并先取 full。已经注册的
  `RouteHandle` 按 #302 逻辑恒等迁移保持有效，不因成本来源过期被撤销。
- **切换放弃**：旧 session 与旧修订候选继续按原 tick 窗口工作；不得仅因出现过候选
  根而换 stream 身份。
- **保存/恢复**：观测/admission session/基线、动态成本快照、未注册候选和成本 provenance 都是
  调用方/派生交付状态，不进 Runtime Snapshot。已注册路线只按 #302/ADR 0029 的
  快照局部 ID + 边稳定标识保存。任何恢复（含同修订）建立新世界世代/观测 stream
  并从新 stream 的初始 `observationStateSequence` 开始；恢复前的 session 与候选不得
  复活。成功切换递增世界世代，放弃不递增。
- **回放**：观测导出不是输入命令，不改变确定性状态摘要；
  `register_candidate_route` 是带观测/成本/admission provenance 的派生准入请求，
  **不是**耐久回放命令。候选成功注册后，宿主命令序列只记录规范化的已准入路线
  注册命令：宿主自有耐久路线 ID、当时的
  `networkRevisionId/networkRevisionDerivationVersion` 与有序 LaneEdge `StableId128`
  序列。它不记录世界世代、观测 stream/tick/state sequence、admission session、成本
  模型或成本摘要。检查点后重放时，宿主先核对当前修订绑定，Runtime 再以
  `SharedIdentityIndex` 解析稳定标识、消费 §4.1 同一容量并进入唯一
  `compile_route`/`register_route` 路径，把新 `RouteHandle` 回映到耐久调用方 ID；
  不重新执行 Routing，也不把已 stale 的成本绑定伪重绑到新世代。跨修订恢复若命令
  绑定与当前根不同，只能经 #302 受信任迁移策略显式迁移稳定引用，否则失败关闭。

  G2 的 Runtime 实现接缝是
  `register_admitted_route(AdmittedRouteRegisterInput) -> Result<RouteHandle, ...>`：输入只含
  `networkRevisionId/networkRevisionDerivationVersion` 和有序 LaneEdge `StableId128`；宿主
  耐久路线 ID 留在宿主命令序列，并用返回的新句柄回映。该同进程 Rust 输入没有独立
  wire/version 轴；若宿主持久化命令，由宿主拥有其容器版本。

## 6. Adapter、Spatial 与 scenario 影响

- `laneflow-spatial` 不依赖观测或 Routing 类型；`committed_pose_sources()` 也不升级
  为交通观测，因为它按车辆输出、为位姿采样服务，不具备边聚合/full/delta 语义。
- `laneflow-bevy` 若暴露新入口，只能像现有 `register_route` 一样薄转发；默认 schedule
  不自动导出、不运行成本模型、不在 ECS component 中缓存另一份路线权威。
- scenario catalog 0.3 仍是示例层预定边序列，可直接 `register_route`。若场景需要
  动态改道，必须由上层系统消费正式观测并走候选入口，不能在 scenario policy 中读
  pose 后静默选路。
- Adapter 只消费成功注册后的 `RouteHandle` / 车辆位姿；不得根据 cost digest、观测
  序号或 revision 自行判定路线安全。

## 7. 资源边界与独立 Gate

度量沿 `LF-P100-REF-01` 同机描述性基线；G2 登记首轮数值。1.0 前预算可依据可复现的
实现、工作负载和产品证据收紧、放宽或重构；每次变化必须说明原判据为何不再成立、
更新同一事实源，并在影响实现合同或验收结论时返回 G1，不能在 G2 静默改口径。所有
条目数、exact bytes、累计分配、峰值 retained bytes（或由累计分配导出的同等保守
上界）、墙钟与 tick 间隔干扰同时报告，不能用单个平均耗时代替。

`LF-P100-REF-01` 只冻结参考机器，不是 workload。每个正式 case 必须绑定可重放的
workload ID、source commit、路网/状态/selection digest、车辆规模与 lifecycle mix、
seed、`WorldConfig` provenance、fixed-step 输入序列、导出 cadence、warm-up、样本/round
规则和 candidate/oracle 边界。观测 full/delta 优先复用
[`core-runtime-performance-baseline.md`](core-runtime-performance-baseline.md) 的
`LF-SYNTH-v1` W1–W4；新增 Routing 专用拓扑或成本 payload 时使用新的具名 workload，
不把“典型”“长路线”当作可重放身份。

| 边界                   | 必测切片                                                                    | 必过不变量                                                                         |
| ---------------------- | --------------------------------------------------------------------------- | ---------------------------------------------------------------------------------- |
| observation full       | All + 显式 1%/10%/100% 分区；零车、稀疏、一万/十万车辆；W1–W4 适用切片      | entryCount 等于选择边数（含零行）；失败零 session 推进                             |
| observation delta      | 0%/1%/10%/100% 行变化；同 tick 零行/生命周期提交；值归零                    | exact changed entries/bytes；严格 base/state 链；无导出时 tick 零新增观测工作/分配 |
| dynamic cost receive   | 合法 payload、length/count/digest 各类错配、上限+1、未知版本                | 容量预检先于解析/分配/哈希；不进入 Runtime tick                                    |
| candidate registration | 1/典型/长边序列；重复边；stale/future/revision/model/identity/topology 错配 | 唯一 route 编译器；direct/candidate 共用 occurrence 容量；失败零路线槽/计数变化    |
| cutover/restore        | prepare 中注册、commit、abort、同修订 restore、跨修订 cutover               | session/candidate 失效矩阵与 §5 精确一致；已注册句柄按 #302 保持                   |
| session retained       | 同 selection 的 1/10 个消费者；1%/10%/100% 选择；open/drop                  | 单 session 与总 logical/retained bytes、open/drop 成本分别报告且按消费者数可解释   |

实现级安全上限必须在读取调用方可变长度数据前从 `WorldConfig`/接收端容量合同取得；
至少覆盖 selection rows、输出 rows、成本 `entryCount/exactByteLength`、全部路线入口
共用的 `route_edge_occurrence_capacity` 和所有乘加溢出。共享根物理 LaneEdge 数不得
冒充路线 occurrence 上限。G2 回写配置值、max、max+1 与不可达值证明。

观测导出与候选注册都是 step 之间的显式调用；允许它们延迟下一 tick，但不允许与
一次 tick 交错或产生半提交状态。无导出调用时，#303 不得新增 per-tick 全网复制、
dirty journal、墙钟任务或 allocator 活动。

### 7.1 `LF-ROUTING-G2-LINEAR-v1` 首轮描述性证据

2026-08-28，G2 以 source commit
`67c4cf2e89f7cef66b3ad6f2892fe8a261b6199d` 在当前
`LF-P100-REF-01` 物理机运行首轮具名 Routing case。机器按当前权威
`laneflow-p100-hardware-identity-v2` 复核通过；实际环境为 Windows 11 Insider build
29648、交流电/平衡电源计划、Rust 1.98.0 / LLVM 22.1.8。厂商性能模式未登记，因此
即使数值满足本 case 的描述性观察，也不能形成 Product Pass。

工作负载与绑定冻结为：

| 输入                   | 值                                                                                       |
| ---------------------- | ---------------------------------------------------------------------------------------- |
| topology               | seed 303；4096-edge directed linear chain；每边 100 m、13.9 m/s；Spatial omitted         |
| state                  | 0 vehicles；64 × 4 ms warm-up；AllLaneEdges；state sequence 64                           |
| `WorldConfig`          | vehicle 0；route 1；occurrence 4096；worker 1；fixed delta 4 ms                          |
| cost receiver          | binding v1；4096 × (`StableId128` + `u64-le`)；98304 exact bytes；上限恰为该 count/bytes |
| candidate              | 1 / 128 / 4096 edges；输入构造在测量区外；成功注册后在测量区外移除                       |
| allocation protocol    | 插桩 release 独立进程；steady tick 各 32 samples；墙钟不在该进程采信                     |
| wall-clock protocol    | 未插桩 release；3 个 fresh-process rounds；每操作 3 warm-up + 21 measured samples        |
| certification boundary | `Product TBD / Uncertified`；不是 `LF-SYNTH-v1` W1–W4，也不覆盖真实车辆/产品组合         |

可重放身份为：LFCA exact bytes 1311878，artifact
`b65e84e4813d9298ce9d3c5f70abbb05c0ab24027b81b9b7762ef04a8e7119e4`，
`NetworkRevisionId = 5c55b3a80c544717bc912b7a3a5049cde6e7f4f9cd136be2fd214daa91d75861`，
workload manifest `2fb3ece10d5c20ba0394d52dfd5dfdda137a1d3dc8b71869b50e0896cc75c326`，
state `621c4e55ee2ec4e2587debc8d4b122929bc61ae3e3058bd99f855191a2ab8e82`，
selection `07854493ce2896c4b39df07c426fa8d73e558f16efe19cff2f37199a09afd204`，
observation set `2ca476504489ee1aead1484a3312efba95d0f1f2dac6f9a813f51a3d86da1505`，
dynamic cost snapshot
`e2d09babfe471eedb44b4a4b6fbb00b99e01d61667c54f948cf275e340413c19`。
allocation 与 wall-clock 分别使用独立 fixed-input-sequence digest
`405c07b9bd7f879ad8de57f473dc87e1da8e549a7151a5be99e7107d0503e72d` /
`2ab6edd3852bd6b3c7989b8389f8bbf259e4a4a7b6b18c27a96be05fbbf7ed01`。

分配/retained 账本如下。`peak live upper bound` 是测量区累计 allocated bytes 给出的
保守上界；candidate 的 `adjusted live` 把在测量区外分配、在注册时被消费的 caller
输入 bytes 加回，防止把 route retained 误算成较小值。

| 操作                   | allocation / reallocation | allocated / deallocated bytes | live/retained 结果                              | peak live upper bound |
| ---------------------- | ------------------------- | ----------------------------- | ----------------------------------------------- | --------------------- |
| steady tick before     | 0 / 0                     | 0 / 0                         | live delta 0                                    | 0                     |
| observation full       | 2 / 0                     | 327680 / 163840               | batch 164032；session 278720；live delta 163840 | 327680                |
| observation delta zero | 1 / 0                     | 163840 / 163840               | batch 192；session 278720；live delta 0         | 163840                |
| cost receiver          | 0 / 0                     | 0 / 0                         | live delta 0                                    | 0                     |
| candidate 1 edge       | 8 / 0                     | 40 / 32                       | adjusted live after registration 24             | 40                    |
| candidate 128 edges    | 12 / 0                    | 11216 / 4096                  | adjusted live after registration 9168           | 11216                 |
| candidate 4096 edges   | 12 / 0                    | 360400 / 131072               | adjusted live after registration 294864         | 360400                |
| steady tick after      | 0 / 0                     | 0 / 0                         | live delta 0                                    | 0                     |

墙钟表中 p50/MAD/p95/p99 均先逐 round 计算、再取三个 round-level 值的中位数；
`worst` 是全部 63 个 measured samples 的最大值。单位均为 ns。

| 操作                   | p50 median | MAD median | p95 median | p99 median | worst   |
| ---------------------- | ---------- | ---------- | ---------- | ---------- | ------- |
| steady tick before     | 19300      | 0          | 19500      | 27400      | 196000  |
| observation full       | 153500     | 10400      | 165700     | 180900     | 247200  |
| observation delta zero | 52500      | 100        | 56900      | 59600      | 196700  |
| cost receiver          | 99900      | 100        | 118500     | 122700     | 147100  |
| candidate 1 edge       | 1000       | 0          | 1100       | 1100       | 1100    |
| candidate 128 edges    | 26300      | 200        | 33700      | 35700      | 53800   |
| candidate 4096 edges   | 821000     | 4200       | 841300     | 1082700    | 3905400 |
| steady tick after      | 19100      | 100        | 19200      | 19600      | 23600   |

证据支持的结论仅为：无显式 #303 调用时，零车稳态 tick 前后保持 0 allocation / 0
reallocation；receiver 在 exact count/bytes 预检后零分配；candidate 的 retained 与墙钟
随 1/128/4096 edges 呈可解释扩展。4096-edge 最坏单样本 3.9054 ms 虽低于本 fixture 的
4 ms quantum，但它是 OS/tail 可见的描述值，不构成产品预算或 SLA。显式调用可能以自身
墙钟延迟下一 tick，但测试前后状态与分配不变量证明其不与一次 tick 交错，也未留下持续
allocator 工作。

完整环境、配置、逐 round raw samples、分配账本与复现命令登记在
[#521 raw evidence comment](https://github.com/illusion-tech/laneflow/pull/521#issuecomment-5449994930)。
该不可变 PR 证据与 source commit 配对，闭合 §7 的首轮具名描述性切片；真实 snapshot
restore/replay 消费仍由 #512 后续集成验证，不能由本 case 替代。

## 8. G2 边界与必测义务

G2 已按 §2 落定完整/增量/分区观测实现：

- `ObservationStateSequence(u64)` 安装/新世代初值 `0`，成功 step 与改变 v1 行的
  Active spawn/replace、park、leave、Active despawn checked 递增；reserve/cancel/rebind、
  parked spawn、Parked/Completed despawn、路线表变更、parking `NoChange`、失败和导出不推进；
- `ObservationStreamBinding` 直接复用世界身份/共同世界世代，不维护第三套 stream
  状态；成功切换与世代递增同界重置状态序号，旧 session stale；
- `AllLaneEdges` 与严格升序显式 LaneEdge 集合在 open 时一次性解析并摘要；full 含
  全零行，delta 严格比较上一次成功基线并显式发送归零行；
- 导出从当前 `VehicleState` / compiled route 重算前杠归属与跨边车身区间并集，不读取
  前一拍 `OccupancyIndex`；所有输出预留可失败，任一失败不推进 session；
- batch/session 的 logical/retained bytes 按 §2.1 的实现内布局函数精确报告。

G2 也已按 §4.1 落定 `route_edge_occurrence_capacity` 的公开配置、计数器与错误拼写，
并复用 #302 的字段私有 `WorldGeneration(u64)`：安装初值 `0`，成功切换 checked
递增并与 root 同界提交，失败/耗尽不变；`WorldBinding` 同时校验世界身份与该世代，
旧描述符即使 origin 相同也会 stale。恢复核对随 #302 快照实现接入。

G2 已进一步落定 `ObservationSetBinding`、`DynamicCostSnapshotBinding`、字段私有的
`RoutingAdmissionSession`、`CandidateRouteInput` 与规范化
`AdmittedRouteRegisterInput`；candidate/replay 都先把稳定标识解析为当前根 ordinal，
最终仍调用唯一 `register_route_edges`，不复制路线编译器。`open_routing_admission`
因无失败原因按事实落为无分配、不可失败入口；candidate/admitted Rust 输入也不新增
无事实依据的独立版本轴。

仅用于 contract/performance 的成本 receiver fixture 以
`u64::try_from(payload.len())` 度量 `exactByteLength`，并由调用方显式提供
`maxEntryCount/maxExactByteLength`；它在解析、分配和哈希前先拒绝 bytes/count 上限，
随后验证实际固定宽度 fixture 条目数与 §3 摘要。该配置是宿主 receiver provenance，
不进入 `WorldConfig`，也不是 LaneFlow 产品默认成本格式。§7.1 已登记具名 Routing
工作负载的 receiver 配置值与首轮 P100 参考机描述性结果；G2 剩余集成边界是 #512 的
真实 restore/replay 消费。若实现证明必须
新增跨进程 wire、`laneflow-routing` 算法 crate、tick 维护 journal、持久化成本
provenance，或无法复用唯一 route 编译器，必须停止并返回 G1。上述清单不穷尽返回
条件：即使未命中枚举，只要实现、实测或真实产品约束证明权威边界、字段、格式选择、
过期政策或预算有误，也必须修正设计，而不是服从错误文档。

#303 不另发明世界身份/世代：观测与 Routing token 必须直接复用上述
`TrafficWorld` / `worldBinding` 的唯一身份、世代字段和失效点，但不把命令/事件
游标复制进 Routing 绑定。
G2 同时交付的最小成本 receiver fixture 仅存在于 `#[cfg(test)]`，让宿主实现验证
length/count/digest 和上限矩阵；该 fixture 不成为 Routing 产品或算法 API。

自动化 contract tests 除 §7 矩阵外至少覆盖：full 首批约束；delta 缺批/重排/重复/
跨 selection 拒绝；全零清除；导出失败 session 不前移；同 tick 生命周期提交推进
状态序号；不同状态序号的分区拒绝拼接；成功跨边 step 后立即导出以及 step 间
spawn/park/leave/despawn/replace 后立即导出；不得复用旧 `OccupancyIndex` 形成混合状态；整数聚合
跨边车身；Parked/Completed 排除；前保险杠在 denied hop、permitted hop 与最后一边
端点的归属；未来/过期边界恰好等于两端时的判定；旧 tick 绑定大于当前值的状态序号
拒绝；修订相等但 StableId 内容非法；cost digest 不授予拓扑信任；direct/candidate/
cutover/restore/replay 在重复边与 occurrence capacity max/max+1 上同义且失败零路线槽/
计数变化；切换成功/放弃与 restore 的 session/candidate 失效；检查点后候选成功注册
只重放规范化已准入路线注册命令、不调用 Routing、不提交旧成本绑定。
