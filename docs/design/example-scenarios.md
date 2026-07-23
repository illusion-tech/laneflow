# 示例场景设计

**文档状态**: Accepted（#184 G1）<br>
**最后更新**: 2026-07-23<br>
**适用范围**: v0.8 Signalized Corridor MVP 的直行信号化走廊、制品生成、启动配置、人口与车辆回流基线

**关联 ADR**:

- [`../adr/0013-engine-neutral-spatial-geometry-and-length-authority.md`](../adr/0013-engine-neutral-spatial-geometry-and-length-authority.md)
- [`../adr/0015-bounded-f32-canonical-spatial-frames.md`](../adr/0015-bounded-f32-canonical-spatial-frames.md)
- [`../adr/0016-scenario-population-and-recycle-lifecycle-authority.md`](../adr/0016-scenario-population-and-recycle-lifecycle-authority.md)

## 1. 目标与交付边界

v0.8 提供一个可持续运行、可复现且使用 production loader 的 native reference 场景：一条双向六车道主干道与两条双向四车道次干道垂直相交，形成两个平面信号交叉口。场景只允许直行，但必须证明多路口信号、道路限速、50–200 辆车人口和出口回流可以在现有 Core/Spatial/Adapter 分层下组合工作。

v0.8 包含：

- 物理道路轴线总长不超过 2 km，默认 1.4 km；
- 6 个 portal-level 直行 movement，展开为 14 条 lane-level routes；
- 主干道 60 km/h、次干道 40 km/h 的 per-edge 限速；
- 两套可配置固定时制信号控制器；
- `50..=200` 可调车辆人口、显式 seed 和确定性出口回流；
- 同一 Bevy proxy/model 复用，但每次回流获得新的 Core `VehicleHandle`；
- Traffic v0.7 current contract、SpatialPackage v0.1 和 ScenarioManifest v0.1；
- checked-in 默认制品、确定性 generator 与 production loader 往返验证。

v0.8 不包含转向、换道、路径搜索、感应或自适应信号、运行时热修改信号、行人、停车、匝道、路网编辑器和保存/恢复 runtime snapshot。受保护左转、直行和右转属于 v0.9 #194 的强制范围，不能从长期路线图中删除。

Traffic v0.7 per-edge 限速的 production schema/Core 基础由 #185 交付；Core atomic replace 由 #186 承担，v0.8 caller-owned 人口/回流策略由 #203 在 `laneflow-scenario` 落地，generator、Adapter 与 native example 由 #187–#189 承担，#195 独立收口。#203 的 crate/API、严格 validation、两阶段 bootstrap、transport-neutral lifecycle 和零分配边界见 [`signalized-corridor-population.md`](signalized-corridor-population.md)。

## 2. 单位、坐标与道路尺度

### 2.1 单位与坐标

- 距离：米；
- 速度：制品和 Core 使用米每秒；
- 时间：authoring/startup config 使用整数毫秒，normalize 后进入 Core fixed-tick 时间模型；
- canonical 场景采用右手、Y-up 坐标；
- 主干道沿 X 轴，西向东为 `+X`；
- 次干道沿 Z 轴，北端为 `Z = -150`、南端为 `Z = +150`，南向北为 `-Z`；
- 默认车道宽度 `3.5 m`；
- 采用右侧通行，lane index `0` 从中央分隔线向道路外侧递增。

### 2.2 物理长度口径

“道路总长”只计算三条物理道路的轴线，不把双向 edge、各 lane 或交叉口 connector 重复相加：

| 物理道路   | 轴线范围                     |    长度 |
| ---------- | ---------------------------- | ------: |
| 主干道     | `X = -400..+400`             |   800 m |
| 1 号次干道 | `Z = -150..+150`，`X = -200` |   300 m |
| 2 号次干道 | `Z = -150..+150`，`X = +200` |   300 m |
| 合计       | 三条轴线之和                 | 1,400 m |

默认值为 1.4 km，generator 必须拒绝轴线总长大于 2 km 的配置。directed lane edge 和 connector 的累计长度只用于 Traffic/Spatial 一致性及 route progression，不属于产品道路总长指标。

两个交叉口中心分别为 `(-200, 0, 0)` 与 `(+200, 0, 0)`。交叉口范围、停止线退距和 connector 几何由 generator 的同一中心线输入派生；不得分别手写 Traffic 长度与 Spatial 折线。

