+++
schema_version = 1
id = "design-signalized-corridor-protected-turning"
kind = "design"
status = "accepted"
revised_date = "2026-11-23"
summary = "定义信号化走廊受保护转向、跨组冲突区与清空判据。"
scope = "v0.9 双路口走廊的受保护左转、直行、右转、lane-level ManeuverPath、完整 Route catalog、固定时制、跨组冲突区与验证边界"
applicability = "#applicability"
issues = ["https://github.com/illusion-tech/laneflow/issues/1272", "https://github.com/illusion-tech/laneflow/issues/196", "https://github.com/illusion-tech/laneflow/issues/1496", "https://github.com/illusion-tech/laneflow/issues/1498", "https://github.com/illusion-tech/laneflow/issues/1500", "https://github.com/illusion-tech/laneflow/issues/1507"]
related = ["adr-0017", "design-example-scenarios", "design-road-junction-model", "design-signal-system", "design-signalized-corridor-population", "design-spatial-geometry", "design-traffic-runtime-conflict-occurrence", "design-traffic-runtime-right-of-way-policy", "design-waiting-zone-conflict-right-of-way"]
+++

# Signalized Corridor Protected Turning Profile

<a id="applicability"></a>
## 适用边界与依据

默认相位时长对齐检入配置的 16 ms Tick。§7.3 记录的车辆行为假设按 #196 G1 的停车可行性保持更新，已随 #1505 实现交付。

#1507 G1 将相位交接的空间安全从“下一组转绿前内部边全空”改为跨组冲突区约束（§7.4），清空核验相应改为 §8.1 的冲突安全与活性判据。该合同尚待实现：在实现交付前，检入制品不含冲突区，清空核验仍按旧判据运行，#1446 工作包 2 维持旧合同下的失败登记。

实现状态： #229 已交付通用 Traffic 0.8 static domains 与一等
`ManeuverGate`；#190-#192 已交付 production profile artifacts、当时 catalog 0.2、
scenario policy、native example、扩大验证与独立最终审阅；#262 已将相同 profile
artifact 迁移到 current Traffic v0.10 并保持 protected-turning behavior 不变

关联文档：

- [`road-junction-model.md`][road-junction-model]
- [`example-scenarios.md`][example-scenarios]
- [`signalized-corridor-population.md`][corridor-population]
- [`signal-system.md`][signal-system]
- [`spatial-geometry.md`][spatial-geometry]
- [`../adr/0017-static-road-junction-maneuver-and-gate-identity.md`][adr-0017]

## 1. 结论、边界与 authority

本文接受一个无需兼容旧格式的 v0.9 profile。两个 Junction 各自显式拥有
Movement 和 lane-level ManeuverPath；LaneGraph 继续拥有全部 LaneEdge；Route
继续拥有有限、显式 edge sequence；Maneuver occurrence 由 Traffic Runtime 在 route
registration 时编译；`laneflow-scenario` 继续拥有示例的人口、回流和 Route
选择 policy。

本 profile 的安全承诺分两层：

- **信号层**：authoring 不会在同一时刻授权任何已知冲突 path（§7.3）；
- **空间层**：不同 SignalGroup 之间相交或汇入同一出口车道的 path，由显式
  ConflictZone 约束空间占用（§7.4）。相位交接时，前一组已合法入场的车辆可以仍在
  Junction 内，下一组车辆按 Traffic Runtime 的冲突占用、预约与下游净空在 Gate 前
  等待，不以“下一组转绿前内部边全空”作为安全条件。

本 profile 只消费 Traffic Runtime 已有的冲突仲裁，不提供 gap acceptance、
permissive turn、跨组 priority 或 adaptive solver。SignalController 只执行完整、
已校验的 phase program，不根据 turn、国家或设施类型写 private heuristic。

本文是现有 ADR 0017 的具体场景 profile，不改变其 owner、identity、Route
occurrence 或 Gate 决策，因此不新增 ADR。若实施需要改变上述长期架构事实，必须先
回到 G1，而不能在 #229 内隐式改写本文。

## 2. 场景坐标与道路 envelope

沿用 v0.8 的右手 Y-up canonical frame、右侧通行和 `3.5 m` lane width：

- 主干道沿 X 轴，轴线范围 `X = -400..+400`，长度 `800 m`；
- 1 号次干道位于 `X = -200`，沿 Z 轴长 `300 m`；
- 2 号次干道位于 `X = +200`，沿 Z 轴长 `300 m`；
- `junction-1`、`junction-2` 中心分别为 `(-200, 0, 0)`、`(200, 0, 0)`；
- 三条物理道路轴线合计仍为 `1,400 m`，不重复计算 directed lanes 和
  internal connectors。

相对每个 Junction 中心，横向道路铺装 envelope 与 road/internal split 固定为：

| connector profile | 横向道路铺装 envelope | internal edge endpoints | same-lane length |
| ----------------- | --------------------- | ----------------------- | ---------------: |
| main              | `x = -7.0..+7.0 m`    | `x = -10.5..+10.5 m`    |         `21.0 m` |
| secondary         | `z = -10.5..+10.5 m`  | `z = -14.0..+14.0 m`    |         `28.0 m` |

