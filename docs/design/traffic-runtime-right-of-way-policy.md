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

每条门合规规则与通行流规则必须具有可追溯依据：要么继承策略级非空
`regulation.source`，要么有非空 `evidenceKeys`，且每个 key 都解析到该策略的非空
locator；两者同时缺失即编译失败。仅在策略中存在未被规则引用的 evidence 不满足
要求。策略级来源须适用于继承它的规则，特定标志或局部限制可另附规则依据。
法规解释引用对应版本的法规或条款；工程参考与合成测试策略可以引用项目版本化设计
或 fixture 说明，并如实标识其工程性质，不伪称法规。此检查验证来源存在和引用闭合，
不声称自动证明法义正确，也不联网抓取或验证页面存活。

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

灯型声明在同一静态根的整个 Gate 范围内必须一致，不能按 policy 或车型改变物理事实。
编译器汇总所有策略中指向该 Gate 的门规则，代码 3 声明圆形灯，代码 4/5 声明右转
方向灯；两类声明并存即拒绝，即使分属不同 class、策略或被 specificity 遮蔽。
代码 1/2 不声明灯型，也不能覆盖已有声明；代码 0 与 Group 绑定不相容，仍按绑定
检查拒绝。Protected/Permissive 与车型禁令可以不同，均不改变 Gate 的物理灯型。
该校验独立于 protected coherence，在共同编译管线及受检 LFCA 的共享根构建中执行；
不新增灯具实体或另一份可与规则矛盾的持久化灯型表。

`CnCircularRightTurn` 是本轮已核实红灯右转规则的有限解释类型，不声称完整中国
交通法行为。黄灯仍采用已接受的限制性 PreGate 策略；已 crossing 的车辆按 reservation
继续清空，不倒退或重新施加入口停止线。熄灭、闪烁、故障信号与现场交警指挥未进入
当前三灯态合同，不能用 None、未知 enum 或缺失 group 冒充这些状态。

`prohibition:u8` 为 `None=0 | Always=1 | OnRed=2`。None 是显式输入；缺字段不能
补 None。Always 拒绝所有入口候选，OnRed 只在绑定 group 为红时拒绝，Uncontrolled
不得使用 OnRed。永久 Access deny 继续按其原有优先级和 specificity 求值，不能被
门合规规则覆盖。实际标志条件由官方编制来源明确给出，不运行文本识别器。

### 3.2 解析与候选合成

对每份 policy 分别解析：静态 Access 允许到达的 `(Gate, VehicleProfile)` 恰好选择
一条门合规规则，可能进入 Conflict passage 的 `(stream, VehicleProfile)` 恰好选择
一条通行流规则；不能用另一份策略的规则补齐缺项。
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
evidence 集必须编码为空向量，不能用缺字段代替；evidence 为空仅在满足 §2.2 策略级
来源继承时合法。gap 的条件存在性按 §2.3 额外验证。共同编译管线与受检 LFCA 的共享根
构建均检查这些跨表约束，不因线格式允许空向量而跳过规则来源验证。
实体表按 typed ordinal，局部表按 `(policy ordinal, key 的 UTF-8 字节序)` 排序。
跨 chunk 延续同一全局顺序和唯一性，引用可以跨 chunk，不能以 chunk 为语义边界。

LFSM 增加 owner-local role `PolicyEvidence=33`、`PolicyGapProfile=34`、
`PolicyStreamRule=35`、`PolicyGateRule=36`：owner 是 policy StableId，member index
是该 owner 同类局部成员按 key 排序后的下标。来源路径保留实际字段位置；它不是
持久规则身份。局部引用由完整成员 payload 检查，不向既有只容纳稳定实体的 relation
tuple 填入假 StableId。LFSD 4 的局部成员变更使用 §4.3 的专用表、稳定值投影与
两端闭合规则；不得把局部成员伪装成 EntityChange 的稳定 subject 或关系 target。

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

解析结果以 `(policy ordinal, owner ordinal, profile ordinal)` 唯一定位，owner 在门表
中是 Gate、在流表中是 stream；CSR 先按 policy 划分，再按 owner 划分 profile 行。
每个流规则的 exact yield-target-cell ranges 归属同一 policy/stream/profile 行，目标
车型的 effective priority 也只取该 policy 的解析结果。世界安装时绑定所选 policy 的
只读范围；规则归因和派生阈值使用同一 policy，不存在跨策略 fallback。

