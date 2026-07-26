# ADR 0018：多模式横断面 owner 与参与者准入 overlay

**状态**: Accepted<br>
**日期**: 2026-07-26<br>
**适用范围**: LaneFlow 的 RoadCorridor/RoadSection/LaneGroup/FacilityBand 横断面分层、FacilityKind、ParticipantClass、AccessRule 时间/地区 overlay 与 Traffic v0.9 演进边界<br>
**关联文档**:

- 上游决策:
  - `0001-project-scope.md`
  - `0005-core-identity-and-handle-model.md`
  - `0007-traffic-data-crate-and-loader-boundary.md`
  - `0008-pre-1.0-data-format-version-policy.md`
  - `0009-signal-indication-gate-and-policy-separation.md`
  - `0013-engine-neutral-spatial-geometry-and-length-authority.md`
  - `0017-static-road-junction-maneuver-and-gate-identity.md`
- 详细设计:
  - `../design/cross-section-access.md`
  - `../design/road-junction-model.md`
  - `../design/lane-graph.md`
  - `../design/data-format.md`
- GitHub:
  - #227
  - #228
  - #234
  - #237

## 背景

ADR 0017 冻结了方向性 `RoadSection` 与可选 `LaneGroup` 的术语，但二者尚未生产化；
LaneEdge 完全无类型，既没有多模式横断面（机动车道、非机动车道、人行道、分隔带、
设施带）的结构 owner，也没有参与者分类与准入规则。中国城市道路需要联合组织这些
设施，而公交专用、车型限制、分时禁行等不能用单一封闭 `laneType` 枚举长期承载。

ADR 0017 同时要求同一物理 traversal 只有一个规范 ManeuverPath，车辆类别、cost、
priority 等差异必须由独立 overlay 表达；ADR 0009 已确立 indication/policy/safety
分层与"法规来源、版本和适用地区必须可审计"的先例。准入与设施身份因此需要在
production 化之前拥有明确 SSOT。

## 决策

### 1. 横断面总 owner 是新增 `RoadCorridor`，方向性 RoadSection 不足够

`RoadCorridor` 是非方向性的道路结构组合，拥有唯一有序 cross-section；元素是
一等实体引用，二选一：方向性、承载车道的 `RoadSection`，或非方向、非遍历的
`FacilityBand`。元素按 corridor 声明的参考方向（`referenceSectionId`）从左到右
排列，与 Spatial 的正横向偏移约定一致；同一 corridor 的 `elements[]` 内不得
重复引用同一元素（唯一有序序列定义的直接推论）。corridor 所有元素纵向共延伸
（覆盖同一纵向区段）是接缝边界整边界语义的前提；与同向不变量同类，无横向
几何/stationing 时不可由 Core 校验，由 authoring 保证、tooling 与横向几何 G1
补强。

RoadCorridor 遵循 JunctionGroup 先例：结构组合，非行为实体——不拥有 route
planner、conflict solver、controller clock 或 runtime availability。横断面只冻结
横向**顺序**；宽度、横向偏移与横坡不进入 Traffic schema。

### 2. RoadSection/LaneGroup 获得生产语义，设施带不伪装成 LaneEdge

- RoadSection：有方向，`lanes[]` 按 lateral index 排序，index 框架是
  **corridor reference 方向**（index 0 = reference 方向最左，与 OpenDRIVE
  相对参考线编号同构）而非各 section 自己的行驶方向——成员 section 可能与
  reference 反向行驶，各自行驶方向系会让接缝邻居派生依赖 Core 无法获知的
  方向关系；统一 reference 系后派生零方向数据、构造性确定。
  每条 lane 是连续 LaneEdge 链；一条 LaneEdge 至多属于一条 lane，同一 lane 链内
  也不得重复；lane index 顺序与 corridor elements 顺序共同构成未来 lane
  adjacency 的事实源。**横向边界锚点 v1 是整边界级的**，分两层：section 内
  边界 `(section, 相邻 lane 对)` 由 lane index 顺序构造保证；corridor 接缝
  边界 `(corridor, 相邻元素对)` 由 elements 顺序构造保证——`kindId` 在
  section 级，跨 kind 相邻（如机动车道与非机动车道接缝）必然跨 section，
  接缝锚点是完整横断面邻接图的必要组成。两者供 #237 的边界标线与变道许可
  设计消费。段级锚定 `(section, lane 对, 段 k)` 需要跨 lane 的共享
  纵向分段，而没有横向几何时该对应无法良定义（段数相等不代表纵向对齐，等长
  又与弯道弧长冲突），段级边界 identity 整体推迟到横向几何 G1，v1 不冻结
  任何形式的跨 lane 段对应。
