# 运行时快照

**文档状态**: Accepted（#302 G1）<br>
**最后更新**: 2026-08-28<br>
**适用范围**: 版本化 Runtime Snapshot 的设计原则、绑定集、保存/恢复语义、回放、确定性状态摘要与跨修订迁移入口<br>
**关联文档**:
[`../adr/0020-compiler-owned-static-network-and-static-image.md`](../adr/0020-compiler-owned-static-network-and-static-image.md)（§12；static image / receipt 条款已被 ADR 0025 §8 取代，origin 以 LFCA 为准）、
[`../adr/0021-city-simulation-game-traffic-foundation.md`](../adr/0021-city-simulation-game-traffic-foundation.md)、
[`../adr/0028-integer-millimeter-traffic-geometry.md`](../adr/0028-integer-millimeter-traffic-geometry.md)、
[`../adr/0029-retire-precompiled-static-route.md`](../adr/0029-retire-precompiled-static-route.md)、
[`traffic-runtime-revision-cutover.md`](traffic-runtime-revision-cutover.md)、
[`traffic-observation-and-routing-integration.md`](traffic-observation-and-routing-integration.md)、
[`retire-precompiled-static-route.md`](retire-precompiled-static-route.md)、
[`shared-static-network.md`](shared-static-network.md)

本文是 ADR 0020 §12（按 ADR 0025 §8 修订后语义）的 #302 G1 设计。路线与车辆的
快照表示沿用 ADR 0029 §6 已冻结形状；容器 schema、字段映射与测试构造由 G2
落定。

本文的 #303 观测/Routing 恢复与回放接缝已由 #303 G1 接受，与 #302 合同共同构成
当前唯一实现权威。

## 1. 问题与设计立场

城市游戏需要存档、恢复与回放。Runtime Snapshot 是每世界可变状态的独立版本化
制品：不进入 LFCP 发布链，真实性由宿主存档清单在对象外绑定（ADR 0021）。
设计立场：

- **保存逻辑状态，不保存机器**：持久的是稳定引用、快照局部标识与整数毫米值
  （ADR 0028）；进程句柄、槽位、generation、密集序号、数组布局不持久，恢复
  时新分配句柄。
- **派生即不存**：凡确定性派生的状态不入快照——信号灯色由 `time_ms` 与共享根
  program 派生，车道占用索引每 tick 从已提交状态重建。
- **单一时点**：保存在固定步进安全边界原子捕获一个快照点，编码只读该不可变
  捕获，世界随后可恢复步进；不存在跨提交状态的混合捕获。

## 2. 版本轴与绑定集

版本轴分离：容器 `formatVersion` 与被绑定事实的版本（runtime 版本、
static-contract versions、`networkRevisionDerivationVersion`、identity registry
revision）不混用单一数字；未知版本值失败关闭。

必绑：LFCA origin；`CommittedNetworkSource`（editable 世界 = committed
`RoadEditingState`；runtime-only 世界 = `PublishedLfcaReference`）；上列版本；
世界身份（快照局部）；`tick` / `time_ms` / 输入命令与已提交事件双游标（切换
事务的 `worldBinding` 需要双游标基线）；全部每世界可变状态（§3）；
`WorldConfig`。

`WorldConfig` 分两类，恢复语义不同：**行为语义配置**（`fixed_delta_time_ms`
与语义容量）参与恢复核对——`fixed_delta_time_ms` 必须精确相等；语义容量只许
放大（不得小于容纳快照状态所需）；容量差异不改变恢复合法性，但**精确回放的
对拍前提是语义容量一致**——不一致容量下的重放分歧按失同步信号处理，不判为
实现缺陷（容量差异会改变重放中生命周期命令的成败）；**可重建执行计划
字段**（worker 数等）不参与——执行计划按当前硬件重建、精确结果与 worker 数
无关（ADR 0021）。

**#303 G1 已接受合同**：`route_edge_occurrence_capacity` 是行为语义容量，按全部
存活动态路线边序列的 occurrence 总数核对；重复边重复计数。恢复配置必须至少容纳
快照路线表的总 occurrence，检查点后精确回放还要求与原配置一致。

世界世代是活动进程内失效状态，不是被恢复的逻辑内容：`WorldGeneration` 不进入
快照；同进程原位恢复必须从活动 world checked 取得下一世代，重新安装的世界从
初值建立且不得与仍存活的旧 `world_id`/会话并存。恢复实现不得从快照复制旧世代，
也不得用初值回绕伪装一次原位恢复。