仅保存各策略下 Access 允许的实际组合；不能预分配全局 policy × owner × profile ×
route 数组。循环生成、内存预留、来源和派生计数均 checked，按实际解析行、CSR 和
target ranges 计入既有 CompileLimits/BuildLimits，超限失败，不随意提高一百万静态
实体基线的格式上限。不同世界不复制这些静态表。LFCA 保存声明语义，builder 只派生
当前根的执行表，不存在同时持久化一份可与声明不一致的 resolved 副本。

| 持有者                  | 持有内容                                                                                                                 |
| ----------------------- | ------------------------------------------------------------------------------------------------------------------------ |
| `SharedNetworkRevision` | policy 身份/日期区间、不可变门/流解析表、gap 值、稳定规则归因、exact target ranges                                       |
| 每世界 route table      | #559 occurrence、Gate coverage、分段距离和后继边界、Waiting dependency operands                                          |
| `TrafficWorld`          | 选定策略与日期、依 fixedDelta 派生的阈值、firstEligibleTick、reservation、occupancy、last-clear、ledger、frontier 和输出 |

同一根可被不同 fixedDelta 的 world 共享。`requiredLeadMs` 和 proof horizon 必须在
world 安装时 checked 派生，不能把某个世界的步长写进共享根。world 只保留选中策略的
派生阈值，不复制静态规则表。

### 4.3 LFSD 4 策略局部成员变更

本节是 LFSD 4 的封闭增量，复用[规范制品格式](portable-canonical-artifact.md) §2、
§5 的 framing、两端绑定及既有六节；仅本节列出的登记和字段分类改变。
`magic="LFSD"`、`semanticDiffFormatVersion=4`，七节按 kind `1..7` 排列，逻辑表总数为 7。
新增 section `0x0007 PolicyLocalChanges`，其中唯一 table 为
`0x0001 PolicyLocalChange`（策略局部成员变更）；`sectionFormatVersion=2`、
`tableSchemaVersion=1`。首节偏移为 `32 + 7 * 24 = 200 (0x00c8)`，沿用分块目录、
chunk digest、RowV1/FieldV1 及零 flags。无成员变化时第七节仍存在，表以零 chunk
表示；不能省略该节或写空 TableV1。

#### 4.3.1 行字段、身份与操作

以下为外层变更行的完整字段集合，未登记 tag、类型或 enum 一律拒绝。R 为必需，
O 的存在性由后表唯一决定，不接受额外 local index、fieldTag 或 subjectStableId。

| tag | 字段                  | 类型        | 存在性                                                  |
| --- | --------------------- | ----------- | ------------------------------------------------------- |
| 1   | `changeKind`          | U8          | R；`0=Add, 1=Remove, 2=Modify`                          |
| 2   | `ownerPolicyStableId` | StableId128 | R；对应侧必须解析为 EntityKind 24                       |
| 3   | `memberKind`          | U8          | R；`0=Evidence, 1=GapProfile, 2=StreamRule, 3=GateRule` |
| 4   | `memberKey`           | Utf8        | R；复用 §2.2 编制 key 值域及上限                        |
| 5   | `beforeValue`         | Bytes       | O；完整规范成员值 RowV1                                 |
| 6   | `afterValue`          | Bytes       | O；完整规范成员值 RowV1                                 |

成员键 `K = (ownerPolicyStableId, memberKind, memberKey)` 不包含内容摘要、来源位置或
根内 ordinal。key 使用原始 UTF-8 字节，不做大小写折叠或 Unicode 归一化；编码为
普通 Utf8 FieldV1，其 byte length 来自字段头，不增加第二个字符串长度前缀。
memberKind 是本节自有封闭代码，不能拿 LFSM role 33–36 当它的 wire 值。

| base 中 K | target 中 K      | 唯一操作 | beforeValue | afterValue |
| --------- | ---------------- | -------- | ----------- | ---------- |
| 不存在    | 存在             | Add      | 禁止        | 必需       |
| 存在      | 不存在           | Remove   | 必需        | 禁止       |
| 存在      | 存在且规范值不同 | Modify   | 必需        | 必需       |
| 存在      | 存在且规范值相同 | 无行     | —           | —          |
| 不存在    | 不存在           | 无行     | —           | —          |

以 K 配对后才按 `(changeKind, ownerPolicyStableId, memberKind, memberKey)` 严格排序。
整数按数值序，StableId 和 key 按无符号字节字典序；顺序和唯一性跨 chunk 延续。
同一 K 在整个表中最多一行，不能用 Remove+Add 代替同键 Modify。改 key 或换 owner
意味着旧 K Remove、新 K Add；无 Move/Reconnect，插入其他成员造成的局部下标变化
不产生变更。空 Bytes、同值 Modify、缺侧/错侧载荷和重复 K 均拒绝。

