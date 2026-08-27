# 修订切换事务

**文档状态**: Accepted（#302 G1）<br>
**最后更新**: 2026-08-27<br>
**适用范围**: `TrafficWorld` 在线路网修订切换的状态机、切换描述符、迁移策略、
迁移增量日志、切换事件批次、原子晋升、失败关闭不变量与 G1 预算<br>
**关联文档**:
[`../adr/0020-compiler-owned-static-network-and-static-image.md`](../adr/0020-compiler-owned-static-network-and-static-image.md)、
[`../adr/0021-city-simulation-game-traffic-foundation.md`](../adr/0021-city-simulation-game-traffic-foundation.md)、
[`../adr/0025-checked-canonical-network-and-shared-static-network.md`](../adr/0025-checked-canonical-network-and-shared-static-network.md)、
[`shared-static-network.md`](shared-static-network.md)、
[`traffic-runtime-snapshot.md`](traffic-runtime-snapshot.md)、
[`retire-precompiled-static-route.md`](retire-precompiled-static-route.md)、
[`portable-canonical-artifact.md`](portable-canonical-artifact.md)

本文是 ADR 0020 §6（切换状态机与原子性语义；经 ADR 0025/#300 G1 部分修订：静态
镜像制品取消、「镜像切换」改名为修订切换事务）与 ADR 0025 的实现级合同。为什么
在线切换必须是失败关闭事务见 ADR；Runtime Snapshot 制品与恢复合同见
`traffic-runtime-snapshot.md`。G2 决定 Rust 拼写。

## 1. 对象与职责

- **活动聚合**：`CommittedRoadNetwork`（术语表）逻辑聚合 `CommittedNetworkSource`、
  对应 `Arc<SharedNetworkRevision>` 与 editable session 的 optional exact
  diff-base binding。working / candidate 不属于该对象。
- **候选**：一次切换中完全拥有、不可变、可独立丢弃的下一修订世界镜像；包含新
  `Arc<SharedNetworkRevision>`、迁移后的每世界动态状态草稿与未提交切换事件批次。
  候选不进入任何公开查询路径。
- **切换权威**：`TrafficWorld` 拥有事务执行；迁移权限权威在宿主/上层（§2）。
  Runtime 不拥有出行需求、游戏规则或"哪些实体可牺牲"的裁决。

## 2. 切换描述符（`NetworkRevisionCutoverDescriptor`）

术语表把具体字段与 trust anchor 留给本 G1。冻结如下：

| 字段                  | 语义                                                                                                                                        |
| --------------------- | ------------------------------------------------------------------------------------------------------------------------------------------- |
| `formatVersion`       | 描述符封闭契约版本；未知值失败关闭                                                                                                          |
| `baseLfcaOrigin`      | base 侧 LFCA origin 四联：LFCA digest / byte length / `NetworkRevisionId` / `networkRevisionDerivationVersion`（与 LFSD base binding 同构） |
| `targetLfcaOrigin`    | target 侧 LFCA origin：同上四联                                                                                                             |
| `semanticDiffOrigin`  | LFSD origin：`semanticDiffFormatVersion` / digest / byte length；同修订恢复可缺失                                                           |
| `migrationPolicyKind` | 封闭种类选择器（术语表：封闭种类选择器）：`same_revision_restore` / `cross_revision_direct`；不是协议版本，兼容拒绝由 `formatVersion` 承担  |
| `worldBinding`        | 目标世界身份与基线命令/事件游标；事务启动时一次性比对，作为迁移增量日志起点                                                                 |

- `worldBinding` 语义：携带目标世界身份与**基线**命令/事件游标（描述符签发时点对齐
  的基线，不是实时值）。比对时点为事务启动时一次性比对，比对结果作为迁移增量日志
  的应用起点；静默期由等价证明（§5）复核最终游标，不做二次签发。证据的二进制编码
  由 G2 落定并回写本文。
