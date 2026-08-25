# 交通运行时共享静态路网消费

**文档状态**: Accepted（#301 G1；#469 合入后收口）<br>
**最后更新**: 2026-08-25<br>
**适用范围**: `laneflow-runtime` / `TrafficWorld`、`laneflow-spatial` 目标 session、
1-worker 车辆 tick、#301 端到端证据，以及 current `laneflow-core` / JSON 运行时入口拆除<br>
**关联文档**: `../adr/0020-compiler-owned-static-network-and-static-image.md`、
`../adr/0021-city-simulation-game-traffic-foundation.md`、
`../adr/0025-checked-canonical-network-and-shared-static-network.md`、
`../adr/0026-merge-governance-rebuild.md`、
`network-compiler.md`、`shared-static-network.md`、
`portable-canonical-artifact.md`、`current-package-import.md`、
`adapter-api.md`、
`../adr/0003-runtime-tick-and-determinism.md`、
`../adr/0028-integer-millimeter-traffic-geometry.md`、
`../adr/0017-static-road-junction-maneuver-and-gate-identity.md`、
`../adr/0018-multimodal-cross-section-and-access-overlay.md`

本文是 #301 的实现级 G1 输入。已提交一维几何以
`traffic-runtime-integer-geometry.md` / ADR 0028 为现行合同。它不授权 #302
在线修订切换、#441 系统化性能账本、#303 Routing 或 #294 残留文档/Skill 改名。

## 1. 结论

#301 交付目标交通运行时对已构建 `SharedNetworkRevision` 的消费路径，并使它成为
**唯一可运行的交通世界**。仓库没有在跑的外部消费者、未发布 1.0，因此不保留
「Core 继续生产、Runtime 作旁路」的双轨，也不用 `CoreWorld` 作预言机。

冻结句：

1. 新建 `laneflow-runtime`。`TrafficWorld` 安装完整 `Arc<SharedNetworkRevision>`，
   只分配每世界可变状态与 1-worker 执行计划。热路径借共享根静态连续 accessor，
   **并**读本世界已提交/已编译表（车辆、占用、动态 Route occurrence）；不把动态
   occurrence 写回共享根，也不把 `SharedIdentityIndex` 推进 steady tick。
2. `laneflow-spatial` 依赖 `laneflow-static-network` / `laneflow-static-contract`，
   **不**依赖 Runtime。`SpatialSession::bind` 只接受根 `Arc`。world 与 session 配对
   必须 `Arc::ptr_eq`（或两者来自同一保留的根 `Arc`）。pose 批次使用与 Runtime
   无关的不透明记录身份，并携带该 `Arc` 的 `NetworkRevisionId`。
3. Runtime **禁止**依赖 Spatial、compiler、Serde、文件系统、`laneflow-core`。
4. 正确性证据是 compiler 拥有的 `lfca-full-spatial` 加上 Runtime 2 车 1-worker
   集成测试，以及同一编制上的最小 Bevy 示例。禁止同一场景对拍 `CoreWorld`。
5. #301 的完成 PR 合入 `main` 时，`laneflow-core`、current JSON 运行时入口和
   LIR→Core 投影一并消失；Bevy / 示例不得再构造 `CoreWorld`。
6. 不抽第三求解 crate。`RuntimeExecutionPlan` 本切片只表达 1-worker 身份。
7. 第一刀仍是车辆 / 动态 Route / 停车占用特化，不得写成终态唯一参与者模型。
8. 凡 S1、最小 Bevy、§6.4 要调用的公开入口只列在 §4。G2 不得另发明生产路径
   API 来凑这些证据。

## 2. 明确不做

- 不实现 #302 的修订切换、Runtime Snapshot、动态状态迁移或 committed 道路状态晋升。
- 不交付 #441 的 retained/scratch/多 world 字节账与 wall-clock 证据；#301 只做
  「克隆根 `Arc`、不复制静态 component」的功能断言。
