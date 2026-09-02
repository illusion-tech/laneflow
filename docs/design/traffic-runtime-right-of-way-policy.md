# 路权策略编译与 ConflictArbiter 实施合同

**文档状态**: Review（#284 实施细化；不表示生产能力已经交付）<br>
**最后更新**: 2026-09-02<br>
**适用范围**: 官方编制来源、LFCA、共享静态路网、策略绑定、信号解释、冲突资源、
运行时快照与修订切换<br>
**设计依据**: ADR 0009、0019、0025、0028、0029；
[`waiting-zone-conflict-right-of-way.md`](waiting-zone-conflict-right-of-way.md) §6

## 1. 决策与适用关系

本文把 #235 已接受的算法合同落实到当前 compiler → LFCA → `SharedNetworkRevision`
→ `TrafficWorld` 路径。Waiting 本地状态仍由 #282 拥有，路线冲突出现项仍由 #559
合同定义。本文不复制 directed ETA、gap、组合资源账本或循环等待算法；其唯一算法
依据仍是联合设计 §6.3–§6.10。

已确认的产品范围是：中国机动车普通红灯条件右转，以及一套具名、显式选择的参考
间隙参数。法规依据和机动车支持边界见联合设计 §6.2.1–§6.2.2。本文新增的输入形状、
版本矩阵和迁移细节是待审阅实施合同；现行格式文档中的 LFCA 4、LFRS 4 等仍描述
已交付生产事实，不能提前改成本文计划值。

本次选择：

- 两个官方前端都进入共同有类型模块；不新增法规文件加载器、脚本表达式或 Runtime 回调。
- 路权策略集（`RightOfWayPolicySet`）是新增的唯一独立静态实体；规则、间隙参数和依据
  是策略集的局部成员，不为每个组合派生稳定实体。
- 每个世界只选择一份策略集及一个法规适用日期；策略内容由所安装的路网修订固定。
- 信号解释产生候选，`ConflictArbiter` 决定车辆实际放行。Controller 继续只推进灯态。
- 编译器和共享根持有只读解析表；world 持有时钟相关阈值、车辆意图和可变资源。
- 新能力、存档与切换闭环、旧保护移除一次性交付；不会单独发布提前放开保护的中间态。

## 2. 来源、身份与值域

### 2.1 正式输入

合成有类型 DSL 的 `SyntheticModuleBuilder` 与道路编辑来源的 builder/reader/writer
新增同构 `RightOfWayPolicySetInput`。道路编辑 FlatBuffers 跟随 §8 原子升级，保持
`LFRE` 根版本的前置拒绝。两条前端均按现行 import、module namespace、来源位置、
预算和失败原子性规则接入；不从 LFSM、外部 URL 或 Runtime 对象回填缺失声明。

同一策略集及其局部成员由一个来源模块拥有；成员可引用已导入模块中的 stream、Gate
和 class。不能把一份策略集拆成无显式所有者的跨模块补丁，也不新增第三条 merge API。
引用旧 Identity 种类时复用现有 typed reference；新增策略实体使用同样的 module/key
寻址机制。

### 2.2 身份与法规来源

Identity registry revision 从 3 升为 4：新增 `EntityKind::RightOfWayPolicySet = 24`，
slug 为 `right-of-way-policy-set`；新增 `FieldTag::RightOfWayPolicySetKey = 35`。
身份字段顺序为 `(AuthoringNamespaceId, RightOfWayPolicySetKey)`。既有种类、字段代码与
Identity v1 编码不变；registry revision 不进入身份前像。未改变身份字段的既有实体
保持原 StableId；增加新种类的独立 known vectors，保留既有向量作为回归证据。

法规身份（`RegulationIdentity`）是值，不是第二个稳定实体：

```text
RegulationIdentity { jurisdiction, version, source? }
RightOfWayPolicySet {
  policySetKey,
  regulation,
  effectiveFrom?, effectiveUntil?,
  evidence[], gapProfiles[], streamRules[], gateRules[]
}
```