- **trust anchor**：描述符是宿主/上层在对象外可信提供的封闭契约输入（例如随宿主
  存档清单或发布信任链交付）。Runtime 只做一致性验证：两侧 LFCA origin 四联与已
  加载/已认证制品逐项匹配（含 `networkRevisionDerivationVersion`，与 LFSD
  base/target binding 交叉一致）、LFSD origin 与两侧修订绑定一致、策略种类已知。
  任何不匹配、缺失或不可信来源都失败关闭；不得仅凭 `StableId128`、LFSD 或调用方
  自报 revision 授予迁移权限，也不得复活 #299 历史 `revision-cutover-v1` receipt
  或静态镜像 descriptor。
- 两侧 `SharedIdentityIndex` 职责（#302 范围要求）：base 侧索引由活动聚合当前根
  提供；target 侧索引在候选构建成功后随候选提供，并在等价证明中与 base 侧共同
  复核稳定引用映射。两个索引都不进入快照制品，各自随所属修订重建。
- 描述符不是可扩展协议：未知字段、未知版本、附加载荷一律失败关闭（封闭契约）。

## 3. 迁移策略（封闭种类选择器）

迁移策略是 Runtime 内建的封闭集合，以 `migrationPolicyKind` 选择，**不提供宿主
回调**——回调会把游戏规则与 Runtime 规则复制进迁移路径，破坏封闭契约与失败关闭
审计。策略语义演进随描述符 `formatVersion` 整体拒绝或放行，不用 kind 值分派
未登记行为。

| kind                    | 语义                                                                                                                                                                                                                                                                                                                                            |
| ----------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `same_revision_restore` | 判据：两侧 `NetworkRevisionId` 相等、`networkRevisionDerivationVersion` 相等且 identity / constraint / execution-constraint versions 精确相等；origin digest / byte length 仅承担来源审计与同字节快速路径（ADR 0020 §12 / ADR 0025 §8）。动态状态按快照局部标识 + `StableId128` 原样重绑。触发用例：同修订 source rebase 或重发布制品的原子换根 |
| `cross_revision_direct` | 经 LFSD 把每个动态实体的稳定引用直移到 target；**任一实体无法映射或重绑失败时整个事务失败关闭**（见下）                                                                                                                                                                                                                                         |

- **句柄保持（ADR 0029 §6）**：成功切换（两种 kind 均适用）对当期
  `RouteHandle` / `VehicleHandle` 保持有效——同进程切修订原地更新既有槽位，
  不得新分配句柄、不得丢弃已保住的句柄；Adapter 持有的句柄在切换后继续寻址
  同一逻辑实体。句柄布局不进入快照。

**不可映射实体处置（本 G1 产品决策）**：以下任一情形视为不可映射，整个迁移事务
失败关闭，旧修订、旧动态状态、旧 `CommittedNetworkSource` 与旧 editable diff
base 全部原样继续生效，零切换事件：

1. 引用不存在：LFSD 显示车辆所在边、路线边序列引用或停车绑定在 target 中不存在；
2. 重绑后非法（changed-but-present）：实体 `StableId128` 仍存在但语义已变化，
   原样重绑会违反 target 不变量——例如边长缩短到低于车辆 `progress_mm`、profile
   的 class / 长度变化导致现有状态非法、路线在 target 准入下不再合法。重绑等价于
   在 target 修订上对该实体重新执行注册/准入的全部结构校验（`register_route`
   检查、spawn 合法性等），任一失败即不可映射。

"删路前先把车开走/清场/劝阻"的体验由宿主在发起切换**之前**用 LFSD 预检与普通
生命周期命令编排；声明式逐类丢弃或自动改道策略属于未来独立 G1，不在 v1 契约内。

## 4. 状态机

沿用 ADR 0020 §6（经 ADR 0025/#300 G1 部分修订，语义不变）：

```text
Prepare → Delta Catch-up → Quiescent Commit → Retire
```