#### 4.3.2 四种完整规范成员值

每个 before/after Bytes 精确包含一个 RowV1，包括标准 rowByteLength、fieldCount
及零 reserved，不包含 TableV1 或其他封套，无尾随字节。使用下表唯一 schema；
tag 沿用 LFCA 局部行的 `3..`，省略原 tag 1 的 policy ordinal 和 tag 2 的 key，
二者由外层 K 表达。类型名称均为现有 FieldV1 登记，`?` 表示可选，其余必需。

| memberKind   | 唯一字段（`tag:name:type`）                                                                                                    |
| ------------ | ------------------------------------------------------------------------------------------------------------------------------ |
| 0 Evidence   | `3:locator:Utf8, 4:description:Utf8?`                                                                                          |
| 1 GapProfile | `3:parameterVersion:Utf8, 4:minimumLeadGapMs:U64, 5:minimumLagGapMs:U64, 6:clearanceBufferMs:U64`                              |
| 2 StreamRule | `3:stream:Bytes, 4:classes:Bytes?, 5:priority:I32, 6:yieldToStreams:Bytes, 7:gapProfileKey:Utf8?, 8:evidenceKeys:RecordVector` |
| 3 GateRule   | `3:gate:Bytes, 4:classes:Bytes?, 5:interpretation:U8, 6:prohibition:U8, 7:evidenceKeys:RecordVector`                           |

有类型稳定引用的 Bytes 精确复用 `StableRefV1 = entityKind:u16_le || stableId[16]`，
共 18 bytes：stream 必须为 ParticipantStream，gate 必须为 ManeuverGate。
classes 和 yieldToStreams 的 Bytes 为 `count:u32_le || StableRefV1[count]`；前者
只允许 ParticipantClass，后者只允许 ParticipantStream，均按 StableId 字节序严格
递增、无重复。必须先用对应侧 LFCA 的 Identity 表解析 ordinal，再编码和排序；
不把两侧相同数字的 ordinal 当成同一引用。

evidenceKeys 复用唯一 child row `{tag1 key:Utf8}`，按 key 原字节严格排序、无重复。
gapProfileKey、evidence key 都是同一 ownerPolicy 下的局部引用，须在对应侧完整
LFCA 中存在。classes 缺失表达 fallback，显式空集拒绝；空 yield/evidence 仍编码
count=0 的向量，gap 的条件存在性及来源继承按 §2.2–§2.3 检查。未知 enum、空的
必填 token 和整数值域等约束沿用声明合同，不能因处于 Bytes 中而放宽。

所有整数小端，字段 tag 严格递增；字符串、向量和引用均只采用以上规范形式。
字节投影包含证据描述、参数版本及规则全部语义字段；只改变 locator、gap 数值或
prohibition 也必须能观察到。语义引用相同但两根 ordinal 排列不同，投影字节必须相同。

#### 4.3.3 与既有变更表的排他分工

| LFCA 所有者/字段                                   | 唯一 LFSD 表与操作                                                      |
| -------------------------------------------------- | ----------------------------------------------------------------------- |
| RightOfWayPolicySet 实体新增/删除                  | 既有 EntityChange Add/Remove，保存所在侧完整实体 RowV1；不内嵌局部成员  |
| 保留 policy 的 tag 3–7（法域、版本、来源、有效期） | 既有 StaticRuleChange Modify；按实际字段存在性保存 SemanticFieldValueV1 |
| policy tag 1/2（typed ordinal/StableId）           | Identity/derived；不产生字段 Modify                                     |
| 四类局部表全部成员及字段                           | 仅 PolicyLocalChange；每个变化 K 恰好一行完整前后值                     |
| 保留 Movement 的可选 tag 7 turnDirection           | 既有 EntityChange Modify；按实际字段存在性保存 U8 SemanticFieldValueV1  |

policy 内不虚构成员数组字段或成员 StableId，也不为局部行的 stream/gate/classes/
yield/evidence 引用再发射 RelationChange。LFSM role 33–36 仅用于来源定位，不因此
进入 LFSD 的稳定实体关系 tuple。既有其他字段和 relation role 的分类保持不变。

