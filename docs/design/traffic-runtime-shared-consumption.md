# 交通运行时共享静态路网消费

**文档状态**: Review（#301 G1 冻结草案）<br>
**最后更新**: 2026-08-22<br>
**适用范围**: `laneflow-runtime` / `TrafficWorld`、`laneflow-spatial` 目标 session、
1-worker 车辆 tick、#301 端到端证据，以及 current `laneflow-core` / JSON 运行时入口拆除<br>
**关联文档**: `../adr/0020-compiler-owned-static-network-and-static-image.md`、
`../adr/0021-city-simulation-game-traffic-foundation.md`、
`../adr/0025-checked-canonical-network-and-shared-static-network.md`、
`../adr/0026-merge-governance-rebuild.md`、
`network-compiler.md`、`shared-static-network.md`、
`portable-canonical-artifact.md`、`current-package-import.md`、
`../adr/0003-runtime-tick-and-determinism.md`

本文是 #301 的实现级 G1 输入。它不授权 #302 在线修订切换、#441 系统化性能账本、
#303 Routing 或 #294 残留文档/Skill 改名（若 #301 已删除 `laneflow-core` crate）。

## 1. 结论

#301 交付目标交通运行时对已构建 `SharedNetworkRevision` 的消费路径，并使它成为
**唯一可运行的交通世界**。仓库没有在跑的外部消费者、未发布 1.0，因此不保留
「Core 继续生产、Runtime 作旁路」的双轨，也不用 `CoreWorld` 作预言机。

冻结句：

1. 新建 `laneflow-runtime`。`TrafficWorld` 安装完整 `Arc<SharedNetworkRevision>`，
   只分配每世界可变状态与 1-worker 执行计划；热路径只借共享根的连续 accessor。
2. `laneflow-spatial` 依赖 `laneflow-static-network` / `laneflow-static-contract`，
   **不**依赖 Runtime。`SpatialSession::bind` 只接受根 `Arc`。world 与 session 配对
   必须 `Arc::ptr_eq`（或两者来自同一保留的根 `Arc`）。pose 批次使用与 Runtime
   无关的不透明记录身份，并携带该 `Arc` 的 `NetworkRevisionId`。
3. Runtime **禁止**依赖 Spatial、compiler、Serde、文件系统、`laneflow-core`。
4. 正确性证据是 compiler 拥有的 `LFCA-V1-FULL-SPATIAL` 加上 Runtime 2 车 1-worker
   集成测试，以及同一编制上的最小 Bevy 示例。禁止同一场景对拍 `CoreWorld`。
5. #301 的完成 PR 合入 `main` 时，`laneflow-core`、current JSON 运行时入口和
   LIR→Core 投影一并消失；Bevy / 示例不得再构造 `CoreWorld`。
6. 不抽第三求解 crate。`RuntimeExecutionPlan` 本切片只表达 1-worker 身份。
7. 第一刀仍是车辆 / 动态 Route / 停车占用特化，不得写成终态唯一参与者模型。

## 2. 明确不做

- 不实现 #302 的修订切换、Runtime Snapshot、动态状态迁移或 committed 道路状态晋升。
- 不交付 #441 的 retained/scratch/多 world 字节账与 wall-clock 证据；#301 只做
  「克隆根 `Arc`、不复制静态 component」的功能断言。
- 不实现多 worker / 分区算法，不把最终 partition 写入共享静态路网。
- 不把城市经济、出行需求或路线选择策略放进 Runtime。
- 不做完整 Adapter 生产接线或 corridor 规模人口；最小 Bevy 只证明同一根上的
  tick + pose 能驱动代理位移。