每个 internal endpoint 都在横向道路铺装边缘之外一个 `3.5 m` lane-width setback。
对具体 approach，上游 endpoint 同时是 entry road edge 的末端、StopLine 的
`edgeEnd` 和 `transitionIndex = 0` Gate crossing 的起点；下游 endpoint 是 internal
edge 到 exit road edge 的 split。本文后续的 entry/exit boundary 都指该
road/internal split，不指铺装 envelope，也不在 `±10.5 m` / `±14.0 m` 之外再增加
一次 setback。lane index `0` 从道路中心侧向外递增。

## 3. Identity、owner 与精确计数

### 3.1 稳定 ID

两个 Junction ID 固定为 `junction-1`、`junction-2`。其他新增 identity 使用：

```text
movement-junction-{1|2}-{west|east|north|south}-{left|straight|right}
path-junction-{1|2}-{approach}-{turn}-lane-{entry}-to-{exit}
edge-junction-{1|2}-{approach}-{turn}-lane-{entry}-to-{exit}-i0
gate-junction-{1|2}-{approach}-{turn}-lane-{entry}-to-{exit}
stop-line-junction-{1|2}-{approach}-lane-{entry}
signal-group-junction-{1|2}-{main-left|main-through-right|secondary-left|secondary-through-right}
signal-controller-junction-{1|2}
```

`approach` 表示车辆进入 Junction 前所在的方位，而不是行驶朝向。本文 Route 表使用
短记号 `J{1|2}-{W|E|N|S}-{L|S|R}-{entry}>{exit}`，它严格展开为上述
ManeuverPath ID。例如 `J1-W-S-1>0` 等价于
`path-junction-1-west-straight-lane-1-to-0`。短记号不是 wire identity。
每个 path record 必须显式引用同 Junction/approach/turn 的 parent Movement；
每个 Gate 必须显式引用 path，不能把上述命名模式当作 runtime 关系推断。

### 3.2 Road edge 与 internal edge

LaneGraph 独立拥有 34 条 road edges：

```text
edge-main-{w2e|e2w}-lane-{0..2}-road-{0|2|4}
edge-side-{1|2}-{n2s|s2n}-lane-{0..1}-road-{0|2}
```

main `road-0` 是 portal 到先到达 Junction 的 approach，`road-2` 是两个 Junction
之间的 lane，`road-4` 是后一个 Junction 到 portal 的 exit。side `road-0` 是
portal approach，`road-2` 是 Junction exit。

每条 ManeuverPath 恰好拥有一个不同的 internal edge，完整 path edge sequence
固定为：

```text
[entry road edge, exactly one internal edge, exit road edge]
```

LaneGraph 仍是 internal edge 的实体 owner；每条 internal edge 通过唯一
ManeuverPath membership 派生到一个且仅一个 Junction owner。禁止根据 ID
substring 推断 owner。

### 3.3 总量 invariant

| 实体                      | 每个 Junction | 走廊总计 |
| ------------------------- | ------------: | -------: |
| Junction                  |             1 |        2 |
| Movement                  |            12 |       24 |
| ManeuverPath              |            16 |       32 |
| internal LaneEdge         |            16 |       32 |
| ManeuverGate              |            16 |       32 |
| StopLine                  |            10 |       20 |
| road LaneEdge             |             — |       34 |
| 全部 LaneEdge             |             — |       66 |
| Route                     |             — |       28 |
| Route/Maneuver occurrence |             — |       44 |

ConflictZone 与 ParticipantStream 的数量由 §7.4 的编制规则唯一确定，不另行手写。
实现交付时把两个 Junction 的精确数量回写本表，并纳入同一 exact invariant；数量与
规则不一致时 generator 失败。

## 4. Movement 与 lane assignment

每个 Junction 的 west/east approach 属于主干道，north/south approach 属于次干道。
每个 approach 都有 left/straight/right 三个道路级 Movement，不含 U-turn。

### 4.1 主干道 approach

| entry lane | 允许用途 | exit road/lane | path 数 | phase class          |
| ---------: | -------- | -------------- | ------: | -------------------- |
|          0 | left     | secondary 0    |       1 | `main-left`          |
|          1 | straight | main 0 或 1    |       2 | `main-through-right` |
|          2 | straight | main 2         |       1 | `main-through-right` |
|          2 | right    | secondary 1    |       1 | `main-through-right` |

lane 0 是专用左转；lane 1 是专用直行，但可在一个 Junction 内平滑落入下游 main
lane 0 或保持 lane 1；lane 2 允许直行或右转。

### 4.2 次干道 approach

| entry lane | 允许用途 | exit road/lane | path 数 | phase class               |
| ---------: | -------- | -------------- | ------: | ------------------------- |
|          0 | left     | main 0         |       1 | `secondary-left`          |
|          1 | straight | secondary 1    |       1 | `secondary-through-right` |
|          1 | right    | main 2         |       1 | `secondary-through-right` |

lane 0 是专用左转；lane 1 允许直行或右转。一个 entry lane 的全部候选 path 必须
属于同一个 phase class。该 invariant 防止同 lane 的 left 为 red、straight 为
green 时发生无法由当前纵向模型解决的队首阻塞。