Genesis 必须为每个 policy 实体发射 Entity Add，并为其全部局部成员发射独立的
PolicyLocal Add；Artifact 中整个 policy 新增/删除也必须逐项发射其成员 Add/Remove。
父实体记录不能代替局部成员记录；跨表按两端完整事实验证，不把表顺序当逐步应用命令。
同键规则引用发生变化是一次完整 Modify，不再重复记成员内部字段变化。policy 身份
改变时沿用旧实体及成员 Remove、新实体及成员 Add，不猜测 lineage。

#### 4.3.4 两端闭合与资源约束

1. 先按现行绑定合同验证 base/target exact digest、length、NetworkRevisionId 和共有
   Identity 前像。LFSD 4 的目标及非 Genesis 基线均为 LFCA 5，身份/约束/执行合同
   使用 §8 同一组合；不支持 LFCA 4→5 的跨格式 diff。Genesis 保留原四项零 base 值。
2. emitter 与独立 checker 分别从两侧 LFCA 5 的 section 4、table 2–5 局部表和
   Identity 表重建 K、稳定成员值及必需操作；Genesis 将 base 成员集视为空。
   不以 LFSD 自报的键、载荷或来源位置决定哪一侧应当存在成员。
3. 实际记录必须与独立重算结果逐键、逐操作、逐字节相等。完整检查所有 owner 和成员，
   包括未被当前世界选择的 policy；漏一行、多一行、换 kind/owner、载荷篡改或漏掉
   整个第七节都拒绝。其他表的排他分类同时复核，不能以在另一表重复表达来补缺项。
4. 外层行与每个完整载荷必须受现有单 chunk 16,777,216 bytes、65,536 行、字段和
   向量上限约束。内层 UTF-8、RecordVector 及 Bytes 中的稳定引用向量分别计入对应
   字符串/向量累计预算；before/after 都收费，Bytes 封装不绕过语义预检。checked
   长度、count×18、来源和派生计数在分配前验证；超限按现行失败原子性处理。
5. 局部载荷逐成员发射，不把整份 policy 拼成一个巨型 Bytes 或新增全量历史。
   独立投影与扫描 scratch 计入既有编译/后发射预算，Runtime tick 不读取 LFSD 表。
   未完成本节闭合的差异制品不得用于发布候选或跨修订认证；目标根仍由完整 LFCA
   重建，差异表本身不授予迁移权限。

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

运行时仍使用 #559 的 `(ParticipantStreamOrdinal, passageLocalIndex)` 地址；该地址
仅在当前根有效，不写入存档或原样用于跨修订匹配。冲突通行段持久定位值
（`ConflictPassageLocator`）固定为两个 `StableId128`：

```text
ConflictPassageLocator { participantStreamStableId, conflictZoneStableId }
```

LFCA 已保证同一 `(ParticipantStream, ConflictZone)` 至多一条 passage，因此 locator
能在绑定根中唯一找到既有局部关系；它不是新增实体或第二个运行时地址。快照的 LFCA
origin 固定其完整 entry/exit 与所属路径。restore 先解析两种 StableId 并验证关系
确实存在，再派生当前根的 local index，不从几何或局部下标猜测另一条 passage。

live reservation 还保存 `snapshot_vehicle_id`、`snapshot_route_id`、本次 maneuver
的 entry route edge index 和 Gate StableId，再携带所持有 passages 的 locator；
结合重编译路线核验该次 occurrence、entry/clearance 和车辆位置。只按 stream/zone
不能区分循环路线中的重复出现。资源物理区间使用 edge StableId、route hop、整数
progress 和必要的车尾/clearance 锚点。grant 未发生 crossing 时不提交任何成员或
reservation；whole-vehicle lifecycle 释放必须同事务清理 Waiting、Conflict、
downstream owner 与 route 引用。

### 6.1 快照分类

| 类别     | LFRS 5 / runtime state 5 合同                                                                                               |
| -------- | --------------------------------------------------------------------------------------------------------------------------- |
| 策略绑定 | selection tag；Pinned 保存 policy StableId 与 regulationDate；exact 内容仍由快照的 LFCA origin 绑定                         |
| 排序历史 | 每车当前 Gate occurrence 的 firstEligibleTick；None 与 tick 0 明确区分                                                      |
| 通行状态 | Clearing、reservation owner、acquired tick、Gate/passages 的稳定 occurrence 地址、committed downstream 区间                 |
| 冲突历史 | 以 ConflictPassageLocator 键控的 ConflictLagReference；包含历史类别及时间，无历史与时间 0 区分                              |
| Waiting  | 继续保存既有 membership、occupancy、counter 与 traversal；queue link 由历史重建                                             |
| 派生状态 | 不保存 tick-local grant、entitlement、top-two frontier、dense handles、target ranges、graph、scratch 或 latest output batch |

