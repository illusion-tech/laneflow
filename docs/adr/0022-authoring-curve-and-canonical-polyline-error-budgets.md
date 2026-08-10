# ADR 0022：编制曲线与规范折线的误差预算分层

**状态**: Proposed（#296 G1 纠偏重启；B1 内部验证路径已获产品确认）<br>
**日期**: 2026-08-07<br>
**适用范围**: 道路编辑来源格式中的解析曲线、编译器几何中层表示、已验证规范
低层中间表示中的有界 `f32` 折线、当前/目标 Spatial 采样，以及 Adapter 表现几何边界<br>
**扩展**: ADR 0013、0015；不取代 ADR 0015 的有界 `f32` 坐标、运行时表示误差、
连接容差、长度绑定或性能契约<br>

**关联文档**:

- `0013-engine-neutral-spatial-geometry-and-length-authority.md`
- `0015-bounded-f32-canonical-spatial-frames.md`
- `0020-compiler-owned-static-network-and-static-image.md`
- `../design/road-editing-source-and-geometry-frontend.md`
- `../design/network-compiler.md`
- `../design/spatial-geometry.md`

> 历史状态：本 ADR 于 2026-08-07 随 #296 旧 Geometry JSON G1 记为 Accepted。
> 2026-08-10 产品来源前提纠偏使 #296 返回 G1；`2 cm`、`5 cm`、`10 cm` 三档产品
> 方向继续保留。产品负责人于 2026-08-10 选择 B1：先实现确定、可完整试玩和可测量的
> 工程质量目标，不把三档声明为连续曲线硬上限，也不形成长期存档兼容承诺；是否需要
> 后继的连续硬保证由 B1 实际证据重新决策。

## 背景

ADR 0013/0015 冻结的是调用方已经提供一条高保真折线后，LaneFlow 如何把它转换为
有界 `f32` 规范几何、验证长度和连接，并在运行时确定性采样。ADR 0015 的 `1 cm`
位置误差和 `0.5°` 切线误差比较同一条折线的 `f64` 参考与 `f32` 运行时表示；它没有
定义解析曲线如何离散为该折线。

#296 的道路编辑前端把 cubic Bézier 作为首批生产编制候选。编译器必须离线把
解析曲线细分为唯一的规范折线，因为 LIR、静态镜像和 Traffic/Spatial 只保留折线，
不保留 authoring evaluator 或引擎样条权威。若继续把 ADR 0015 的 `1 cm` 直接当成
全部曲线细分预算，会混淆两个误差来源，并为游戏道路生成远超视觉和运动需要的点数；
若只冻结一个 `10 cm` 停止阈值，又会没有余量覆盖量化、station 插点和连接检查，且
无法让近景质量与大规模路网在同一受检契约中显式取舍。

游戏引擎可以在 Adapter/Presentation 层增加网格三角形、样条展示或细节层次，但它
无法恢复编译器已经丢失的原始 Bézier/后继螺旋线语义，也不得覆盖规范中心线、长度、
station 或拓扑锚点。因此表现细分不能替代编译器误差契约。

## 决策

### 1. 五层几何表示分别计账

| 层次                   | 权威与用途                                                            | 是否进入运行时规范权威 |
| ---------------------- | --------------------------------------------------------------------- | ---------------------- |
| 编制解析曲线           | 道路编辑来源前端在 `f64` 中求值的 line/cubic Bézier                   | 否，仅在前端/MIR 存续  |
| Station reference 基表 | 配置档无关的固定 `f64` 参数区间与累计弦长，只解释 source station      | 否                     |
| 配置档近似参考         | 所选位置/方向档下，按 B1 固定采样规则独立细分的 `f64` 点序列          | 否                     |
| 规范运行时折线         | 量化后的有界 `f32` 点、弧长、切向、Traffic length 与 Spatial sampling | 是                     |
| Adapter 表现几何       | 路面 mesh、材质、视觉样条、LOD、动画和物理表现                        | 否                     |

编译器只发布规范运行时折线。编制解析曲线、station 基表和配置档近似参考不能成为第二份运行时几何，
Adapter 表现几何也不能反向修改规范长度、station、拓扑连接或语义锚点。

### 2. 几何精度使用三个封闭配置档

几何精度配置档（Geometry Accuracy Profile）是道路编辑来源前端的显式编译选项：