每个 approach/lane 只有一个 StopLine。该 lane 的各候选 ManeuverGate 引用同一个
StopLine，但保持不同 Gate identity。所有 Gate 都位于 path 的 entry transition，
即 `transitionIndex = 0`。

## 5. Geometry、length 与 speed

### 5.1 Canonical point sequence

Traffic `EdgeLength` 与 Spatial polyline 必须从同一个 canonical point sequence
派生，不能分别手写：

- connector 从 §2 的上游 StopLine/Gate split 开始，在下游 road/internal split
  结束；不存在 StopLine 到 internal edge 之间的未建模 gap；
- same-lane straight 使用 entry/exit endpoint 组成的两点折线；
- main `lane 1 -> lane 0` straight 使用一个 internal edge。令 `t=i/64`，
  longitudinal 分量线性插值，lateral 分量使用
  `s(t)=3t²-2t³`，再组合为 canonical point；
- left/right turn 使用由两个 endpoint 与其道路切线唯一确定的 XZ
  axis-aligned quarter-ellipse，参数角按 `i/64` 等分；
- lane-shift straight 和所有 turn 均为 64 段/65 点；
- 每个点先量化到 `1 mm`，`Y=0`，再以量化后的相邻点弧长和生成 Traffic
  `EdgeLength`；
- Spatial 继续执行 `1 cm` Traffic/Spatial length binding、`5 mm` join 和
  既有 pose validation。

外侧到外侧 right-turn centerline radius 从 v0.8 的 `1.75 m` 提高为 `5.25 m`。
默认生成值必须落在以下 reference envelope；实现测试使用 generator 的量化后精确
golden，不把表中三位小数反向当作输入：

main same-lane 的 `21 m = 14 m` 铺装宽度 `+ 2 × 3.5 m` setback；secondary
same-lane 的 `28 m = 21 m + 2 × 3.5 m`。因此下表长度已经包含 StopLine setback，
不能再额外加长 `3.5 m`。

| connector                        | reference length | 其他边界                                   |
| -------------------------------- | ---------------: | ------------------------------------------ |
| main same-lane straight          |         `21.000` | 两点折线                                   |
| secondary same-lane straight     |         `28.000` | 两点折线                                   |
| main `lane 1 -> lane 0` straight |        `~21.346` | endpoint sampled tangent 偏差 `~0.524°`    |
| left                             |        `~22.077` | 相向 protected-left 最小中心线净距 `~6.49` |
| right                            |         `~8.246` | centerline radius `5.25 m`                 |

所有 64-segment connector 的最短 sampled segment 约为 `0.128 m`，必须高于
Spatial 的 `0.1 m` 最小线段约束。若量化或公式改变导致该条件不成立，generator
必须失败。

### 5.2 Speed limit

| connector class    |  公示值 |
| ------------------ | ------: |
| main straight      | 60 km/h |
| secondary straight | 40 km/h |
| left               | 25 km/h |
| right              | 15 km/h |

road edge 继续使用道路本身的 main `60 km/h` / secondary `40 km/h` 限速。
车辆期望速度不能覆盖 edge speed ceiling、SignalStop、Following 或 no-overlap
authority。

## 6. 完整 Route catalog

### 6.1 Route identity 与 occurrence

下表冻结全部 28 条 Route。`lane` 是 entry PortalLane；`weight` 只在该
PortalLane 内归一化。raw weights 跨整个 portal 相加为 100 仅用于审阅，不表示在
lane draw 之前直接按 100 分配概率。