## 3. Portal、movement 与 lane route

### 3.1 Portal catalog

六个外部入口/出口使用稳定 ID：

| Portal ID             | 位置           | 驶入场景方向 | 车道数 |
| --------------------- | -------------- | ------------ | -----: |
| `portal-main-west`    | 主干道西端     | 东向         |      3 |
| `portal-main-east`    | 主干道东端     | 西向         |      3 |
| `portal-side-1-north` | 1 号次干道北端 | 南向         |      2 |
| `portal-side-1-south` | 1 号次干道南端 | 北向         |      2 |
| `portal-side-2-north` | 2 号次干道北端 | 南向         |      2 |
| `portal-side-2-south` | 2 号次干道南端 | 北向         |      2 |

每个 portal 同时是某组 route 的入口和相反方向 route 的出口。回流时“另一入口”表示排除车辆刚驶出的 portal 后，从剩余五个 portal 中选择。

### 3.2 六个 portal-level movement

v0.8 只允许道路轴线方向的直行：

1. 主干道西到东；
2. 主干道东到西；
3. 1 号次干道北到南；
4. 1 号次干道南到北；
5. 2 号次干道北到南；
6. 2 号次干道南到北。

主干道 movement 各有三条 lane routes，次干道 movement 各有两条，共 14 条 concrete routes：

| Route ID pattern               | 数量 | 起点 -> 终点          | 穿越路口数 |
| ------------------------------ | ---: | --------------------- | ---------: |
| `route-main-w2e-lane-{0..2}`   |    3 | west -> east          |          2 |
| `route-main-e2w-lane-{0..2}`   |    3 | east -> west          |          2 |
| `route-side-1-n2s-lane-{0..1}` |    2 | side-1 north -> south |          1 |
| `route-side-1-s2n-lane-{0..1}` |    2 | side-1 south -> north |          1 |
| `route-side-2-n2s-lane-{0..1}` |    2 | side-2 north -> south |          1 |
| `route-side-2-s2n-lane-{0..1}` |    2 | side-2 south -> north |          1 |

每条 route 是 finite explicit edge sequence，不使用 runtime pathfinding。主干道 route 包含三个道路区段和两个直行 connector；次干道 route 包含 approach、一个直行 connector 和 exit。不同 lane route 不互相连接，因此 v0.8 不发生换道。

generator 必须为每个交叉口生成独立的直行 connector edge，并让 Traffic edge length 与 Spatial polyline quantize 后弧长通过既有长度绑定校验。不能用一个跨越停止线和 conflict area 的长 edge 替代 connector，否则 Signal gate 无法绑定明确的 `(from, to)` traversal。

## 4. 限速与车辆纵向行为

Traffic v0.7 current contract 在每个 lane edge 上要求严格正、有限的 `speedLimit`，单位为 m/s：

| 道路类别                 |  公示值 |                   制品值 |
| ------------------------ | ------: | -----------------------: |
| 主干道及其直行 connector | 60 km/h | `16.666666666666668` m/s |
| 次干道及其直行 connector | 40 km/h |  `11.11111111111111` m/s |

`VehicleProfile.desiredSpeed` 继续表达车辆自由流期望速度，不替代道路限速。纵向控制每 tick 至少合并当前 edge 的 speed ceiling、下游更低限速边界的 advance-braking spatial target、leader/no-overlap、SignalStop 和 route completion。

车辆不得以超过当前 edge 限速的初始速度 spawn/replace。车辆不得在 crossing 下游限速边界时仍超过新限速。若多个约束同时存在，沿 route 最近且最严格的可行约束生效；道路限速不得绕过既有 SignalStop 或 no-overlap hard projection。

默认初始与回流速度均为 `0 m/s`，减少入口容量和限速切换的歧义。

## 5. 信号控制器

### 5.1 控制器与 signal group

每个交叉口拥有独立 controller：`controller-intersection-1` 与 `controller-intersection-2`。Signal group ID 在 package 内全局唯一：1 号路口使用 `group-intersection-1-main` / `group-intersection-1-secondary`，2 号路口使用 `group-intersection-2-main` / `group-intersection-2-secondary`；每对 group 分别控制本路口主干道双向六条和次干道双向四条直行 lane movement。