- 不把 `laneflow-core-design` Skill 标识符改名（若仍需独立残留 Issue，不得反向
  保留 `laneflow-core` crate）。

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
```

| 包 | 拥有 | 禁止 |
| --- | --- | --- |
| `laneflow-runtime` | 固定步进、已实现执行域的每世界可变状态、动态 Route occurrence 编译、1-worker 执行计划 | Spatial、compiler、Serde、文件系统、`laneflow-core`、LFCA 解析 |
| `laneflow-spatial` | 规范位姿采样、session scratch/output；pose 批次只使用不透明 `PoseRecordId` 与共享根序号 | Traffic tick 权威、compiler、引擎、Runtime、车辆 handle |

`network-compiler.md` 历史 crate 图中的 Spatial → Runtime 作废。几何属于修订根，
session 是 revision-scoped，不是 world-scoped。N 个 `TrafficWorld` 共用一份
`SharedSpatialNetwork`。

pose 记录身份（不承诺最终 Rust 拼写）：

- `PoseRecordId`：调用方分配的不透明 `u32`。Spatial 不解释为车辆、也不导入
  Runtime/Core handle。
- `PoseSource::Lane`：`LaneEdgeOrdinal` + 与共享根边长同域的进度。
- `PoseSource::Parking`：共享根上的停车位序号。
- 批次头保存 `bind` 所用 `Arc` 的 `NetworkRevisionId`。

Adapter / Runtime 在组合根把车辆 handle 映射到 `PoseRecordId`。禁止 Spatial 依赖
`VehicleHandle`，禁止为此再抽第三 crate。

「不能独立安装 component」由根类型保证：`SharedSpatialNetwork` 不 `Clone`、无公开
构造器。Spatial crate 再依赖 Runtime 并不能加强这点，只会把采样绑到 tick 对象上。

## 4. 安装与绑定

语义形状（不承诺最终 Rust 字段名）：

```rust
TrafficWorld::install(
    revision: Arc<SharedNetworkRevision>,
    config: WorldConfig,
) -> Result<TrafficWorld, InstallError>;