| Route ID                                      | entry lane | weight | exit portal           | ordered Maneuver occurrences |
| --------------------------------------------- | ---------: | -----: | --------------------- | ---------------------------- |
| `route-main-west-near-left`                   |          0 |     20 | `portal-side-1-north` | `J1-W-L-0>0`                 |
| `route-main-west-far-left-via-lane-1`         |          1 |     12 | `portal-side-2-north` | `J1-W-S-1>0`, `J2-W-L-0>0`   |
| `route-main-west-through-lane-1-to-0`         |          1 |     12 | `portal-main-east`    | `J1-W-S-1>1`, `J2-W-S-1>0`   |
| `route-main-west-through-lane-1-to-1`         |          1 |     12 | `portal-main-east`    | `J1-W-S-1>1`, `J2-W-S-1>1`   |
| `route-main-west-near-right`                  |          2 |     20 | `portal-side-1-south` | `J1-W-R-2>1`                 |
| `route-main-west-through-lane-2-to-2`         |          2 |     12 | `portal-main-east`    | `J1-W-S-2>2`, `J2-W-S-2>2`   |
| `route-main-west-far-right-via-lane-2`        |          2 |     12 | `portal-side-2-south` | `J1-W-S-2>2`, `J2-W-R-2>1`   |
| `route-main-east-near-left`                   |          0 |     20 | `portal-side-2-south` | `J2-E-L-0>0`                 |
| `route-main-east-far-left-via-lane-1`         |          1 |     12 | `portal-side-1-south` | `J2-E-S-1>0`, `J1-E-L-0>0`   |
| `route-main-east-through-lane-1-to-0`         |          1 |     12 | `portal-main-west`    | `J2-E-S-1>1`, `J1-E-S-1>0`   |
| `route-main-east-through-lane-1-to-1`         |          1 |     12 | `portal-main-west`    | `J2-E-S-1>1`, `J1-E-S-1>1`   |
| `route-main-east-near-right`                  |          2 |     20 | `portal-side-2-north` | `J2-E-R-2>1`                 |
| `route-main-east-through-lane-2-to-2`         |          2 |     12 | `portal-main-west`    | `J2-E-S-2>2`, `J1-E-S-2>2`   |
| `route-main-east-far-right-via-lane-2`        |          2 |     12 | `portal-side-1-north` | `J2-E-S-2>2`, `J1-E-R-2>1`   |
| `route-side-1-north-corridor-left-far-left`   |          0 |     20 | `portal-side-2-north` | `J1-N-L-0>0`, `J2-W-L-0>0`   |
| `route-side-1-north-through`                  |          1 |     60 | `portal-side-1-south` | `J1-N-S-1>1`                 |
| `route-side-1-north-away-right`               |          1 |     20 | `portal-main-west`    | `J1-N-R-1>2`                 |
| `route-side-1-south-away-left`                |          0 |     20 | `portal-main-west`    | `J1-S-L-0>0`                 |
| `route-side-1-south-through`                  |          1 |     60 | `portal-side-1-north` | `J1-S-S-1>1`                 |
| `route-side-1-south-corridor-right-through`   |          1 |     15 | `portal-main-east`    | `J1-S-R-1>2`, `J2-W-S-2>2`   |
| `route-side-1-south-corridor-right-far-right` |          1 |      5 | `portal-side-2-south` | `J1-S-R-1>2`, `J2-W-R-2>1`   |
| `route-side-2-north-away-left`                |          0 |     20 | `portal-main-east`    | `J2-N-L-0>0`                 |
| `route-side-2-north-through`                  |          1 |     60 | `portal-side-2-south` | `J2-N-S-1>1`                 |
| `route-side-2-north-corridor-right-through`   |          1 |     15 | `portal-main-west`    | `J2-N-R-1>2`, `J1-E-S-2>2`   |
| `route-side-2-north-corridor-right-far-right` |          1 |      5 | `portal-side-1-north` | `J2-N-R-1>2`, `J1-E-R-2>1`   |
| `route-side-2-south-corridor-left-far-left`   |          0 |     20 | `portal-side-1-south` | `J2-S-L-0>0`, `J1-E-L-0>0`   |
| `route-side-2-south-through`                  |          1 |     60 | `portal-side-2-north` | `J2-S-S-1>1`                 |
| `route-side-2-south-away-right`               |          1 |     20 | `portal-main-east`    | `J2-S-R-1>2`                 |

该表必须精确产生 28 Route、44 occurrence，并覆盖 32 条 ManeuverPath 至少一次。
Route 的完整 Traffic edge sequence 由 road edges 和每个 occurrence 的三 edge
path 连续拼接后去除共享边重复得到；catalog 不复制该 sequence。

### 6.2 Portal 汇总

| entry portal          | entry lanes | Route 数 | raw weight sum |
| --------------------- | ----------: | -------: | -------------: |
| `portal-main-west`    |           3 |        7 |            100 |
| `portal-main-east`    |           3 |        7 |            100 |
| `portal-side-1-north` |           2 |        3 |            100 |
| `portal-side-1-south` |           2 |        4 |            100 |
| `portal-side-2-north` |           2 |        4 |            100 |
| `portal-side-2-south` |           2 |        3 |            100 |

该表从上到下的 portal 顺序、每个 portal 内升序 lane index，以及 §6.1 每个 lane
内的 Route 顺序都是 catalog normalization 与 deterministic draw 的规范顺序。

3-route portal 的 raw weights 为 straight 60、turn away 20、turn toward corridor
后执行唯一可达 protected turn 20。4-route portal 为 straight 60、turn away 20、
turn toward corridor 后 straight 15、继续 right 5。

### 6.3 Catalog ownership

scenario-local catalog 现行 exact `0.4`。当时从 exact `0.1` clean-break 到 exact `0.2`；
#498 的 0.3 起有序边键由 catalog 拥有，不进 LFCA；0.4 另增加必填
`policy_selection`，显式钉住生成器声明的 `protected-entry` 策略。闭合形状与绑定顺序见
[`signalized-corridor-population.md`][corridor-population-catalog]：

- Portal 拥有 ordered PortalLane；
- PortalLane 拥有 weighted RouteChoice，并引用一个共享 entry SpawnSlot；
- RouteCatalogEntry 记录 Traffic Route 到 exit portal 的 cross-reference；
- SpawnSlot 只拥有 physical portal/lane/edge/progress，不再拥有单一 Route；
- 多条 Route 共享同一个 PortalLane entry slot；
- 出现记录由 `register_route` 从 catalog 边序列编译，catalog 不复制 path sequence。

