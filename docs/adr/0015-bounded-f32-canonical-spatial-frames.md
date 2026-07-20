# 0015 有界 f32 Canonical 空间框架

**状态**: Accepted

**日期**: 2026-07-19

**适用范围**: LaneFlow Spatial 的 canonical primitive、坐标范围、几何输入、位姿输出、运行时性能边界，以及 Adapter 的 frame 映射（#133-#138）

**取代范围**:

- 取代 ADR 0013 中“Spatial canonical 标量、顶点、弧长和标准位姿必须使用 `f64`”以及“同时提供标准 `f64` 与局部 `f32` 位姿”的精度条款。
- 取代 ADR 0014 中“Spatial 标准几何、弧长和标准位姿继续使用 `f64`”以及先在 `f64` 中减局部原点再转换的 Spatial 专属条款。
- 不取代上述 ADR 的分层、长度权威、制品配对、Core 数值权威、失败原子性、坐标手性或 Adapter 隔离决策。

**关联文档**:

- `0013-engine-neutral-spatial-geometry-and-length-authority.md`
- `0014-residual-aware-f32-core-authority-and-migration-gates.md`
- `../design/spatial-geometry.md`
- `../design/adapter-api.md`
- `../reference/v0.6-spatial-validation.md`

## 1. 背景

ADR 0013 最初选择 `f64` canonical geometry，目标是让不同宿主共享高精度几何，再在 Adapter 前减去局部原点并转换为 `f32`。这能覆盖大范围绝对坐标，但也让默认使用单精度的游戏引擎承担双精度几何、双套位姿类型和额外转换成本。

LaneFlow 的产品目标是园区、厂区、校园、景区、停车场和道路片区中的 NPC 车流效果展示，不是测绘、交通工程或高精度车辆动力学系统。运行时最重要的约束是可预测的内存、批量吞吐、稳定的引擎接入和厘米级视觉连续性。把全部场景强制放进一个绝对坐标框架，不符合这一边界；大型宿主本来也需要分块、流式加载或相机相对定位。

在每轴 `±16_384 m` 的 `f32` 范围内，最大 ULP 为 `0.001953125 m`。使用量化后的 runtime 坐标进行几何校验，并把最终位置误差限制在 `1 cm`，可以给计算和绑定留下明确余量，同时让点、向量、切向量和位姿直接匹配主流宿主的单精度热路径。

## 2. 决策

### 2.1 每个 canonical frame 是有界局部空间

- canonical 坐标继续使用米、右手坐标系和 `+Y` 上方向；道路前方向继续由有向中心线切向量定义。
- 每个 `SpatialRegistry` 只拥有一个稳定 `CanonicalFrameId`。frame 表示独立局部空间，不表示地理坐标参考系统（CRS），也不暗含与其他 frame 的变换。
- canonical 点的每个轴都必须位于闭区间 `[-16_384 m, 16_384 m]`。越界输入返回结构化错误，不截断、不饱和，也不回退为 `f64`。
- 大于单 frame 范围的场景由创作/加载边界拆成多个 frame 或 tile。#134 固定制品中的 frame identity；#136 使用调用方颁发的 batch-level `FramePlacementToken` 固定宿主放置版本和失效边界。放置、切换和原点生命周期不进入 Core 车辆状态。

### 2.2 运行时 canonical primitive 使用 LaneFlow-owned f32

Spatial 公共 primitive 固定为：

```text
CanonicalPoint3F32
CanonicalVector3F32
CanonicalUnitVector3F32
```

- `F32` 后缀是公共类型名的一部分；不提供 `F64` 别名或构建期精度开关。
- 点与向量保持不同类型。点应用 frame 范围；向量只要求每个分量有限，因为两个合法端点之差可以达到 `±32_768 m`。
- 所有构造器和返回点的运算重新验证有限性与点范围，并把 `-0.0` 规范化为 `+0.0`。
- 单位方向使用按最大绝对分量缩放的稳健归一化，拒绝零向量，避免有限 `f32` 在平方求和时中间上溢或下溢。
- 公共表面只提供受检命名运算，不实现可绕过不变量的裸算术 trait。
- #133 不提供公开 `f64 -> f32` 转换。空间包可以用 `f64` 或等价高保真值暂存原始 JSON 数值，但 #134 必须在路径化诊断后受检转换为唯一的 runtime `f32` 权威。

### 2.3 Registry 顺序和构建保持稳定且不泄漏布局

- Spatial 直接复用 opaque Core `EdgeHandle`；不增加第二套 Spatial handle，也不要求 Core 公开 ordinal。
- immutable registry 的 dense entry 顺序固定为 `LaneGraph::edges()` 顺序，不依赖空间包 JSON 顺序或散列表迭代。
- 初始 lookup 使用私有 `HashMap<EdgeHandle, u32>`；map 只负责查询，不公开 hasher、slot 或迭代顺序。
- #135 的公开 `SpatialRegistry::try_new` 借用 `&LaneGraph` 与 `SpatialEdgeInput<'a>`，按 frame/capacity、输入 unknown/duplicate、逐折线 geometry/length、LaneGraph missing、graph/next-edge join 的顺序校验。只有完整覆盖且所有连接连续后才返回 registry，不修改 Core，也不返回部分聚合。
- 只有性能证据证明 `EdgeHandle -> slot` 是瓶颈时，才通过独立设计修改 Core/Spatial API；#133 不为推测性优化扩展句柄表面。

