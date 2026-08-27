# 交通观测与 Routing 接入

**文档状态**: Review（#303 G1；接受并在 Issue 记录后才可申请 G2）<br>
**最后更新**: 2026-08-27<br>
**适用范围**: 已提交交通观测的 full/delta/partition 导出、动态成本绑定、候选路线注册、过期语义、#302 切换/快照交互与独立性能门禁<br>
**关联文档**:
[`../adr/0020-compiler-owned-static-network-and-static-image.md`](../adr/0020-compiler-owned-static-network-and-static-image.md)、
[`../adr/0021-city-simulation-game-traffic-foundation.md`](../adr/0021-city-simulation-game-traffic-foundation.md)、
[`../adr/0029-retire-precompiled-static-route.md`](../adr/0029-retire-precompiled-static-route.md)、
[`traffic-runtime-shared-consumption.md`](traffic-runtime-shared-consumption.md)、
[`traffic-runtime-revision-cutover.md`](traffic-runtime-revision-cutover.md)、
[`traffic-runtime-snapshot.md`](traffic-runtime-snapshot.md)

本文提出 #303 的 G1 冻结候选。接受并在 Issue 记录 G1 Pass 后，Rust 名称、错误枚举
与具体容器布局才由 G2 落定；G2 不得静默改变本文的权威分层、绑定集、时点、失败关闭
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
`occupy_parking` 等生命周期命令可在同一 tick 内改变观测事实。因此每个活动世界还
维护一个只在当前世界世代/观测 stream 内单调递增的
`observationStateSequence`：成功 `step` 以及每个会改变 v1 观测行的成功生命周期
提交各推进一次；失败、只读查询、导出和不影响 v1 行的路线表变更不推进。安装、
恢复或成功切换建立新 stream 时从该 stream 的初始值重新开始，旧 stream 的序号
不得跨世界世代比较。其精确整数类型和溢出错误由 G2 落定，但不得用饱和、回绕或
仅看 tick 代替严格单调语义。

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

每批头至少携带：封闭 `bindingVersion`、#302 世界身份/世界世代、
`NetworkRevisionId/networkRevisionDerivationVersion`、stream 身份、
`selectionDigest`、full/delta kind、base/current delivery sequence、tick 与
`observationStateSequence`、精确 `entryCount`。
批次另报告 `logicalBytes`（实际初始化的头/行存储）和 `retainedBytes`（批次拥有的实际
容量）；两者是当前实现版本的精确资源观测，不是跨语言编码或稳定 ABI。G2 冻结计算
函数并验证无 spare-capacity 伪报。

### 2.2 分区选择

观测分区是调用方声明的逻辑选择，不是 `RuntimeExecutionPlan` 的 worker/partition：

- `AllLaneEdges`；或
- 当前修订下按 `StableId128` 升序、无重复、非空的显式 LaneEdge 集合。

创建导出 session 时，Runtime 用 `SharedIdentityIndex` 一次性解析并验证选择，计算带
域分隔版本的 SHA-256 `selectionDigest`。未知标识、错误实体 kind、重复或不规范排序
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

session 是调用方持有、Runtime 签发的不可伪造能力，绑定：#302 同一世界身份/世代、当前
`NetworkRevisionId` / `networkRevisionDerivationVersion`、`selectionDigest`、观测
stream 身份、上一成功 delivery sequence / `tick` / `observationStateSequence` 与
上一行值。它不进入世界确定性状态。

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

过期策略只使用 fixed tick：候选在 `currentTick < observationTick` 时是“来自未来”，
在 `currentTick > validThroughTick` 时过期；两者都失败关闭。同 tick 下若当前
`observationStateSequence <` 绑定值，同样是“来自未来”并拒绝。状态序号证明成本的
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
) -> Result<RoutingAdmissionSession, CandidateRouteError>;