默认 spawn pitch 从 `20 m` 改为 `10 m`。generator 只在六个 portal 的真实
approach road edges 上生成约 212 个唯一 physical slots；不得在某 portal 已经穿越
Junction 后的中段或 exit edge 上为该 portal 继续生成 slot。exact slot 数由量化后
edge capacity 和 no-overlap 规则导出，并由 generated golden 冻结。

### 6.4 Seeded selection

每次 completion 的选择顺序固定为：

```text
uniform(排除刚完成 exit portal 后的 5 个 entry portals)
  -> uniform(目标 portal 的 ordered 2/3 个 PortalLane)
  -> weighted(选中 PortalLane 的完整 RouteChoice)
```

每次成功冻结新 plan 固定调用 portal/lane/route 三个 logical bounded draw site。
它们继续使用 v0.8 的 `uniform(bound)` rejection sampling；“三个”冻结的是调用点和
顺序，不表示发生 rejection 时 `next_u64` 最多推进三次。route site 不对单一
RouteChoice 特判：始终先按规范顺序求全部正整数权重的
`totalPositiveWeight`，再以一次 `uniform(totalPositiveWeight)` 的结果做
cumulative selection。某 lane 只有一个 RouteChoice 时，bound 仍是该 choice 的
正整数 raw weight，draw 后必然选中该 Route；不得改用 `uniform(1)`、跳过 draw 或
预先约分权重。

blocked retry 不 draw、不改变 frozen plan。初始化先对全部 physical slots 做
Fisher–Yates，然后按 logical slot order 对其 PortalLane 各执行一次 weighted
route draw。

raw weight 只在选中的 PortalLane 内归一化。因此“每个 portal raw weights 合计
100”是 catalog 审阅 invariant，不是最终 Route 概率；测试必须直接覆盖
portal-first/lane-second/route-third 的实际分布与 golden draw order。

## 7. Signal permission matrix

### 7.1 SignalGroup membership

每个 Junction 有四个 SignalGroup：

| group suffix              | path 数 | path membership per Junction                       |
| ------------------------- | ------: | -------------------------------------------------- |
| `main-left`               |       2 | `W-L-0>0`, `E-L-0>0`                               |
| `main-through-right`      |       8 | `W/E-S-1>0`, `W/E-S-1>1`, `W/E-S-2>2`, `W/E-R-2>1` |
| `secondary-left`          |       2 | `N-L-0>0`, `S-L-0>0`                               |
| `secondary-through-right` |       4 | `N/S-S-1>1`, `N/S-R-1>2`                           |

表中的 `W/E`、`N/S` 分别展开两个 approach。所有 16 条 production path 都有一个
引用其 group 的 entry ManeuverGate。禁止使用 `signalControl:none` 表达无条件
right-of-way。

### 7.2 完整 12-phase program

aspect 列顺序固定为 main-left（ML）、main-through-right（MTR）、
secondary-left（SL）、secondary-through-right（STR）：

| phase ID                                      | duration | ML     | MTR    | SL     | STR    |
| --------------------------------------------- | -------: | ------ | ------ | ------ | ------ |
| `phase-main-left-green`                       | 10,000ms | green  | red    | red    | red    |
| `phase-main-left-yellow`                      |  3,008ms | yellow | red    | red    | red    |
| `phase-after-main-left-all-red`               |  1,008ms | red    | red    | red    | red    |
| `phase-main-through-right-green`              | 30,000ms | red    | green  | red    | red    |
| `phase-main-through-right-yellow`             |  3,008ms | red    | yellow | red    | red    |
| `phase-after-main-through-right-all-red`      |  1,008ms | red    | red    | red    | red    |
| `phase-secondary-left-green`                  |  8,000ms | red    | red    | green  | red    |
| `phase-secondary-left-yellow`                 |  3,008ms | red    | red    | yellow | red    |
| `phase-after-secondary-left-all-red`          |  1,008ms | red    | red    | red    | red    |
| `phase-secondary-through-right-green`         | 20,000ms | red    | red    | red    | green  |
| `phase-secondary-through-right-yellow`        |  3,008ms | red    | red    | red    | yellow |
| `phase-after-secondary-through-right-all-red` |  1,008ms | red    | red    | red    | red    |

本表使用检入 authoring config 的默认时长；yellow 与 all-red 均对齐 `16 ms` Tick。
cycle 为 `10000 + 30000 + 8000 + 20000 + 4 × 3008 + 4 × 1008 = 84,064 ms`，
即 `5,254` Tick。`signal-controller-junction-1`、
`signal-controller-junction-2` offsets 分别为 `0 ms`、`42,000 ms`。每个 phase
必须完整列出四个 group aspect；任一时刻最多一个 group 为 green 或 yellow。

### 7.3 Compatibility 与 clearance

group-level compatibility matrix 固定为：

| active with | ML  | MTR | SL  | STR |
| ----------- | --- | --- | --- | --- |
| ML          | yes | no  | no  | no  |
| MTR         | no  | yes | no  | no  |
| SL          | no  | no  | yes | no  |
| STR         | no  | no  | no  | yes |

四个不同 group 互不兼容。同一个 group 内的 authoring 必须满足：

