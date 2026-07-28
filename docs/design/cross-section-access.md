# 多模式横断面与准入分层

**文档状态**: Accepted（#234 G1 冻结）<br>
**最后更新**: 2026-07-27<br>
**适用范围**: #234 冻结的多模式道路横断面 owner、RoadSection/LaneGroup/LaneEdge/设施带关系、FacilityKind/ParticipantClass/AccessRule 分层、时间/地区 overlay、identity/authority/validation 与后续最小 production 边界<br>
**实现状态**: 静态模型已由 #262 生产化（Core registries、Data/schema 与 Traffic `0.9`，schema 已发布并经 live 验证；含 (class, Route) 绑定期静态准入校验）；时变规则与 FacilityBand target 规则仍由 capability guard 结构化拒绝

**关联文档**:

- `../architecture.md`
- `../roadmap.md`
- `../adr/0001-project-scope.md`
- `../adr/0005-core-identity-and-handle-model.md`
- `../adr/0007-traffic-data-crate-and-loader-boundary.md`
- `../adr/0008-pre-1.0-data-format-version-policy.md`
- `../adr/0009-signal-indication-gate-and-policy-separation.md`
- `../adr/0013-engine-neutral-spatial-geometry-and-length-authority.md`
- `../adr/0017-static-road-junction-maneuver-and-gate-identity.md`
- `road-junction-model.md`
- `lane-graph.md`
- `spatial-geometry.md`
- `data-format.md`
- GitHub: #227、#228、#229、#234、#237、#262

## 1. 目标、状态与非目标

### 1.1 目标

本文冻结：

- 横断面总 owner 判断：`RoadCorridor` 作为非方向性结构组合，方向性 `RoadSection`
  不足以单独承担；
- `RoadCorridor`、`RoadSection`、`LaneGroup`、`LaneEdge` 与非可遍历 `FacilityBand`
  的职责、identity 与关系；
- `FacilityKind`（物理设施身份）与 `AccessRule`（参与者准入）的显式分离；
- 可扩展 `ParticipantClass` 分类法及其与 VehicleProfile/Route/ManeuverPath 的关系；
- `AccessRule` 的 target、effect、参与者、时间窗口、法规 provenance 与确定性组合
  语义；
- 准入 overlay 对 LaneEdge/LaneGroup/RoadSection/ManeuverPath/facility 的引用方式，
  保持 ADR 0017 的全局 ManeuverPath traversal coherence；
- Core/Data/Spatial/Authoring/Adapter/Presentation 的 authority、external ID/handle、
  normalization、first-error、确定性与性能边界；
- 最小 production 实现 Issue 的拆分边界。

### 1.2 当前 production

| 范围           | Current production                                                                      |
| -------------- | --------------------------------------------------------------------------------------- |
| Traffic        | exact-current `0.10`（继承 0.9 横断面/准入片段，并含 #281 WaitingZone static）          |
| Lane topology  | LaneEdge 无类型；RoadSection/LaneGroup 已生产化为 LaneGraph 之上的结构 overlay          |
| 参与者分类     | ParticipantClass 已存在（数据声明、单继承）；VehicleProfile 必填 `participantClassId`   |
| 准入规则       | AccessRule 静态模型已存在；(class, Route) 绑定期校验；时变/FacilityBand target 规则拒绝 |
| 横断面横向几何 | 不冻结（宽度/偏移继续留在 Spatial/Adapter 外）                                          |

### 1.3 非目标

- 不在本文实现 Core、schema、Adapter、编辑器或示例。
- 不实现非机动车跟驰、行人行为、CrossingFacility、lane change、动态车道或
  conflict solver。
- 不冻结车道宽度、横向偏移、道路横坡或任何横断面几何；横断面只冻结横向
  **顺序** 拓扑。
- 不把 LaneFlow 扩张为专业交通工程仿真器或完整 SUMO-like 系统。
- 不修改 #229 已交付的 v0.9 最小 Junction/Movement/ManeuverPath 实现范围。
- 不引入动态拓扑；横断面与准入 registry 与 lane graph 一样按初始化后稳定处理。

## 2. 横断面总 owner 判断

### 2.1 判断：方向性 RoadSection 不足够，新增 `RoadCorridor`

方向性 `RoadSection` 无法回答三个横断面问题：

- 中央分隔带、两侧人行道与设施带不属于任何一个行车方向；
- 双向道路的横向顺序（从左到右的元素排列）是单一物理事实，拆进两个方向性
  RoadSection 会产生两份需要手工保持一致的 lateral order；
- authoring、validation 与 presentation 需要把一条物理道路当作一个整体单元。

因此新增 `RoadCorridor`：非方向性的道路结构组合，拥有唯一有序的 cross-section。
它遵循 ADR 0017 对 JunctionGroup 的同一先例——结构组合，非行为实体：

- 不成为 route planner、conflict solver 或 runtime availability owner；
- 不复制 RoadSection/LaneEdge 的拓扑与长度权威；
- 只提供稳定 identity、横向顺序与成员关系。

### 2.2 横断面元素

Cross-section 是 RoadCorridor 内**有序的横向元素序列**。元素为一等实体引用，
二选一：

- `RoadSection`：有方向、承载车道的可遍历分段；
- `FacilityBand`：非方向、非遍历的设施带（人行道、分隔带、绿化/设施带等）。

顺序语义：元素按 corridor 声明的参考方向**从左到右**排列，与
`spatial-geometry.md` §7 的正横向偏移约定（沿行驶方向左侧为正）一致。corridor
用 `referenceSectionId` 指向其某个成员 RoadSection 声明参考方向；不允许以
FacilityBand 或外部字符串推断方向。

横断面只冻结**顺序**；元素宽度、路缘位置、横向偏移不进入 Traffic schema，继续
由未来横向几何 G1 或 Adapter 表现层处理（对齐 ADR 0013：当前契约没有横向几何
权威层）。

## 3. 实体职责与关系

### 3.1 RoadCorridor

