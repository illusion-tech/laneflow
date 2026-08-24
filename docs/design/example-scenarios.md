# 示例场景设计

**文档状态**: Accepted（#184 G1；#196 v0.9 增量）<br>
**最后更新**: 2026-08-24<br>
**适用范围**: 信号化走廊几何、受保护转向 profile、catalog 0.2、人口和车辆回流策略。
JSON 制品与 production loader 已删除。

**关联 ADR**:

- [`../adr/0013-engine-neutral-spatial-geometry-and-length-authority.md`](../adr/0013-engine-neutral-spatial-geometry-and-length-authority.md)
- [`../adr/0015-bounded-f32-canonical-spatial-frames.md`](../adr/0015-bounded-f32-canonical-spatial-frames.md)
- [`../adr/0016-scenario-population-and-recycle-lifecycle-authority.md`](../adr/0016-scenario-population-and-recycle-lifecycle-authority.md)

**关联设计**:

- [`signalized-corridor-population.md`](signalized-corridor-population.md)：走廊
  `50..=200` caller-owned reference policy。
- [`lust-bevy-population-control.md`](lust-bevy-population-control.md)：LuST/Bevy
  示例层一至一万调节契约（#256）；**不**扩展本走廊人口上限，也不把 LuST 滑杆语义
  写回本文件。
- [`real-road-workloads.md`](real-road-workloads.md)：LuST static / TOPO / DEMAND
  权威。

## 1. 目标与交付边界

v0.8 首先交付一个可持续运行、可复现的直行 native reference 场景。v0.9 在保持道路
envelope、限速、50–200 辆车人口、出口回流和 Runtime/Spatial/Adapter 分层不变的
前提下，clean-break 切换为受保护左转、直行和右转 profile。