- 非共享 polyline 不 crossing；
- 不同 approach 不争用同一个 exit lane；
- 不在进入 route-aware Following 前发生跨 branch merge；
- `main-through-right` 最小非共享中心线距离至少 `3.5 m`；
- shared-entry alternatives 只来自同一 lane，并由已冻结 Route 与
  route-aware Following 串行约束。

yellow + following all-red 合计约 `4 s`，是相位交接的常规间隔，不承担“下一组转绿前
前一组车辆全部驶离 internal edge”的安全责任；跨组空间安全由 §7.4 承担。

#1272 黄灯困境区许可引入黄灯期入场后，按 `t_entry + T_clear ≤ Y + R` 核验 internal
edge 清空的结果未通过（#1446 工作包 2）。#196 G1 的停车可行性保持
（`traffic-runtime-integer-geometry.md` §5.1、`signal-system.md` §6）交付后，黄灯期
入场只剩按已提交状态确实不可紧急停车的车辆，超时样本从 409 降为 6；剩余样本都以
connector 限速入场，按限速行驶也无法在 `Y + R` 内驶离 internal edge。若继续以
internal edge 全空为安全条件，只能在排除 internal edge 上静止排队的运行边界下延长
全红；允许等待前车时，任何有限的固定时长都无法覆盖全部情况。#1507 因此改由冲突区
约束承担交接安全。`t_c − t_y` 保留为诊断量，不再作为判据；不得以放宽判据或宽限期
代替冲突区约束（§8.2）。

### 7.4 跨组冲突区

**编制范围**：

- 两条 production path 属于同一 Junction 的不同 SignalGroup，且按 §5.1 canonical
  point sequence 以 `3.5 m` lane width 扫掠的带状区域相交（交叉），或以同一条 exit
  road lane 为下游边（合流）时，generator 为该路径对声明一个 ConflictZone；
- 同组路径已由 §7.3 保证不交叉、不争用出口车道，不声明冲突区；generator 发现同组
  路径相交或共享出口车道时按 §8.4 失败，不能用冲突区补洞；
- 共用同一 entry lane 的路径属于同一组，由已冻结 Route 与 route-aware Following
  串行约束，不声明冲突区；
- 带宽取 lane width：车辆横向不越出所在车道，按车道宽度扫掠的区域覆盖车身可能占用
  的范围，冲突区不随车型改变。

**passage 区间**：

- 交叉：每条路径的 passage 覆盖其扫掠带与对方扫掠带重叠的路线区间；车尾越过区间
  终点才算清空；
- 合流：passage 从两条扫掠带开始重叠处，延伸到共享出口车道内 `min_gap` 处，保证后
  进入的车辆有完整车长与净距；
- passage 归属该路径唯一的 entry ManeuverGate；静态地址、路线锚点与 Gate range 按
  `traffic-runtime-conflict-occurrence.md` 编译，Runtime 不从几何重新推导。

**ParticipantStream 与路权**：

- 每条参与至少一个 ConflictZone 的 path 声明一条 ParticipantStream，全部 stream
  使用同一 priority；跨组先后只由信号相位与冲突占用决定，不声明让行间隙参数；
- protected-entry 策略不变，全部 Gate 以 `ProtectedGroup` 解释。`Protected` 只跳过
  yield/gap，不跳过冲突占用、预约、下游净空与运动安全
  （`waiting-zone-conflict-right-of-way.md` §6.2）；
- 策略安装的 protected coherence 检查必须通过。§7.2 每个 steady phase 至多一组
  非红，共享冲突区的两组不会在同一 phase 同时得到 `Protected` 候选；检查失败即
  authoring 错误。

**相位交接**（Traffic Runtime 现行合同的推论，本 profile 不新增 Runtime 行为）：

- 车辆在 Gate crossing 时取得预约，车头进入 passage 建立占用，车尾越过 clearance
  后释放（`waiting-zone-conflict-right-of-way.md` §6.8）；前一组已合法入场的车辆
  在下一组转绿后继续持有占用；
- 下一组车辆在冲突区仍被占用或预约时得不到授权，在 Gate 前等待。占用属于已知持续
  阻塞，在制动前视窗内参与停止归约（`traffic-runtime-phase-protocol.md` §3.1.1），
  车辆不在 internal edge 上停车等待冲突；
- 取得授权还要求下游净空（`waiting-zone-conflict-right-of-way.md` §6.6）。出口车道
  容不下整车时，车辆同样在 Gate 前等待，不驶入 Junction 排队；
- 黄灯困境区许可只解除黄灯造成的拒绝，资源授权、下游净空与 no-overlap 继续生效
  （`traffic-runtime-right-of-way-policy.md` §3.3）。黄灯期确实不可停、又得不到
  授权的车辆只能由具名投影处理，§8.1 要求规范矩阵中不出现这种投影。

`Y + R` 保持 §7.2 现值。是否调整配时属于通行效率问题，由 §8.1 的交接等待诊断提供
依据，不再是清空安全的前提。

## 8. Runtime、验证与 failure policy

### 8.1 必须验证的事实

- exact 28 Routes、44 Maneuver occurrences、32 ManeuverPaths，且 path coverage
  无遗漏；
