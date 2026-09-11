# 复杂路口只读观测与跨层验证

**文档状态**: Review（#285 G1 提案；不表示新增 API 已实现或设计已接受）<br>
**最后更新**: 2026-09-11<br>
**适用范围**: 道路机动车复杂路口的 Runtime 只读定位、Bevy 观测与参考场景、
正确性和一万／十万规模验证

**关联文档**:

- [ADR 0019](../adr/0019-waiting-zone-conflict-right-of-way-authority.md)
- [ADR 0021](../adr/0021-traffic-infrastructure-and-host-boundary.md)
- [联合仲裁合同](waiting-zone-conflict-right-of-way.md)
- [路权策略实施合同](traffic-runtime-right-of-way-policy.md)
- [Adapter API](adapter-api.md)
- [Bevy Reference Adapter](bevy-reference-adapter.md)
- [性能基线](core-runtime-performance-baseline.md)
- [中国城市工作负载](chinese-style-city-workload.md)
- [需求与无界面验证](urban-demand-harness.md)
- [后发射检查与最小发布闭合](compiler-post-emission-check-and-minimal-publication-closure.md)

## 1. 决策与边界

复杂路口验证消费已经存在的 Waiting、Conflict、policy、snapshot 与 cutover authority。
本提案只增加精确的路线机动门只读定位，以及由活动 Bevy Session 借出的领域观测视图；
不改变准入、间隙、下游净空、预约、信号解释或最终运动。

静态输入仍走 compiler → LFCA → `SharedNetworkRevision`，动态路线仍由每世界
`register_route` 注册。只读观测不新增 LFCA、catalog、LFRS 或摘要版本，不保存
Adapter 缓存，也不恢复已删除的 JSON/Core 入口或公共 JSON schema 发布义务。

现有 `TrafficWorld` 已提供车辆状态、Waiting occupancy/member/decision、Conflict
decision/reservation 和 transition event。本提案复用这些类型和顺序，不另建第二套
原因枚举，不导出 arbiter、claim ledger、frontier 或候选求值的可变能力。

领域观测与 Routing 的 `CommittedTrafficObservationBatch` 用途不同：前者解释复杂
路口已提交状态，后者向宿主成本策略提供道路交通观测。不得把本提案的机动门、等待区
或预约记录混入 Routing full/delta 协议，也不建立独立的历史事件服务。

## 2. 精确机动门定位

`ConflictRouteAnchor`、`WaitingRouteAnchor`、traversal phase 与 reservation 已保留
动态路线出现项／hop。Adapter 当前不能仅凭公开路线边序列可靠地取得 Runtime 编译后的
机动门，尤其不能把重复经过同一静态 Gate 的两个 hop 折叠。

新增只读入口的语义形状如下；字段私有，提供只读 accessor：

```rust
TrafficWorld::route_gate(
    route: RouteHandle,
    hop: u32,
) -> Option<RouteGateObservation>

RouteGateObservation {
    route: RouteHandle,
    hop: u32,
    gate: ManeuverGateOrdinal,
    edge: LaneEdgeOrdinal,
    progress_mm: u32,
}
```

- 只查询本世界已注册 `CompiledRoute.hop_gate`、相应边与共享根整数毫米边长。
  Gate 在该 hop 的 from-edge 末端；不在 Adapter 重跑 route normalization。
- stale route、越界 hop 或该 hop 没有 Gate 时返回 `None`，不就近选择另一道 Gate。
- `route` 与 `hop` 一起标识本世界中的出现位置；`gate` 是当前共享根内的静态序号。
  它们不得跨世界／世代／修订直接复用。跨独立运行的比较使用调用方路线身份、hop 和
  从相应根解析的静态稳定身份，不比较 raw handle 字面值。
- 读取为 O(1)、无 heap allocation、不改变世界或摘要。不得向路线增加重复缓存表。
- 这是位置观察，不是通行许可、停止原因或下一步预测。当前 pose 仍通过 Session 的
  封闭提取入口获得；Gate 几何只能用于静态调试标记。