```text
RoadCorridor
  externalId
  referenceSectionId      -> 成员 RoadSection
  elements[]              -> ordered (sectionId | bandId)
```

- 非方向性结构组合；唯一 cross-section owner。
- 每个 RoadSection/FacilityBand 恰好属于一个 RoadCorridor；该关系构成完备
  所有者树（Complete Owner Tree），但 LaneGraph 中存在不属于任何 RoadSection 的
  LaneEdge 是合法的（横断面是 LaneGraph 之上的可选结构 overlay）。
- 同一 corridor 的 `elements[]` 内不得重复引用同一 section/band——cross-section
  是唯一有序横向序列，重复引用会让同一元素占据两个横向位置，与 §2.2 定义
  矛盾。
- **纵向共延伸不变量**：corridor 的所有元素覆盖同一纵向区段——接缝边界
  （§3.2.1）的整边界语义以此为准。与同向不变量同类：没有横向几何与
  stationing，Core 无法度量或比较元素的纵向范围（FacilityBand 甚至没有
  纵向数据），该不变量由 authoring 保证，几何校验由 tooling 或横向几何
  G1 补强，不进入 v1 Core validation。
- 不拥有单一 conflict solver、SignalController clock、route planner 或 runtime
  availability。

### 3.2 RoadSection

ADR 0017 冻结的术语获得生产语义：有方向的道路结构分段。

```text
RoadSection
  externalId
  kindId                  -> lane-bearing FacilityKind
  lanes[]                 -> ordered lane
```

每条 lane：

```text
lane
  edgeIds[]               -> 非空 ordered LaneEdge 链
  laneGroupId?            -> 可选 LaneGroup
```

规则：

- lane index 按 **corridor reference 方向**从左到右排序（index 0 = reference
  方向最左），而不是按各 section 自己的行驶方向：成员 section 可能与
  reference 反向行驶，若以各自行驶方向定 index，接缝邻居派生（§3.2.1）将
  依赖 Core 无法获知的方向关系。统一 reference 系后，反向 section 的 index
  顺序相对其行驶方向反转，但任何 section 的"reference 系最外侧 lane"都
  无需方向数据即确定性可得（与 OpenDRIVE 相对参考线编号同构）。lane
  adjacency（#237 消费）的事实源是该 index 顺序，与 LaneGroup 无关。
- lane 的 edge 链内相邻 edge 必须存在 LaneGraph directed connection（链是沿纵向
  的连续 traversal）。各 lane 的**内部切分点**独立，无跨 lane 段级对齐要求——
  没有横向几何，Core 无法定义或校验跨 lane 的纵向段对应（段数相等不等于纵
  向对齐，长度相等又与弯道弧长冲突），v1 不冻结段级对应关系。**lane 纵向
  共延伸不变量**：同一 section 的所有 lane 覆盖同一纵向区段（链的首尾端面
  一致）——一条 lane 中途开始或结束意味着横向组成在该处变化，按本节
  section 边界定义那必须是 section 边界。该不变量使 section 的纵向范围良
  定义（= 各 lane 的共同范围），是 `(RoadSection, i)` 整边界锚点与 §3.1
  corridor 共延伸的前提。Core 不可校验：lane 端面一致不等于首尾 node 相同
  （junction 下游 section 的各 lane 从不同 exit edge 起始），位置比较需要
  几何；由 authoring 保证，tooling/横向几何 G1 补强。
- 一条 LaneEdge 至多属于一条 lane、至多一个 RoadSection；未被覆盖的 edge 合法
  （Junction internal edge、未分段路段等）。同一条 lane 链内 edge 也不得重复
  （如 `[A, B, A]`）——重复 edge 会让同一物理 edge 占据同一 lane 的两个纵向
  位置，占用/锚定语义自相矛盾。
- RoadSection 的方向由其 lane edge 链的方向派生，不存储方向字段。**同向不变
  量**：同一 section 的所有 lane 链必须同向行驶——反向链会让 section 方向
  派生自相矛盾（同一 section 两个行驶方向），lane index 的 reference 系语义
  也无法统一。该不变量由 authoring 保证：Core 无横向几何，无法在
  normalization 确定性地区分同向与反向链（平行车道间通常无 directed
  connection 可供判别）；几何感知的离线校验（如比对绑定中心线的 heading）
  由 tooling 或横向几何 G1 补强，不进入 v1 Core validation。
- RoadSection 引用 edge，不复制 length、connection 或 speed limit；LaneEdge 继续
  只由 LaneGraph 拥有。
- section 边界是 authoring 语义：横向组成（车道数、设施带、分隔形式）发生变化的
  位置即边界；Core 只验证结构规则，不验证"边界划得好不好"。

#### 3.2.1 车道边界（lane boundary）锚点

完整横向边界全集由两层结构锚点构成，v1 边界语义均为**整边界级**：

- **section 内边界**：lane `i` 与 lane `i + 1` 之间的边界由 `(RoadSection, i)`
  唯一标识（lane index 顺序构造保证）；整边界语义以 §3.2 lane 纵向共延伸
  不变量为前提（authoring 保证）。
- **corridor 接缝边界**：相邻元素 `elements[j-1]` 与 `elements[j]` 之间的
  边界由 `(RoadCorridor, j)` 唯一标识（elements 顺序构造保证）。接缝两侧的
  横向邻居由元素顺序确定性派生：section 元素贡献其 index 序最外侧 lane
  （lane index 已按 corridor reference 方向排序，§3.2，派生不需要任何方向
  数据），FacilityBand 元素作为非遍历侧参与接缝（如路缘/分隔带标线锚点）。
  接缝的整边界语义以 §3.1 纵向共延伸不变量为前提（authoring 保证）。
  接缝锚点是完整横断面邻接图的必要组成：`kindId` 在 section 级，跨 kind
  相邻（如 motorLane section 与 nonMotorLane section 的接缝）必然跨 section，
  只有 section 内锚点无法表达这类物理边界。