禁绑（出现即拒绝）：runtime handle / slot / entity generation、`WorldGeneration`、
密集序号、共享静态数组、`EditableDiffBase`、partition / worker assignment、数组地址 / layout /
capacity、调用方自有 seed/随机流（宿主存档清单绑定；Runtime 无自有随机流）。

## 3. 每世界可变状态

| 状态        | 快照表示                                                                                                                                                                                                                                                                                                              |
| ----------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 路线表      | ADR 0029 §6 形状：`snapshot_route_id` + 有序边 `StableId128` 序列（允许重复边）                                                                                                                                                                                                                                       |
| 车辆        | ADR 0029 §6 形状 + 每车唯一 `snapshot_vehicle_id`：所属 `snapshot_route_id`、`route_edge_index`、`progress_mm` / `carry_um` / `speed_mm_s` / `status`；profile / class 等静态绑定用 `StableId128`                                                                                                                     |
| 停车状态    | 车辆 `snapshot_vehicle_id` ↔ 停车位 `StableId128` 占用绑定与 parked 状态。当前 `TrafficWorld` 的停车命令面只冻结占用互斥（`occupy_parking`，消费契约 §4.3）；`parking-system.md` 的 reserve/commit/leave/rebind 预约状态机属已退役核心世界（#108/#109）的能力，不在当前可运行状态面内，未来重新生产化时本契约随之扩展 |
| live 顺序   | 车辆 `snapshot_vehicle_id` 的规范排序序列                                                                                                                                                                                                                                                                             |
| tick / 时钟 | `tick` / `time_ms` / 输入命令游标 / 已提交事件游标                                                                                                                                                                                                                                                                    |

车辆是运行时实体，没有 `StableId128`：它以 `snapshot_vehicle_id` 持存并被
停车、live 序引用；静态实体（边、profile、class、停车位）用 `StableId128`。
快照局部标识只在单个快照内稳定，恢复经 `SharedIdentityIndex` 与局部标识表
重建。一维数值全部整数毫米 / 微米 / `u32` mm/s，无浮点字段。

**#303 G1 已接受合同**：观测导出/admission session/基线、动态成本快照、未注册
候选与成本 provenance 是调用方拥有或可重建的交付状态，不进入快照。候选注册
成功后形成的普通路线只按上表路线表示保存。任何成功恢复建立新的世界世代/
观测 stream；恢复前的 session 与
未注册候选全部 stale，不因同修订恢复而复活；新 stream 从初始
`observationStateSequence` 开始（Routing 合同 §5）。路线表恢复还必须经唯一
`register_admitted_route` 规范化入口核对修订、稳定标识、
`route_edge_occurrence_capacity` 与共同路线编译器；超限零部分恢复。

输入命令游标只统计成功应用的生命周期命令；direct / candidate / admitted 三类路线
注册在共同提交点恰好计数一次，保存不计数。游标以 checked arithmetic 推进；耗尽时
所有路线、生成、替换与停车入口都在修改状态前失败关闭，不允许回绕或饱和伪装成功。

## 4. 容器

封闭契约：size-prefixed FlatBuffers、file identifier `LFRS`、`formatVersion`；
schema 位于 `schemas/runtime-snapshot/v1`，生成物隔离于私有 wire package（沿
`laneflow-road-editing-wire` 先例）。读取 verifier-first：语义 lowering 前完成
长度、基数与版本预检。发布链的自定义规范制品仍只有 LFCA / LFSM / LFSD / LFCP；
快照不是发布对象，不要求跨实现字节规范序，只要求逻辑确定性（§6）。
G2 writer 入口为 `encode_lfrs(&CapturedSnapshot)`：它只读边界捕获，不回读活动
world；输出为带 `LFRS` file identifier 的 size-prefixed buffer，必需空表也编码为
存在的空 vector。

## 5. 保存与恢复

- 保存只接受活动聚合的 `CommittedNetworkSource`；working / candidate 与
  `EditableDiffBase` 不入档。候选准备期的保存仍只捕获旧聚合（切换文档 §8）。
- **恢复对称原则**：editable 与 published 两类来源都先重建可信根再恢复——
  editable 从 committed `RoadEditingState` 重编译，published 经
  `PublishedLfcaReference` 重新认证。同修订判据 = `NetworkRevisionId` +
  `networkRevisionDerivationVersion` + 契约版本精确相等（与切换文档 §3 一致）；
  origin 字节差异仅承担来源审计，published 重发布的摘要错配在判据满足时允许
  恢复。资产缺失、重编译失败、版本不匹配或缺 `EditableDiffBase` 对应关系失败
  关闭。