TrafficWorld::register_candidate_route(
    admission: &RoutingAdmissionSession,
    input: CandidateRouteInput,
) -> Result<RouteHandle, CandidateRouteError>;
```

`RoutingAdmissionSession` 是 Runtime 签发、调用方持有的不可伪造能力，绑定当前
世界身份/世代、修订和调用方显式选择的 `(costModelId, costModelVersion)`；它不保存成本
payload，也不进入 Runtime Snapshot。模型升级必须开新 session，不存在“较新版本兼容”。

Runtime 按下列顺序失败关闭，任一失败不占路线槽、不留下 compiled occurrence：

1. O(1) 预检候选边数与配置/共享根上界，完成 checked 大小计算，再做 Runtime 解析/
   compiled 分配；空序列拒绝。
2. 核对 `bindingVersion`、世界身份/世代、修订标识/派生版本、观测
   tick/state sequence/set digest、条目数/bytes/digest 字段完整且自洽；成本模型
   身份/版本必须与 admission session 精确相等。
3. 按 §3 比较当前已提交 tick 与 `[observationTick, validThroughTick]`，并拒绝同 tick
   来自未来的 `observationStateSequence`。
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
包装成隐藏 Routing。跨观测/成本边界的候选必须走本节严格入口。

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
- **回放**：观测导出不是输入命令，不改变确定性状态摘要；候选注册成功是普通路线
  生命周期命令，必须由宿主命令序列以耐久调用方 ID 记录。重放不重新执行 Routing。

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
条目数、exact bytes、累计分配、峰值 retained bytes、墙钟与 tick 间隔干扰同时报告，
不能用单个平均耗时代替。

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
| candidate registration | 1/典型/长边序列；重复边；stale/future/revision/model/identity/topology 错配 | 唯一 route 编译器；失败零路线槽变化；成功成本与墙钟按边数报告                      |
| cutover/restore        | prepare 中注册、commit、abort、同修订 restore、跨修订 cutover               | session/candidate 失效矩阵与 §5 精确一致；已注册句柄按 #302 保持                   |
| session retained       | 同 selection 的 1/10 个消费者；1%/10%/100% 选择；open/drop                  | 单 session 与总 logical/retained bytes、open/drop 成本分别报告且按消费者数可解释   |

实现级安全上限必须在读取调用方可变长度数据前从 `WorldConfig`/接收端容量合同取得；
至少覆盖 selection rows、输出 rows、成本 `entryCount/exactByteLength`、候选 edge count
和所有乘加溢出。G2 回写默认值、max、max+1 与不可达值证明。

观测导出与候选注册都是 step 之间的显式调用；允许它们延迟下一 tick，但不允许与
一次 tick 交错或产生半提交状态。无导出调用时，#303 不得新增 per-tick 全网复制、
dirty journal、墙钟任务或 allocator 活动。

## 8. G2 边界与必测义务

G2 落定并回写：Rust 类型/错误枚举、世界/stream 不可伪造 token 与
`observationStateSequence` 的精确表示、
`exactByteLength` 度量函数、接收上限默认值与首轮 P100 描述性结果。若实现证明必须
新增跨进程 wire、`laneflow-routing` 算法 crate、tick 维护 journal、持久化成本
provenance，或无法复用唯一 route 编译器，必须停止并返回 G1。上述清单不穷尽返回
条件：即使未命中枚举，只要实现、实测或真实产品约束证明权威边界、字段、格式选择、
过期政策或预算有误，也必须修正设计，而不是服从错误文档。

#303 不另发明世界身份/世代：其 token 必须直接复用 #302 G2 对 `worldBinding` 中世界
身份和活动聚合世代的唯一实现与失效点，但不把命令/事件游标复制进 Routing 绑定。
G2 同时交付仅用于
contract test/性能证据的最小成本 receiver fixture，
让宿主实现验证 length/count/digest 和上限矩阵；该 fixture 不成为 Routing 产品或算法
API。

自动化 contract tests 除 §7 矩阵外至少覆盖：full 首批约束；delta 缺批/重排/重复/
跨 selection 拒绝；全零清除；导出失败 session 不前移；同 tick 生命周期提交推进
状态序号；不同状态序号的分区拒绝拼接；成功跨边 step 后立即导出以及 step 间
spawn/park/replace 后立即导出；不得复用旧 `OccupancyIndex` 形成混合状态；整数聚合
跨边车身；Parked/Completed 排除；前保险杠在 denied hop、permitted hop 与最后一边
端点的归属；未来/过期边界恰好等于两端时的判定；修订相等但 StableId 内容非法；
cost digest 不授予拓扑信任；注册失败路线表完全不变；切换成功/放弃与 restore 的
session/candidate 失效；回放只重放成功注册命令、不调用 Routing。