`jurisdiction/version/source` 复用现有准入法规来源的字符串验证和值语义；实现时将
`AccessRegulationInput` 的共同值定义收敛到一处，不保留两个分别解释法域的模型。
这不让 AccessRule 的来源信息获得路权裁决能力。既有准入来源的一致性检查继续执行；
被选策略与已声明 Access regulation 的法域、法规版本必须一致，source URL 不代替版本。

日期统一为 `RegulationDate`：外层文本是严格 `YYYY-MM-DD`，内部和 LFCA 为合法公历
日期 `YYYYMMDD:u32`，年份 `0001..9999`，检查闰年与月日。不是 Unix 时间，也不从
模拟时钟加算。省略端点表达无该方向界限；有界有效期必须满足 from < until，选择日期
满足 `[from, until)`。未知日期和解析失败不能用 0 代替。

局部 key 使用现行编制 key 值域、不做 Unicode 归一化；各成员种类内 key 唯一。
`(policy StableId, member kind, member key)` 是持久归因地址，dense ordinal 只在当前根内
有效。参考间隙参数另有必填 `parameterVersion`，与法规版本分开。

### 2.3 成员输入

| 成员       | 字段                                                                                                   | 约束                                                                                    |
| ---------- | ------------------------------------------------------------------------------------------------------ | --------------------------------------------------------------------------------------- |
| 依据       | `evidenceKey, locator, description?`                                                                   | 只读来源；非空 locator 不触发网络抓取                                                   |
| 间隙参数   | `profileKey, parameterVersion, minimumLeadGapMs, minimumLagGapMs, clearanceBufferMs`                   | 三项均为 `u64` 毫秒，允许零；无隐式数值默认                                             |
| 通行流规则 | `ruleKey, stream, participantClasses?, priority:i32, yieldToStreams[], gapProfileKey?, evidenceKeys[]` | class 省略为 fallback、显式空集拒绝；yield 有项时必须绑定局部 gap profile，空时必须省略 |
| 门合规规则 | `ruleKey, gate, participantClasses?, interpretation, prohibition, evidenceKeys[]`                      | class 同上；interpretation 与 prohibition 是 §3 的封闭枚举                              |

所有集合拒绝重复引用；canonicalization 对集合按稳定身份或局部 key 排序。顺序不
承载让行优先级。引用不存在、非法值、重复 key、来源未导入均按现行 compiler
错误前缀之后的 policy 阶段报告，不留下半份模块。

## 3. 灯态解释与禁令

### 3.1 封闭解释类型

门合规规则不接受用户脚本、自由条件字符串或“直接 grant”。`interpretation:u8`
采用以下闭合登记；表中 `P` 为 Protected 候选，`Y` 为 Permissive 候选，`U` 为
Uncontrolled 候选，`D` 为 DenyAndStop。

| 代码 | 类型                         | 必需 signal binding     | 红  | 黄  | 绿  |
| ---- | ---------------------------- | ----------------------- | --- | --- | --- |
| 0    | `Uncontrolled`               | 无                      | U   | U   | U   |
| 1    | `ProtectedGroup`             | Group                   | D   | D   | P   |
| 2    | `PermissiveGroup`            | Group                   | D   | D   | Y   |
| 3    | `CnCircularRightTurn`        | Group，适用的普通圆形灯 | Y   | D   | Y   |
| 4    | `DirectionalRightProtected`  | Group，适用的右转方向灯 | D   | D   | P   |
| 5    | `DirectionalRightPermissive` | Group，适用的右转方向灯 | D   | D   | Y   |

当前 `MovementInput` 只有引道身份，没有转向分类。本次在两个正式来源及共同模块中
增加可选机动方向（`ManeuverDirection`），闭合值为 `Straight=0, Left=1, Right=2,
UTurn=3`；缺字段表示未声明，不能推断为直行或右转。方向是 Movement 的非身份属性，
不参与 StableId；对应 LFCA 3/6 既有 Movement 表新增可选 tag 7 `turnDirection:U8`。

无信号绑定时不读取虚构灯色，表中三个 U 只表示结果与灯色无关。后三项必须绑定
显式 `Right` 的 Movement；编译器核对机动元数据和规则输入，不从空间角度猜测。圆形灯与
方向灯是该 Gate 的受检解释声明，SignalGroup 仍只提供 indication；引用同一 group
不等于可以把不适用该方向的箭头灯指定为右转解释。