段级锚定 `(RoadSection, i, 段 k)` 需要跨 lane 的共享纵向分段，而这在没有横向
几何时无法良定义：段数相等不代表纵向对齐（切分点可以任意错开），长度相等又
与弯道弧长冲突。因此段级边界 identity 整体推迟到横向几何 G1——届时可用
stationing 定义共享纵向分段。#237 首版按整边界语义设计（如整条边界实线/虚线，
或以 Junction/section 边界为界的分段）。

本设计不定义边界标线或变道许可本身（见 §15）：实线/虚线是物理标线事实，变道
许可是 policy 层，二者都不改变本节冻结的 lane 顺序与链结构。

### 3.3 LaneGroup

LaneGroup 是 RoadSection 内可选的命名分组，获得生产语义：

```text
LaneGroup
  externalId
  roadSectionId           -> 恰好一个 RoadSection
```

- 成员关系由 lane 的 `laneGroupId` 表达（child-owned reference，不在两个方向重复
  持久化）；一个 LaneGroup 至少聚合一条 lane。
- LaneGroup 是语义/准入引用单元（如"公交道组"），不影响 lane 顺序，不嵌套。
- 它不是 Core root entity，但拥有 external ID 与 dense handle，因为 AccessRule 与
  Adapter observation 需要稳定引用。

### 3.4 FacilityBand

非方向、非遍历设施带：

```text
FacilityBand
  externalId
  kindId                  -> non-traversable FacilityKind
```

- 不得把人行道、分隔带或设施带建模为 LaneEdge：它们没有 length/speedLimit 语义，
  不进入 route cursor、occupancy 或 traversal graph。
- 横向位置由所属 corridor 的 cross-section 顺序表达；不携带几何。
- 行人过街与步行网络由 #236 独立 G1 决定；本设计只保证 FacilityBand 拥有稳定
  identity，可被未来步行 traversal 与 AccessRule 引用。

### 3.5 LaneEdge

LaneEdge 职责不变（`lane-graph.md`）：有方向、可遍历的最小拓扑与纵向进度单元。
本设计不给 LaneEdge 增加任何类型字段；它的设施身份通过
`RoadSection.lanes[].edgeIds` 的成员关系反查获得。

## 4. FacilityKind：物理设施身份

### 4.1 与 AccessRule 的分离

LaneFlow 不使用单一封闭 `laneType` 枚举。四类关注点显式分层：

| 关注点                   | 承载者                             |
| ------------------------ | ---------------------------------- |
| 物理设施是什么           | `FacilityKind`                     |
| 谁（哪类参与者）可以使用 | `AccessRule` + `ParticipantClass`  |
| 什么时间适用             | `AccessRule.timeWindows`           |
| 哪个地区/法规版本        | `AccessRule.regulation` provenance |

一条公交专用道 = `motorLane` 设施 + 一条 deny-motorVehicle / allow-bus 的
AccessRule 组合；物理上不"是"公交道。

### 4.2 词汇与扩展

FacilityKind 是开放 token 词汇，SSOT 保留 seed 值：

| kind            | 类别            | 可承载实体   |
| --------------- | --------------- | ------------ |
| `motorLane`     | lane-bearing    | RoadSection  |
| `nonMotorLane`  | lane-bearing    | RoadSection  |
| `sidewalk`      | non-traversable | FacilityBand |
| `median`        | non-traversable | FacilityBand |
| `plantingStrip` | non-traversable | FacilityBand |
| `facilityStrip` | non-traversable | FacilityBand |
| `shoulder`      | non-traversable | FacilityBand |

扩展规则：

- 项目自定义 kind 必须使用 `x-` 前缀，类别由前缀细分：`x-lane-` 前缀声明
  **lane-bearing**（可作 RoadSection `kindId`，如 `x-lane-tram`），其余 `x-`
  kind 一律为 non-traversable band kind；两者都不被 Core 赋予任何行为语义，
  Adapter/authoring 可自行解释。类别由 token 约定承载与 `x-` 前缀机制同源，
  无需新增声明结构。通用性得到验证的自定义 kind 应回流 SSOT seed 表，
  `x-lane-` 是项目级逃生口而非词汇分叉的捷径。
- 未在 seed 表且无 `x-` 前缀的 unknown kind 是 load error（防拼写漂移）。
- kind 的类别（lane-bearing / non-traversable）用错实体类型是 validation error。
- **lane-bearing 只声明结构语义**（lane 链、顺序、准入 target），不激活任何
  参与者行为：v1 Core 没有非机动车行为，`nonMotorLane` section 只是横断面
  结构与 AccessRule 引用目标；非机动车/步行是否获得 traversal 与行为语义由
  #236 的 G1 决定（§15），本表不预决该产品边界。
- FacilityKind 永远不携带参与者、时段或地区语义；新增物理 kind 只扩展 SSOT 表，
  不改变 AccessRule 语义。

## 5. ParticipantClass：参与者分类

### 5.1 数据声明的分类法

ParticipantClass 在 Traffic data 中显式声明，Core 不内置任何类别：

```text
participantClasses[]
  id
  extendsId?              -> 单继承父类
```

- `extends` 构成无环单继承层级；匹配语义为"自身或传递祖先"。
- 所有 class 引用必须解析到声明（closed per package，extensible across packages），
  拼写错误即 load error，不会静默匹配空集。
- 层级匹配是 Core 的通用机制；Core 行为不依赖任何具体类名。

文档级 seed 分类法（示例与 fixture 的推荐声明，非 Core 常量）：

```text
motorVehicle
  car
  bus
  truck
  emergencyVehicle
nonMotor
  bicycle
  electricBicycle
pedestrian
```

### 5.2 与 VehicleProfile / Route / ManeuverPath 的关系

- `VehicleProfile` 新增必填 `participantClassId`，恰好引用一个声明的
  ParticipantClass。**单继承只能表达一个分类维度**：同一 profile 无法同时落在
  功能维度（bus）与尺寸维度（largeVehicle）两条独立链上；v1 的权宜是按主导
  维度建模（如 `largeBus extends bus` 或 `largeTruck extends truck`），并由
  authoring 保证规则与该维度一致。真正的多维度分类（多 membership + 绑定期
  跨成员组合）是独立 G1 的候选扩展，见 §15。