| Rust 变体     | 稳定名            | 代码 | B1 细分采样目标 | B1 对外显示目标 | 预期用途                 |
| ------------- | ----------------- | ---: | --------------: | --------------: | ------------------------ |
| `Fine2Cm`     | `fine-2cm-v1`     |    1 |        `0.01 m` |        `0.02 m` | 近景、高质量道路与急弯   |
| `Balanced5Cm` | `balanced-5cm-v1` |    2 |       `0.025 m` |        `0.05 m` | 常规生产推荐档           |
| `Compact10Cm` | `compact-10cm-v1` |    3 |        `0.05 m` |        `0.10 m` | 大规模路网与资源优先场景 |

调用方必须对每个 Geometry 模块显式选择，不提供任意 `f64` 容差或 `Default`。同一编译
单元内全部 Geometry 模块必须使用相同档位；不得逐 road、span、lane 或 offset curve
混用，也不得按输入规模、平台、构建模式、资源压力或运行时负载自动选择/降级。
`Balanced5Cm` 是产品推荐档，不是隐式默认值。

表中厘米值是确定性工程**目标**，不是对解析曲线到规范折线连续最大距离的数学保证。
B1 可以进入编辑器、生成器、游戏初始化和车辆路径的内部完整验证，但在后续产品复核前：

- 不进入公开 schema publication，不声明长期保存兼容；
- 不把测试地图或 B1 LIR 当成未来硬保证格式可以原地升级的承诺；
- UI、文档和诊断只能写“2/5/10 cm 目标档”，不能写“最大误差不超过”；
- 后继若需要连续硬保证，必须重新进入 G1、提升来源/几何语义版本并重新生成指纹和证据。

档位进入 Geometry 模块的 `frontendOptionsDigest`、编译约束和 LIR 语义指纹。改变档位
不改变实体 `CanonicalIdentity` / `StableId128` 或固定 reference station 基表，但允许
改变规范点、派生弧长、几何数值和语义指纹；不同档位输出不能作为逐 bit 相同制品互换。

### 3. 方向使用独立的三个封闭配置档

方向配置档（Geometry Direction Profile）与位置档正交、同样由调用方显式选择：

| Rust 变体      | 稳定名             | 代码 | 最终 f32 折线方向跳变上限 | f64 细分候选端点切向与弦半角 |
| -------------- | ------------------ | ---: | ------------------------: | ---------------------------: |
| `Smooth1Deg`   | `smooth-1deg-v1`   |    1 |                      `1°` |                       `0.5°` |
| `Balanced2Deg` | `balanced-2deg-v1` |    2 |                      `2°` |                         `1°` |
| `Compact5Deg`  | `compact-5deg-v1`  |    3 |                      `5°` |                       `2.5°` |

调用方不能传任意角度，方向档不实现 `Default`；`Balanced2Deg` 是推荐档而非隐式默认。
同一编译单元内全部 Geometry 模块必须使用相同方向档。位置与方向共形成九种受支持组合，
两种档位代码均进入 `frontendOptionsDigest` 与语义指纹。

### 4. 运行时表示预算独立保持

| 预算项                                      | 上限或规则          | 说明                            |
| ------------------------------------------- | ------------------- | ------------------------------- |
| f64 细分候选的端点切向与弦夹角              | `<= 所选方向档半角` | 只形成候选，不代替最终检查      |
| 最终 f32 相邻弦及相连 edge 首尾弦夹角       | `<= 所选方向档全角` | 在量化和强制插点后直接检查      |
| `f64` 参考折线到 `f32` 规范折线的位置误差   | `<= 0.01 m`         | 继续执行 ADR 0015，不因档位放宽 |
| `f64` 参考折线到 `f32` 规范折线的切线角误差 | `<= 0.5°`           | 继续执行 ADR 0015               |
| 相连 edge 的实际 `f32` 端点间隙             | `<= 0.005 m`        | 继续执行 ADR 0015；不 snap      |

每档按第 6 节固定的有限采样规则决定是否继续二分，并在量化和强制插点后直接检查最终
`f32` 折线。该规则保证固定来源、固定档位和相同 compiler 语义版本在全部受支持平台
产生相同点序与逐 bit 相同的规范语义；它只保证算法和制品确定，不把有限采样结果提升为
连续曲线硬证明。