| 阶段             | 行为                                                                                                                            | 失败/放弃                                                   |
| ---------------- | ------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------- |
| Prepare          | 旧世界继续固定步进；从基准提交状态构造候选；开始记录迁移增量日志（§5）                                                          | 构造失败 → 丢弃候选，旧世界无感知                           |
| Delta Catch-up   | 候选在后台按规范顺序应用迁移增量，逐步追近旧世界提交点；不模拟未来、不重执行输入命令                                            | 追不上、日志溢出、迁移失败、预算超限 → 放弃候选并继续旧修订 |
| Quiescent Commit | 在固定步进安全边界短暂**暂停旧世界步进（含输入）**，排空日志尾，证明候选状态/事件批次等价于"迁移函数作用于旧世界最新已提交状态" | 证明失败 → 放弃候选；旧世界从暂停点恢复步进                 |
| Retire           | 新聚合与规范排序切换事件批次作为**同一原子提交**只发布一次；旧修订在全部借用视图/令牌退出后回收                                 | —（提交后不可失败；回收延迟见 §9）                          |

- 候选在提交前不发布任何行为结果；放弃候选时切换事件批次零发布。
- **在途唯一性**：每世界同时至多一个在途切换；发起第二个切换的前置条件是第一个
  已提交或已放弃。作为防线，Quiescent Commit 在原子换绑前复核活动聚合的
  origin / 世界世代仍与本候选的 base 一致；base 已前移（并发路径下被其他事务
  替换）则本候选过期，失败关闭并放弃。
- **target 来源绑定**：候选携带目标 `CommittedNetworkSource` 并在晋升前完成与
  `targetLfcaOrigin` 的对应验证（editable：该 committed `RoadEditingState` 重编译
  产物与 target LFCA origin 逐项一致；published：`PublishedLfcaReference` 与
  target origin 一致）。验证失败按描述符不一致处置；不存在"晋升一个未经绑定
  验证的 source"。
- **目标绑定执行计划**：提交边界前完成目标修订的运行时执行计划重建/旧计划失效，
  其激活与晋升同界，首个新修订 tick 不得经旧计划的分区/序号执行（术语表：运行时
  执行计划为每世界可重建对象；现行单 worker 实现中它是派生对象，按 target 重建
  即满足本条）。
- 维护暂停模式只能由宿主显式声明，整个准备期停表；不得作为在线玩家改路的隐式
  语义。维护暂停的完整停顿单独预算（§9）。

## 5. 迁移增量日志

- 记录内容 = 已提交变更流（术语表）：旧世界原子提交后产生的规范排序动态状态变化、
  生命周期变化，以及命令/事件游标。不记录未提交中间态、不重复发布旧世界事件。
- **有界**：以字节为上界（含文档化默认值，G2 定具体数值并登记到容量合同）；溢出
  即放弃候选并失败关闭，不截断、不丢弃尾记录、不静默降级为全量重放。
- 溢出放弃后，宿主可显式改用维护暂停模式重试（停表下不需要追赶，日志退化为短尾）；
  不为此新增第二种在线降级路径。
- 静默提交前排空日志尾并对候选做等价证明：候选状态 + 事件批次 ≡ 迁移函数直接
  作用于旧世界暂停点已提交状态。证明采用结构性增量论证（迁移增量重放与迁移函数
  作用等价），**不在窗口内对全量状态重算迁移函数**；同时**独立重算候选迁移态的
  确定性状态摘要并与期望值比对**（ADR 0021 §5 的静默期复核要求）。期望值 =
  **旧世界在静默点由自身摘要机制独立计算的确定性状态摘要**——迁移在稳定引用
  键控的逻辑状态上是恒等变换（直移只改序号绑定，不改逻辑内容），因此候选摘要
  必须与旧世界摘要精确相等；期望值不取自候选路径，候选侧损坏（追赶丢弃/误用
  迁移增量）在比对处暴露并失败关闭。两项成本计入静默窗口预算（§9），精确证明
  形式由 G2 落定并回写本文。