- Route 不携带 class；Route 继续是实际 traversal authority，不因参与者类别分叉。
- ManeuverPath 不携带 class；同一物理 traversal 仍然只有一个规范 ManeuverPath
  definition，参与者差异只能由 AccessRule overlay 表达（ADR 0017 §3）。
- AccessRule 匹配某 profile，当且仅当 profile 的 class 等于或是规则任一
  `participantClassIds` 成员的传递后代。

## 6. AccessRule：准入 overlay

### 6.1 Wire 概念形状

```text
AccessRule
  externalId
  target                  -> 恰好一个引用
    kind: laneEdge | laneGroup | roadSection | maneuverPath | facilityBand
    id
  effect: allow | deny
  participantClassIds[]   -> 非空
  timeWindows[]?          -> 缺省 = 永远适用；字段存在时至少一个窗口
    days[]                -> mon..sun 子集；标识窗口的**起始日**
    startMinuteOfDay
    endMinuteOfDay        -> 允许跨午夜（start > end 表示跨日窗口）
  regulation?             -> 法规 provenance
    jurisdiction
    version
    source?
  priority?               -> 有符号 32 位整数（i32，[-2^31, 2^31-1]），缺省 0
```

`priority` 取 i32：JSON 整数无界而 DTO 必须定宽，不定界会让不同实现接受/拒绝/
排序同一 rule set 的结果漂移；i32 落在 JS safe-integer 范围内（authoring 工具
可精确表示），对确定性裁决的粒度绰绰有余，越界由 schema range 校验拒绝。

跨午夜窗口语义（确定性，`simulation clock` 求值）：`days` 标识窗口**起始日**；
窗口从起始日 `startMinuteOfDay` 起生效，`start > end` 时延续到次日
`endMinuteOfDay`（含 Sun→Mon 回绕）。例：`days: [mon], 22:00–02:00` 覆盖周一
22:00 至周二 02:00；周二 01:00 属于该窗口，周一 01:00 不属于。

配套约定（同属已冻结的静态语义）：

- **周原点**：simulation time 0 定义为周一 00:00:00，周按 7 天循环。固定原点
  而非 package 级可配 epoch——可配原点是为 v1 guard 拒绝的能力增加 schema
  面，且任何期望的起始相位都可用绝对周时刻表达。
- **端点包含性**：窗口是半开区间 `[start, end)`（`end` 时刻本身不适用；
  跨午夜窗口对两侧区间同样半开）。相邻窗口由此无缝拼接（`8:00–17:00` 与
  `17:00–20:00` 在 17:00 时刻只有后者适用），time segment 切分、组合歧义
  检查与未来的表切换共享同一边界约定，无重叠也无空隙。
- **端点界值**：`startMinuteOfDay ∈ [0, 1439]`（起点必须落在当日内）；
  `endMinuteOfDay ∈ [1, 1440]`，`1440` 表示当日 24:00，仅作半开终点合法
  （跨午夜窗口的 end 是次日时刻，界值相同）。全天窗口编码为 `0–1440`；
  `start == end` 由 §10 shape 检查拒绝，不产生空窗口歧义。

### 6.2 Target 平面与展开

AccessRule 的 target 分为两个求值平面，不得互相展平：

**edge 平面**（laneEdge / laneGroup / roadSection）：规则对一条 LaneEdge 适用，当
target 是该 edge 本身，或 target 是包含该 edge 的 LaneGroup / RoadSection（经
lane 成员展开）。edge 平面规则在 normalization 消解为
`(edge, class) -> effect` resolved 表。

**path 平面**（maneuverPath）：规则对一个 Maneuver occurrence（某条 Route 对某条
path 的完整连续匹配）适用，**不展开为 edge**。ADR 0017 允许不同 path 共享 entry
transition 与 internal edge，若把 path 规则展平进 per-edge 表，`deny truck @
左转 path` 会误伤共享 edge 上合法的直行/右转 traversal。path 平面规则在
normalization 与 edge 平面同构地消解为 `(path, class) -> effect` resolved 表
（参与者/优先级组合裁决一次完成，§6.4）；occurrence 语义保留在绑定期——
(class, Route) 绑定时对该 Route 的 pending occurrence 逐个 O(1) 查表，
不在绑定期做任何层级匹配或组合裁决。

target specificity 排序（仅用于 edge 平面内组合；path 平面只有单一 target 类型，
不参与跨平面比较）：

```text
laneEdge > laneGroup > roadSection
```

FacilityBand target 在 v1 由 **capability guard 结构化拒绝**：production 对
FacilityBand target 的规则返回 capability-unavailable 结构化错误，拒绝载入，
不得静默无效。Core 不内置参与者类名，无法在 v1 区分 pedestrian 与其他类；
band 准入语义（可能引入参与者 category/capability 字段）由 #236 的 G1 定义后
解禁。

### 6.3 适用性与默认语义

- 规则适用于 (profile, target, time) 当参与者匹配、target 覆盖且时间落在任一
  timeWindow 内；无 timeWindows 即永远适用。
- **默认语义**：没有任何适用规则 = 准入 overlay 无约束。与 ADR 0009 的
  `signalControl:none` 同一原则——"无约束"不等于"永久自由通行"，也不解除
  leader、speed limit、ParkingStop、RouteEnd、safe-speed、no-overlap 等其他
  约束。
- `regulation` 是 provenance/审计字段（对齐 ADR 0009"法规来源、版本和适用地区
  必须可审计"），v1 不参与计算语义；不同法规版本 = 不同 rule set/package。
  为保证审计口径一致，同一 package 内所有声明了 `regulation` 的规则必须共享
  同一 `(jurisdiction, version)`（`source` 可不同）；未声明 `regulation` 的
  规则视为 provenance 未指定，不参与该约束。normalization 对违例返回结构化
  错误（§10）。

### 6.4 确定性组合语义

**平面内裁决**。对同一 (vehicle profile, edge)（edge 平面）或
(vehicle profile, occurrence)（path 平面）收集该平面全部适用规则后，按字典序
裁决：