每个 lane connector 的 MovementGate 绑定对应入口 edge 与 connector edge。authoring/generator 负责完整枚举 gate 与 phase group state，并证明同一 phase 内不存在主/次干道冲突开放；Core 不推断 conflict matrix。

### 5.2 固定六阶段程序

| 顺序 | 阶段             | 主干道 group | 次干道 group | 时长来源           |
| ---: | ---------------- | ------------ | ------------ | ------------------ |
|    1 | main green       | Green        | Red          | `mainGreenMs`      |
|    2 | main yellow      | Yellow       | Red          | `yellowMs`         |
|    3 | all red          | Red          | Red          | `allRedMs`         |
|    4 | secondary green  | Red          | Green        | `secondaryGreenMs` |
|    5 | secondary yellow | Red          | Yellow       | `yellowMs`         |
|    6 | all red          | Red          | Red          | `allRedMs`         |

red time 由完整 phase program 推导，不提供独立 `redMs`。v0.8 只在生成/启动时读取配置，不支持运行中的热修改。

默认 TOML 配置为 `main_green_ms=30000`、`secondary_green_ms=20000`、`yellow_ms=3000`、`all_red_ms=1000`，两个 `intersection_offsets_ms` 均为 `0`。所有时长必须为严格正整数且不小于 fixed delta，但不要求能被 fixed delta 整除；offset 必须已经满足 `0 <= offset < cycle`，generator 不做静默取模。generator/loader 沿用 Signal System 的完整 group-state validation；默认值只保证可复现和无冲突，不宣称交通优化。

## 6. 人口、初始分布与启动参数

### 6.1 Native runtime 参数

native reference 至少公开：

```text
--vehicles <50..=200>   # 默认 100
--seed <u64>            # 默认 0
--config <path>         # 默认使用 checked-in authoring/startup config
```

非法车辆数、解析失败、未知 portal/route、无足够 spawn slots 或 artifact validation failure 都必须在第一个 Core step 前返回明确错误，不能静默 clamp 或降级。

### 6.2 Stable spawn-slot catalog

generator 从相同 lane centerline 和车辆安全间距规则生成稳定的 initial spawn-slot catalog。slot：

- 只位于 portal 到第一个 stop line 之间或路口间的普通 road segment；
- 不位于 connector、conflict area、停止线 hard projection 范围或 route completion 边界；
- 带稳定 portal、route、edge 与 progress identity；
- 以文档化稳定顺序进入 catalog；
- 通过 Core production spawn validation 最终确认 overlap 和 route invariant。

默认几何和 profile 必须提供至少 200 个合法 slot，否则 generator/config validation 失败。catalog 的规范顺序依次使用本文件 Portal 表顺序、route lane index 升序、route edge index 升序和 edge-local progress 升序；不得依赖 hash map、文件系统或 ECS iteration order。初始化使用显式 seed 对完整 catalog 执行从末尾到开头的 Fisher–Yates：按 `i = len-1, len-2, ..., 1` 的降序依次计算 `j = uniform(i+1)` 并交换 `slot[i]` / `slot[j]`，完成后取前 N 个 slot。

checked-in 默认配置使用 `20 m` slot pitch，并在每个 eligible segment 两端保留 `vehicle length + minGap = 6.5 m`；由此确定性生成 230 个 slot。slot 只落在每条 route 的入口 edge（`route_edge_index = 0`），以及六条主干道 route 的路口间 edge（`route_edge_index = 2`），不在 terminal exit segment 生成。

每个 logical population slot 拥有稳定 external vehicle ID，例如 `corridor-vehicle-000`。初始 spawn 和后续 replace 都使用该 external ID，但每次旅程拥有新的 Core handle generation。

## 7. 出口回流

### 7.1 两级均匀选择

车辆完成 route 后不从场景消失。#203 的 caller-owned reference policy 为该 logical slot 建立 pending plan；Core 本身不自动回流：

1. 从除刚驶出 portal 外的其余 5 个 portal 中均匀选择目标 portal；
2. 从该 portal 的 2 或 3 条入口 lane routes 中均匀选择一条；
3. 使用目标 route 的入口 spawn point 和 `0 m/s` 构造 replacement；
4. 在下一 fixed-step lifecycle boundary 尝试原子 replace。

这是 portal-first、lane-second 的均匀分布，不是对全部 14 条 routes 直接均匀；因此主干道 portal 与次干道 portal 被选中的概率相同。