`CnCircularRightTurn` 是本轮已核实红灯右转规则的有限解释类型，不声称完整中国
交通法行为。黄灯仍采用已接受的限制性 PreGate 策略；已 crossing 的车辆按 reservation
继续清空，不倒退或重新施加入口停止线。熄灭、闪烁、故障信号与现场交警指挥未进入
当前三灯态合同，不能用 None、未知 enum 或缺失 group 冒充这些状态。

`prohibition:u8` 为 `None=0 | Always=1 | OnRed=2`。None 是显式输入；缺字段不能
补 None。Always 拒绝所有入口候选，OnRed 只在绑定 group 为红时拒绝，Uncontrolled
不得使用 OnRed。永久 Access deny 继续按其原有优先级和 specificity 求值，不能被
门合规规则覆盖。实际标志条件由官方编制来源明确给出，不运行文本识别器。

### 3.2 解析与候选合成

对静态 Access 允许到达的 `(Gate, VehicleProfile)` 恰好解析一条门合规规则，对可能
进入 Conflict passage 的 `(stream, VehicleProfile)` 恰好解析一条通行流规则。
两类规则先执行 nearest-ancestor specificity；通行流规则随后按 priority 选择，最高
优先级仍有多重匹配则拒绝。门规则没有 priority，同 specificity 多重匹配即拒绝。
不按声明顺序挑选；其余规则合法性、coverage-min 和 protected coherence 继续执行
联合设计 §6.2。

编译得到 `ResolvedGatePolicyCell` 与 `ResolvedStreamRule`：每个 cell 回指稳定规则
地址，并只含 typed ordinal、封闭枚举、整数和 flat ranges。pure Waiting 的门合规
求值仍有效，但没有 Conflict stream row 就没有法规 priority，不能伪造 0 priority。

运行时先用解释类型和 committed aspect 产生 signal/regulatory result，再组合 Access
及其他有效 deny。原有 `group_is_restrictive` 的红灯布尔结果不能先成为无法撤销的停止
约束，否则圆形红灯右转永远无法进入候选。这个修改只替换法规解释入口，不取消 leader、
Waiting、ParkingStop、RouteEnd 或 no-overlap 约束。

参考中国策略必须覆盖：普通圆形红灯右转有条件候选、右转箭头红灯拒绝、禁令拒绝、
同车道前车等候、被放行冲突车流优先、没有方向灯时转弯让直行及相对方向右转让左转。
规则配置的 priority/yield 集必须满足这一受测拓扑的相容性；provenance 标签不是法律
正确性的自动证明。没有建模的行人、非机动车仍是支持边界，不能伪造为零冲突输入。

## 4. LFCA 与共享根

### 4.1 唯一线格式增量

沿用 LFCA 的 FieldV1/TableV1 与分块 section；不新增对象 magic、嵌套对象编码或
第二套发布载体。LFCA 5 仍为八个 section，逻辑表总数从 33 变为 38；既有 Movement
表仅按 §3.1 扩展字段，新增五表如下。

| section/table | 名称                  | 顺序字段（从 tag 1 起连续）                                                                                                                                  |
| ------------- | --------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| 3/24          | `RightOfWayPolicySet` | `typedOrdinal:U32, stableId:StableId128, jurisdiction:Utf8, regulationVersion:Utf8, regulationSource?:Utf8, effectiveFrom?:U32, effectiveUntil?:U32`         |
| 4/2           | `PolicyEvidence`      | `policy:U32, key:Utf8, locator:Utf8, description?:Utf8`                                                                                                      |
| 4/3           | `PolicyGapProfile`    | `policy:U32, key:Utf8, parameterVersion:Utf8, minimumLeadGapMs:U64, minimumLagGapMs:U64, clearanceBufferMs:U64`                                              |
| 4/4           | `PolicyStreamRule`    | `policy:U32, key:Utf8, stream:U32, classes?:OrdinalVectorU32, priority:I32, yieldToStreams:OrdinalVectorU32, gapProfileKey?:Utf8, evidenceKeys:RecordVector` |
| 4/5           | `PolicyGateRule`      | `policy:U32, key:Utf8, gate:U32, classes?:OrdinalVectorU32, interpretation:U8, prohibition:U8, evidenceKeys:RecordVector`                                    |