1. **参与者 specificity**：匹配路径更深的 class 获胜（规则通过
   `participantClassIds` 中使 profile 匹配成功的最深类计）；
2. **target specificity**：仅 edge 平面，按 §6.2 排序，更具体的 target 获胜；
3. **显式 priority**：数值更高者获胜；
4. 经过 1–3 仍在 allow/deny 间并列的，属于 authoring 歧义，normalization 返回
   结构化错误，拒绝载入。**不设 deny-overrides 兜底**——静默的保守裁决会把
   authoring 错误变成不可见的运行时行为，与可审计目标冲突。歧义检查按
   **(time segment, target 展开单元, class)** 进行：永不同时适用的规则
   （如仅早高峰的 allow 与仅晚高峰的 deny）不构成歧义；always-active segment
   也必须检查。

**跨平面合取**。最终准入 = 两平面结果合取：任一适用平面给出 deny 即 deny；
allow 只在平面内充当豁免，不跨平面解除 deny。

该顺序保证常见模式可表达且可审计：

- 公交专用道：deny `motorVehicle`（深 1）+ allow `bus`（深 2）→ 公交放行；
- 单车道例外：section 级 deny + edge 级 allow → edge 获胜；
- 分时禁行：deny `truck` + timeWindow 7:00–9:00 → 窗口内禁行，窗外无约束。

参与者 specificity 先于 target specificity 是刻意选择：身份豁免（公交、应急）
是主导模式，豁免规则只需在粗 target 声明一次。两轴意见相反时的冻结示例：

```text
deny motorVehicle @ edge-1        （参与者深 1，target=laneEdge）
allow bus         @ roadSection-A （参与者深 2，target=roadSection）
```

对 class=bus 的 profile，参与者轴先裁决：allow `bus`（深 2）胜，公交可通行
edge-1。若 authoring 意图是"edge-1 连公交也禁"，正确处方是
`deny bus @ edge-1`：参与者同深（bus = 深 2）后 target specificity 决胜，
laneEdge 胜过 roadSection。在参与者轴先裁决的顺序下，给 `motorVehicle` deny
加 priority 或把 bus allow 移到更细 target 都无法让该 deny 生效——例外必须以
同等或更深 class 表达，意图因此永远显式、可审计。

### 6.5 运行时集成边界

准入是 ADR 0009 分层中的 regulatory constraint，不是终态判定：

- **静态规则**（无 timeWindows）：enforce 点是 **(ParticipantClass, Route)
  绑定**——vehicle spawn 或 runtime 路线指派时，用 edge 平面 resolved 表与
  path 平面 occurrence 规则校验该 (class, route) 组合，命中 deny 即原子拒绝该
  绑定并返回结构化错误。校验只覆盖车辆**当前 route cursor 起的可达后缀**：
  spawn/replace 可以在非零 `routeEdgeIndex` 发生，cursor 之前的 edge 与
  occurrence 不会被 traversal，不参与校验。pending occurrence 的确切范围：
  `exitRouteEdgeIndex` 严格大于 cursor 的 occurrence——`cursor < entry` 的
  未来 occurrence 与 `entry ≤ cursor < exit` 的进行中 occurrence 都作为
  原子整体校验；`cursor == exitRouteEdgeIndex` 时该 maneuver traversal 已
  完成（exit edge 本身的准入由 edge 平面覆盖），occurrence 不参与校验。
  Route 保持 class-agnostic：同一 Route 可被
  公交合法使用、被货车拒绝，准入判断只在有 class 上下文的绑定点发生；
  `register_route` 本身不做准入判断。v1 只有严格语义：**违规/劝诫式准入
  （软约束、记录事件但不拦截）是行为设计，必须独立 G1**，不得通过放宽 deny
  语义私下引入。
- **时变规则**：作为 Core constraint pipeline 的 runtime constraint 在 edge-entry
  决策点求值；只有 motion 表现（entry 停让、合规窗口判断与 event）由时变
  runtime G1 冻结，时间语义不在其内（见下）。在其实现前，
  **v1 production 对声明了 timeWindows 的规则返回 capability-unavailable 结构
  化错误，拒绝载入**——guard 必须是显式拒绝，不得让声明了时段限制的规则
  静默无效（那会让车辆在无报错的情况下穿越已声明的限制）。绑定期校验只
  消费静态规则。**推迟边界**：时变规则的 timeWindow 语义、time segment
  切分、按 segment 的组合歧义裁决与 resolved 表切换（§6.4/§11）是静态
  normalization 语义，已在本文冻结，时变 runtime G1 不得改变；只有 motion
  集成（entry 停让、合规窗口判断与 event）留给该 G1。
- 任何 allow 都不能覆盖 safety 约束；deny 只能追加约束，不能移除其他域的约束。
- Adapter 只能 query/render 准入状态，不得裁决、覆盖或注入绕行结果。

## 7. Identity、handle 与 registry

- RoadCorridor、RoadSection、LaneGroup、FacilityBand、ParticipantClass、AccessRule
  均为一等实体：wire 用 external ID（沿用 current Traffic ASCII token 规则），
  Core runtime 用 dense typed handle（`RoadCorridorHandle`、`RoadSectionHandle`、
  `LaneGroupHandle`、`FacilityBandHandle`、`ParticipantClassHandle`、
  `AccessRuleHandle`）。ParticipantClass 的外部身份同样是一等 handle：
  VehicleProfile、Adapter observation 与 query API 需要稳定引用参与者类别；
  准入求值在 normalization 后编译为 dense class index 与层级子树区间（§5），
  字符串不进入 steady-tick 求值路径。
- registry 静态 immutable，初始化后稳定，不需要 generation（ADR 0005 的
  lane-graph 先例）；handle 不持久化、不跨 CoreWorld 混用。
- Core 提供与 Junction registry 同形的 resolver、normalization-order iteration、
  parent/member query 与 borrowed slice iteration；public API 不暴露内部
  index/range。