SpatialSession::bind(
    revision: Arc<SharedNetworkRevision>,
) -> Result<Option<SpatialSession>, SpatialBindError>;
```

- `WorldConfig` 含每世界容量、1-worker 计划，以及正整数 `fixed_delta_time_ms`
  （ADR 0003：同一 world 运行中不得改变；`TickInput` 若带 delta 必须相等）。
  不接受 LFCA 字节、调用方自报 digest / `NetworkRevisionId`、或裸 component。
- 失败原子：失败不留下可观察的半个 world / session。
- 多世界：再次 `install`，只克隆根 `Arc`。
- `spatial()` 为 `None`：`bind` 返回 `Ok(None)`，不建 session（headless）。
- 有 Spatial 但无 `lane_pose()`：不得走车辆 pose 采样。
- `bind` 的长期所有权只认根 `Arc`，不把 `&SharedSpatialNetwork` 提升为可独立持有
  的 session。短期函数借用可以。
- Runtime 可以提供只读转发 `TrafficWorld::revision()` /
  `traffic()`；**不**持有 `SpatialSession`，**不** `use` Spatial 类型。
- 同时持有 world 与 session 的调用方（#301 harness、最小 Bevy、以后的 Adapter）
  必须 `Arc::ptr_eq`，或证明两者来自同一保留的根 `Arc`。禁止只比较
  `NetworkRevisionId`：同一 LFCA 可构建两次，headless 与带 Spatial 的根可以同 ID。
  pose 批次仍携带该 `Arc` 的 `NetworkRevisionId`，供 Adapter 在提交宿主变换前复核。

动态 Route 仍按 ADR 0017：compiler 预编译静态初始路线；Runtime 新注册的动态
Route 用 typed dense candidate handle 编译 occurrence。

## 5. Tick

`TrafficWorld` 的 1-worker 车辆 tick 直接读取 `SharedTrafficNetwork` /
`SharedIdentityIndex` 的连续 slice（后继 CSR、准入、路线 occurrence、信号 program、
停车静态关系）。禁止：

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

地图：compiler 已冻夹具 `LFCA-V1-FULL-SPATIAL`
（`crates/laneflow-compiler/tests/fixtures/portable-v1/lfca-v1-full-spatial/`）。
它含 22 类实体 Identity、信号、停车、lane-pose 几何和一条
`entry → middle → exit` 静态路线。

Runtime 在该根上 spawn **两辆**同一路线前后排列的车，1-worker 固定步数。CI 集成
测试（无窗口）必须断言：

- `install` 成功且只保留一根 `Arc`；
- 两车都能推进；后车受前车约束可观察（跟车或至少不能穿透前车占用）；
- 因夹具有 `lane_pose`，`SpatialSession::bind` 同一 `Arc` 能产出 pose 批次；
- 安装或步进失败不留下半个 world；
- 测试 crate 不链接 `laneflow-core`。

不要求：完整停车离场状态机、红灯停止线的独立故事夹具、多 worker、与任何历史
Core 轨迹相等。S1 是共享根集成证据，**不能**单独替代下列 Runtime 原生覆盖。

### 6.4 拆除 Core 行为套件前的 Runtime 覆盖

§5 的第一刀（跟车、信号遵守、停车占用）在删除对应 Core 测试之前，必须有 **Runtime
原生** 测试覆盖同等行为。现有 `vehicle_following` / `signals_compliance` /
`parking_runtime` 套件保护这些语义；空实现若只过 S1 两车推进/pose 不得视为完成。

这些 Runtime 测试：

- 只链接 `laneflow-runtime`（及共享根/format/compiler 夹具），不链接 `laneflow-core`；
- 不把 Core 轨迹当预言机；
- 不要求逐条搬迁 Core 测试文件，但必须覆盖跟车安全间隙、信号停车/许可通行、停车占用
  权威（占用互斥与失败原子性）。

完整离场/预约生命周期若超出第一刀占用语义，可缩小断言，但不得变成 no-op。

### 6.3 最小 Bevy 示例

同一编制产物驱动一个最小 Bevy example：fixed tick + 代理位移。GUI 不进 CI；
CI 对该 example 做 `check`（可与现有 `native-example` feature 对齐）。这是「新的
端到端示例」，不是 corridor 规模演示。

## 7. 拆除（合入 `main` 的完成定义）

#301 使用 **一个完成 PR**（同一分支上的提交序列）。合入后 `main` 上：

必须消失或不再作为运行时入口：

- `crates/laneflow-core` 与 `laneflow-core-test-support`（前提：§6.4 的 Runtime
  原生跟车/信号/停车占用覆盖已存在）；
- `laneflow-data` / `laneflow-current-source` 作为 Core 的 JSON 加载入口；
- `laneflow-compiler-test-support` 的 LIR→Core 投影；
- `laneflow-spatial` 对 `laneflow-core` 的依赖（改为共享根 bind）；
- `laneflow-bevy`、`laneflow-scenario`、`laneflow-corridor-generator` 以及
  campus / `native_reference` / `signalized_corridor` 中任何 `CoreWorld` /
  `InitialTrafficData` 构造。接得上 S1 就改接；接不上就删除。
- research 代码中以 Core 为 `fixture-oracle` 的可选依赖：删除或改为非 CI 遗留，
  不得再作为可运行入口。

合入 `main` 时禁止出现「Runtime 已在、Bevy/示例仍跑 Core」的双入口。分支内部
提交可以暂时并存；**最后一个进入默认分支的状态**必须满足本节。

#294 不再拥有生产切换或拆除旧运行时路径。若本 PR 已删除 `laneflow-core` crate，
#294 只可能残留文档导航 / Skill 标识符改名；不得再把 Core 当正式世界。

#305 不再要求「current 与 target 同时存在时的等价矩阵」。阶段 7 认证目标路径
本身；其 G1 另开，不在本文展开。

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
- `LFCA-V1-FULL-SPATIAL` 无法支撑两车跟车的可观察断言，需要新的 S2 编制；
- 认为必须恢复 Core 预言机或产品双轨。

## 10. 对 G2 的输入

G2 开工前 Issue 须为 `Ready` 或等价。实现按本文一次完成，不拆成可独立交付、
合入后语义不完整的子 Issue。允许同一 PR 内分提交，提交顺序须保证审查时能看出
「先有 Runtime 再拆旧入口」，且默认分支终态满足 §7。