## 3. Session 借用式批量观测

```rust
LaneFlowSession::junction_observation(
    &self,
) -> LaneFlowJunctionObservation<'_>
```

视图借用活动 Session，不拥有可变世界、输出队列或第二份静态路网。创建视图为 O(1)
且无 heap allocation；Rust 借用期间禁止对同一 Session 推进或提交生命周期命令。
headless Session 也能读取领域观察；几何绘制另外要求有效 Spatial 配对。

视图至少提供以下内容；具体 iterator 类型可以隐藏，条目语义和顺序必须保持：

| 内容                        | 来源与顺序                                                                            |
| --------------------------- | ------------------------------------------------------------------------------------- |
| 读取上下文                  | 当前世界身份、世代、修订、tick、command cursor、event cursor；从同一个 world 取得     |
| 静态根                      | 活动 Session 的同一共享根；只借用或克隆根 `Arc`，不复制静态表                         |
| live 车辆行                 | 沿 `live_vehicles()` 顺序，返回句柄、现有 `VehicleState` 与可选 `ConflictReservation` |
| Waiting 区计数              | 按当前根 WaitingZone ordinal 升序遍历 `waiting_zone()`                                |
| Waiting member              | 原样借用 `waiting_zone_members()`，保持 zone／admission sequence 顺序                 |
| 最近 Waiting／Conflict 决策 | 原样借用各自 `latest_*_decisions()`，保留完整 anchor、outcome 与原顺序                |
| 最近 transition             | 原样借用 `latest_transition_events()`，不重排、不丢失同 tick 多事件                   |
| 机动门定位                  | 转调 §2 的只读入口，保留 route/hop，不按静态 Gate 去重                                |

车辆全量遍历为 O(N_live)，Waiting 计数为 O(N_zone)，各 slice 遍历与实际条目数线性。
迭代不得为每个车辆扫描全部决策、全部区域或全部路线。按车辆聚合决策时，消费方对整个
批次建立一次可复用索引或顺序合并，并独立计入观测成本。

信号组继续使用现有 `committed_signal_groups()`，其拥有式 materialization 的分配和
耗时单独计账；不因视图本身零分配就声称整个调试／观测链零分配。

### 3.1 当前状态与最近决策

车辆、member、occupancy 与 reservation 表达读取时的当前已提交状态；latest decision
和 transition 表达 Runtime 保留的最近 successful tick 结果。生命周期命令可能已经
移除车辆或路线，而 Runtime 按现行合同仍保留此前决策。因此：

- 不把最新 `Granted` 当作当前 reservation。显示“持有预约”只读当前预约。
- 不把决策中已 stale 的车辆／路线视为整批错误，不映射到复用该 slot 的新车辆。
- 历史 route 已移除时，Gate 位置显示不可解析；不从新路线或同 ordinal 猜测位置。
- `NotEvaluated`、`NotRequired`、`Deferred`、各类 `NoGrant` 保留原义；车辆不动
  或没有某类决策不能被改写成红灯、容量不足或已经获得许可。
- 视图上下文标记读取边界，不伪造“最后决策生成时的 command cursor”。每 tick 证据
  必须在 `LaneFlowFixedSet::Observe`、下一次 lifecycle 前消费；catch-up 有多次
  successful tick 时分别消费，不只读取 outer frame 最后的批次。
- failed step 不形成新证据行；错误与上一成功 tick 的批次分开报告。跨修订清空与
  同修订保留规则继续服从 Runtime，不由 Adapter 补造事件。

视图不能跨 mutable Session 操作保留。需要持有历史的调用方自行复制有限记录，附上
读取上下文并明确作为历史；展示时先核对世界／世代／修订和目标句柄。历史副本不授予
更新 Transform 或反向提交命令的能力。

## 4. Bevy 参考场景与调试