- Normalized storage 沿用 road-junction-model §5 的 flat 形状：dense definitions、
  flat member handles + per-parent range、flat edge handles + per-lane range。
- ParticipantClass 层级在 normalization 编译为 per-class 子树区间（无环单继承
  森林的 Euler tour `(enter, exit)`，O(classes) 存储与初始化，匹配为 O(1)
  区间包含查询）；
  两个平面 AccessRule 的组合裁决（§6.4）都在 normalization 期**完全消解**——
  edge 平面为 `(edge, class) -> effect`、path 平面为 `(path, class) -> effect`
  的 resolved 表（稀疏行物化：仅受约束单元占 class 行，route-shared，不按
  vehicle 复制）；绑定期对 edge 与 pending
  occurrence 只做 O(1) 查表，steady tick 不做 external-ID lookup、字符串匹配、
  层级匹配或组合裁决。

## 8. Authority 矩阵

| 事实/行为                        | Authority                                 | 非 Authority                       |
| -------------------------------- | ----------------------------------------- | ---------------------------------- |
| LaneEdge 拓扑/length/speed limit | LaneGraph/Core                            | RoadSection、RoadCorridor、Adapter |
| 横断面横向顺序                   | RoadCorridor cross-section                | Adapter 推断、Spatial 几何         |
| lane 顺序与 adjacency 事实源     | RoadSection lane index                    | LaneGroup、AccessRule              |
| 设施物理身份                     | FacilityKind（SSOT seed + `x-` 扩展）     | AccessRule、laneType 式封闭枚举    |
| 参与者分类                       | Traffic data 声明 + Core 层级匹配         | Core 内置类名、SignalController    |
| 准入许可                         | AccessRule overlay + Core constraint 管线 | FacilityKind、ManeuverPath 复制    |
| 法规来源/版本审计                | AccessRule.regulation provenance          | SignalController 内嵌 if/else      |
| 最终 motion/safety               | Core longitudinal/traversal pipeline      | allow 规则、Adapter                |
| 横断面渲染/宽度/材质             | Adapter/Presentation                      | Core/Data                          |
| 中心线几何/pose                  | Spatial edge binding（不变）              | 横断面 overlay                     |

## 9. Wire shape 与版本

### 9.1 概念 JSON

以下概念 JSON 只展示新增片段形状，非可加载 package；字段命名以 production
schema 为准，语义不得偏离：

```json
{
  "formatVersion": "0.9",
  "participantClasses": [
    { "id": "motorVehicle" },
    { "id": "bus", "extendsId": "motorVehicle" }
  ],
  "facilityBands": [
    { "id": "band-median-1", "kindId": "median" }
  ],
  "roadSections": [
    {
      "id": "section-main-east",
      "kindId": "motorLane",
      "lanes": [
        { "edgeIds": ["edge-main-e-l1", "edge-main-e-l1-b"], "laneGroupId": "group-bus" },
        { "edgeIds": ["edge-main-e-l2", "edge-main-e-l2-b"] }
      ]
    }
  ],
  "laneGroups": [
    { "id": "group-bus", "roadSectionId": "section-main-east" }
  ],
  "roadCorridors": [
    {
      "id": "corridor-main",
      "referenceSectionId": "section-main-east",
      "elements": [
        { "bandId": "band-median-1" },
        { "sectionId": "section-main-east" }
      ]
    }
  ],
  "accessRules": [
    {
      "id": "rule-bus-lane",
      "target": { "kind": "laneGroup", "id": "group-bus" },
      "effect": "deny",
      "participantClassIds": ["motorVehicle"]
    },
    {
      "id": "rule-bus-lane-allow-bus",
      "target": { "kind": "laneGroup", "id": "group-bus" },
      "effect": "allow",
      "participantClassIds": ["bus"]
    }
  ]
}
```

### 9.2 版本政策

- production 实现按 ADR 0008 原子 clean-break：Traffic `0.8 -> 0.9`，loader 只接受
  exact current，不留 dual schema/alias/migration shim。
- `vehicleProfiles[].participantClassId` 为必填新字段，属破坏性变更，随同一
  bump 切换。
- SpatialPackage 与 ScenarioManifest 保持 `0.1`；横断面不改变 edge centerline
  binding。
- 已发布 schema/bytes 继续 immutable（ADR 0011）。

## 10. Validation 与 first-error

新校验 phase 按数据依赖分两段插入 canonical loader 顺序（`data-loading.md`：
profiles → lane graph → Junction → Signals → Parking → Routes）：

- **profile 域段**（下表 phase 1–2）：随 `normalize_profiles` 一并完成，在
  lane graph 之前——ParticipantClass 与 profile 的 class 引用不依赖任何
  拓扑，放在拓扑 phase 之后会让 class/profile 错误与拓扑错误的首错顺序
  背离 canonical 顺序；
- **拓扑依赖段**（phase 3–10）：在现有 lane graph / Junction / Signals
  phase 之后——RoadSection 需解析 LaneGraph edge，AccessRule 需解析
  Junction ManeuverPath 与 lane/section 成员。

同 phase 内按 input order 返回首错，任一错误不得发布部分 registry：

1. ParticipantClass：ID syntax/duplicate、unknown `extendsId`、继承环；
2. VehicleProfile：unknown `participantClassId`（依赖 phase 1 的 class identity
   解析结果；profile 其余字段校验沿用现有 pipeline，本设计不新增）；
3. FacilityBand：ID syntax/duplicate、unknown kindId、kind 类别错误；
4. RoadSection identity：ID syntax/duplicate（先于 LaneGroup parent 解析，
   否则 `laneGroups[].roadSectionId` 指向 malformed/重复 ID 时
   "unknown parent" 无确定性归因）；
5. LaneGroup identity：ID syntax/duplicate、unknown roadSectionId（先于
   RoadSection 成员检查，否则 lane 的 `laneGroupId` 无法无歧义解析）；