### 2.4 后续几何和误差参数以 f32 runtime 值为准

以下参数已由 #135 实现并在量化后的 runtime `f32` 坐标上验证：

| 语义               | 接受值                                |
| ------------------ | ------------------------------------- |
| 最小中心线线段     | `0.1 m`                               |
| 相连 edge 端点容差 | `0.005 m`                             |
| 长度一致性几何容差 | `max(0.01 m, length × 1e-6)`          |
| Core 长度量化余量  | 作为独立加项，不隐藏进几何容差        |
| projected-up 下限  | `sin(0.5°) ≈ 0.008_726_535`，等号有效 |
| 最终位置误差       | 相对 `f64` 参考 `<= 0.01 m`           |
| 最终切线方向角误差 | 相对 `f64` 参考 `<= 0.5°`             |

- 相邻点在受检转换后若量化为同一点，必须作为退化线段拒绝，不能静默合并。
- 连接连续性使用实际 runtime `f32` 点判断，不能用转换前高精度值掩盖运行时断缝。
- `f64` 参考只用于离线 oracle、输入诊断和候选对照，不成为第二份运行时几何权威。

### 2.5 性能是后续交付的显式接受条件

#137 在固定参考机器、release 构建和固定输入上验证：

- 10,000 条位姿提取 p95 不超过 `2 ms`；100,000 条 p95 不超过 `20 ms`。
- 稳态零分配并复用调用方输出缓冲区；10k 到 100k 的扩展不超过 `12x`。
- 相同布局的 `f32` 候选相对 `f64` 候选 retained memory 至少降低 `25%`。
- `f32` 吞吐不得比 `f64` 候选慢超过 `5%`。
- 输入/输出顺序稳定；同一目标重复运行结果一致；无 `NaN`、Infinity 或部分输出。
- 把 `EdgeHandle -> slot` 与 slot-resolved sampling 分开测量；高频路径不解析 external ID。

绝对毫秒只在固定性能机上作为 Gate；普通共享 CI 运行正确性与分配回归，不用噪声较大的 wall time 阻断。

## 3. 后果

正向后果：

- Spatial 点、向量和位姿直接采用主流引擎的单精度运行时宽度，不需要维护默认双精度 canonical 输出。
- 每个点从三个 `f64` 分量缩小为三个 `f32` 分量，为折线、累计数据和批量位姿提供明确的内存优化方向。
- frame 范围、输入质量、连接容差和最终视觉误差形成可验证的闭合契约。
- Core 仍可无 Spatial 运行；Spatial、Data 与 Adapter 的依赖方向不变。

成本与风险：

- 不能把任意大世界绝对坐标直接塞进一个 Spatial registry；创作和 Adapter 必须显式管理 frame/tile。
- 旧 `f64` 原型、文档和未提交草案不能作为兼容 API；#133 直接采用新的 F32 类型名。
- #134 已提供受检转换和路径化错误，#135 已按量化结果验证几何，#136 已以 frame ID、placement token、调用方双缓冲和失败原子性管理宿主放置边界；#137 已证明零分配、retained memory 和固定机性能 Gate。
- 5 mm 连接容差和 1 cm 最终误差适合展示 runtime，不适合测绘或专业工程用途；后者不属于 LaneFlow 当前目标。

## 4. 被拒绝的替代方案

### 保留全量 f64 canonical geometry

它提供超出当前产品需求的绝对坐标精度，却增加热数据宽度、Adapter 转换和双套输出表面。LaneFlow 已选择有界 frame，因此拒绝把大世界能力绑定到每个 runtime 点。

### 无范围的绝对 f32 世界坐标

随着坐标增大，ULP 会持续增大，无法同时保证端点连续与 1 cm 最终误差；拒绝。

### 同时提供 f32/f64 构建或运行时模式

两套类型、布局、测试和行为路径会扩大 API 与确定性矩阵，并让制品含义依赖构建配置；拒绝。

### 把 frame ID 存进每个点或向量

它会显著增加折线热数据并重复同一身份。registry/batch 聚合拥有 frame ID 足以阻止边界混用；拒绝逐项存储。

## 5. 实施与治理

1. #133 交付 F32 primitives、frame ID、结构化错误、immutable registry、ADR/design 同步和基础测试。
2. #134 交付空间包、场景清单、高保真解析暂存、受检 F32 转换、external ID 解析和完整路径诊断。
3. #135 已交付量化后折线校验、current-f64 零量化余量长度绑定、端点连续、basis 和确定性采样。
4. #136 已交付 batch-level frame/placement 生命周期、批量位姿、Parking pose 与全批次失败原子性。
5. #137 已交付性质、误差、内存、分配、10k/100k 性能和伪 Adapter smoke。
6. #138 对照本 ADR 和所有实施证据完成 Spatial 收口；不得用后续收口放宽本 ADR 的范围或性能 Gate。