current 走廊几何与人口策略仍按下列边界描述。Traffic / Spatial / Manifest JSON
与 production JSON loader 已随 #301 删除；仓库保留 catalog 0.2 与 LFCA。可运行世界从
共享静态路网安装。现行走廊 Bevy 最小路径见
[#472](https://github.com/illusion-tech/laneflow/issues/472)；50–200 人口与回流见
[#475](https://github.com/illusion-tech/laneflow/issues/475)。

走廊能力包含：

- 物理道路轴线总长不超过 2 km，默认 1.4 km；
- 66 条 LaneEdge、24 个 Movement、32 条 lane-level ManeuverPath/Gate、28 条
  Route 和 44 个编译后 Maneuver occurrence；
- 主干道 60 km/h、次干道 40 km/h 的 per-edge 限速；
- 两套可配置固定时制信号控制器，每个 Junction 四组、12 phase/84 秒；
- `50..=200` 可调车辆人口、显式 seed 和确定性出口回流（#475）；
- 同一 Bevy proxy/model 复用，但每次回流获得新的 Runtime 车辆句柄（#475）；
- scenario-local catalog 0.2；
- 确定性 generator 写出 catalog 与 LFCA。

current 场景不包含换道、路径搜索、permissive turn、红灯右转、感应或自适应信号、
运行时热修改信号、行人、停车、匝道、路网编辑器和 runtime snapshot。

Traffic v0.7 per-edge 限速基础由 #185 交付；#229 已把同一场景制品 clean-break
迁移到 Traffic v0.8 并显式增加 2 Junction、8 Movement 与 20
ManeuverPath/ManeuverGate。TrafficWorld 原子替换与走廊回流由 #475 交付；历史
`CoreWorld` 命令见已关闭的 #186/#203。#262 随后把 canonical Traffic
artifact 原子迁移到 v0.9，并加入 ParticipantClass、CrossSection 与 AccessRule
静态模型；protected-turning topology、Signals、人口和回流行为保持不变。

v0.9 的 Accepted 受保护转向 SSOT 见
[`signalized-corridor-protected-turning.md`](signalized-corridor-protected-turning.md)。
它以 catalog 0.2、32 条 ManeuverPath 和 28 条 Route 替换 v0.8 的直行 profile；
#190 交付该具体 profile 与最小 native 集成，#191 扩大 cross-layer 验证，#192
只执行独立收口。

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

## 3. Portal、Movement 与 Route

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

### 3.2 Current lane-level movement

每个 Junction 显式拥有四个 approach 的 left/straight/right Movement，共 12 个；
两个 Junction 共 24 个 Movement。lane assignment、32 条 ManeuverPath、28 条
finite Route、44 个 ordered Maneuver occurrence 和 route-local raw weight 的完整表
由
[`signalized-corridor-protected-turning.md`](signalized-corridor-protected-turning.md)
§3、§4 和 §6 唯一拥有，本文件不复制第二份 identity catalog。

每条 ManeuverPath 固定为 `[entry road edge, exactly one internal edge, exit road
edge]`，ManeuverGate 位于 transition 0。不同 Route 可以共享 road edge 和
PortalLane entry SpawnSlot；Core 在 Route 注册期编译 Maneuver/Gate occurrence，
steady tick 不做 runtime pathfinding、external-ID lookup 或 geometry matching。

## 4. 限速与车辆纵向行为

Traffic v0.7 引入且 current Traffic v0.10 继承的 contract 在每个 lane edge 上要求
严格正、有限的 `speedLimit`：

| edge class                         |  公示值 |
| ---------------------------------- | ------: |
| main road / straight internal      | 60 km/h |
| secondary road / straight internal | 40 km/h |
| left internal                      | 25 km/h |
| right internal                     | 15 km/h |

`VehicleProfile.desiredSpeed` 继续表达车辆自由流期望速度，不替代道路限速。纵向控制每 tick 至少合并当前 edge 的 speed ceiling、下游更低限速边界的 advance-braking spatial target、leader/no-overlap、SignalStop 和 route completion。

车辆不得以超过当前 edge 限速的初始速度 spawn/replace。车辆不得在 crossing 下游限速边界时仍超过新限速。若多个约束同时存在，沿 route 最近且最严格的可行约束生效；道路限速不得绕过既有 SignalStop 或 no-overlap hard projection。

默认初始与回流使用当前 spawn edge 的正常行驶初速度：
`min(VehicleProfile.desiredSpeed, current edge speedLimit)`。Core 仍在提交 initial
batch/replace 时验证 overlap、route 和速度上限；入口被占用时回流返回可重试的
`Blocked`，既不把速度降为零后强塞，也不绕过首个 fixed tick 的 leader、SignalStop
和 no-overlap authority。

## 5. 信号控制器

### 5.1 控制器与 signal group

每个 Junction 拥有独立 controller，以及 `main-left`、
`main-through-right`、`secondary-left`、`secondary-through-right` 四个 group。
每个 approach/lane 只有一个 StopLine；同 lane 候选 ManeuverGate 共享该 StopLine
和 phase class，但保持不同 Gate identity。authoring/generator 完整枚举 topology、
Gate 与 phase state，Core 不推断 conflict matrix。

### 5.2 固定 12-phase program

每个 controller 按 `left green -> left yellow -> all red -> through/right green ->
through/right yellow -> all red` 依次服务 main 与 secondary active set，共 12 phase。
默认时长为：

| active set              | green |
| ----------------------- | ----: |
| main left               |  10 s |
| main through/right      |  30 s |
| secondary left          |   8 s |
| secondary through/right |  20 s |

yellow 固定 `3 s`，每个 active set 后 all-red 固定 `1 s`，完整 cycle 为 `84 s`；
两个 controller offset 为 `[0, 42000] ms`。配置只在生成/启动时读取，不支持运行中
热修改；完整 phase/aspect 表与安全矩阵由 protected-turning SSOT §7 拥有。

## 6. 人口、初始分布与启动参数

### 6.1 Native runtime 参数

现行走廊 Bevy 最小路径不恢复 50–200 人口或 `--vehicles` CLI。它 `include_bytes` 检入的
catalog 0.2 与 LFCA，prepare 绑到已安装共享路网修订，再 spawn 少数车辆。运行命令为：

```powershell
cargo +1.96.0 run --locked -p laneflow-bevy --example signalized_corridor --features native-example
```

#475 交付 headless `TrafficWorld` 上的 `50..=200` caller-owned 回流、原子替换和
Adapter 同一 Entity 换绑；不把 `--vehicles` / HUD / 灯具 / orbit camera 作为完成条件。
非法车辆数、未知 portal/route 或无足够 spawn slots 必须在第一个 `TrafficWorld`
step 前失败，不能静默 clamp。`signalized_corridor` 继续复用 opt-in `native-example`。
车辆 pose 仍以车辆前保险杠为原点。

### 6.2 Stable spawn-slot catalog

generator 从相同 lane centerline 和车辆安全间距规则生成稳定的 physical
spawn-slot catalog。slot：

- 只位于真实 portal approach 的普通 road edge；
- 不位于 connector、conflict area、停止线 hard projection 范围或 route completion 边界；
- 带稳定 portal、PortalLane、edge 与 progress identity，不拥有单一 Route；
- 以文档化稳定顺序进入 catalog；
- 通过 Core production spawn validation 最终确认 overlap 和 route invariant。

默认几何和 profile 必须提供至少 200 个合法 slot，否则 generator/config validation
失败。catalog 的规范顺序依次使用 Portal 表顺序、lane index、route edge
occurrence、edge-local progress 和 slot ID；不得依赖 hash map、文件系统或 ECS
iteration order。初始化使用显式 seed 对完整 physical catalog 执行从末尾到开头的
Fisher–Yates，取前 N 个 slot 后，再按 logical slot 顺序对其 PortalLane 执行一次
weighted RouteChoice draw。

checked-in 默认配置使用 `10 m` slot pitch，并在每个 eligible segment 两端保留
车辆安全 clearance；由此确定性生成 212 个 physical slot。

Runtime 没有 external ID 字符串。每个 logical population slot 跟踪当前 `VehicleHandle`；每次旅程拥有新的 handle generation。初始 spawn 和后续 replace 使用 `VehicleSpawnInput`。

## 7. 出口回流

### 7.1 Portal/Lane/Route 三个 draw site

车辆完成 route 后不从场景消失。caller-owned reference policy 为该 logical slot 建立 pending plan；TrafficWorld 本身不自动回流：

1. 从除刚驶出 portal 外的其余 5 个 portal 中均匀选择目标 portal；
2. 从该 portal 的 2 或 3 条 PortalLane 中均匀选择一条；
3. 对该 lane 的完整正整数 raw weights 执行 cumulative weighted RouteChoice；
4. 使用该 lane 的共享 entry SpawnSlot 和目标 Route 构造 replacement；
5. 在下一 fixed-step lifecycle boundary 尝试原子 replace。

每次成功冻结 plan 都固定调用 portal、lane、route 三个 logical bounded draw site；
单一 RouteChoice 也不得跳过 raw-weight draw。它不是对全部 28 条 Route 直接均匀；
因此 portal/lane 公平性与 lane-local route preference 保持分层。

### 7.2 Blocked retry

入口 overlap 或其他可恢复容量条件阻止 replace 时：

- old vehicle 保持 Completed 且 handle 仍 live；
- proxy 保留最后一次合法 Transform；
- portal/lane/route plan 原样保留到下一 fixed boundary；
- retry 不消耗 PRNG，也不重新抽签；
- 同一 boundary 内该 plan 只尝试一次；
- 其他 pending plan 继续按稳定 insertion order 尝试。

成功后 TrafficWorld 原子使 old handle stale 并返回 new handle，Adapter 在同一公开事务中把同一 Entity 从 old handle 切换到 new handle。人口的 logical slot 数保持目标值，Bevy 不 despawn/respawn proxy 或 model。不得用退役后再 `spawn` 充当回流。

不可恢复的配置或 invariant 错误进入明确 fatal/diagnostic 路径，不能无限伪装成入口阻塞。

### 7.3 Fixed-step 顺序

每个 TrafficWorld fixed step 的 caller-owned 顺序固定为：

```text
apply pending lifecycle commands
  -> TrafficWorld fixed step
  -> consume ordered Completed vehicles
  -> enqueue pending plans for the next lifecycle boundary
```

若一个 outer frame 运行多个 catch-up step，每个 step 间仍执行上述顺序。Presentation 每个 outer frame 最多提交一次，因此 frame chunking 不改变 Runtime/population 决策序列。

## 8. PRNG 契约

v0.9 沿用项目自有 `SplitMix64`，state 由 `u64 seed` 直接初始化，零 seed 合法。实现使用 wrapping `u64` 运算和下列固定常量：

```text
increment = 0x9E3779B97F4A7C15
mul1      = 0xBF58476D1CE4E5B9
mul2      = 0x94D049BB133111EB
```

`next_u64` 的混合顺序为：state 加 increment；`z ^= z >> 30` 后乘 `mul1`；`z ^= z >> 27` 后乘 `mul2`；返回 `z ^ (z >> 31)`。

有界抽样 `uniform(bound)` 要求 `bound > 0`，且 `bound` 与 draw `r` 都是 `u64`。使用 rejection sampling：以 unsigned wrapping 语义计算 `threshold = bound.wrapping_neg() % bound`（等价于 `2^64 mod bound`），拒绝 `r < threshold`，接受后返回 `r % bound`。不得用一次直接 modulo 代替。

initial physical-slot permutation、每个 initial slot 的 weighted Route draw，以及
completion 的 portal/lane/route draw 共享一个显式 state 和冻结的调用顺序。回流
portal candidate 按本文件 Portal 表顺序移除刚驶出的 portal 后构造；PortalLane 按
lane index，RouteChoice 按 Traffic Route 输入顺序规范化。blocked retry 不 draw。
实现必须用 golden tests 固定至少：

- seed `0` 和非零 seed 的前若干 `next_u64`；
- bound `2`、`3`、`5` 的抽样序列；
- 50/100/200 初始 slot 选择；
- 多车同 tick completion 的 portal/lane/route 决策顺序；
- blocked 若干 boundary 后恢复时与未阻塞车辆的 draw state。

确定性承诺仍限定同一 LaneFlow 实现版本和运行环境；更改算法、catalog 0.2 规范顺序、
raw weights 或 draw order 必须经过新的版本/迁移决策，不能静默改变 replay。

## 9. 制品与配置边界

### 9.1 Production 制品

current v0.10 场景的可运行制品是：

- `v0.2-signalized-corridor.lfca`：编译器从走廊合成模块发射的 LFCA（含 Spatial）；
- `v0.2-signalized-corridor.catalog.toml`：scenario-local catalog 0.2。

历史 Traffic package / SpatialPackage / ScenarioManifest JSON 已随 #301 删除，不再是
现行制品。seed、车辆数、runtime handle、Entity 或 engine asset metadata 不写入 LFCA
或 catalog。

### 9.2 Authoring config 与 scenario-local catalog

`examples/config/v0.10-signalized-corridor.toml` 是仓库内部 authoring SSOT，使用
exact config `0.2`，包含道路轴线长度、交叉口位置、lane width、10 m spawn-slot
pitch、main/secondary/left/right 限速、四组 active-set timing、两个 offset 和
artifacts 输出文件名。它不包含车辆数、seed、回流策略或展示资源。

generator 写出 `v0.2-signalized-corridor.catalog.toml` 与
`v0.2-signalized-corridor.lfca`。catalog 记录 ordered PortalLane、weighted
RouteChoice、共享 entry SpawnSlot、Route→exit portal 和全部 physical slot
cross-reference。authoring config 与 catalog 都是内部 TOML。可运行世界只安装 LFCA
构建的共享路网修订。50–200 确定性回流见
[#475](https://github.com/illusion-tech/laneflow/issues/475)。

同一配置和 generator 版本必须 byte-deterministically 生成相同 artifacts、size、digest 和 catalog。仓库根目录使用下列命令生成或只读检查：

```powershell
cargo +1.96.0 run --locked -p laneflow-corridor-generator -- generate --config examples/config/v0.10-signalized-corridor.toml
cargo +1.96.0 run --locked -p laneflow-corridor-generator -- check --config examples/config/v0.10-signalized-corridor.toml
```

`check` 不写文件。catalog 与 LFCA 字节对拍由 `laneflow-corridor-generator` 测试覆盖，随
`cargo test --workspace` 进入 `Rust checks`，不再单独跑 generator `check`。

## 10. 分层权威与实施切片

| 关注点                                                                   | 权威层                               | 实施 Issue     |
| ------------------------------------------------------------------------ | ------------------------------------ | -------------- |
| per-edge speed limit、v0.8 引入且 current v0.10 继承的 topology/纵向约束 | Data/Core                            | #185/#229/#262 |
| caller-driven atomic replace、overlap 与 identity invariant              | TrafficWorld                         | #475           |
| 目标人口、seed、portal/lane 决策与 blocked retry                         | `laneflow-scenario` reference policy | #475           |
| typed lifecycle transaction 与 proxy binding                             | Bevy Reference Adapter               | #475           |
| 场景 generator、固定时制配置与三类静态制品                               | Data/Authoring                       | #188           |
| native UI/CLI、道路/车辆/灯具呈现与场景集成                              | Bevy Reference Adapter               | #189           |
| 独立审阅、性能/可视/回归证据                                             | Cross-layer closure                  | #195           |
| protected profile artifacts、catalog 0.2、scenario/native 集成           | Cross-layer                          | #190           |
| expanded clearance/replay/proxy/performance 证据                         | Cross-layer validation               | #191           |
| v0.9 独立 closure review，不新增 runtime 行为                            | Cross-layer closure                  | #192           |

TrafficWorld 是 vehicle identity、状态、overlap、Route、SignalStop 和 speed-limit
behavior 的权威，但不限制车辆数量，也不拥有回流 policy。`laneflow-scenario` 是目标人口、seed、
catalog 0.2 normalization 和 portal/lane/weighted-route 决策的 reference authority；
未来城市游戏可以完全替换它。Traffic/Spatial 是静态拓扑和几何的权威；Adapter 是
VehicleHandle/Entity 部分双射与宿主 schedule 的权威；Presentation 只拥有
proxy/model/Transform/灯具和 route-class 视觉。

## 11. 验收矩阵

current v0.10 至少验证：

| 类别     | 必须证明的事实                                                                      |
| -------- | ----------------------------------------------------------------------------------- |
| 几何     | 1.4 km 默认、<=2 km、66 edges、32 paths、28 routes、Traffic/Spatial 长度绑定        |
| 限速     | 主 60/次 40、超限 spawn 拒绝、下游降速提前制动、与 leader/signal 组合               |
| 信号     | 两 controller、每路口四 group/12 phase、共享 StopLine、冲突 movement 不同时开放     |
| 人口     | 50/100/200 成功初始化、无 overlap、非法范围和容量不足明确失败                       |
| 回流     | 排除原出口、portal/lane/weighted-route 三 draw site、blocked 不重抽、人口保持       |
| 生命周期 | old handle stale/new live、same Entity/proxy、Runtime+mapping 失败原子              |
| 确定性   | 同 seed/fixed input 相同、不同 outer-frame chunking 相同、golden PRNG               |
| 制品     | generator byte deterministic、检入 catalog 与 LFCA 对拍、prepare bind 到共享路网修订 |
| 可视     | 左/直/右可识别、灯具状态一致、Adapter pose 和 same-Entity recycle 正常              |
| 性能     | 200 车持续运行无 unbounded queue/retained growth；稳态 lifecycle 不做全人口临时分配 |

## 12. 治理与完成边界

- 设计来源：#184，G1 冻结记录见 <https://github.com/illusion-tech/laneflow/issues/184#issuecomment-5041612599>；
- #185、#186–#189 与 #203 分别按自身 Gate Ledger 推进；各 Issue 的当前状态以 GitHub 为准，任一设计 Issue 或上游 G1 都不自动授权下游开工；
- #184 的 Delivery PR 只交付设计与 ADR，不授权下游自动开工；
- v0.8 Milestone 由 #193 跟踪，只有 #195 独立收口通过并满足父目标退出条件后才可完成；
- v0.9 #194 的受保护左转、直行、右转车道/相位能力由 #196 冻结；#229 已交付通用
  static substrate，#190 交付具体 profile 与最小 native 集成，#191/#192 分别负责
  expanded validation 与独立 closure。