切线约束与位置约束分别成立。f64 半角细分不能假定来源 segment 自动切向连续，也不能
把 ADR 0015 的 `0.5°` 表示误差叠加到公开档位之外。因此量化后按实际 f32 点直接检查
每个内部折点、source segment join 和相连 edge join；超限失败关闭，不做隐式平滑。
source segment join 不再要求两侧水平 `left` 逐 bit 相同：两侧独立求值后按实际位置间隙
与所选方向档验收。reference curve 继续共享显式端点；offset join 在通过 `5 mm` 位置
检查后使用前一段末端作为唯一规范边界点，下一段的 B1 采样检查从该点开始，不能绕过
位置目标。跨 edge join 仍保留各自端点并执行实际 `f32` 间隙/方向检查，不进行修补。
只满足位置误差的长线段可能造成车辆朝向或控制目标在折点明显阶跃；把固定极小切线角
强加给所有场景又会让较粗位置档失去资源收益。因此 #296 使用三个封闭方向档，并在 G2
对九种组合的最终方向跳变和车辆路径跟踪分别验证。

### 5. 语义锚点不消费近似预算

下列位置不是“误差范围内可移动”的普通采样点：

- 每个来源曲线段的显式端点；
- cross-section span 的 station 边界；
- lane edge 起终点和显式 predecessor/successor 连接；
- StopLine、ManeuverGate、WaitingZone、Parking 等绑定到 edge/station 的语义锚点。

曲线段端点必须进入规范点表；station 边界必须按冻结的 reference station 参数表插点；
相连端点继续在实际 `f32` 坐标上执行 `5 mm` 检查。不得以任何档位的总预算执行 snap、
最近点吸附或连接修补。

Reference station 基表固定使用与输出配置档无关的 `0.01 m` / `0.5°` B1 目标政策和 `f64`
累计弦长。每行是只属于一个 source segment 的
`(segmentOrdinal, t0, t1, cumulativeStart, cumulativeEnd)` 区间；source station 对累计
终点 lower-bound 后只在该行的局部参数内插值。精确内部 segment 边界规范归属前一段
`t = 1`，而下一 station 区间从新段局部 `t = 0` 开始。最终参数规范化把
`(segment i, 1)` 与 `(segment i+1, 0)` 视为同一边界并只输出一次。强制点只进入最终
点表，不回写基表。因而切换任一输出档位不会移动 span 边界的 authoring 参数位置。

### 6. offset 曲线也受所选档位约束

reference line 的通过不能说明 lane/facility offset curve 已达到同一工程目标。横向偏移
会放大曲率与切线变化，而且一般 offset evaluator 不再是 cubic Bézier。因此道路编辑
来源前端必须在相同曲线参数域对每条最终中心线独立二分，不能复用 reference curve 的
接受树，也不能把 B1 的有限采样写成连续最大误差证明。

令 reference curve 为 `B(t)`，LaneFlow 规范 up 为 `+Y`，水平导数
`H(t) = (B'_x(t), 0, B'_z(t))`。每个实际求值点都必须得到有限且非零的 `H(t)`，并按固定表达式

```text
L(t) = (+Y × H(t)) / ||H(t)|| = (H_z(t), 0, -H_x(t)) / ||H(t)||
```

取得左向单位向量。把 `RoadCorridor.elements` 及每个 section 的 lane 从左到右展平为
member `0..n`，reference lane 下标为 `r`，member 宽度为 `w_i(s)`。对 corridor station
端点 `s0/s1`，先分别以从 reference 向外的固定求和顺序计算中心偏移端点：

```text
d_r = 0
d_i =  0.5*w_r + w_(r-1) + ... + w_(i+1) + 0.5*w_i,  i < r
d_i = -0.5*w_r - w_(r+1) - ... - w_(i-1) - 0.5*w_i,  i > r
```

不存在中间 member 时省略中间项；这同时冻结了从 reference 向外的逐项舍入顺序。再用
`u = (s - s0) / (s1 - s0)` 和
`d(s) = d0 + (d1 - d0) * u` 求值；不得逐点重新改变求和顺序。reference station 基表每行
`(t0, t1, cumulativeStart, cumulativeEnd)` 内的 station 固定为

```text
s(t) = cumulativeStart
     + (cumulativeEnd - cumulativeStart) * ((t - t0) / (t1 - t0))
```

因此最终 offset evaluator 唯一为 `O(t) = B(t) + d(s(t)) * L(t)`。source segment、
corridor station 边界和 reference station 基表行边界都是强制二分边界，不能让一个
候选区间跨越它们。宽度端点、偏移端点和上述表达式全部使用 IEEE 754 binary64、round to
nearest ties to even；每个写出的运算符后都完成一次舍入，禁止重结合、FMA/`mul_add` 和
fast-math，结果 `-0.0` 规范化为 `+0.0`。

line/cubic 的 `B(t)` 使用固定 de Casteljau 求值：
`lerp(a,b,t) = a + t * (b - a)` 严格按先减、再乘、再加执行；cubic 做三层 lerp，line
做一层 lerp。切向使用同一控制多边形的一阶导数 de Casteljau 求值。offset 的一阶切向
只需要 scalar binary64 求值，不建立区间 jet、任意精度数或连续证明层。