`evidenceKeys` 的唯一 child row 为 `{tag1 key:Utf8}`，不嵌套 RecordVector。空 yield /
evidence 集必须编码为空向量，不能用缺字段代替；gap 的条件存在性按 §2.3 额外验证。
实体表按 typed ordinal，局部表按 `(policy ordinal, key 的 UTF-8 字节序)` 排序。
跨 chunk 延续同一全局顺序和唯一性，引用可以跨 chunk，不能以 chunk 为语义边界。

LFSM 增加 owner-local role `PolicyEvidence=33`、`PolicyGapProfile=34`、
`PolicyStreamRule=35`、`PolicyGateRule=36`：owner 是 policy StableId，member index
是该 owner 同类局部成员按 key 排序后的下标。来源路径保留实际字段位置；它不是
持久规则身份。局部引用由完整成员 payload 检查，不向既有只容纳稳定实体的 relation
tuple 填入假 StableId。LFSD 对 policy 及局部表的增删改产生 owner-keyed payload
变化，并重算两端规范内容；未知局部成员不静默跳过。

新的语义表位于 section 3/4，进入现行 NetworkRevisionId 对前六个 section 的规范
字节覆盖；Movement 字段同样被覆盖，派生算法不变。发布 provenance 的排除
规则不扩张成“法规规则不入摘要”。LFCA origin 继续绑定 exact bytes，LFSM/LFSD 继续
绑定相应两端的完整版本组合。LFCP 只绑定新的受检 LFCA/LFSM，不承载第二份策略正文。

### 4.2 编译阶段与共享数据

1. 来源准入：检查 closed shape、局部 key、值域与声明预算，失败不修改 builder。
2. HIR：绑定跨模块静态引用、法规来源和 class 层次，闭合所有局部引用。
3. MIR：计算可准入 profile 集，选择门规则/stream 规则；验证 totality、specificity、
   yield、priority cycle、protected coherence 和法规有效期形状。
4. LIR：冻结规范表、稳定排序和来源映射；发射 LFCA/LFSM/LFSD，并执行完整后发射检查。
5. 共享根构建：从受检 LFCA 重建只读解析表和 exact passage target ranges，完成当前
   builder 的语义闭合后一次 seal。Runtime 不读 compiler IR 或源码。

解析结果是 `(owner ordinal, profile ordinal)` 的稠密行与每 owner 的 CSR 范围，仅存
Access 允许的实际组合；不能分配全局 stream × profile × route 数组。循环生成、内存
预留、来源和派生计数均 checked，超过既有 CompileLimits/BuildLimits 失败，不随意
提高一百万静态实体基线的格式上限。LFCA 保存声明语义，builder 只派生当前根的执行表，
不存在同时持久化一份可与声明不一致的 resolved 副本。

| 持有者                  | 持有内容                                                                                                                 |
| ----------------------- | ------------------------------------------------------------------------------------------------------------------------ |
| `SharedNetworkRevision` | policy 身份/日期区间、不可变门/流解析表、gap 值、稳定规则归因、exact target ranges                                       |
| 每世界 route table      | #559 occurrence、Gate coverage、分段距离和后继边界、Waiting dependency operands                                          |
| `TrafficWorld`          | 选定策略与日期、依 fixedDelta 派生的阈值、firstEligibleTick、reservation、occupancy、last-clear、ledger、frontier 和输出 |

同一根可被不同 fixedDelta 的 world 共享。`requiredLeadMs` 和 proof horizon 必须在
world 安装时 checked 派生，不能把某个世界的步长写进共享根。world 只保留选中策略的
派生阈值，不复制静态规则表。

## 5. 世界和场景绑定

`TrafficWorld::install` 的单一入口在现行参数后增加 `WorldPolicySelection`，不提供
安装后 setter。保留 `CommittedNetworkSource` 与共享根的修订一致性检查：