## 6. 切换事件批次

- 由迁移函数生成、准备期保持不可见、只与新共享修订/状态绑定**原子提交恰一次**的
  规范排序事件集合（术语表：切换事件批次）。
- 放弃候选时零发布；不存在"部分事件已可见"的中间态。
- 事件枚举 v1 在 §3 整体失败关闭策略下**不产生实体消失类生命周期事件**（一切
  不可映射即失败，无第三种处置）；v1 允许空集起步，至多保留修订切换通知类事件。
  具体枚举由 G2 按实现落定并在本文登记，不得扩展为通用事件通道。

## 7. 原子晋升与内存共存

同一安全边界原子替换的对象清单（全部或全不）：

1. 活动聚合指向的新 `Arc<SharedNetworkRevision>`；
2. 迁移后的每世界动态状态；
3. 新 `CommittedNetworkSource`；
4. editable aggregate 的 `EditableDiffBase`（仅当与新根 origin 精确一致时更替）；
5. 修订绑定的 Spatial facade / 只读快照（若存在绑定）：与 Traffic 根同界原子
   发布（ADR 0025 §5 修订绑定；消费契约要求 `TrafficWorld` 与 `SpatialSession`
   使用同一根）；
6. 规范排序切换事件批次（恰一次发布）。

**Spatial / Adapter 绑定失效**：切换提交后，绑定旧修订的 `SpatialSession` /
catalog controller 失效（沿 ADR 0029 §5 的 `(世界令牌, NetworkRevisionId)`
失效模式），调用方按新修订重新 bind、不得继续用旧修订绑定消费 target 位姿；
首个切换后位姿提取不得落在旧根上。旧修订回收仍按最后一个借用退出触发，旧会话
的在途借用可完成（`shared-static-network.md` §8）。

失败时以上全部保持旧值。**仅 editable 聚合**：提交成功后 target LFCA 原子成为
下一次 LFSD 的 diff base（`shared-static-network.md` §9 口径）；runtime-only
发布聚合没有 `EditableDiffBase`、不发射 LFSD，本条不适用。允许短暂共存的内存
对象：current root +
retained base LFCA + target LFCA/LFSM/LFSD + candidate root + scratch
（`shared-static-network.md` 共存模型）；**迁移期动态双份**（旧世界动态状态与候选
动态状态草稿）、迁移增量日志与未提交切换事件批次的存活字节同样计入候选共存峰值
（§9）。旧修订回收按最后一个 Runtime/Spatial/Adapter 借用视图/令牌退出触发，延迟
单独量化；不得为提前回收撤销有效借用。

## 8. 失败关闭故障映射

| 故障                    | 行为                                                                                                   |
| ----------------------- | ------------------------------------------------------------------------------------------------------ |
| 描述符不一致/不可信     | 事务拒绝启动；旧聚合原样生效                                                                           |
| 候选构造失败（Prepare） | 丢弃候选，旧世界无感知；零事件                                                                         |
| 日志溢出                | 放弃候选；旧世界继续；零事件（§5）                                                                     |
| 追赶失败/预算超限       | 放弃候选；旧世界继续；零事件                                                                           |
| 语义证据缺失            | 等价证明失败 → 放弃候选；零事件                                                                        |
| 不可映射实体（§3）      | 整个事务失败关闭；旧修订/状态/source/diff base 全保留；零事件                                          |
| 保存发生在候选准备期    | 只捕获旧 `CommittedNetworkSource` 与对应 Runtime Snapshot（见快照文档 §5）；事务不受影响，可继续至提交 |

「候选准备期」= 自 Prepare 起至 Quiescent Commit 原子提交或放弃之前的候选存活
全程。

## 9. G1 预算

度量协议：同机描述性基线（沿 `LF-P100-REF-01` 先例），各维度在量化切片以基线
推导登记初值；1.0 前允许按实测修订，方向只许收紧。