- LaneGroup：RoadSection 内可选命名分组，拥有一等 identity 供准入与观测引用，
  不影响 lane 顺序。
- FacilityBand：人行道、分隔带、绿化/设施带等非遍历设施，不进入 traversal
  graph，不携带 length/speedLimit，横向位置由 cross-section 顺序表达。
- 每个 RoadSection/FacilityBand 恰好属于一个 RoadCorridor；LaneEdge 不被任何
  RoadSection 覆盖是合法的（横断面是 LaneGraph 之上的可选结构 overlay）。

### 3. FacilityKind 与 AccessRule 显式分离，拒绝单一封闭 laneType

四类关注点各归其位：物理设施身份 = `FacilityKind`（开放 token 词汇，SSOT seed +
`x-` 前缀扩展，永远不含参与者/时段/地区语义）；参与者准入 = `AccessRule`；
时间 = `AccessRule.timeWindows`；地区/法规版本 = `AccessRule.regulation`
provenance。公交专用道 = `motorLane` 设施 + deny/allow 规则组合，物理上不是
"公交道"。

### 4. ParticipantClass 是数据声明的单继承分类法，Core 无内置类

`participantClasses[]` 在 Traffic data 中声明（`id` + 可选 `extendsId`，无环单
继承）；匹配语义为"自身或传递祖先"。所有引用必须解析到声明，unknown 即 load
error。`VehicleProfile` 新增必填 `participantClassId`（恰好一个）。单继承只能
表达一个分类维度（功能/尺寸不可兼得）；v1 按主导维度建模，多维度分类
（多 membership + 绑定期组合）是独立 G1 的候选扩展。Route 与 ManeuverPath 不
携带 class；参与者差异只能由 AccessRule overlay 表达，不得复制相同 edge
sequence 的 ManeuverPath（ADR 0017 §3 不变）。

### 5. AccessRule 是 versioned、可审计的准入 overlay

```text
AccessRule = target(laneEdge|laneGroup|roadSection|maneuverPath|facilityBand)
           + effect(allow|deny) + participantClassIds[] + timeWindows[]?
           + regulation{jurisdiction,version,source?}? + priority?
```

- 默认语义：无适用规则 = 准入 overlay 无约束；不表示永久自由通行，不解除任何
  其他域约束（ADR 0009 原则）。
- target 分两个求值平面：edge 平面（laneEdge/laneGroup/roadSection，展开为
  per-edge 事实）与 path 平面（maneuverPath，只对 occurrence 求值，**不展平进
  per-edge 表**——ADR 0017 允许 path 共享 edge，展平会误伤共享 edge 的合法
  traversal）。跨平面合取：任一平面 deny 即 deny。
- 平面内组合按字典序裁决：**参与者 specificity 优先于 target specificity**
  （身份豁免是主导模式），然后显式 priority；经三步仍在 allow/deny 间并列的
  是 authoring 歧义，normalization 直接拒绝——不设 deny-overrides 兜底，否则
  歧义拒绝永远不可达，authoring 错误会变成不可见的运行时行为。
- `regulation` 是 provenance/审计字段，v1 不参与计算语义；同一 package 内
  所有声明 regulation 的规则必须共享同一 `(jurisdiction, version)`，
  normalization 强制校验，保证审计口径一致。
- 静态规则在 **(ParticipantClass, Route) 绑定期**（spawn/路线指派）校验并原子
  拒绝违规绑定，校验只覆盖当前 route cursor 起的可达后缀（cursor 内
  occurrence 作为原子整体）；Route 保持 class-agnostic 可复用，`register_route`
  无 class
  上下文、不做准入判断（v1 仅严格语义；违规/劝诫式准入是独立 G1）。时变规则
  作为 Core constraint pipeline 的 runtime constraint，其实现由独立 G1 冻结；
  在其实现前，**v1 production 对声明 timeWindows 的规则与 FacilityBand
  target 的规则返回 capability-unavailable 结构化错误，拒绝载入**——guard
  必须是显式拒绝，不得让已声明的限制静默无效。timeWindow
  的 `days` 标识窗口起始日，跨午夜延续到次日（含 Sun→Mon 回绕），保证分段
  确定性。任何 allow 不得覆盖 Core safety
  约束。