参考场景由正式编制来源和显式版本化 policy selection 创建，同一配置产生可重复
LFCA／catalog 摘要。复用现有编译器、scenario prepare、Session、车辆绑定和封闭 pose
提取，不引入另一套 renderer 或调度器。native 示例使用现有 `native-example` 功能，
无窗口 smoke 复用其初始化和数据提取路径。

最小可视化包含道路／车辆、静态 Gate 标记、Waiting 区间、存在规范几何的 Conflict
区域，以及选定车辆的 phase、最近决策原因和当前预约摘要。绘制可以使用已有 Bevy
mesh／文本能力，不为了恢复历史 Gizmos API 增加兼容层。

- 静态几何从同一根的 lane geometry、Gate 位置与可选 `ConflictZoneRegion` 读取。
  Waiting 用 entry/release Gate 之间的规范路径表示；不新造 Waiting polygon authority。
- 缺少区域几何时显示有身份的文字／标记并注明无区域，不计算几何相交来补冲突事实。
- 多次经过同一 Gate 的车辆显示各自 route/hop。重复标记的合并仅属画面去重，不能
  合并其决策和预约。
- Spatial 配对、canonical frame、placement token 和车辆 Transform 继续遵守
  `adapter-api.md`。静态绘制缓存换根后重建；旧世代的动态标记不得覆盖当前状态。
- 默认调试关闭。打开／关闭和改变呈现比例，在相同命令输入下必须保持 Runtime
  状态与事件摘要相同；渲染、文本、模型的耗时不能混作 Runtime 性能。

## 5. 有限正确性矩阵

每项先登记已有测试／制品证据，再补缺失端到端路径。成功与失败都通过生产入口，
不得用少量 smoke 取代规模证明，也不得把全部理论组合乘进所有规模。

| 场景族                 | 必须覆盖的观察与不变量                                                                                       |
| ---------------------- | ------------------------------------------------------------------------------------------------------------ |
| 多阶段 Waiting         | entry/release、容量与物理存储、先后顺序、Deferred／组合拒绝、释放与无残留 membership                         |
| 保护通行               | 信号许可仍受 occupancy／reservation／下游约束，最终运动保持 leader、safe-speed、minimum-gap 与 no-overlap    |
| 无保护转向／无信号让行 | 有限版本化 policy、lead／lag、不可证明 ETA、实际车长／minimum gap、拒绝归因与最终清空                        |
| 重复路线与组合资源     | 同静态 Gate 的不同 occurrence 不折叠；多冲突区 bundle 原子取得，Waiting cycle 零提交拒绝                     |
| 生命周期与失败         | route replace、despawn、completion、同修订恢复和跨修订迁移；失败状态／tick／输出不变，历史决策不冒充当前预约 |
| 展示与重放             | headless／调试开关结果一致，独立同输入 state/event digest 一致，catch-up 每成功 tick 不漏读                  |

错误与极限边界专项复用现有 Waiting/Conflict、snapshot/cutover 和 shared-root 测试；
静态声明、proposal/reducer 与 raw-handle permutation 在小型可解释场景中验证其权威
不变量。只有已确认的现实缺陷才扩大矩阵。raw-handle permutation 比较按逻辑身份
归一化的结果，不要求本来含不同句柄的序列化载荷逐字节相等。

制品闭合验证覆盖编制→后发射检查→LFCA→共享根→Runtime/Spatial。相同来源重建的
规范制品必须逐字节一致；不同声明顺序比较按现行制品合同定义的语义／来源范围，
不得把来源映射字节变化误报为 Runtime 非确定性。发布认证复用当前 LFCP v2 合同。

## 6. 规模、性能与证据归属

一万是产品基线、十万是扩展目标。适用的 P10/P100 硬件角色、完整观察窗口、三个独立
进程轮次、p50/p95/p99/max、普通／catch-up 帧与重测规则服从性能基线，不使用仲裁
专项的短采样冒充产品认证。现行组件预算包括一万 Runtime p95 ≤ 2 ms/tick、
Spatial+Adapter ≤ 4 ms/frame；十万 Runtime p95 ≤ 16 ms/tick、Spatial+Adapter
≤ 4 ms/frame，完整帧及硬预算的其他条件仍按性能基线。