| 维度             | G1 上限/口径                                                                                                                                                                        |
| ---------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 静默提交窗口     | v1 ≤ 2 个固定步进边界（调度约束）**且**窗口内全部工作（排空日志尾、摘要复核、原子换绑）受墙钟/主线程停顿上限约束（ADR 0021 §8 在线静默停顿预算），任一超限即放弃候选；典型 1 个边界 |
| 在线准备干扰     | 不改变旧世界已提交状态与事件语义；稳态 tick 不因准备新增分配；资源干扰（CPU/内存）描述性报告                                                                                        |
| 迁移增量日志     | 字节上界 + 文档化默认值；溢出失败关闭                                                                                                                                               |
| 候选共存峰值     | current root + retained base LFCA + target LFCA/LFSM/LFSD + candidate root + scratch + 迁移期动态双份 + 迁移增量日志 + 未提交事件批次的共存峰值（§7）按基线登记                     |
| 候选规范制品字节 | LFCA/LFSM/LFSD 候选 exact bytes 分别量化（#302 范围要求）                                                                                                                           |
| save/load 停顿   | Runtime Snapshot 保存/恢复停顿按基线登记（#302 范围要求；完整维度见快照文档 §8）                                                                                                    |
| 维护暂停完整停顿 | 单独预算；只在宿主显式停表时发生                                                                                                                                                    |
| 旧修订回收延迟   | 最后借用退出后的回收延迟描述性量化                                                                                                                                                  |

## 10. 必测项（G2）

- §8 前 6 行故障（含候选构造失败）至少一个注入测试：断言旧聚合、旧动态状态、旧
  source、旧 diff base 全保留且切换事件批次零发布；保存并发的断言见快照文档 §9。
- 静默提交窗口超限（构造 >2 边界场景**或**墙钟上限超限）放弃候选。
- 日志溢出放弃后，宿主以维护暂停模式重试成功的端到端路径。
- 不可映射实体整体失败关闭：引用不存在三类（删边上有车 / 路线引用被删边 / 停车
  绑定失效）与重绑后非法三类（边长缩短低于 `progress_mm` / profile class 或长度
  变化致状态非法 / 路线在 target 准入下不合法）；宿主清场后重试成功。
- 切换事件批次恰一次：提交后事件序可见一次（v1 空批次也验证零重复）；放弃路径
  零事件（无"半批次"）。
- 原子晋升：提交边界前后对外可见聚合与状态的一致性检查（无中间态可观察）。
- 旧修订回收：借用视图全部退出后回收、有存活借用不回收。
- 描述符未知版本/未知字段/未知策略种类（`migrationPolicyKind` 未知值）/origin
  不匹配全部失败关闭。
- 在线准备干扰不变量：准备期稳态 tick 零新增分配、旧世界事件语义不变的确定性
  断言。
- `same_revision_restore`：同修订重发布/重编译制品原子换根后动态状态原样重绑、
  句柄语义保持；同修订不同字节允许（判据见 §3，对照快照文档 §7）。
- **句柄保持（两种 kind）**：跨修订切换成功后当期 `RouteHandle` /
  `VehicleHandle` 继续寻址同一逻辑实体（ADR 0029 §6 原地更新语义）。
- **在途唯一性**：构造第二候选并发场景，静默期 base 世代复核使过期候选失败
  关闭、零事件。
- **摘要复核**：注入追赶损坏（丢弃/误用一条迁移增量且记账自洽），静默期状态
  摘要比对失败 → 放弃候选、零事件。
- **派生版本门**：`same_revision_restore` 在两侧
  `networkRevisionDerivationVersion` 不等时拒绝。
- **target 来源绑定**：source 与 `targetLfcaOrigin` 对应验证失败 → 按描述符
  不一致失败关闭，不晋升该 source。
- **Spatial 绑定失效**：切换提交后旧修订绑定的 `SpatialSession` / controller
  失效、重 bind 后可消费 target 位姿；首个切换后位姿提取不落旧根。