```text
install(revision, config, source, world_id, policy_selection)
WorldPolicySelection = NotRequired | Pinned(PolicyPin)
PolicyPin { policy: RightOfWayPolicySetId, regulationDate: RegulationDate }
```

`RightOfWayPolicySetId` 是 policy StableId 的有类型包装，不是根内 dense handle。
`PolicyPin` 构造器不接受未经验证的 ordinal 或可执行 closure。install 用给定根解析
StableId，验证版本、有效期、法规来源相容性、完整解析表和依 fixedDelta 的 gap 加法。
所有检查在 world 发布前完成，失败不返回半安装世界。现有唯一 install 签名直接修改，
不保留旧签名或隐式 protected-only 默认。

`NotRequired` 仅当整个根没有 ManeuverGate、ConflictZone、ParticipantStream 时合法；
存在 Waiting 必然有 Gate，因此也需要显式策略。这使无门自由道路仍有明确的无策略
表示，又避免只检查当前已注册路线而在后来注册时悄悄启用另一套语义。

走廊 catalog 升为 0.4，顶层增加必填 `policy_selection` 闭合 tagged value：
`not_required` 不带其他字段，`pinned` 必带 policy 的规范 StableId 文本与
`regulation_date`。prepare 解析到 `PolicyPin` 后只调用上述 install；来源政策内容
仍来自同一 LFCA。所有现行带信号的例子由 generator 发射显式 protected-entry 策略并
pin，不能在场景库或 Adapter 内复制绿色放行算法。无 Gate 的例子显式 NotRequired。

读器只接受 catalog 0.4；旧 catalog/fixture 与生成器同步替换。外部宿主可以直接传
有类型 pin，不必采用走廊 catalog；Runtime 不因此新增 TOML/JSON 依赖。

## 6. 仲裁、持久状态与生命周期

固定步进仍按联合设计的阶段执行：tick-start committed state → route/leader/frontier
→ 门候选 → zone-local entitlement → stable single-writer 原子 bundle → motion →
crossing 与 tail-clear → 一次提交。policy priority 只排序已取得本地 entitlement 的
bundle；frontier 使用静态 passage cell 的 top-two distinct owners，不随动态 Route
复制。normal denial 与 tick error 的边界、first-error 和事件总序继续沿用联合设计。

安装、spawn、replacement、leave/rebind、restore/cutover 必须检查实际车型的空下游
容纳性。存在正式仲裁能力不等于可以把车辆直接生成在冲突区里：没有已提交 reservation
的 Active 候选必须在相关 coverage 的准入 Gate 之前，或全车身已经清空该 coverage。
位于两者之间而需要补造 crossing/claim 历史时拒绝。正常 step crossing 是新 reservation
的唯一创建路径；restore/rebind 只迁移可验证的既有 authority。

live reservation 按 vehicle + snapshot route occurrence + Gate/stream/passages
稳定地址定位；资源物理区间使用 edge StableId、route hop、整数 progress 和必要的
车尾/clearance 锚点。不能仅凭 Gate StableId 合并 repeated occurrence。grant 未发生
crossing 时不提交任何成员或 reservation；whole-vehicle lifecycle 释放必须同事务清理
Waiting、Conflict、downstream owner 与 route 引用。

### 6.1 快照分类

| 类别     | LFRS 5 / runtime state 5 合同                                                                                               |
| -------- | --------------------------------------------------------------------------------------------------------------------------- |
| 策略绑定 | selection tag；Pinned 保存 policy StableId 与 regulationDate；exact 内容仍由快照的 LFCA origin 绑定                         |
| 排序历史 | 每车当前 Gate occurrence 的 firstEligibleTick；None 与 tick 0 明确区分                                                      |
| 通行状态 | Clearing、reservation owner、acquired tick、Gate/passages 的稳定 occurrence 地址、committed downstream 区间                 |
| 冲突历史 | 有历史的 passage cell 的 lastClearTimeMs；无历史与时间 0 区分                                                               |
| Waiting  | 继续保存既有 membership、occupancy、counter 与 traversal；queue link 由历史重建                                             |
| 派生状态 | 不保存 tick-local grant、entitlement、top-two frontier、dense handles、target ranges、graph、scratch 或 latest output batch |