对每个候选区间 `[ta,tb]`，参数固定为：

```text
tm  = ta + (tb - ta) / 2
tq1 = ta + (tm - ta) / 2
tq3 = tm + (tb - tm) / 2
```

先把两端 evaluator 点分别量化为最终 `f32`、再无损提升回 `f64` 作为候选弦端点；若该区间
左端是已通过的 offset source join，则使用第 4 节选定的前段规范边界点。依次在
`tq1/tm/tq3` 求解析点，并与候选弦的 `u=1/4,1/2,3/4` 点比较三维欧氏距离。三项均
`<=` 所选档位的 B1 细分采样目标，且两端切向与弦夹角均 `<=` 方向档半角时接受；否则
只按 `tm` 二分并重复。所有减、乘、加、平方和平方根按文字顺序执行，禁止 `pow/powi`、
FMA 和 fast-math。

方向比较不调用 `acos/cos`：先要求点积为正，再按现有 profile 已冻结的 binary64
`cos²` 常量比较 `dot*dot >= cos_squared * norm_left_squared * norm_right_squared`，乘法与
分量平方和顺序固定。最终 `f32` 弦无损提升为 `f64` 后执行同一比较；阈值相等时接受。

若中点不再严格位于两端之间、任一规定求值非有限、规定采样点的水平导数为零，或在
位置/方向采样通过前命中几何点、二分深度或存续内存上限，numeric freeze 必须失败关闭。
offset 可以增加自己的采样点，但不能反向改变 reference station。最终量化点还必须执行
第 4 节的方向、连接和 ADR 0015 表示检查。

G2 校准另对每个不跨 source segment/station 行的 evaluator 区间使用固定 4097 点均匀
参数网格（含两端）测量解析 evaluator 到最终参数化折线的**观测**距离，记录 P50、P95、
P99 和最大观测值、对应来源地址与参数。该网格只用于离线证据，不参与 production
accept/reject，也不声称覆盖采样点之间的连续最大值。验证与资源报告必须覆盖九种组合、
最内侧、最外侧、跨 span 宽度变化以及最终车辆路径跟踪，不能只测零偏移 reference line。

不可遍历 FacilityBand 的最终 offset 中心线仍属于 canonical LIR 语义：道路编辑来源
派生的 FacilityBand 进入显式携带 `FacilityBandOrdinal`、按该 ordinal 排列的稀疏
`facility_band_geometries` 范围表和共享规范点表，并参与
点数、LIR 记录、逻辑输出字节和摘要；它不产生 Spatial segment、Traffic length、
successor 或路线可遍历性。这样独立细分的结果有唯一归宿，不会由 G2 实现选择丢弃或
隐式变成 Adapter 数据；没有道路编辑 geometry intent 的其他官方前端 FacilityBand 不产生占位行。

Numeric freeze 每个模块只执行一次，并且只生成由该模块本地 alignment、owner tree、
宽度和显式曲线决定的最终点 payload 与精确 `GeometryPointCount`。#315 common admission
仍在 HIR/MIR 前对完整模块计数执行原子累计；跨模块普通引用、approach frame、successor
和拓扑连续性在完整编译单元绑定后检查，只消费已生成点，不重新细分。配置档混用在单元
HIR 才可识别，因此可能浪费已受单模块上限约束的 freeze 计算，但最终 `build` 原子失败且
不返回部分输出。任何会影响点生成的 owner 关系都必须按道路编辑来源 v1 限定在同一模块，
否则该阶段划分不可实施。

### 7. Adapter 和车辆物理误差不进入编译预算

Adapter 可以从规范折线生成更密的路面 mesh、视觉样条或 LOD，近景车辆也可以通过
宿主物理产生悬架、轮胎和横向运动偏差。这些属于表现或动态行为，不进入静态位置/
  方向配置档，也不能被用来掩盖未达到目标的规范折线。

反过来，Adapter 为视觉连续性增加点或平滑朝向时不得改变规范 edge length、station、
连接关系或 Traffic/Spatial 采样权威。需要精确原始曲线的后继 Adapter 制品必须通过
新的 G1 冻结非权威表现载荷，不能让每个引擎从粗折线自行猜测原始曲线。

## 后果

正向后果：

- `2 cm`、`5 cm`、`10 cm` 的产品判断变成三个封闭、确定、可测试的内部验证档位，而不是
  任意浮点或运行时质量滑杆；