- Traffic/Spatial/catalog 从同一 config canonical、byte-deterministic regeneration；
- 现行可运行制品是受检 LFCA `v0.2-signalized-corridor.lfca` 与 catalog 0.4
  `v0.2-signalized-corridor.catalog.toml`。文件名是场景制品名，不是格式版本。
  见[示例场景 §9.1][example-scenarios-artifacts]；
- 50/100/200 vehicles、seed 0 和规范 stress seeds；规范 stress seed 集为
  `0/7/42/101`（含默认 seed 0），沿用 #191 v0.9 cross-layer 证据所用集合；
- 所有 left/straight/right path 的 headless traversal、SignalStop/release、
  Following/no-overlap；
- §7.4 的 ConflictZone、passage 与 ParticipantStream 按编制规则生成，策略安装的
  protected coherence 检查通过；
- 跨组冲突安全、交接活性与资源停止不依赖具名投影（见下文清空核验）；
- native pose、lamp、Route observation 与 Traffic Runtime handles 一致；
- weighted draw order、blocked retry、初始 permutation 和不同 outer-frame
  chunking 的 replay；
- steady tick 不做 external-ID lookup、ManeuverPath matching、catalog scan 或
  per-vehicle allocation。

清空核验（#1496）由 `laneflow-scenario` 的 `signalized_corridor_clearance` 测试
承载，使用检入 LFCA 与 catalog，在 50/100/200 × 规范 stress seed 集上各运行 4 个
完整周期。判据只读每个提交边界的公开车辆状态、已提交灯态与 §7.4 的静态 passage
区间，不调用 Runtime 的门决策、冲突仲裁、困境区豁免或停止求解器：

- **冲突安全**：任一提交边界，同一 ConflictZone 内至多一条 ParticipantStream 的
  车身与其 passage 区间相交；车身区间取 `[front − length, front]`；
- **红灯跨越**：以拍初红灯提交的 `fromEdge → internal edge` 跨越一律拒绝；
- **资源停止不依赖具名投影**：所在组未拒绝通行时在 Gate 前减速的车辆，相邻提交
  边界的减速度不超过其车型的 `b_e`；
- **活性**：任一车辆连续零位移的时长不超过一个完整周期（§7.2，84,064 ms）。红灯
  等待短于一个周期，超过即视为冲突或下游净空造成的停滞；窗口结束时尚未达到该时长
  的等待不计为违例。

同一次运行另输出诊断，不参与上述判定：黄灯期入场车辆的 `t_e − t_y` 与
`t_c − t_y`，以及下一组转绿后车辆因冲突区占用或下游净空在 Gate 前等待的时长分布。

失败输出车辆、路线、路径、组、冲突区与占用区间。实现交付前，检入制品不含冲突区，
本测试仍按 internal edge 全空的旧判据运行，现行结果为未通过（#1446 工作包 2）；
实现与新判据同批交付后在合入提交上重跑。完整矩阵以 release 显式运行：

```text
cargo +1.99.1 test --release --locked -p laneflow-scenario --test signalized_corridor_clearance -- --ignored --nocapture
```

同一次运行另为每个黄灯期跨越样本输出诊断（#1498），未闭合样本同样输出并标注
`t_c` 未知：黄灯起点时的速度、到停止线距离与紧急停车距离，跨越前后两个提交边界的
位置与速度，以及到车尾清空或窗口结束为止的逐拍事实。候选原因由这些事实推断，
可以多项同时成立，也可以暂无法确定；诊断只附加输出，不参与上述判定，逐样本结果
不入仓。

接近阶段的停止决策另由 Runtime 的 `motion-trace` 诊断构建重放选定样本（#1500）：
逐拍记录停止目标查询、门决策与困境区豁免、P5 的实际求值路径（列式重新求值或复用
P2 预览）、舒适与紧急两档可行域、前方限速下降约束、停车可行性保持对常规候选的改写（#1505），以及已提交的速度、进度与余数。
记录只读，开启与关闭时的已提交状态与事件一致；该 feature 默认关闭，宿主构建不开启。

```text
cargo +1.99.1 test --locked -p laneflow-scenario --features motion-trace --test signalized_corridor_yellow_trace -- --nocapture
```

### 8.2 Phase handoff 的阻断条件

相位交接时前一组车辆仍在 Junction 内是合法状态，由 §7.4 的冲突区约束保证安全。
如果 50/100/200、seed 0 或规范 stress seed 违反 §8.1 清空核验的任一判据：

1. 测试不得放宽；
2. 不得延长 hard-coded grace period 掩盖问题；
3. 不得在 profile 或 scenario 中加 private heuristic 绕过 Runtime 的冲突仲裁；
4. 必须回到 G1，明确是 §7.4 的编制规则、配时还是 Traffic Runtime 合同需要修订，
   再实施。

### 8.3 组合 authority