冲突 occupancy 从 reservation、车辆位置和 passages 重建，不能同时信任一份独立计数。
保存后捕获结构的语义字段全部参与 deterministic digest 7，派生内容不参与。
普通 restore 重建规则/route/ledger，核对每个 owner、时间、full-body footprint 和 wait-for
graph；存在两 owner 以上的 committed cycle、悬空 owner 或不合法历史即整体失败。

恢复后的 latest decision/event batch 为空，标识为没有新成功 tick；下一成功 tick 按
同一输入重建。无状态 batch 不能伪装为从档中复活的 grant；回放比较从一致 checkpoint
继续，必须比较每个后续成功 tick 的 decision/event/state。

### 6.2 修订切换

same-revision 保持 pin、日期及全部逻辑历史。cross-revision 仍以 LFSD 和现有完整根
事务切换；目标必须保留同一个被选 policy StableId、法域/法规版本和适用日期，且日期
在目标区间内。若需更换 policy identity 或适用日期，首版通过新建世界显式完成，
不增加独立 policy hot-swap 命令。

同一 policy 在目标路网中的 Gate/stream 规则可随道路编辑改变，内容由目标
NetworkRevisionId 绑定。未 crossing 的车辆从提交后的下一 tick 使用新解析表；既有
firstEligibleTick 只在同一 arrival occurrence 和 predicate 仍成立时保留，否则按联合
设计清除。任何这类目标相关规范化必须同时进入静默点的独立期望值构造和迁移增量追赶，
不能只修改候选状态并宣称 digest 一致。

已有 reservation 保留其既有 owner 和 acquired tick，不能按新规则重新授予。必须将
原 physical claims、passages/clearance、车辆全长和 Waiting 依赖精确映射到目标，验证
无新增未持有的 incompatible coverage、空间不足或 committed cycle；不能映射就整次
失败。没有 authority 的车辆若被目标新增冲突覆盖包在内部，同样拒绝，不补 grant。
删掉仍影响 lag check 的 last-clear 历史会丢失 gap 语义。无 live 引用的 cell 只有在无
历史，或静默点 elapsed 已不小于源/目标所选策略所有 gap profile 的
`minimumLagGapMs + clearanceBufferMs` 最大值时才可删除；空 profile 集的最大值为 0，
加法溢出或 future timestamp 拒绝。这样不要求永久保留已经失效的历史。该删除判定同样
进入独立期望值与增量追赶；保留下来的 cell 原样迁移历史。目标新增 cell 在不存在
occupant/reservation 时以无 clear 历史初始化。

以上目标规范化扩展现有描述符的迁移语义，因此描述符版本升级为 2，kind 名称仍为
`same_revision_restore` 和 `cross_revision_direct`；字段形状不新增暗含 policy 选择的
自由 JSON。晋升之前旧根、旧 pin、world/tick/time、命令游标和输出保持不变。

## 7. 参考参数与实测口径

参考 profile 的 key/parameterVersion 与三个整数值通过普通策略局部成员发射，不是
隐藏在 Runtime 的默认常量。首次推荐数值在正式 solver 上通过受控场景校准后与源模块
一起交付；设计阶段不以未经测量的数字宣称产品通过。算法单测使用显式人工 profile
覆盖 equality、+1 ms 等边界，不把测试参数冒充推荐参数。

必须报告每个场景的实际车长/间距、需求、固定步长、观察长度、通过车辆数、等待分布、
拒绝原因和稳定排队情况。保护性算法可能导致次要车流长等，不能为了通过通行量用例
引入等待提权、同 tick release 复用或不可证明时放行。

性能取证沿用当前产品基线的 workload/hardware 分类。一万报告完整 tick p50/p95、
仲裁增量、retained bytes 和暖机后分配；十万报告正确性、visited passages、top-two
cells/bytes、claim/query/collision 与 wait-for node/edge/visit counts。开发机数据不
替代 P10 认证。最终跨层场景和 Adapter 收口仍由 #285 承担。

## 8. 原子版本矩阵

下表是本实施候选选择的唯一切换组合。仅在完整 #284 交付时替换现行 writer/reader；
本文审阅本身不修改代码常量、不安装新格式，也不接受跨行混搭。