- 不实现多 worker / 分区算法，不把最终 partition 写入共享静态路网。
- 不把城市经济、出行需求或路线选择策略放进 Runtime。
- 不做完整 Adapter 生产接线或 corridor 规模人口；最小 Bevy 只证明同一根上的
  tick + pose 能驱动代理位移。`signalized_corridor` 现行最小路径由
  [#472](https://github.com/illusion-tech/laneflow/issues/472) 交付；50–200 原子替换见
  [#475](https://github.com/illusion-tech/laneflow/issues/475)。
- 不把 `laneflow-core-design` Skill 标识符改名（若仍需独立残留 Issue，不得反向
  保留 `laneflow-core` crate）。
- 不恢复独立 `despawn`；原子替换由 #475 交付。不冻停车预约/到场/离场状态机，也不把
  `CoreEvent` 枚举搬进 Runtime。
- 不实现时变准入；`register_route` 不做 `(ParticipantClass, Route)` 判断。

## 3. 包依赖

依赖箭头表示左侧依赖右侧：

```text
laneflow-compiler ──────────────> laneflow-format
laneflow-static-network ────────> laneflow-format
laneflow-static-network ────────> laneflow-static-contract
laneflow-runtime ───────────────> laneflow-static-network
laneflow-runtime ───────────────> laneflow-static-contract
laneflow-spatial ───────────────> laneflow-static-network
laneflow-spatial ───────────────> laneflow-static-contract
Adapter / 示例 ─────────────────> laneflow-runtime
Adapter / 示例 ─────────────────> laneflow-spatial
（Adapter 生产 graph 不直接依赖 laneflow-static-contract；停车序号由 Runtime 再导出。）
```

| 包                 | 拥有                                                                                    | 禁止                                                           |
| ------------------ | --------------------------------------------------------------------------------------- | -------------------------------------------------------------- |
| `laneflow-runtime` | 固定步进、已实现执行域的每世界可变状态、动态 Route occurrence 编译、1-worker 执行计划   | Spatial、compiler、Serde、文件系统、`laneflow-core`、LFCA 解析 |
| `laneflow-spatial` | 规范位姿采样、session scratch/output；pose 批次只使用不透明 `PoseRecordId` 与共享根序号 | Traffic tick 权威、compiler、引擎、Runtime、车辆 handle        |

`network-compiler.md` 历史 crate 图中的 Spatial → Runtime 作废。几何属于修订根，
session 是 revision-scoped，不是 world-scoped。N 个 `TrafficWorld` 共用一份
`SharedSpatialNetwork`。

pose 记录身份（不承诺最终 Rust 拼写）：

- `PoseRecordId`：调用方分配的不透明 `u32`。Spatial 不解释为车辆、也不导入
  Runtime/Core handle。
- `PoseSource::Lane`：`LaneEdgeOrdinal` + 与共享根边长同域的进度 `progress_mm: u32`。
- `PoseSource::Parking`：共享根上的停车位序号。
- 一批必须同一 canonical frame；混 frame 整批失败。
- 批次头保存：`bind` 所用 `Arc` 的 `NetworkRevisionId`、该批 `CanonicalFrameId`
  （或共享根 `CanonicalFrameOrdinal`）与调用方 `FramePlacementToken`。Spatial
  原样回显 token；Adapter 提交宿主变换前复核。不把 frame 身份逐条写入 record。

Adapter / Runtime 在组合根把车辆 handle 映射到 `PoseRecordId`。禁止 Spatial 依赖
`VehicleHandle`，禁止为此再抽第三 crate。

「不能独立安装 component」由根类型保证：`SharedSpatialNetwork` 不 `Clone`、无公开
构造器。Spatial crate 再依赖 Runtime 并不能加强这点，只会把采样绑到 tick 对象上。

## 4. 公开入口

凡 S1、最小 Bevy、§6.4 要驱动或观察的行为，公开入口只在本节。生命周期命令
（`register_route` / `remove_route` / `spawn_vehicle` / `occupy_parking` /
`replace_completed_vehicle`）只在 `step` 之间调用；单条失败原子，不得在一次
`step` 中间改实体集合。`Completed` 车辆保留到 replace：不进 pose、不占车道，占容量。

语义形状（不承诺最终 Rust 字段名）：

```rust
TrafficWorld::install(
    revision: Arc<SharedNetworkRevision>,
    config: WorldConfig,
) -> Result<TrafficWorld, InstallError>;

TrafficWorld::static_route(route: /* 共享根静态路线序号 */) -> Result<RouteHandle, LookupError>;
TrafficWorld::register_route(input: RouteRegisterInput) -> Result<RouteHandle, RouteError>;
TrafficWorld::remove_route(route: RouteHandle) -> Result<(), RouteError>;
TrafficWorld::spawn_vehicle(input: VehicleSpawnInput) -> Result<VehicleHandle, SpawnError>;
TrafficWorld::replace_completed_vehicle(
    old: VehicleHandle,
    input: VehicleSpawnInput,
) -> Result<VehicleReplaceRecord, ReplaceError>;
TrafficWorld::vehicle(handle) -> Option<VehicleState>;
TrafficWorld::live_vehicles() -> &[VehicleHandle];
TrafficWorld::occupy_parking(
    vehicle: VehicleHandle,
    space: /* 共享根停车位序号 */,
) -> Result<(), ParkingError>;

TrafficWorld::step(input: TickInput) -> Result<StepOutcome, StepError>;

TrafficWorld::revision() -> Arc<SharedNetworkRevision>;
TrafficWorld::traffic() -> /* 共享根 Traffic 借用 */;
TrafficWorld::tick_index() -> u64;
TrafficWorld::time_ms() -> u64;
TrafficWorld::committed_pose_sources() -> CommittedPoseSourceBatch;
TrafficWorld::committed_parking_occupant(space) -> Option<VehicleHandle>;
TrafficWorld::committed_signal_groups() -> CommittedSignalGroupBatch;

SpatialSession::bind(
    revision: Arc<SharedNetworkRevision>,
) -> Result<Option<SpatialSession>, SpatialBindError>;
SpatialSession::extract_pose_batch(/* PoseRecordId + PoseSource */)
    -> Result<CanonicalPoseBatch, PoseError>;
```

### 4.1 安装与绑定

- `WorldConfig` 含每世界容量、1-worker 计划，以及 `fixed_delta_time_ms`（同一
  world 运行中不得改变）。步长 `∈ [4, 1000]`，每个 phase
  `durationMs % dt == 0 && durationMs >= dt`，否则 `install` 失败关闭、不留下
  world。短相位不得靠 tick 跳过。不接受 LFCA 字节、调用方自报 digest /
  `NetworkRevisionId`、或裸 component。
- 失败原子：失败不留下可观察的半个 world / session。
- 多世界：再次 `install`，只克隆根 `Arc`。
- `spatial()` 为 `None`：`bind` 返回 `Ok(None)`，不建 session（headless）。
- 有 Spatial 但无 `lane_pose()`：不得走车辆 pose 采样。
- `bind` 的长期所有权只认根 `Arc`，不把 `&SharedSpatialNetwork` 提升为可独立持有
  的 session。短期函数借用可以。
- Runtime 可以只读转发 `revision()` / `traffic()`；**不**持有 `SpatialSession`，
  **不** `use` Spatial 类型。
- 同时持有 world 与 session 的调用方必须 `Arc::ptr_eq`，或证明两者来自同一保留的
  根 `Arc`。禁止只比较 `NetworkRevisionId`。
- `extract_pose_batch` 使用 §3 的 `PoseRecordId` / frame 合同；混 frame 整批失败。

### 4.2 路线与车辆

动态 Route 仍按 ADR 0017：compiler 预编译静态初始路线；Runtime 新注册的动态
Route 用共享根边序号编译 occurrence。

- 静态路线：`install` 后 `static_route(共享根静态路线序号)` 取得 `RouteHandle`，
  不必再 `register_route`。
- `register_route`：输入为共享根 `LaneEdgeOrdinal` 有序非空序列（不要 JSON
  字符串 ID）；按共享根连通校验（车道后继或机动转移候选），把 occurrence 编进
  **本世界**表，返回代际感知 `RouteHandle`。首边或末边不得落在路口内部边上，末边
  不得带 StopLine；occurrence 覆盖与静态路线重建同一规则，且 hop 半开区间
  `[entry, exit)` 不得相交。非法序列失败，不留下半条路线。不做准入判断
  （ADR 0018：Route 无 class 上下文）。
- `remove_route`：只移除本世界 **动态** 路线。静态路线句柄必须拒绝。仍有 live
  车辆引用则失败；成功后旧动态句柄 stale，本世界动态 occurrence 表去掉该路线。
- 人口是调用方所有：`install` 不接受初始车辆。`VehicleSpawnInput` 含共享根车辆
  profile 序号、已有 `RouteHandle`、**路线序列下标**（ADR 0017 `routeEdgeIndex`：该
  `RouteHandle` 边序列上的 occurrence 位置，不是共享根 `LaneEdgeOrdinal`；`[A, B, A]`
  的两个 A 靠下标区分）、该 occurrence 对应边上与共享根边长同域的进度、初速。
  进度与初速为 `progress_mm` / `speed_mm_s`，新车 `carry_um = 0`，进度
  `0..=length_mm`，初速 `<=` 当前边限速且 `<= 100_000` mm/s；禁止未文档化的
  米→毫米量化。下标必须落在 `0..len`，越界失败、不留车。tick 内部车辆状态继续
  带着这个序列下标前进。`committed_pose_sources` 的 `PoseSource::Lane` 仍用该
  occurrence 解出的 `LaneEdgeOrdinal` + `progress_mm`。
- `spawn_vehicle` 返回代际感知 `VehicleHandle`（不是 `PoseRecordId`）。由 profile
  解析 `ParticipantClass`，对静态和动态 `RouteHandle` 都按 ADR 0018 做
  `(class, Route)` 绑定期准入（只查当前 cursor / 序列下标起的可达后缀）。初速可以
  等于该 occurrence 当前边的基础限速，超过则拒绝。重叠、非法路线/下标/进度、未知
  profile、超容量、准入 deny、超限速失败时不得留下半辆车。
- #475 交付 `replace_completed_vehicle`；不恢复独立 `despawn`。到达终点写成 `Completed`，保留句柄、容量与动态路线引用，不进 pose、不占车道；replace 成功时再迁移路线引用。

### 4.3 停车占用

只冻占用互斥，不冻预约/到场/离场。

- `occupy_parking`：每车至多一个车位、每车位至多一车。车辆必须当前未占用其他
  停车位；已占用 **同一** 车位则幂等成功。成功后该车 parked，pose 源为该停车位，
  原车道占用释放。车或停车位非法、目标已被其他车占用、该车已占用别的车位则失败，
  不留下半占用。
- `committed_parking_occupant`：按停车位序号读占用者。

### 4.4 步进与已提交查询

```text
TrafficWorld::step(TickInput) -> Result<StepOutcome, StepError>
```

- `TickInput.delta_time_ms` 必须等于 `WorldConfig.fixed_delta_time_ms`，否则拒绝，
  不推进。`tick_index` 与 `time_ms` 用 checked 加法；任一溢出则 `StepError`，世界
  不变。成功 `StepOutcome` 含 `tick_index` 与 `time_ms`；`tick_index()` /
  `time_ms()` 与之一致。不得回绕后当作合法时间去推信号 snapshot。
- 时序沿用 `signal-system.md`：运动、跟车、信号遵守、占用判断都读已提交状态
  **T**；算完再原子提交 **T + D**（时间、pose、占用、信号 snapshot）。同 tick 内
  不得把未提交位移当成其他车的权威前车。相位边界落在 `[T, T + D)` 时，该拍车辆
  仍用 snapshot(T) 的灯色，不得提前用 T + D。
- 失败不推进时间，不留下半更新；成功 tick 不因错误边界新分配诊断。
- 不把 `CoreEvent` 搬进本切片。跟车/信号遵守用已提交 pose 进度与信号组 aspect
  观察。
- `committed_pose_sources()`：稳定顺序的 `(VehicleHandle, PoseSource)`。车道用
  `LaneEdgeOrdinal` + 同域进度（**当前** `f64` 米；**G2** `progress_mm: u32`），
  停车用停车位序号。已完成或已移除车辆不出现。Adapter 映射为 `PoseRecordId`
  再交给 Spatial；Spatial 在边界把 mm 换成弧长比例，不得把米制当权威。
- `committed_signal_groups()`：稳定按组序号的当前 aspect，由已提交 `time_ms` +
  共享根 program + offset 导出。`install` 成功后 time 0 已有有效 snapshot；成功
  `step` 之后查询到的是 T + D。初始化不发事件。
- 上述查询只读已提交状态；`step` 失败时与失败前一致。不冻完整 snapshot/event
  套件，也不把 Spatial 绑到这些查询类型。

## 5. Tick

公开推进入口是 `TrafficWorld::step`（§4.4）。`TrafficWorld` 的 1-worker 车辆 tick
读取：

- 共享根 `SharedTrafficNetwork` 的连续 slice（后继 CSR、准入 resolved 表、静态
  路线 occurrence、信号 program、停车静态关系）；
- **本世界**已提交/已编译表（车辆列、车道与停车占用、动态 Route occurrence）。

不得把动态 occurrence 写回共享根。`SharedIdentityIndex` 不进入 steady tick；只用于
install 核对、`register_route` 重建，以及后继 #302 快照/修订切换。禁止：

- 先投影成 `LaneGraph` / 各 registry 再调用任何 `CoreWorld` 步进；
- Runtime 依赖 `laneflow-core`；
- 抽出 `laneflow-motion` 或同职责第三包。

第一刀覆盖当前车辆特化：跟车、信号遵守、停车占用。不把这套写成终态全部交通参与
单元的公共基类。人口、Routing 和游戏规则的 seed/随机流仍属 caller，不进入
`TrafficWorld` 隐藏状态。

`RuntimeExecutionPlan` 本切片固定为单 worker；worker 数不得改变已提交状态。分区
算法、边界缓冲和置换等价测试不在 #301。

## 6. 证据：零预言机与 S1 e2e

### 6.1 禁止的证明

- 同一编制分别喂给 `CoreWorld` 与 `TrafficWorld` 再比较 event / pose / 轨迹。
- 以 campus JSON、`InitialTrafficData` 或 LIR→Core 投影作为 Runtime 正确性预言机。
- 以走廊 50–200 辆人口或 LuST 作为本切片完成条件。

### 6.2 必做的证明

地图：compiler 已冻夹具 `lfca-full-spatial`
（`crates/laneflow-compiler/tests/fixtures/portable/lfca-full-spatial/`）。
它含 22 类实体 Identity、信号、停车、lane-pose 几何和一条
`entry → middle → exit` 静态路线。

Runtime 在该根上 `static_route` 取得夹具静态路线，再 `spawn_vehicle` **两辆**同一路线前后排列的车，经 `step` 做 1-worker 固定步数。CI 集成
测试（无窗口）必须断言：

- `install` 成功且只保留一根 `Arc`；
- 两车都能推进；后车受前车约束可观察（跟车或至少不能穿透前车占用）；
- 因夹具有 `lane_pose`，同一 `Arc` 上 `bind` 后用 `committed_pose_sources()`
  构造 pose 输入，`extract_pose_batch` 能产出 pose 批次；
- 安装或步进失败不留下半个 world；
- 测试 crate 不链接 `laneflow-core`。

不要求：完整停车离场状态机、红灯停止线的独立故事夹具、多 worker、走廊规模人口、
与任何历史 Core 轨迹相等。S1 是共享根集成证据，**不能**单独替代下列 Runtime
原生覆盖。

### 6.3 最小 Bevy 示例

同一编制产物驱动一个最小 Bevy example：fixed tick + 代理位移。GUI 不进 CI。
CI 必须同时：

- 对该 example 做 `check`（可与现有 `native-example` feature 对齐）；
- 跑无窗口 Bevy `App` smoke：schedule 调用 `TrafficWorld::step`，用
  `committed_pose_sources()` 构造 pose 输入，`extract_pose_batch` 后断言至少一个
  代理 `Transform` 发生可观察变化。

`cargo check` 不能单独作为完成证据。这是「新的端到端示例」，不是 corridor 规模
演示。

`signalized_corridor` 薄路径、catalog 绑定与走廊 LFCA 由
[#472](https://github.com/illusion-tech/laneflow/issues/472) 交付。50–200 原子替换见
[#475](https://github.com/illusion-tech/laneflow/issues/475)。#301 只要求它们不再以
`CoreWorld` 为可运行入口。

### 6.4 拆除 Core 行为套件前的 Runtime 覆盖

删除 `laneflow-core` 及其测试支持 crate 之前，必须有 **Runtime 原生** 测试覆盖下列
保留的当前运行时合同（不把 Core 轨迹当预言机，不要求逐条搬迁测试文件）：

- 跟车安全间隙（`step` 读已提交 T 的前车；`committed_pose_sources` 进度）；
- 信号停车与许可通行（车辆用 snapshot(T)；成功 step 后 `committed_signal_groups`
  为 T + D。相位边界落在 `[T, T + D)` 时该拍不得提前用 T + D 灯色）；
- 停车占用权威（`occupy_parking` 双向互斥与失败原子性，含已停车辆再占其他车位
  必须失败、同一车位重复占用幂等成功；`committed_parking_occupant`）；
- 确定性固定步进（`step`：正的 `fixed_delta_time_ms`、delta 不匹配则拒绝、
  `tick_index`/`time_ms` 溢出则拒绝且世界不变、同输入序列同结果）；
- 信号 program 每个 phase `durationMs >= fixed_delta_time_ms`，否则 install 失败
  （#301 现行覆盖）。#496 G2 把该条替换为 `durationMs` 必须是步长的正整数倍；

- 安装/步进/命令失败原子性（失败不留下半个 world 或已提交半更新）；
- 成功 tick 不因错误边界新分配诊断（不要求继承 Core `TickInvariantError` 的
  `Copy` / 64 / 72 字节布局）；
- 动态 Route 注册与编译 occurrence（`register_route` / `remove_route`，ADR 0017；
  tick 读本世界动态 occurrence 表；不含走廊级人口与回流）；
- spawn 绑定期准入（静态与动态 Route 均按 ADR 0018 `(ParticipantClass, Route)`
  后缀拒绝，失败不留车）；
- spawn 初速等于当前边基础限速须成功，超过则拒绝且不留车
  （#301 现行，`f64` 米每秒）。#496 G2 改为 `speed_mm_s`，且 `<= 100_000`；
- `remove_route` 拒绝静态路线句柄。

空实现若只过 S1 两车推进/pose 不得视为完成。完整停车离场/预约、受保护转向走廊、
50–200 辆人口、vehicle replace 的全部历史变体，不进本切片；需要时另开 Issue。

这些测试只链接 `laneflow-runtime`（及共享根/format/compiler 夹具），不链接
`laneflow-core`。

## 7. 拆除（合入 `main` 的完成定义）

#301 使用 **一个完成 PR**（同一分支上的提交序列）。合入后 `main` 上：

必须消失或不再作为运行时入口：

- `crates/laneflow-core` 与 `laneflow-core-test-support`（前提：§6.4 覆盖已存在）；
- `laneflow-data` / `laneflow-current-source` 作为 Core 的 JSON 加载入口（本完成 PR 已删除 crate）；
- current JSON schema：`laneflow-data-v0.10`、`laneflow-spatial-v0.1`、
  `laneflow-scenario-manifest-v0.1`（本完成 PR 已删除文件）。不得删除 `schemas/road-editing/`。
  走廊生成器不再构造 current JSON；编制走 compiler，检入 catalog 与 LFCA；
- `laneflow-compiler-test-support` 的 LIR→Core 投影；
- `laneflow-spatial` 对 `laneflow-core` 的依赖（改为共享根 bind）；
- `laneflow-bevy`、`laneflow-scenario`、`laneflow-corridor-generator` 以及
  campus / `native_reference` / `signalized_corridor` 中任何 `CoreWorld` /
  `InitialTrafficData` 构造。campus / native 接得上 S1 就改接，接不上就删除。
  走廊示例允许从本切片删除 Core 入口；迁到 Runtime 是 follow-up，不作为本 PR
  成功标准。
- research 代码中以 Core 为 `fixture-oracle` 的可选依赖：删除或改为非 CI 遗留，
  不得再作为可运行入口。

合入 `main` 时禁止出现「Runtime 已在、Bevy/示例仍跑 Core」的双入口。分支内部
提交可以暂时并存；**最后一个进入默认分支的状态**必须满足本节。

#294 不再拥有生产切换或拆除旧运行时路径。若本 PR 已删除 `laneflow-core` crate，
#294 只可能残留文档导航 / Skill 标识符改名；不得再把 Core 当正式世界。

#305 不再要求「current 与 target 同时存在时的等价矩阵」。阶段 7 认证目标路径
本身；其 G1 另开，不在本文展开。#301 拆除 Core **不以** #441/#305 的性能、规模或
安全门禁通过为前提：仓库没有生产回退路径，权威在 #301 切换。本切片的「生产切换」
即此权威切换，定义见 `../reference/glossary.md`；不再要求先过等价、性能或安全
门禁。#301 完成后 `laneflow-runtime` 为当前态，不得再把 `laneflow-core` 写成当前
生产路径。

## 8. 路线图关系

阶段 6 的完成语义改为：共享静态路网（#300 子切片）加上 #301 使 `TrafficWorld`
成为唯一可运行交通世界并拆除 current Core/JSON 运行时入口。

阶段 8 不再是「那一天才把权威从 Core 交给 Runtime」。权威在 #301 完成时已经切换。
#302 仍独占在线修订与 snapshot。#441 仍等至少一个 production kernel，但不再要求
以 Core 为对照。

历史 ADR 中「#294 G4 前 current JSON/Core 仍是 production contract」的时间表由
ADR 0020 §10 的本 G1 修订取代。编译器拥有静态路网、Runtime 终态名、Runtime ↛
Spatial、不得用 Core 对象图当 compiler IR，保持有效。

## 9. 返回 G1 的条件

实现中出现下列任一情况必须停止并修订本文后再求新的 G1：

- 热路径无法只借共享 accessor，需要复制静态表才能步进；
- Spatial 不得不依赖 Runtime 才能正确采样，或 pose 批次不得不嵌入车辆 handle；
- 完成 PR 无法在不合入 Core 双入口的前提下拆除旧 crate，需要改变 L1/Q；
- `lfca-full-spatial` 无法支撑两车跟车的可观察断言，需要新的 S2 编制；
- 认为必须恢复 Core 预言机、产品双轨，或以 #305 通过为拆除前提。

## 10. 对 G2 的输入

G2 开工前 Issue 须为 `Ready` 或等价。本文已 Accepted。不得在 Review 稿上开工拆
Core。

实现按本文一次完成，不拆成可独立交付、合入后语义不完整的子 Issue。允许同一 PR
内分提交，提交顺序须保证审查时能看出「先有 Runtime 再拆旧入口」，且默认分支终态
满足 §7。