- 保留 ADR 0015 已验证的 `f32` 表示、连接和性能边界，不重写 current Spatial 行为；
- 相比毫米级曲线细分显著减少规范点、编译 scratch、静态制品和运行时采样表；
- 位置、方向、语义锚点和 Adapter 表现不再共用一个含糊的“视觉误差”。

成本与风险：

- 两类三档扩大 API、已知向量、等价 fixture 和九组合性能矩阵；每个含曲线工作负载都必须分别
  测量，不能只测试推荐档；
- `Balanced5Cm` / `Compact10Cm` 的规范折线会比毫米级候选更粗；依赖折线折点直接
  设置朝向的 Adapter 需要遵守方向连续性契约或在表现层平滑；
- offset 曲线可能比 reference line 需要更多点，最外侧曲线必须单独计入
  `GeometryPointCount` 和性能证据；
- FacilityBand 增加一张不可遍历 geometry 范围表及其规范点；这是独立细分选择的制品
  与内存成本，换取跨 target 一致的设施几何语义；
- v1 importer/generator 必须先把圆弧、螺旋线、NURBS 等转换为 line/cubic Bézier，并以
  同一 B1 校准网格记录观测误差；这不是原始 primitive 到 cubic 的连续误差证明。后继若
  把它们加入来源 curve union，必须提升格式版本并重新冻结 evaluator；
- 改变任一阈值、求值顺序或总预算都改变规范点 bit pattern、语义指纹和制品含义，
  必须重新进入 G1 并提升对应 frontend/constraint 版本。

## 被拒绝的替代方案

### 把 ADR 0015 的 `1 cm` 当作解析曲线到运行时的全部预算

该值原本验证同一折线的 `f64`/`f32` 表示差异。把它扩大为全部曲线细分政策会混淆
权威层，并在典型游戏道路上产生缺少产品收益的额外点，因此拒绝。

### 接受任意浮点容差或逐道路混用

任意值会造成无界测试/缓存/制品组合，逐道路或逐模块混用会让连接、station 和资源
判断依赖局部隐藏配置。因此位置和方向分别只接受编译单元一致的三个登记档位。

### 直接使用总位置上限作为曲线细分停止阈值

它不给量化、station 和连接留下独立余量，也会让“总上限”和“算法停止值”无法区分，
因此每档分别冻结更小的曲线细分子预算。

### 在 B1 之前直接承担连续硬保证

连续硬保证需要保守区间求值、定向舍入、端点求值误差和跨平台阈值 known vectors；当前
产品尚无证据证明这些复杂度能改善可见道路或车辆行为。先以 B1 完整试玩和测量，再决定
是否建立新的 certified 语义版本；不得把未来可能需要 A 当作当前兼容包袱。

### 只约束位置，不约束方向

长弦可以满足横向位置误差，却在折点产生较大的朝向/控制目标阶跃。LaneFlow 同时为
确定性交通采样服务，不能只按静态俯视图验收，因此拒绝。

### 依赖 UE、Bevy 或其他 Adapter 重新拟合

引擎可以增加表现细节，但无法知道丢失的原始曲线类型和 authoring 参数；不同引擎的
拟合还会形成多份长度与 station 权威。因此拒绝把 Adapter 重拟合作为规范正确性条件。

## 实施与治理

1. #296 新 G1 同时审阅本 ADR 与 `road-editing-source-and-geometry-frontend.md`，冻结
   算法、配置档、workload identity、测量协议和候选预算；在新的 G1 Pass 前不修改
   production Rust。
2. #296 G2 实现两类三档编码/摘要、混用拒绝、固定四分点/中点二分、station、offset、
   语义锚点、方向跳变和九组合自动化矩阵，并生成 exact fixture/manifest、4097 点离线
   观测、参考机点数/时间/资源与车辆路径证据。
3. G3 前新增 append-only B1 calibration closure，忠实记录每档观测分布、最坏来源、视觉/
   车辆结果和资源收益；不得把观测最大值改写为连续硬上限。
4. #296 的 B1 实现可以合入内部验证路径，但在新的产品复核前不得公开发布 schema、承诺
   长期道路编辑存档兼容或写成 production hard-bound capability。若实际证据要求 A，必须
   新开 G1、提升来源/几何语义版本并重新生成 fixture、摘要和资源证据；若证据表明 B1
   已满足产品，也仍需显式产品决定后才能把它提升为正式存档语义。
5. #294 G4 生产切换前，target static image/Traffic Runtime/Spatial shared-image path
   必须消费同一规范折线，不得恢复独立引擎样条权威。