| 轴                             | 当前  | #284  | 原因                                           |
| ------------------------------ | ----- | ----- | ---------------------------------------------- |
| Identity encoding              | 1     | 1     | 编码算法不变                                   |
| Identity registry              | 3     | 4     | 新 policy 实体和 key 标签                      |
| LFCA / canonical format        | 4     | 5     | 一实体、四局部表、机动方向和策略语义           |
| constraint contract            | 2     | 3     | 门解释与组合资源约束                           |
| static execution contract      | 4     | 5     | policy 解析与 conflict 执行输入                |
| LFSM                           | 3     | 4     | 新 owner-local 角色和实体登记                  |
| LFSD                           | 3     | 4     | policy 局部 payload 增删改闭合                 |
| LFCP                           | 2     | 2     | descriptor 形状不变，精确绑定新版 LFCA/LFSM    |
| NetworkRevision derivation     | 1     | 1     | 现有规范静态覆盖算法；新表和契约值进入现有输入 |
| chunked / singleton section    | 2 / 1 | 2 / 1 | 不改 framing、chunk 或 field 编码              |
| Road Editing schema / frontend | 3 / 3 | 4 / 4 | 正式来源新增 policy 声明                       |
| Synthetic frontend             | 4     | 5     | 同构 policy 声明与机动方向                     |
| LFRS / runtime state           | 4 / 4 | 5 / 5 | pin、reservation、Clearing 和历史              |
| deterministic digest           | 6     | 7     | 新增语义字段与目标规范化                       |
| cutover descriptor             | 1     | 2     | 新策略、冲突历史和规范化语义                   |
| corridor catalog               | 0.3   | 0.4   | 必填 policy selection                          |

几何计算和浮点量化算法未变，其独立 geometry semantics 版本保持不变。

旧 LFRS、LFCA、LFSM、LFSD、Road Editing 与 catalog reader 同切片删除或替换；
不写转换器、不双写、不用 feature flag 选择旧路径。重新生成参考制品与 codegen，
补齐新 Identity known vectors；不会把 LFCP 版本未变误解成允许其绑定旧 LFCA。

## 9. 实施验收与变更面

| 验收组           | 必须证明的结果                                                                                                             |
| ---------------- | -------------------------------------------------------------------------------------------------------------------------- |
| 编制与正式发布   | 两前端同语义同 canonical 输出；未知字段/枚举、重复/悬空成员、预算、版本和失败后重试闭合；LFCA/LFSM/LFSD/LFCP exact binding |
| 策略解析         | totality/specificity、实际允许 profile、exact target ranges、coverage-min、protected coherence；声明排列不改变解析         |
| 中国右转         | 圆形红灯候选且仍让行；方向红灯/禁令拒绝；同车道排队与下游阻塞；错误 signal binding 不被默认为无控制                        |
| 仲裁与运动       | entitlement、repeated occurrence 自排除、ETA/lead/lag 边界、物理 span 和 SCC；最终 crossing 与 grant/reservation 对应      |
| 持久化与生命周期 | 所有 owner 原子释放；restore/replay；same/cross revision；新增覆盖不补造历史；policy drift 和错误历史拒绝                  |
| 确定性与资源     | stable permutations 下 state/decision/event 相同；checked 失败零提交；无暖机后稳态分配；一万/十万实测                      |
| 原子接管         | 全部正式路径已接通后删除 ConflictRuntimeUnavailable；没有单独绕过 guard 的生产入口或测试特权                               |

核心变更面为 static-contract 登记和值、compiler 两前端与 HIR/MIR/LIR/emitter、format
校验与语义差异、static-network builder/共享表、runtime 安装/route/tick/Waiting/
生命周期/snapshot/cutover，以及 scenario/generator 的显式 pin。Spatial 不获得行为
职责；Adapter 只接入新 install 参数和读取结果，不新增法规执行副本。

文档验证运行 Markdown 表格与本地链接检查；生产实现还须满足仓库对应 Rust、codegen、
制品和场景测试。设计候选的检查通过不等于 runtime 已实现、G2 已记录或 #284 已完成。