### 7.2 Blocked retry

入口 overlap 或其他可恢复容量条件阻止 replace 时：

- old vehicle 保持 Completed 且 handle 仍 live；
- proxy 保留最后一次合法 Transform；
- portal/lane/route plan 原样保留到下一 fixed boundary；
- retry 不消耗 PRNG，也不重新抽签；
- 同一 boundary 内该 plan 只尝试一次；
- 其他 pending plan 继续按稳定 insertion order 尝试。

成功后 Core 原子使 old handle stale 并返回 new handle，Adapter 在同一公开事务中把同一 Entity 从 old handle 切换到 new handle。人口的 logical slot 数保持目标值，Bevy 不 despawn/respawn proxy 或 model。

不可恢复的配置或 invariant 错误进入明确 fatal/diagnostic 路径，不能无限伪装成入口阻塞。

### 7.3 Fixed-step 顺序

每个 Core fixed step 的 caller-owned 顺序固定为：

```text
apply pending lifecycle commands
  -> Core fixed step
  -> consume ordered completion events
  -> enqueue pending plans for the next lifecycle boundary
```

若一个 outer frame 运行多个 catch-up step，每个 step 间仍执行上述顺序。Presentation 每个 outer frame 最多提交一次，因此 frame chunking 不改变 Core/population 决策序列。

## 8. PRNG 契约

v0.8 使用项目自有 `SplitMix64`，state 由 `u64 seed` 直接初始化，零 seed 合法。实现使用 wrapping `u64` 运算和下列固定常量：

```text
increment = 0x9E3779B97F4A7C15
mul1      = 0xBF58476D1CE4E5B9
mul2      = 0x94D049BB133111EB
```

`next_u64` 的混合顺序为：state 加 increment；`z ^= z >> 30` 后乘 `mul1`；`z ^= z >> 27` 后乘 `mul2`；返回 `z ^ (z >> 31)`。

有界抽样 `uniform(bound)` 要求 `bound > 0`，且 `bound` 与 draw `r` 都是 `u64`。使用 rejection sampling：以 unsigned wrapping 语义计算 `threshold = bound.wrapping_neg() % bound`（等价于 `2^64 mod bound`），拒绝 `r < threshold`，接受后返回 `r % bound`。不得用一次直接 modulo 代替。

初始 permutation、首次 portal draw、首次 lane draw 共享一个显式 state 和冻结的调用顺序。回流 portal candidate 按本文件 Portal 表顺序移除刚驶出的 portal 后构造；目标 portal 内的 lane route 按 lane index 升序构造。blocked retry 不 draw。实现必须用 golden tests 固定至少：

- seed `0` 和非零 seed 的前若干 `next_u64`；
- bound `2`、`3`、`5` 的抽样序列；
- 50/100/200 初始 slot 选择；
- 多车同 tick completion 的 portal/lane 决策顺序；
- blocked 若干 boundary 后恢复时与未阻塞车辆的 draw state。

确定性承诺仍限定同一 LaneFlow 实现版本和运行环境；更改算法、catalog 顺序或 draw order 必须经过新的版本/迁移决策，不能在 v0.8 内静默改变 replay。

## 9. 制品与配置边界

### 9.1 Production 制品

v0.8 场景由三类 immutable source artifacts 构成：

- Traffic package v0.7：lane graph、14 routes、vehicle profiles、Signals、Parking 空集合和 per-edge speed limit；
- SpatialPackage v0.1：所有 lane/connector centerline 与 canonical frame；
- ScenarioManifest v0.1：Traffic/Spatial 不透明路径、byte size 和 SHA-256 digest 配对。

ScenarioManifest 继续是静态配对清单，不加入 seed、车辆数、spawn slots、runtime handle、Entity 或 engine asset metadata。

### 9.2 Authoring config 与 scenario-local catalog

`examples/config/v0.8-signalized-corridor.toml` 是仓库内部 authoring SSOT，包含道路轴线长度、交叉口位置、lane width、spawn-slot pitch、主/次干道限速、六阶段 signal timing、两个 offset 和 artifacts 输出文件名。它不包含车辆数、seed、回流策略或展示资源；这些分别属于 #203 和 #189 的 caller/Adapter 配置边界。