冲突滞后基准（`ConflictLagReference`）是每个 cell 的单份 tagged value：
`NoHistory | ActualClear(timeMs:u64) | CutoverFloor(timeMs:u64)`，tag 分别为 0/1/2，
仅后两项带时间。前者不检查滞后间隙；后两者均以 timeMs 作为联合设计 §6.5 的
referenceTimeMs，按同一 checked 比较判定。`ActualClear` 保存实际 `lastClearTimeMs`；
`CutoverFloor` 是 §6.2 的保守约束起点，不宣称发生过实际清空，也不生成 crossing/
clear 事件或增加计数。其后的真实 tail-clear 用 `ActualClear(postStepTimeMs)` 替换
旧基准，不维护第二份全量历史。类别和时间均参与 snapshot、digest 与 journal；
restore 拒绝 future timestamp，并保留 CutoverFloor，不能把它降级为 NoHistory。
新建且没有既往运行历史的世界使用 NoHistory；迁移不适用这个初始化理由。
快照只写非 NoHistory 行，按 locator 的 stream StableId、zone StableId 字节序严格
排序并拒绝重复；单根热表仍按静态 cell 保存一个基准，不另存稳定标识副本或历史轨迹。

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

跨修订先绑定 LFSD 的完整 base/target LFCA，按 locator 中 stream 与 zone 的 StableId
定位两侧关系，再核对 role 31 及 `ParticipantStream.passages` tag 5 的完整 before/
after 投影（见[规范制品格式 §5.3](portable-canonical-artifact.md#53-字段变化分类)）。
仅 local index 重排不影响匹配；entry/exit 的解释必须使用各自根的 ManeuverPath
edge occurrence 序列，不能只比较仍可能相同的 boundaryIndex/pathEdgeIndex 数字。
相同 locator 不足以证明语义连续，还须核验路径、派生 admission Gate、物理区间及
本次 route occurrence。live reservation 无法保持原 physical claims 与完整清空条件
时拒绝直移，不能把同键的新范围自动变成既有 authority。

已有 reservation 保留其既有 owner 和 acquired tick，不能按新规则重新授予。必须将
原 physical claims、passages/clearance、车辆全长和 Waiting 依赖精确映射到目标，验证
无新增未持有的 incompatible coverage、空间不足或 committed cycle；不能映射就整次
失败。没有 authority 的车辆若被目标新增冲突覆盖包在内部，同样拒绝，不补 grant。
删掉仍影响 lag check 的基准会丢失 gap 语义。无 live 引用的 cell 只有在 NoHistory，
或从 ActualClear/CutoverFloor 到静默点的 elapsed 已不小于源/目标所选策略所有 gap profile 的
`minimumLagGapMs + clearanceBufferMs` 最大值时才可删除；空 profile 集的最大值为 0，
加法溢出或 future timestamp 拒绝。这样不要求永久保留已经失效的历史。该删除判定同样
进入独立期望值与增量追赶。语义连续的 cell 原样迁移 ConflictLagReference，不能因
仅下标重排或再次切换而重置计时；同键但物理范围等语义不连续、且已通过无 live
authority 校验的 cell 与新增 cell 一样处理。

目标新增或无法继承可信基准的 cell，在已验证无 occupant/reservation 后，必须以
最终静默提交的模拟时刻 `T_commit` 设置 `CutoverFloor(T_commit)`。不能用 Prepare
时刻、后台追赶时刻或宿主墙钟提前开始计时。仅这些 cell 施加保守间隙，其他 cell
继续使用原基准；间隙为零自然无需额外等待。候选准备/追赶只记录哪些 cell 需初始化，
在静默点取同一源世界时间，分别用于候选最终化和从源状态构造的独立期望值；完成
digest 复核后才一次发布。快照/恢复保留该基准，失败丢弃候选，不给旧世界增加历史
或事件。此规则不补造真实清空历史，不要求保存或回放全路网旧轨迹。

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

| 轴                             | 当前  | #284  | 原因                                              |
| ------------------------------ | ----- | ----- | ------------------------------------------------- |
| Identity encoding              | 1     | 1     | 编码算法不变                                      |
| Identity registry              | 3     | 4     | 新 policy 实体和 key 标签                         |
| LFCA / canonical format        | 4     | 5     | 一实体、四局部表、机动方向和策略语义              |
| constraint contract            | 2     | 3     | 门解释与组合资源约束                              |
| static execution contract      | 4     | 5     | policy 解析与 conflict 执行输入                   |
| LFSM                           | 3     | 4     | 新 owner-local 角色和实体登记                     |
| LFSD                           | 3     | 4     | 七节/七表，PolicyLocalChange 完整增删改与两端闭合 |
| LFCP                           | 2     | 2     | descriptor 形状不变，精确绑定新版 LFCA/LFSM       |
| NetworkRevision derivation     | 1     | 1     | 现有规范静态覆盖算法；新表和契约值进入现有输入    |
| chunked / singleton section    | 2 / 1 | 2 / 1 | 不改 framing、chunk 或 field 编码                 |
| Road Editing schema / frontend | 3 / 3 | 4 / 4 | 正式来源新增 policy 声明                          |
| Synthetic frontend             | 4     | 5     | 同构 policy 声明与机动方向                        |
| LFRS / runtime state           | 4 / 4 | 5 / 5 | pin、reservation、Clearing 和历史                 |
| deterministic digest           | 6     | 7     | 新增语义字段与目标规范化                          |
| cutover descriptor             | 1     | 2     | 新策略、冲突历史和规范化语义                      |
| corridor catalog               | 0.3   | 0.4   | 必填 policy selection                             |

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
| 持久化与生命周期 | 所有 owner 原子释放；restore/replay；same/cross revision；新增覆盖仅初始化保守基准；policy drift 和错误历史拒绝            |
| 确定性与资源     | stable permutations 下 state/decision/event 相同；checked 失败零提交；无暖机后稳态分配；一万/十万实测                      |
| 原子接管         | 全部正式路径已接通后删除 ConflictRuntimeUnavailable；没有单独绕过 guard 的生产入口或测试特权                               |

核心变更面为 static-contract 登记和值、compiler 两前端与 HIR/MIR/LIR/emitter、format
校验与语义差异、static-network builder/共享表、runtime 安装/route/tick/Waiting/
生命周期/snapshot/cutover，以及 scenario/generator 的显式 pin。Spatial 不获得行为
职责；Adapter 只接入新 install 参数和读取结果，不新增法规执行副本。

必须补以下定向验收，不扩展成全字段穷举矩阵：

- 同 Gate 的不同车型、不同策略声明圆形/方向灯冲突时，两前端及共享根构建均拒绝；
  相同灯型下的车型禁令差异合法。
- 同根两个世界选择不同策略，对相同 Gate/stream/profile 得到各自门结果、priority、
  yield targets 和 gap；颠倒声明或安装顺序不改变结果，缺项不从另一策略补齐。
- passage 插入导致的局部下标重排可保持原 reservation；同键锚点/路径改变不能误绑；
  同一路线重复 occurrence 不合并，悬空 locator 拒绝。
- 新增空 cell 的滞后约束从最终切换时刻起算：假设测试间隙为 500 ms，切换前 100 ms
  才离开的旧车不能使其立刻放行；499 ms 拒绝、500 ms 仅通过 lag 检查。覆盖 Prepare
  后继续步进、基准为 tick 0、存档恢复、再次切换保持连续 cell 基准，以及失败零发布；
  不生成虚构 clear 事件。
- 策略级来源继承、规则级依据分别可满足来源要求；两者皆无或引用悬空失败，孤立
  evidence 不代替规则引用；工程 fixture 的版本化依据按同一正式入口验证。
- LFSD 4 四类成员各自的固定字节向量及 Add/Remove/Modify，包含只改 gap 数值、
  证据 locator、规则目标/禁令、可选字段存在性；同键修改不得记成 Remove+Add。
  覆盖改名、policy 整体增删、Genesis、纯 ordinal 重排不变、仅 policy 自身字段改变
  不伪造成员变化，以及 Movement tag 7 的分类。
- 独立 checker 拒绝漏掉成员/整节、错 owner/kind、错侧/同值载荷、重复 K（包括跨
  chunk）、错误稳定引用与格式轴；覆盖 Bytes 内层计数超限和失败后重试。验证声明
  顺序置换产生同一 canonical LFSD；不把两端摘要匹配当成变化清单完整的证明。

文档验证运行 Markdown 表格与本地链接检查；生产实现还须满足仓库对应 Rust、codegen、
制品和场景测试。设计候选的检查通过不等于 runtime 已实现、G2 已记录或 #284 已完成。