- edge 平面组合裁决在 normalization 期消解为 `(edge, class) -> effect`
  resolved 表，时变规则按确定性 time segment 切换表；绑定期准入判断是 O(1)
  查表 + occurrence 比对，tick 不做字符串匹配、层级匹配或组合裁决。

### 6. Identity、authority 与版本

新实体一律一等身份：external ID + dense typed handle + resolver + flat storage，
静态 immutable registry 无 generation（ADR 0005/0017 先例）；ParticipantClass
同样拥有一等外部身份（`ParticipantClassHandle`，供 VehicleProfile、Adapter
observation 与 query API 稳定引用），其层级匹配与 per-edge rule refs 在
normalization 编译为 dense class index 与 bitset/resolved 表，steady tick 不查
字符串、不扫描 catalog。准入 wire/schema 归 `laneflow-data`，domain invariant 归
Core constructors（ADR 0007）。production 化按 ADR 0008 原子 clean-break：
Traffic `0.8 -> 0.9`，Spatial/Manifest 保持 `0.1`。

## 后果

### 正向后果

- 多模式横断面有单一 owner 与稳定 identity；车道 adjacency 获得整边界级结构
  锚点（#237 的直接输入）。
- 设施、参与者、时间、地区四类关注点可独立演进，不再被封闭枚举锁死。
- 准入差异不再诱使 authoring 复制 ManeuverPath，保持全局 traversal coherence。
- 组合语义确定性且可审计，常见模式（公交专用、车型禁行、分时禁行、单车道
  例外）无需特例代码。
- Adapter/Presentation 获得横断面顺序的只读观测，无需推断。

### 代价与风险

- Core 增加六个 handle domain 与 registry；Traffic 再次原子迁移（0.8 -> 0.9）。
- RoadSection 的 lane edge 链与 LaneGraph connection 的双重一致性增加
  normalization 校验面。
- 时变准入的 runtime 语义被显式推迟；在其 G1 前，timeWindows 只有静态数据
  意义，必须防止半成品语义被当作已生效。
- 横断面只有顺序没有宽度，presentation 的横向布局质量仍依赖 Adapter 自有
  假设，直到横向几何 G1。
- 同向不变量与 corridor 元素纵向共延伸不变量 v1 都不可由 Core 校验：平行
  lane 链间无共享参考系，链方向即其自身 traversal 方向；元素纵向范围无
  stationing 可度量（FacilityBand 甚至没有纵向数据）。错误 authoring 会被
  静默接受并在 runtime 表现为对向邻接或错位接缝。缓解路径：generator 几何
  感知生成保证两者、离线 tooling 校验（比对绑定中心线 heading/范围）、横向
  几何 G1 后补 normalization 校验钩子；hand-authored 数据承担该残余风险
  （§3.1/§3.2 已声明）。

## 被拒绝的替代方案

### 方向性 RoadSection 兼任横断面 owner

中央分隔带等中央设施不属于任何方向；双向 lateral order 会分裂成两份需手工同步
的事实，因此拒绝。

### 把人行道/分隔带建模为特殊 LaneEdge

会污染 traversal graph 与 route cursor/occupancy 语义，强迫非遍历设施携带
length/speedLimit，因此拒绝。

### 单一封闭 laneType 枚举

无法同时承载物理设施、参与者准入、时段与地区规则，扩展即破坏 schema，因此拒绝
（#234 完成定义的硬性要求）。

### Core 内置 ParticipantClass 常量

会让分类法演进变成 Core 破坏变更，且无法阻止拼写漂移；数据声明 + 单继承层级
同时满足可扩展与可校验，因此拒绝内置。

### VehicleProfile 多 class membership

引入 membership 间 specificity 歧义（公交且大型车命中两条链时无稳定裁决），且
resolved 表形状从 (edge, class) 退化为 (edge, class 集合) 的绑定期组合。v1 以
单引用 + 单维度层级覆盖主导场景，多维需求按独立 G1 扩展，因此 v1 拒绝。