| 证据                       | 拥有范围与复用方式                                                                                                                                                                                   |
| -------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 复杂路口专项（#285）       | §5 领域矩阵、观测入口、参考场景、调试验证及该参考场景一万／十万 integrated run；Waiting/Conflict 专项补充候选密度、top-two frontier bytes、claims／碰撞、passage／clearance visits 与 retained bytes |
| 城市无界面（#544）         | 七个 case × 两档真实需求、逐 tick 校验和独立重放；复用其程序、冻结输入和适用证据，不在 #285 再造需求或 oracle                                                                                        |
| 城市跨层（#545）           | 消费 #285 的已交付观测；按城市合同运行有限 Runtime/Spatial/Adapter、保存恢复与切换矩阵，不重复领域 API                                                                                               |
| 统一产品认证（#539／#305） | 三套工作负载的统一 Gate 与最终产品状态；#285／#545 的局部证据不能替代它                                                                                                                              |

#285 的规模取证至少包括持续持有与反复申请两种 Waiting/Conflict 资源负载。先固定
候选数量、冲突密度、输入命令和实际车型，再运行一万／十万两档。稀疏候选只能证明稀疏场景；不能由一个候选外推
高并发仲裁。新增观测全量遍历另测，不把“仅读取少数车辆”写成全量批量成本。

参考场景规模化复用其正式编制来源、prepare 与有限命令输入，在单个 `TrafficWorld`
中达到目标车辆数，不用多个小世界拼接吞吐。一万提取并应用全部可表现车辆；十万先
全量提取，再按调用方稳定车辆身份选取前 `floor(可表现数量 / 10)` 应用 Transform，
报告全量提取与部分应用的实际计数和成本。该十万行属于观测，不伪称按 10% 选集提取的
产品认证。输入配置、来源／LFCA／catalog 摘要、车辆组成、seed、命令计划及复现命令
必须在正式取证前冻结，不能根据结果挑选输入。

该独立路口参考负载不冠以未经满足生成合同的 `LF-SYNTH-v1` 或 `LF-CN-URBAN-v1`
名称，也不替代两者的证据。#285 完成其自身领域与参考场景验收后供 #545 消费，
不以尚待 #545 完成的城市跨层行作为 #285 的关闭前置，避免反向依赖。

每份报告区分 Runtime tick、领域观测、pose 提取、映射／Transform apply、renderer，
记录全部实际计数；warm-up 后 tick 零 allocation/reallocation 是单独硬不变量。
分别报告逻辑 retained、scratch/output 与进程峰值，不能重复计算共享根或把组件账本
当作进程内存。跨层 percentile 只能来自同一 integrated run，不相加历史分位。

未具名认证硬件、超预算、缺少测量分别如实列出。#285 必须保留适用的预算比较和未满足
项，不因复用 #544 或将最终认证归 #539 而删去自己的规模义务。需要改变既有验收范围
时先更新 Issue 并说明理由，不能在交付时静默降级。

## 7. 实施与收口

G1 接受后，按精确 Gate 定位与借用视图、参考场景／调试、缺失矩阵／规模证据的顺序
实施。每阶段沿用唯一生产入口，不提前宣称整项完成。G1 设计 PR 使用 `Refs: #285`；
最终完成全部验收的实现 PR 才使用 `Closes #285`。

独立收口审阅在 GitHub PR 中逐项映射 ADR 0019、现行联合仲裁合同、policy 实施合同
与本设计，核对实现、测试、API／格式影响、性能未满足项和 follow-up。原始运行记录、
截图及当次审阅结果留在 GitHub／可复核结果包，不回填到长期设计。

如果发现求解语义缺口，转回相应 owner 的设计与实现边界；不得以调试色彩、隐藏车辆、
修改呈现比例或减少需求让错误不可见。普通 review、required checks 与 Merge Queue
继续按现行治理；不恢复 G3/G4、Gate Ledger 或第二套合并授权。
