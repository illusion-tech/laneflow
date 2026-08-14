# CoreWorld 命令域拆分设计

**文档状态**: Review<br>
**最后更新**: 2026-08-14<br>
**适用范围**: `laneflow-core` 的 `CoreWorld` 与 `world.rs` 内部组织；本文是 #381 的
G1 冻结方案，冻结生效以 #381 Gate Ledger 的 G1 记录为准。本文引入的组织术语以
`../reference/glossary.md` 的中文定义为权威<br>
**关联文档**:

- [`core-runtime.md`](core-runtime.md)（Core runtime 主设计文档；本切片同步其结构描述）
- [`core-runtime-scalability-audit.md`](core-runtime-scalability-audit.md)（`step()` 热路径背景）
- [`../reference/glossary.md`](../reference/glossary.md)（命令域术语权威定义）
- [`../governance/development-gates.md`](../governance/development-gates.md)
- [Issue #381](https://github.com/illusion-tech/laneflow/issues/381)、[Issue #389](https://github.com/illusion-tech/laneflow/issues/389)

## 1. 背景与事实修正

#381 架构审查指控（`CoreWorld` god object、单一超大文件、`step()` 双分支复制）
经当前 main（`8f3de846`）核实成立，事实细节按当前代码修正：

- `crates/laneflow-core/src/world.rs` 共 7,434 行：生产实现与私有 helper 约
  4,651 行（1-4651），内联 `mod tests` 2,766 行（4669-7434，含
  `retained_memory` 子模块）。
- `CoreWorld` 共 30 个字段（`world.rs:350-383`）：28 个无条件字段 + 2 个
  `#[cfg(any(test, feature = "test-support"))]` 故障注入钩子；#380 已完成研究
  字段与仪器外移，无遗留 `#[cfg(test)]` 研究状态。
- 单一主 `impl CoreWorld`（385-4590）与故障注入小 `impl`（4639-4651）。
- 函数族区间（按当前 main 行号）：
  - parking 命令族 554-1300（`reserve_parking_space` / `cancel_parking_reservation` /
    `commit_parking` / `spawn_parked_vehicle` / `rebind_reserved_vehicle_route` /
    `leave_parking` 及私有 helper）；
  - signal 查询族 1301-1340；
  - profile/edge/route 查询族 1341-1428；
  - route 生命周期 1429-1869（`register_route` / `register_compiled_route` /
    `build_route_metadata` / `remove_route` 与静态校验 helper）；
  - vehicle 生命周期 1870-2370（`spawn_vehicle` / `replace_completed_vehicle` /
    `despawn_vehicle`）与 route reference index 维护 2371-2515；
  - tick 推进 2516-2826（`step` / `step_with_probe`）；
  - spatial/overlap 校验 2827-3429；
  - occupancy/leader/longitudinal 重建与 horizon 计算 3430-4156；
  - 输入规范化 4157-4299、`advance_vehicle` 4300-4590。
- `step_with_probe` 双分支复制确认（2590-2640 无保留停车 / 2647-2762 保留停车）：
  除 `advance_vehicle::<false|true>` 与停车特有处理外，车辆迭代骨架整体重复。
- 研究/测试仪器：#380 外移后 `world/` 目录下 5 个 `#[cfg(test)]` 模块
  （occupancy / retained_memory / event_merge_research / partitioned_occupancy_research /
  selective_read_research），经 `world.rs:4653-4666` 声明。
- `CoreError` 单枚举现为 160 variant（`error.rs` 1,309 行；#381 正文"95"已过时），
  拆分决策拆出 #389。

## 2. 目标与非目标

目标（对齐 #381 验收标准）：

- `world.rs` 按命令域拆为子模块结构，不再存在单一 7k 行文件；
- `step()` 双分支复制收敛；
- `CoreWorld` 公开 API 签名与语义不变；`lib.rs` re-export 面不变；
- `cargo test --workspace --locked` 全量通过，测试断言零改动；
- 既有 benches 同机对比确认无性能回归。

非目标：

- 不改变任何公开 Core API 签名、行为与确定性语义；
- 不做研究/测试仪器外移（#380 已完成；本切片不重复搬移 `#[cfg(test)]` 代码）；
- 不重排领域逻辑实现（`signal.rs` / `parking.rs` / `longitudinal.rs` 的领域
  代码保持原样，只拆 `CoreWorld` 命令与查询入口）；
- 不改动 Data/Spatial/Scenario/Bevy crate；
- 不拆分 `CoreError`（拆 #389，见 §5）。

## 3. 冻结决策：命令域子模块划分

原则：

- `world.rs` 迁为 `world/mod.rs`，`pub mod world` 路径与
  `world::CoreWorld` 导出不变；
- 子模块只搬移 `impl CoreWorld` 成员与文件级私有 helper，不改变任何函数签名、
  调用关系与可见性（不新增 `pub` / `pub(crate)` 表面）；
- 跨域私有访问沿用 Rust 子模块对父模块私有项的可见性（`use super::*`），不引入
  新 trait、新状态共享或新抽象；
- 内联 `mod tests` 纯文件搬迁为 `world/tests.rs`（零断言改动）；5 个研究测试
  模块保持原文件与 `#[cfg(test)]` 声明，仅把 mod 声明移入 `mod.rs`；
- tick 系列子模块使用 `tick_*` 前缀命名，避免与 crate 级模块
  （`occupancy` / `longitudinal` / `signal` / `parking` / `route`）同名混淆。

冻结模块表（区间以当前 main `8f3de846` 为准，实施时以函数归属为准）：

| 子模块                 | 内容                                                                                                                                                                                                  | 现区间（world.rs）              | 约行数 |
| ---------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------- | ------ |
| `state.rs`             | `CoreWorld` 结构 + `new`/`with_traffic_data` + 基础车辆访问                                                                                                                                           | 350-554                         | ~205   |
| `support.rs`           | 文件级内部结构：RouteReferenceIndex、RouteSlot、VehicleSlot、StableVehicleOrder、CandidateStateScratch、VehicleAdvanceContext、NormalizedVehicleInput、CandidateVehicleOverlap、ParkingStepRelease 等 | 70-349、4593-4638               | ~330   |
| `parking_commands.rs`  | parking 命令族 + 私有 helper（first_reachable_parking_entry、parking_arrived）                                                                                                                        | 554-1300                        | ~747   |
| `signal_queries.rs`    | signal 查询族                                                                                                                                                                                         | 1301-1340                       | ~40    |
| `route_queries.rs`     | profile/edge/route 查询族                                                                                                                                                                             | 1341-1428                       | ~88    |
| `route_lifecycle.rs`   | register/remove route + 静态校验 helper                                                                                                                                                               | 1429-1869                       | ~441   |
| `vehicle_lifecycle.rs` | spawn/replace/despawn + route reference index 维护 + 输入规范化                                                                                                                                       | 1870-2515、4157-4299            | ~790   |
| `tick.rs`              | step/step_with_probe + 收敛后 advance 循环 + 提交阶段 + 故障注入 impl                                                                                                                                 | 2516-2826、2875-2924、4639-4651 | ~374   |
| `tick_spatial.rs`      | command spatial index 重建/同步                                                                                                                                                                       | 2827-2874                       | ~48    |
| `tick_overlap.rs`      | candidate overlap 校验族 + parking leave follower 校验                                                                                                                                                | 2925-3429                       | ~505   |
| `tick_longitudinal.rs` | occupancy/leader/longitudinal 重建族 + horizon/leader 计算                                                                                                                                            | 3430-4156                       | ~727   |
| `tick_advance.rs`      | `advance_vehicle` + `append_signal_events` + route_slot/vehicle_slot                                                                                                                                  | 4300-4590                       | ~291   |
| `tests.rs`             | 内联 `mod tests`（含 retained_memory 子模块）纯文件搬迁                                                                                                                                               | 4669-7434                       | ~2,766 |
| `mod.rs`               | 模块声明 + 5 个 `#[cfg(test)] mod <研究测试>;`                                                                                                                                                        | —                               | ~40    |

拆分后的模块结构总览（AAD 标记见下方 Where）：

```text
laneflow-core crate  (A)
  lib.rs:     pub mod world;  pub use world::CoreWorld  (B)
  consumers:  laneflow-bevy, laneflow-core-test-support,  (C)
              laneflow-compiler-test-support, benches

┌────────────────────────────────────────────────────────────────────────┐
│ world module  (world/mod.rs)                                           │  (D)
│                                                                        │
│ state.rs                 CoreWorld struct: 30 fields (2 cfg hooks)     │  (E)
│                           new / with_traffic_data / basic accessors    │
│                                                                        │
│ support.rs               file-level internal structs                   │  (F)
│                           RouteReferenceIndex, RouteSlot, VehicleSlot, │
│                           StableVehicleOrder, CandidateStateScratch,   │
│                           VehicleAdvanceContext, ...                   │
│                                                                        │
│ parking_commands.rs      parking command family                        │  (G)
│ signal_queries.rs        signal query family                           │  (H)
│ route_queries.rs         profile/edge/route queries                    │  (I)
│ route_lifecycle.rs       route register / remove                       │  (J)
│ vehicle_lifecycle.rs     spawn / replace / despawn                     │  (K)
│                           + route-reference index                      │
│                                                                        │
│ tick.rs                  step / step_with_probe                        │  (L)
│                           single converged advance loop                │
│                           (const PARKING_ACTIVE)                       │
│   tick_spatial.rs           command spatial index                      │  (M)
│   tick_overlap.rs           candidate overlap checks                   │  (N)
│   tick_longitudinal.rs      occupancy / longitudinal                   │  (O)
│   tick_advance.rs           advance_vehicle                            │  (P)
│                                                                        │
│ tests.rs                 inline mod tests, moved verbatim              │  (Q)
│ *_research_tests         occupancy / retained_memory /                 │  (R)
│                           event_merge_research /                       │
│                           partitioned_occupancy_research /             │
│                           selective_read_research                      │
└────────────────────────────────────────────────────────────────────────┘
```

Where：

- (A) **laneflow-core crate**：拆分范围仅限本 crate 内部，不涉及 Data/Spatial/Scenario/Bevy crate。
- (B) **lib.rs**：公开导出面保持不变（`pub mod world` 与 `pub use world::CoreWorld` 原样保留），`CoreWorld` 签名、语义与确定性语义不变。
- (C) **external consumers**：外部消费者路径不变、零影响（laneflow-bevy、laneflow-core-test-support、laneflow-compiler-test-support 与全部 benches），均经 crate 根或 `world::CoreWorld` 访问。
- (D) **world module**：`world.rs` 迁为 `world/mod.rs`（`world/` 目录已存在，无命名冲突）；`#[cfg(test)]` 研究测试模块声明集中于此。
- (E) **state.rs**：`CoreWorld` 结构定义（共 30 字段：28 个无条件字段 + 2 个 `#[cfg(any(test, feature = "test-support"))]` 故障注入钩子）、构造（`new` / `with_traffic_data`）与基础车辆访问器；各子模块经 `use super::*` 私有访问，不扩大可见性。
- (F) **support.rs**：文件级内部结构（`RouteReferenceIndex`、`RouteSlot`、`VehicleSlot`、`StableVehicleOrder`、`CandidateStateScratch`、`VehicleAdvanceContext`、`NormalizedVehicleInput`、`CandidateVehicleOverlap`、`ParkingStepRelease` 等），被命令域与 tick 系列共享。
- (G) **parking_commands.rs**：parking 命令族（`reserve_parking_space` / `cancel_parking_reservation` / `commit_parking` / `spawn_parked_vehicle` / `rebind_reserved_vehicle_route` / `leave_parking` 及私有 helper）；跨域调用 (N) 的 `validate_parking_leave_followers`。
- (H) **signal_queries.rs**：signal 查询族（controller / group / maneuver-gate 快照查询）。
- (I) **route_queries.rs**：profile / edge / route 句柄、外部 ID 与出现项（maneuver / gate / waiting-zone）查询。
- (J) **route_lifecycle.rs**：路线注册/删除与静态校验（`register_route`、`register_compiled_route`、`build_route_metadata`、`remove_route` 及 `validate_*`）。
- (K) **vehicle_lifecycle.rs**：车辆生命周期（`spawn_vehicle` / `replace_completed_vehicle` / `despawn_vehicle`）、route reference index 维护与输入规范化；跨域调用 (N) 的 `validate_candidate_overlap`。
- (L) **tick.rs**：`step` / `step_with_probe` 编排（tick/time 溢出检查 → 信号候选快照 → occupancy/longitudinal 重建 → 车辆推进 → 事件生成 → 一次原子提交）；`step()` 双分支复制收敛为单一 `advance_all_vehicles<const PARKING_ACTIVE: bool>` 循环；`append_signal_events` 与故障注入 impl 亦归此。
- (M) **tick_spatial.rs**：command spatial index 重建（`rebuild_command_spatial_index`，被 (E) 构造路径调用）与成员同步（`sync_changed_command_spatial_memberships`，被 (L) 调用）。
- (N) **tick_overlap.rs**：候选重叠校验族（`validate_candidate_overlap`、`validate_candidate_overlap_excluding`、`find_candidate_overlap` 等）、`validate_initial_vehicle_overlaps` 与 parking leave follower 校验；被 (E)/(G)/(K)/(L) 共用。
- (O) **tick_longitudinal.rs**：occupancy / leader 重建（`rebuild_occupancy_and_leaders`、`rebuild_longitudinal_motions`）与 horizon / leader 计算（speed-limit / parking-stop / signal-stop horizon、`find_leader`、`braking_distance` 等）。
- (P) **tick_advance.rs**：`advance_vehicle`（const generic `PARKING_ACTIVE`）、`route_slot` / `vehicle_slot` 访问器。
- (Q) **tests.rs**：原内联 `mod tests`（含 `retained_memory` 子模块）纯文件搬迁，断言零改动（约 2,766 行）。
- (R) **\*_research_tests**：5 个研究测试模块（#380 外移完成）保持原文件位置与内容，仅 `#[cfg(test)] mod` 声明移入 `mod.rs`，不重复搬移。

大小预算：生产单文件不超过约 800 行；`tests.rs` 约 2.8k 行为纯测试文件，不增加
生产评审导航成本。

## 4. 冻结决策：step() 双分支收敛

现状：`step_with_probe` 内无保留停车（2590-2640）与保留停车（2647-2762）两分支
复制整个车辆迭代骨架。

冻结方案：

- 提取单一 `advance_all_vehicles<const PARKING_ACTIVE: bool>`（沿用
  `advance_vehicle` 既有 const generic 模式）承载车辆迭代骨架单份实现；
- 顶层保留 `parking_runtime.reserved_count() > 0` 运行时分发，两次
  monomorphization（`false` / `true`）；
- 停车特有处理（`parking_stops` 迭代、Reserved binding 上下文解析、release/
  arrival 事件、invariant 校验、故障注入的 `space` 参数）置于
  `if PARKING_ACTIVE { ... }` 内，由编译期折叠消除；
- 行为保持：first-error 语义、事件顺序、失败原子性与
  `#[cfg(any(test, feature = "test-support"))]` 故障注入语义不变。

备选方案（冻结为否决）：

- 运行时 `has_reserved_parking` 每车分支：在无保留停车热路径引入每车分支与
  重复解析，否决；
- trait object / closure 钩子：动态分发或闭包间接，破坏零开销承诺，否决；
- 维持双份：不收敛，否决。

性能护栏：PR 阶段在同一基准机执行 `cargo +1.96.0 bench -p laneflow-core --bench
core_step --locked`（及 `core_commands`）前后对比；bench 不进 CI（`core_step.rs`
头注约定），以历史基线语义比较；若出现可测回归，回退双分支并记录例外。

## 5. CoreError 结论（#381 验收标准 AC4）

- 结论：不纳入本切片，拆为后续 Issue #389（`error.rs` 1,309 行 / 160 variant
  单枚举拆分评估）。
- 依据：`error.rs` 独立于 `world.rs` 结构；单枚举保持全量 exhaustive match 与
  序列化面收益；拆分方案需要独立设计判断，与命令域拆分正交。

## 6. 文档与术语

- `docs/design/core-runtime.md`：在 §3 Core 边界后新增"world 模块结构"小节，
  记录 §3 冻结模块布局（本切片实施时同步）。
- `docs/design/core-runtime-scalability-audit.md` §4.1：step 阶段语义不变，
  无需改写。
- `docs/reference/v0.5-*`、`v0.6-*` 中引用 `world.rs` 的历史验证证据按
  `documentation-policy.md` 保留原样。
- glossary：本设计登记"命令域（command domain）"词条（见 §8）。

## 7. 实施步骤与验收

- 建议单 Delivery PR（搬移 + 收敛 + 文档同步一次完成，bench 对比一次到位）；
  如审阅发现风险，可按 §3 冻结模块分组为 Related PR 切片。
- 实施顺序：① `support.rs`/`state.rs` 抽取；② 各命令域模块搬移
  （parking_commands → signal_queries → route_queries → route_lifecycle →
  vehicle_lifecycle）；③ tick 系列搬移（tick_spatial → tick_overlap →
  tick_longitudinal → tick_advance）；④ step 双分支收敛；⑤ `tests.rs` 搬迁；
  ⑥ `core-runtime.md` 结构小节与 glossary 词条。
- 验收：AC1 拆分 + 双分支收敛；AC2 导出面不变（`pub mod world` +
  `pub use world::CoreWorld`；外部使用点 laneflow-bevy / core-test-support /
  compiler-test-support / benches 全经 crate 根或 `world::CoreWorld` 路径）；
  AC3 `cargo test --workspace --locked` 全量 + bench 同机对比 +
  `check-commit-messages` + `format-md-tables --check`；AC4 见 §5。

## 8. 术语

| 中文规范术语 | 英文辅助名（English Alias） | 精确标识符 / 缩写 | 中文规范含义                                                                                                                |
| ------------ | --------------------------- | ----------------- | --------------------------------------------------------------------------------------------------------------------------- |
| 命令域       | command domain              | —                 | 按领域命令与查询入口划分的 `CoreWorld` 实现组织单位；#381 把 parking、route、vehicle、signal 与 tick 推进各自收敛为子模块。 |