### 把准入写进 SignalController/ManeuverGate

违反 ADR 0009 的 controller 纯净性（不得含国家、车型、设施特例），因此拒绝。

### 提前冻结段级边界 identity（强制段对齐或浮点纵向区间）

段级锚定 `(section, lane 对, 段 k)` 需要跨 lane 的共享纵向分段，而没有横向几何时
该对应无法良定义：段数相等不代表切分点纵向对齐，强制等长又与弯道内外弧长冲突；
浮点纵向区间则引入第二套长度权威（offset 与 EdgeLength 漂移）、放大 first-error
复杂度，运行时还要把区间映射回 edge。v1 只冻结由 lane index 顺序构造保证的
整边界级锚点，段级 identity 待横向几何 G1 用 stationing 良定义后再冻结，因此
两种提前冻结方案都拒绝。

### FacilityKind 下沉到 lane 级（替代 corridor 接缝锚点）

让混合 kind 的 lane 共存于同一 section 以保留单一 section 内邻接，会模糊
RoadSection 作为结构分段的语义、破坏"横向组成变化即 section 边界"的定义，并
把 kind 一致性检查面扩大到每个 lane。corridor 接缝锚点以派生结构（零新增
实体、零新校验面）覆盖同一需求，因此拒绝下沉。

### RoadSection 存储显式方向字段

方向字段没有独立的校验锚点：lane 链方向即其自身 traversal 方向，平行链间无
共享参考系，字段只是重复声明 authoring 意图，Core 无法将其与链结构交叉验证，
只会制造虚假的可校验感并增加一处可漂移的事实，因此拒绝。

### corridor 成员记录显式 orientation（same/opposite）字段

该字段不是可校验数据，只是第三处可漂移的 declaration：cross-section 顺序与
lane index 顺序本身就是声明，方向关系无法与任何结构交叉验证。把 lane index
框架统一定义为 corridor reference 方向后，接缝邻居派生构造性确定、零新增
数据，因此拒绝 orientation 字段。

### 缺少几何证明时拒绝构造 RoadSection

当前契约没有横向几何权威层（ADR 0013），该方案会让 v1 横断面整体不可用，
代价远超收益。同向不变量维持 authoring 保证 + generator/tooling 校验 + 横向
几何 G1 补强（残余风险见"代价与风险"），因此拒绝。

### 合并 #234 与 #237 为单一 G1

#237 的硬核（lane-change plan、lane-use 动态状态、与 following/occupancy 的
交互）是行为设计，不反哺本文静态本体；合并只会拉长冻结周期并扩大评审面。两
Issue 保持独立 G1，以 `cross-section-access.md` §14 的冻结消费契约衔接；契约
不足时回到本 ADR 修订，不得绕开。

### 纯 priority 组合（无 specificity）

公交专用道模式（deny motorVehicle、allow bus）在纯 priority/deny-overrides 下
失败，迫使所有例外都靠手工 priority 编号，不可审计，因此拒绝。

### deny-overrides 作为最终兜底

会让"残留 allow/deny 歧义 = normalization 拒绝"永远不可达：兜底先裁决，歧义
检查成为死代码，authoring 错误被静默吞掉，因此拒绝；三步裁决后仍并列即拒绝
载入。

## 后续

- #234：交付本 ADR 与 `cross-section-access.md`，完成设计 G3/G4。
- 最小 production Issue #262（#234 拆出）：生产化静态模型与 Traffic v0.9、
  (class, Route) 绑定期静态准入校验；时变 runtime 由 capability guard 拦截。
- #237：消费本 SSOT 冻结动态车道用途、车道边界标线（实线/虚线，物理事实）与
  变道许可（policy overlay）的分层、adjacency 与 resolved lane plan。
- #236：决定非机动车/步行 traversal 与 CrossingFacility 产品边界。
- 时变准入 runtime、横向几何、多法规版本共存：各自独立 G1。

如果未来改变横断面 owner 层级、让 FacilityBand 进入 traversal graph、把准入并入
controller、引入多 class membership、或放弃 atomic 版本政策，必须新增或 supersede
本 ADR，不得通过 private 实现静默改变 Accepted 语义。