6. RoadSection body：unknown/non-lane-bearing kindId、empty lanes、empty
   lane chain、unknown edge、chain 内 disconnected transition、lane 链内 edge
   重复、同一 edge 出现在多条 lane/多个 section、unknown laneGroupId、lane
   引用 group 的 `roadSectionId` 与该 lane 所属 section 不一致；
7. LaneGroup membership：empty group（无 lane 引用），在 lane 成员关系已知后
   检查；
8. RoadCorridor：ID syntax/duplicate、empty elements（先于一切 element 依赖
   检查，否则 reference 成员性恒失败、empty 错误不可达）、unknown element
   引用、elements 内重复
   引用同一 section/band、同一 section/band 出现在多个 corridor、section/band
   零归属（§3.1 完备所有者树）、referenceSectionId 不是成员 section；
9. AccessRule：ID syntax/duplicate、unknown target、unknown participant class、
   capability guard（FacilityBand target 或声明 timeWindows 的规则返回
   capability-unavailable 并拒绝载入；guard 依赖 target 已解析，故在 unknown
   检查之后；guard 先于 shape/组合检查——能力整体拒绝后其内部细节校验无
   意义）、timeWindow shape（timeWindows 空数组、days 空集、分钟越界——
   `start ∈ [0, 1439]`、`end ∈ [1, 1440]`、`start == end`）、`priority`
   shape（整数性 + i32 范围；definition 存原始数值字面量，统一在 guard
   之后执行）、
   `regulation`
   shape（jurisdiction/version/source 长度 1 到 128 字符；definition 不预校验，
   统一在 guard 之后执行）、`regulation`
   provenance 混合（声明了 regulation 的规则不共享同一
   `(jurisdiction, version)`）、按平面与 time segment 分别检查 §6.4
   第 4 步的残留组合歧义（edge 平面按 (segment, edge, class)，path 平面按
   (segment, path, class)，含 always-active segment）；
10. 构造 dense storage、member ranges、class 子树区间与 resolved effect 表。

phase order 的组织原则：**identity phase 一律先于引用解析 phase**——LaneGroup
的 identity/membership 拆分与 RoadSection 的 identity/body 拆分同形，任何
"解析对 X 的引用"的检查都排在"X 的 ID syntax/duplicate"之后。

Schema 只校验 syntax/shape/range；owner、reference、containment、组合歧义由
Core constructors/normalization 报告（ADR 0007 分层不变）。

## 11. Determinism 与性能边界

- 沿用 road-junction-model §11/§12：同一 Traffic bytes/Core version/normalization
  path 得到相同 handle allocation、iteration、first-error attribution 与运行结果；
  input permutation 只改变 raw handle 数值与迭代顺序，不改变 external-ID 对齐的
  语义等价。
- class 子树区间、edge 平面 `(edge, class)` 与 path 平面 `(path, class)`
  resolved effect 表、member ranges 在 normalization 一次编译，(class, Route)
  绑定时只做查表与 occurrence 比对；vehicle tick 不查字符串、不匹配层级、
  不做组合裁决、不扫描全局 rule catalog、不做 per-vehicle allocation。
- 两个平面的组合裁决都只发生在 normalization：edge 平面对每条 edge、path
  平面对每条 path，与其 class 集合预计算 §6.4 的字典序结果；绑定时对任意
  (class, edge) 或 (class, pending occurrence) 的准入判断是一次 O(1) 查表。
- 时变规则按全部 timeWindow 边界把 simulation day/week 切成确定性 time
  segment。冻结的是**语义契约**而非表示：normalization 期全量预编译、任意
  segment 的 `(edge, class)` 与 `(path, class)` 准入查询保持 O(1)、窗口切换
  是预定 sim-time 的结构切换（与 SignalController immutable program 的
  phase 推进同构）、不逐 tick 求值窗口。表示由时变 runtime G1 选择，但必须
  在 segment 间共享未变条目（如静态基表 + per-segment sparse delta），内存以
  O((edges + paths) × classes + 变化条目总数) 为界，禁止
  O(segments × (edges + paths) × classes) 的全量物化。timeWindow 语义基于
  simulation clock，不读墙钟。
- catalog 规模不进入无关车辆的 steady-tick 复杂度。

## 12. 影响矩阵

| 层                 | Target 影响                                                           | 本 Issue 变更 | 后续 owner       |
| ------------------ | --------------------------------------------------------------------- | ------------- | ---------------- |
| Core API           | 新增 6 类 handle/resolver/registry；(class, Route) 绑定期准入校验     | 无（设计）    | production Issue |
| LaneGraph          | 不变；edge 成员关系由 RoadSection 引用反查                            | 无            | —                |
| Route              | 保持 class-agnostic；绑定期增加静态准入与 occurrence 级 path 规则校验 | 无            | production Issue |
| Traffic Data       | exact-current `0.9` 新 arrays + profile 必填字段                      | 无            | production Issue |
| Spatial            | shape 保持 `0.1`；横断面为顺序拓扑，无横向几何                        | 无            | —                |
| Manifest           | shape 保持 `0.1`；production 后更新 Traffic size/digest               | 无            | production Issue |
| Authoring          | 显式 corridor/section/lane/rule 输入；generator 显式生成              | 无            | production Issue |
| Adapter            | observation 新增 corridor/section/rule 只读 query；无裁决权           | 无            | production Issue |
| Presentation       | 可按 cross-section 顺序渲染横断面；宽度/材质自有                      | 无            | production Issue |
| Fixtures/artifacts | 原子切换 v0.9 并更新 canonical bytes/digests                          | 无            | production Issue |

## 13. 最小 production 实现拆分

#234 之后拆出一个最小 production Issue（#262）：

**范围**：生产化 §3–§7 的静态模型——RoadCorridor/RoadSection/LaneGroup/
FacilityBand/ParticipantClass/AccessRule 的 Core registry、handle、resolver、
validation、Traffic v0.9 schema/DTO/loader/fixtures、
(class, Route) 绑定期静态准入校验（edge 平面查表 + path 平面 occurrence 校验）、
generator 显式横断面输出与 canonical artifacts 原子切换。