| 关注点             | authority                               | profile 只提供                                          |
| ------------------ | --------------------------------------- | ------------------------------------------------------- |
| population/recycle | `laneflow-scenario` caller policy       | PortalLane、RouteChoice、slot cross-reference 与权重    |
| speed/longitudinal | Traffic Runtime speed limit + Following | 每条 edge 的 speed ceiling；不增加 turn-specific 跟车器 |
| stop/release       | Traffic Runtime Signals + ManeuverGate  | path/Gate/group/phase 的显式 cross-reference            |
| conflict/space     | Traffic Runtime 冲突仲裁与下游净空      | ConflictZone、passage 与 ParticipantStream 的静态声明   |
| geometry/pose      | Traffic length + Spatial centerline     | 同源 point sequence 与 binding                          |
| presentation       | Adapter read-only observation           | stable handles；不让 lamp/mesh/Transform 回写 authority |

Adapter 可以按 ManeuverPath/SignalGroup handle 显示 route、灯具和车辆位姿，但不得
重新判断 turn permission、用几何猜测 Route，或在 presentation 层修正 Traffic Runtime
progress。

### 8.4 结构化错误

loader、generator 和 scenario normalization 必须保留所属 artifact 的 field path
以及可用的 Junction/Movement/Path/Gate/Route/Portal external ID，不以 panic、静默
丢弃或 ID substring fallback 处理 profile 错误。至少区分：

| 错误类别             | 必须覆盖的事实                                                       |
| -------------------- | -------------------------------------------------------------------- |
| topology/owner       | parent 不匹配、owner 不唯一、非法 lane assignment、path 非三 edge    |
| geometry/binding     | 非有限/量化后退化、长度超差、join 超差、pose 失败                    |
| route/coverage       | route 不连续、occurrence 歧义、28/44/32 exact invariant 不成立       |
| catalog/selection    | 版本、顺序、重复 cross-reference、非正 weight、slot/entry 不匹配     |
| signal/permission    | missing/duplicate Gate、错误 StopLine/group、phase aspect 不完整     |
| conflict authoring   | 冲突区编制与 §7.4 规则不一致、同组路径相交、protected coherence 失败 |
| clearance validation | 冲突区同时占用、红灯跨越、资源停止超出 `b_e`、零位移超过一个周期     |

exact Rust enum 由拥有该 boundary 的 #229/#190–#192 crate 定义，但不能把不同类别
压缩成无上下文的字符串错误。

## 9. 版本、迁移与非目标

| artifact/API       | v0.9 决策                                                               |
| ------------------ | ----------------------------------------------------------------------- |
| Traffic package    | exact `0.7 -> 0.8` clean break                                          |
| scenario catalog   | exact `0.1 -> 0.2` clean break                                          |
| SpatialPackage     | shape 保持 exact `0.1`；geometry bytes/length/digest 更新               |
| ScenarioManifest   | shape 保持 exact `0.1`；Traffic/Spatial size 与 digest 更新             |
| Signals Gate API   | pair-based `MovementGate` clean-break 为一等 `ManeuverGate`             |
| loader/catalog DTO | 不保留旧 loader、旧 DTO、deprecated alias、dual query 或 migration shim |

非目标包括 U-turn、lane change、红灯右转、permissive left、gap acceptance、
pedestrian、adaptive/actuated signal、通用 pathfinding、跨组 priority、
RoadSection/LaneGroup/JunctionGroup runtime。ConflictZone 与预约只用于 §7.4 的跨组
空间约束，不扩展为通用 conflict solver。

#1507 的实现在检入 LFCA 与 catalog 中增加冲突区与 ParticipantStream，按 1.0 前规则
直接切换，不保留旧制品、旧 loader 或兼容路径。

## 10. 实施批次与完成边界

| 交付                                                                        |   owner Issue |
| --------------------------------------------------------------------------- | ------------: |
| 通用 Core/Data static model、历史 Traffic 0.8 schema/loader、generator 基础 |          #229 |
| current Traffic 0.10 横断面/准入迁移，protected profile behavior 不变       |          #262 |
| protected profile artifacts、catalog 0.2、scenario/native 最小集成          |          #190 |
| 50/100/200、stress seeds、clearance/replay/proxy/performance 完整证据       |          #191 |
| 独立 closure review、文档/关系/Gate 汇总，不新增 runtime 行为               |          #192 |
| 跨组冲突区与冲突安全/活性清空判据的 G1（§7.4、§8）                          |         #1507 |
| 冲突区生成、制品重新生成与清空核验改写                                      | G1 接受后另开 |

#196 只交付 Accepted profile 及同步文档，不生产 Rust、schema、loader、generator、
fixtures、artifacts 或 native example。#229 提供通用静态底座；#190 才拥有本文具体
profile 的生成制品、catalog/runtime policy 和最小 native smoke。#191 扩大验证样本并
承担 clearance/performance 证明，#192 只做独立最终审阅；任一后续 Issue 都不得借验证或
closure 名义新增 production 行为。

[road-junction-model]: road-junction-model.md
[example-scenarios]: example-scenarios.md
[example-scenarios-artifacts]: example-scenarios.md#91-production-制品
[corridor-population]: signalized-corridor-population.md
[signal-system]: signal-system.md
[spatial-geometry]: spatial-geometry.md
[adr-0017]: ../adr/0017-static-road-junction-maneuver-and-gate-identity.md
[corridor-population-catalog]: signalized-corridor-population.md#3-catalog-契约