- 发布资产启用道路编辑须另带 committed 道路状态并完成同修订
  root / source / diff-base rebase；成功前不得启用编辑。
- **完整性原则**：恢复的状态必须满足与 `install` / spawn 命令路径同一的不变量
  集。语义 lowering 拒绝：重复局部标识、悬空引用、live 序不是活跃车辆的精确排列、
  停车绑定与 parked 状态不一致、车辆值不变量破坏（`carry_um` 越界、进度超
  边长、超速、profile 与 class 不一致、活跃车辆物理重叠），以及时钟不变量
  破坏（`time_ms` 必须等于 `tick × fixed_delta_time_ms`，checked arithmetic
  验证——不可达时钟对会派生出错误的信号与回放分歧）。任一违反零部分恢复。
- 恢复成功后世界处于快照 `tick` 边界的一致状态；`install` 核对与
  `register_admitted_route` 规范化路线重建沿现行消费契约执行。

## 6. 回放与确定性状态摘要

- 输入命令序列由宿主记录并按序重放（术语表：输入命令序列）；Runtime 不新增
  隐藏输入权威。
- **回放身份**：命令目标用耐久身份（快照局部标识或宿主自有 ID），不用进程
  句柄；恢复经「局部标识 → 新分配句柄」映射重提交；检查点之后由命令新建的
  实体，其耐久 ID 由命令载荷自带。
- **#303 G1 已接受合同——候选路线回放**：`register_candidate_route` 的世界世代、
  观测与成本绑定不进入命令序列。成功准入后只记录规范化路线注册命令：宿主耐久
  路线 ID、当时的路网修订标识/派生版本和有序 LaneEdge `StableId128`。恢复后核对
  当前修订、解析稳定标识并经同一 `route_edge_occurrence_capacity` 与唯一路线编译器
  的 `register_admitted_route` 入口重建，再把新句柄映射回宿主 ID；不重新执行
  Routing，不重放 stale admission。
  跨修订不匹配必须经 #302 受信任迁移策略显式迁移命令稳定引用，否则拒绝。
- **确定性状态摘要（术语表）**：对逻辑状态按规范排序计算、与容器编码无关；
  算法冻结为 SHA-256 + 域分隔版本前缀（沿 `laneflow-format` 先例），摘要输入
  的精确规范化序列化由 G2 登记。同一逻辑状态在任何合法编码下摘要相等；该摘要
  同时是切换事务静默期复核的期望值来源（切换文档 §3/§5）。
- 检查点（checkpoint）是回放序列中的快照锚点，与命令游标共同界定重放区间；
  失同步只诊断（报告首个分歧区间），不自动纠偏、不重启世界，处置归宿主。

## 7. 跨修订迁移入口

跨修订恢复必须由受信任 `NetworkRevisionCutoverDescriptor` 驱动、经 LFSD 显式
迁移（切换文档 §3）；旧密集序号不得直接解释为新修订实体，稳定引用经
`SharedIdentityIndex` 完整重建。同修订换根（含重发布制品）走切换事务的
`same_revision_restore` 路径。

## 8. G1 预算与度量

度量协议与切换文档 §9 相同：同机描述性基线；1.0 前可依据可复现证据收紧、放宽
或重构，影响合同或验收结论时返回 G1。维度：快照制品
exact bytes、save 停顿（墙钟与主线程）、load 停顿（editable 重编译与
published 认证分别度量）、恢复峰值内存、保存期间对稳态 tick 的干扰。

## 9. G2 边界与必测义务

G2 落定并回写本文：schema 与版本轴到字段的显式映射、摘要输入的精确规范化
序列化。

必测义务：save → load exact oracle（逻辑状态含双游标与 `time_ms` 全等，句柄
不比对）；检查点 + 命令重放逐点相等与失同步定位；`fixed_delta_time_ms` 不一致
拒绝；语义容量按 §2 判据（恢复只许放大，精确回放对拍要求容量一致，不一致时
分歧按失同步信号处理）；worker 数差异不影响 exact 语义；容器拒绝面（未知版本 / 长度 / 基数 / 禁绑
字段）；完整性拒绝面（标识 / 引用 / 排列 / 停车绑定 / 值不变量 / 时钟不变量）；两类来源
恢复端到端与 published 同修订重发布恢复；边界捕获拒绝跨提交状态混合；候选
准备期保存只捕获旧聚合。#303 G1 已接受合同追加：路线 occurrence 容量 max/max+1 的
恢复原子性；检查点后成功候选注册按规范化命令重放且不调用 Routing/旧 cost binding。