**显式不做**：时变规则的 runtime constraint（v1 对带 timeWindows 的规则返回
capability-unavailable 结构化错误，拒绝载入；runtime 独立后续 G1）、
FacilityBand target 规则语义（同形 guard，归 #236）、违规/劝诫式准入语义
（独立 G1）、lane change/adjacency 消费（#237）、非机动车/行人行为（#236）、
横断面横向几何、Adapter 渲染实现。

**验收对齐**：road-junction-model §16 同形的 identity/owner、reference、
first-error、determinism、permutation、round-trip 与 steady-tick no-scan/
no-allocation 测试矩阵。

## 14. #237 消费契约

#234 与 #237 保持独立 G1（接口共设计，不合并）。本文向 #237 提供以下**冻结
保证**，#237 的 G1 只做契约校验，不得重复冻结：

1. **相邻事实源**：RoadSection 的 lane index 顺序（按 corridor reference
   方向排序，lane `i` 与 `i ± 1` 相邻，与 LaneGroup 无关）+ RoadCorridor
   的 elements 顺序（相邻元素互为横向邻居）（§3.2/§3.2.1）。
2. **边界锚点**：`(RoadSection, 相邻 lane 对)` 标识 section 内横向边界，
   `(RoadCorridor, 相邻元素对)` 标识 corridor 接缝边界，均为整边界级
   （§3.2.1）；前者以 lane 纵向共延伸（§3.2）、后者以 corridor 元素纵向
   共延伸（§3.1）为前提（均 authoring 保证）；段级锚定
   `(section, lane 对, 段 k)` 依赖跨 lane 共享纵向分段，
   整体推迟到横向几何 G1，#237 首版不得依赖段级边界 identity。
3. **overlay 模式**：ParticipantClass 层级匹配、AccessRule 五元与确定性组合
   （§5、§6），变道许可、动态车道用途、lane-use state 的参与者/时段例外复用
   该模式，不发明第二套规则语义。
4. **物理/policy 分层**：边界标线（实线/虚线）是物理标线事实，变道许可是
   policy 层；默认解释与例外都由 #237 在 policy 层冻结，不写进标线事实。

若 #237 发现契约不足，必须回到 #234/ADR 0018 修订，不得绕开或私下扩展。

## 15. Future extension boundary

- **#237（动态车道用途/lane change/resolved lane plan）**：在 §14 消费契约上
  冻结车道边界标线表达、变道许可 policy、静态 kind 与动态/条件 rule、
  lane-use state 与 lane-change plan 的引用关系。
- **#236（非机动车/步行）**：决定是否让 nonMotorLane/sidewalk 获得 traversal
  语义与 CrossingFacility；本文只提供 identity 与准入引用通道。
- **时变准入 runtime**：timeWindow 的 motion 集成（entry 停让、合规窗口判断与
  event）必须独立 G1；不得通过 private heuristic 提前生效。
- **违规/劝诫式准入**：软约束、事件记录不拦截等 violation 语义必须独立 G1；
  v1 deny 只有严格语义。
- **横向几何**：车道宽度、横向偏移、路缘与横坡需要独立几何 G1（对齐 ADR 0013
  首版限制）；本文的顺序拓扑是其自然锚点，同向不变量的几何校验补强与段级
  边界锚定也在该 G1 解冻。
- **多法规版本共存**：v1 强制同一 package 内 regulation provenance 单一
  （§6.3/§10）；跨法域 rule set 的组合与冲突裁决留待真实需求触发，用
  package 级分离。
- **多维度参与者分类**：v1 单继承单维度（§5.2）；多 membership + 绑定期跨
  成员组合（含 specificity 歧义裁决与 resolved 表形状变化）必须独立 G1。

## 16. G1 冻结结论

本设计接受：

- `RoadCorridor` 作为横断面唯一 owner 的非方向性结构组合；方向性 RoadSection
  保留为车道承载单元；横向边界全集 = section 内边界 `(section, i)` + corridor
  接缝边界 `(corridor, j)`，均为整边界级锚点（§3.2.1）；元素纵向共延伸为
  authoring 语义（§3.1）；
- RoadSection 的 corridor reference 系 ordered lanes + edge 链 + 单 section
  归属 + 同向不变量 + lane 纵向共延伸（authoring 语义）；LaneGroup 为可选
  命名分组，不影响 lane 顺序；
- FacilityBand 非遍历、非 LaneEdge；FacilityKind seed + `x-` 开放词汇只承载
  物理设施身份；
- ParticipantClass 数据声明、单继承、Core 无内置类；VehicleProfile 单引用；
- AccessRule 五元（target/effect/参与者/时间/法规 provenance）+ 双平面
  （edge / path occurrence）求值 + 参与者优先的确定性三步裁决与残留歧义
  拒绝（无 deny-overrides 兜底）+ 跨平面合取；
- 准入 overlay 只引用、不复制 ManeuverPath，保持 ADR 0017 全局 traversal
  coherence；
- edge/path 两平面组合裁决都在 normalization 期消解为 resolved effect 表
  （`(edge, class)` / `(path, class)`），时变规则按
  time segment 切换 segment 索引结构（共享未变条目，禁止全量物化），tick
  O(1) 查表；
- Traffic `0.8 -> 0.9` 原子 clean-break；Spatial/Manifest 保持 `0.1`；
- 静态规则在 (ParticipantClass, Route) 绑定期 fail-fast（Route 保持
  class-agnostic，v1 仅严格语义）；绑定期校验只覆盖车辆**当前 route cursor
  起的可达后缀**（cursor 之前的 edge/occurrence 不会被 traversal，不参与
  校验；cursor 落在某 occurrence 内部时该 occurrence 作为原子整体校验）；
  时变规则与 FacilityBand target 规则在 v1 由 capability guard 结构化拒绝，
  语义各自独立 G1；
- 与 #237 的接口共设计契约（§14），两 Issue 不合并。

若 production 实现发现必须改变 owner 层级、组合语义、版本政策或 steady-tick
复杂度，必须回到 #234 更新本设计与 ADR 0018，不得通过 private 实现静默改变。