generator 另行生成 `v0.1-signalized-corridor.catalog.toml`，记录稳定 portal、route、entry slot 和全部 spawn-slot cross-reference。authoring config 与 catalog 都是内部 TOML，不进入 ScenarioManifest，也不改变 Traffic/Spatial production interchange contract。native example 必须先通过 production Traffic/Spatial/Manifest loader，不能直接把 generator 内存结构塞进 Core。

同一配置和 generator 版本必须 byte-deterministically 生成相同 artifacts、size、digest 和 catalog。仓库根目录使用下列命令生成或只读检查：

```powershell
cargo +1.96.0 run --locked -p laneflow-corridor-generator -- generate --config examples/config/v0.8-signalized-corridor.toml
cargo +1.96.0 run --locked -p laneflow-corridor-generator -- check --config examples/config/v0.8-signalized-corridor.toml
```

`check` 不写文件；CI 发现任一 checked-in byte 差异即失败。

## 10. 分层权威与实施切片

| 关注点                                                      | 权威层                               | 实施 Issue |
| ----------------------------------------------------------- | ------------------------------------ | ---------- |
| per-edge speed limit、Traffic v0.7 与纵向约束               | Data/Core                            | #185       |
| caller-driven atomic replace、overlap 与 identity invariant | Core runtime                         | #186       |
| 目标人口、seed、portal/lane 决策与 blocked retry            | `laneflow-scenario` reference policy | #203       |
| typed lifecycle transaction 与 proxy binding                | Bevy Reference Adapter               | #187       |
| 场景 generator、固定时制配置与三类静态制品                  | Data/Authoring                       | #188       |
| native UI/CLI、道路/车辆/灯具呈现与场景集成                 | Bevy Reference Adapter               | #189       |
| 独立审阅、性能/可视/回归证据                                | Cross-layer closure                  | #195       |

Core 是 vehicle identity、状态、overlap、route 和 speed-limit behavior 的权威，但不限制车辆数量，也不拥有回流 policy。`laneflow-scenario` 中 #203 的 caller-owned reference policy 是 v0.8 示例目标人口、seed、catalog normalization 和 portal/lane 决策的权威；未来城市游戏可以完全替换它。Traffic/Spatial 是静态拓扑和几何的权威；Adapter 是 VehicleHandle/Entity 部分双射与宿主 schedule 的权威；Presentation 只拥有 proxy/model/Transform/灯具表现。

## 11. 验收矩阵

设计转为 production 后，v0.8 至少验证：

| 类别     | 必须证明的事实                                                                      |
| -------- | ----------------------------------------------------------------------------------- |
| 几何     | 1.4 km 默认、<=2 km validation、两处垂直平交、14 routes、Traffic/Spatial 长度绑定   |
| 限速     | 主 60/次 40、超限 spawn 拒绝、下游降速提前制动、与 leader/signal 组合               |
| 信号     | 两 controller、完整六阶段、可配置时长/offset、conflict movement 不同时开放          |
| 人口     | 50/100/200 成功初始化、无 overlap、非法范围和容量不足明确失败                       |
| 回流     | 排除原出口、portal-first/lane-second、blocked 不重抽、目标人口保持                  |
| 生命周期 | old handle stale/new live、same Entity/proxy、Core+mapping 失败原子                 |
| 确定性   | 同 seed/fixed input 相同、不同 outer-frame chunking 相同、golden PRNG               |
| 制品     | generator byte deterministic、Manifest digest/size 正确、production loader 往返     |
| 可视     | 道路、车辆、两套灯具状态一致；长期运行车辆不永久消失                                |
| 性能     | 200 车持续运行无 unbounded queue/retained growth；稳态 lifecycle 不做全人口临时分配 |

## 12. 治理与完成边界

- 设计来源：#184，G1 冻结记录见 <https://github.com/illusion-tech/laneflow/issues/184#issuecomment-5041612599>；
- #185、#186–#189 与 #203 分别按自身 Gate Ledger 推进；各 Issue 的当前状态以 GitHub 为准，任一设计 Issue 或上游 G1 都不自动授权下游开工；
- #184 的 Delivery PR 只交付设计与 ADR，不授权下游自动开工；
- v0.8 Milestone 由 #193 跟踪，只有 #195 独立收口通过并满足父目标退出条件后才可完成；
- v0.9 #194 必须在此基线上加入受保护左转、直行、右转与相关车道/相位能力。
