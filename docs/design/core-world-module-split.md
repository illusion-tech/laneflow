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

#381 架构审查指控（`CoreWorld` 巨型对象（god object）、单一超大文件、`step()` 双分支复制）
经当前 main（`8f3de846`）核实成立，事实细节按当前代码修正：

- `crates/laneflow-core/src/world.rs` 共 7,434 行（含空行的总行数口径；非空行
  7,039）：生产实现与私有 helper 约 4,651 行（1-4651），内联 `mod tests`
  2,336 行（4669-7004），内联 `mod retained_memory` 429 行（7006-7434）。
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
- `CoreError` 单枚举现为 160 variant（`error.rs` 1,309 行含空行；#381 正文"95"
  已过时），拆分决策拆出 #389。

## 2. 目标与非目标

目标（对齐 #381 验收标准）：

- `world.rs` 按命令域（command domain）拆为子模块结构，不再存在单一 7k 行文件；
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
  调用关系与**既有可见性**；仅把原 private 成员提升为 `pub(super)`（world 模块
  树内跨域共享，对 `world` 及其全部后代含 `tests` 可见），`parking_runtime`
  的既有 `pub(crate)` 保持原样（crate 级 `parking.rs` 仍跨模块读取）；不新增
  crate 外公开表面，`pub mod world` 与 `pub use world::CoreWorld` 导出面不变；
- `use super::*` 仅承担名字导入（world.rs 无宏，无宏坑），不引入新 trait、新
  状态共享或新抽象；
- 内联 `mod tests`（4669-7004）纯文件搬迁为 `world/tests.rs`；内联
  `mod retained_memory`（7006-7434）保持 world 根 `#[cfg(test)]` 模块位置，纯文件
  搬迁为 `world/retained_memory.rs`（`retained_memory_tests.rs` 的
  `use super::retained_memory` 路径不变）；5 个既有测试模块文件保持原位；共
  7 个 `#[cfg(test)]` 模块（tests、retained_memory + 5 个）由 `mod.rs` 声明；
- tick 系列子模块使用 `tick_*` 前缀命名，避免与 crate 级模块
  （`occupancy` / `longitudinal` / `signal` / `parking` / `route`）同名混淆。

冻结模块表（区间以当前 main `8f3de846` 为准，实施时以函数归属为准）：

| 子模块                 | 内容                                                                                                                                                                                                                                                                                                                   | 现区间（world.rs）              | 约行数 |
| ---------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------- | ------ |
| `state.rs`             | `CoreWorld` 结构（27 个无条件私有字段与 2 个 `#[cfg(any(test, feature = "test-support"))]` 钩子字段以 `pub(super)`、`parking_runtime` 保持 `pub(crate)`）+ `new`/`with_traffic_data` + 基础车辆访问 + route_slot/vehicle_slot 通用访问器                                                                               | 350-554、4579-4590              | ~217   |
| `support.rs`           | 文件级内部结构（项一律 `pub(super)` 含字段，不持有 `impl CoreWorld` 成员）：RouteReferenceIndex、RouteSlot、VehicleSlot、StableVehicleOrder、CandidateStateScratch、VehicleAdvanceContext、NormalizedVehicleInput、CandidateVehicleOverlap、ParkingStepRelease、PreparedVehicleReplaceIds、parking_emergency_travel 等 | 70-349、4593-4638               | ~330   |
| `parking_commands.rs`  | parking 命令族 + 私有 helper（first_reachable_parking_entry、parking_arrived）                                                                                                                                                                                                                                         | 554-1300                        | ~747   |
| `signal_queries.rs`    | signal 查询族                                                                                                                                                                                                                                                                                                          | 1301-1340                       | ~40    |
| `route_queries.rs`     | profile/edge/route 查询族                                                                                                                                                                                                                                                                                              | 1341-1428                       | ~88    |
| `route_lifecycle.rs`   | register/remove route + 静态校验 helper                                                                                                                                                                                                                                                                                | 1429-1869                       | ~441   |
| `vehicle_lifecycle.rs` | spawn/replace/despawn + route reference index 维护 + 输入规范化                                                                                                                                                                                                                                                        | 1870-2515、4157-4299            | ~790   |
| `tick.rs`              | step/step_with_probe + 收敛后 advance 循环 + 提交阶段 + 故障注入 impl                                                                                                                                                                                                                                                  | 2516-2826、2875-2924、4639-4651 | ~374   |
| `tick_spatial.rs`      | command spatial index 重建/同步                                                                                                                                                                                                                                                                                        | 2827-2874                       | ~48    |
| `tick_overlap.rs`      | candidate overlap 校验族 + parking leave follower 校验                                                                                                                                                                                                                                                                 | 2925-3429                       | ~505   |
| `tick_longitudinal.rs` | occupancy/leader/longitudinal 重建族 + horizon/leader 计算                                                                                                                                                                                                                                                             | 3430-4156                       | ~727   |
| `tick_advance.rs`      | `advance_vehicle`（const generic `PARKING_ACTIVE`）                                                                                                                                                                                                                                                                    | 4300-4578                       | ~278   |
| `tests.rs`             | 内联 `mod tests` 纯文件搬迁                                                                                                                                                                                                                                                                                            | 4669-7004                       | ~2,336 |
| `retained_memory.rs`   | 内联 `mod retained_memory` 纯文件搬迁（保持 world 根 `#[cfg(test)]` 模块位置，`retained_memory_tests.rs` 路径不变）                                                                                                                                                                                                    | 7006-7434                       | ~429   |
| `mod.rs`               | 文件头 doc/use 导入（1-69）+ 模块声明 + re-export（`pub use state::CoreWorld`、`pub(super) use support::*` 等）+ 7 个 `#[cfg(test)]` 模块声明（tests、retained_memory + 5 个研究/白盒测试）                                                                                                                            | —                               | ~120   |

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
│                           (~2,336 LOC, zero assertion change)          │
│ retained_memory.rs       world-root cfg(test) module (~429 LOC)        │  (R)
│ *_tests                 occupancy / retained_memory_tests /            │  (S)
│                           event_merge_research /                       │
│                           partitioned_occupancy_research /             │
│                           selective_read_research                      │
└────────────────────────────────────────────────────────────────────────┘
```

Where：

- (A) **laneflow-core crate**：拆分范围仅限本 crate 内部，不涉及 Data/Spatial/Scenario/Bevy crate。
- (B) **lib.rs**：公开导出面保持不变（`pub mod world` 与 `pub use world::CoreWorld` 原样保留），`CoreWorld` 签名、语义与确定性语义不变。
- (C) **external consumers**：外部消费者路径不变、零影响（laneflow-bevy、laneflow-core-test-support、laneflow-compiler-test-support 与全部 benches），均经 crate 根或 `world::CoreWorld` 访问。
- (D) **world module**：`world.rs` 迁为 `world/mod.rs`（`world/` 目录已存在，无命名冲突）；`mod.rs` 承载文件头 doc/use 导入、`pub use state::CoreWorld` 与 `pub(super) use support::*` 等 re-export、7 个 `#[cfg(test)]` 模块声明（tests、retained_memory + 5 个研究/白盒测试）。
- (E) **state.rs**：`CoreWorld` 结构定义（共 30 字段：28 个无条件字段 + 2 个 `#[cfg(any(test, feature = "test-support"))]` 故障注入钩子）、构造（`new` / `with_traffic_data`）、基础车辆访问与 route_slot/vehicle_slot 通用访问器；27 个无条件私有字段与 2 个 `#[cfg(any(test, feature = "test-support"))]` 钩子字段以 `pub(super)`、`parking_runtime` 保持既有 `pub(crate)`，供兄弟模块 impl 访问，不扩大 crate 外表面。
- (F) **support.rs**：文件级内部结构（`RouteReferenceIndex`、`RouteSlot`、`VehicleSlot`、`StableVehicleOrder`、`CandidateStateScratch`、`VehicleAdvanceContext`、`NormalizedVehicleInput`、`CandidateVehicleOverlap`、`ParkingStepRelease`、`PreparedVehicleReplaceIds`、`parking_emergency_travel` 等），项一律 `pub(super)`（含字段）且不持有 `impl CoreWorld` 成员，被命令域与 tick 系列共享。
- (G) **parking_commands.rs**：parking 命令族（`reserve_parking_space` / `cancel_parking_reservation` / `commit_parking` / `spawn_parked_vehicle` / `rebind_reserved_vehicle_route` / `leave_parking` 及私有 helper）；跨域调用 (N) 的 `validate_parking_leave_followers`。
- (H) **signal_queries.rs**：signal 查询族（controller / group / maneuver-gate 快照查询）。
- (I) **route_queries.rs**：profile / edge / route 句柄、外部 ID 与出现项（maneuver / gate / waiting-zone）查询。
- (J) **route_lifecycle.rs**：路线注册/删除与静态校验（`register_route`、`register_compiled_route`、`build_route_metadata`、`remove_route` 及 `validate_*`）。
- (K) **vehicle_lifecycle.rs**：车辆生命周期（`spawn_vehicle` / `replace_completed_vehicle` / `despawn_vehicle`）、route reference index 维护与输入规范化；跨域调用 (N) 的 `validate_candidate_overlap`。
- (L) **tick.rs**：`step` / `step_with_probe` 编排（对齐 `core-runtime-scalability-audit.md` §4.1 七阶段：tick/time 与 Signal 候选快照 → occupancy/leader 重建 → longitudinal 重建 → 车辆推进 → Parking release 等跨域 invariant 批校验 → 事件与派生 index → 一次原子提交）；`step()` 双分支复制收敛为单一 `advance_all_vehicles<const PARKING_ACTIVE: bool>` 循环；`append_signal_events` 与故障注入 impl 亦归此。
- (M) **tick_spatial.rs**：command spatial index 重建（`rebuild_command_spatial_index`，被 (E) 构造路径调用）与成员同步（`sync_changed_command_spatial_memberships`，被 (L) 调用）。
- (N) **tick_overlap.rs**：候选重叠校验族（`validate_candidate_overlap`、`validate_candidate_overlap_excluding`、`find_candidate_overlap` 等）、`validate_initial_vehicle_overlaps` 与 parking leave follower 校验；被 (E)/(G)/(K)/(L) 共用。
- (O) **tick_longitudinal.rs**：occupancy / leader 重建（`rebuild_occupancy_and_leaders`、`rebuild_longitudinal_motions`）与 horizon / leader 计算（speed-limit / parking-stop / signal-stop horizon、`find_leader`、`braking_distance` 等）。
- (P) **tick_advance.rs**：`advance_vehicle`（const generic `PARKING_ACTIVE`）。
- (Q) **tests.rs**：原内联 `mod tests`（4669-7004，约 2,336 行）纯文件搬迁，断言零改动。
- (R) **retained_memory.rs**：原内联 `mod retained_memory`（7006-7434，约 429 行）保持 world 根 `#[cfg(test)]` 模块位置纯文件搬迁——`retained_memory_tests.rs` 的 `use super::retained_memory` 路径不变。
- (S) **\*_tests**：5 个 `#[cfg(test)]` 测试模块（3 个 A 类研究原型 + occupancy 行为白盒 + retained_memory 保留内存账本，#380 外移完成）保持原文件位置与内容，仅 `#[cfg(test)] mod` 声明移入 `mod.rs`，共 7 个 `#[cfg(test)]` 模块声明。

大小预算：生产单文件不超过约 800 行；`tests.rs` 约 2.3k 行与 `retained_memory.rs`
约 0.4k 行为纯测试文件，不增加生产评审导航成本。

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
- **release 批校验落点**：`validate_reserved_pair` 批校验（现 2774-2792）保持为
  advance 循环完成之后、事件生成与提交之前的独立阶段，不折入逐车循环——折入
  会改变"早车辆非法 release、晚车辆 advance 报错"组合下的 first-error 优先级
  （现语义：advance 错误优先）。

收敛核对清单（实施时逐条对照）：

- 事件顺序：reserved 完成先 `ParkingReservationReleased` 后 `VehicleCompletedRoute`（现 2722-2730）；
- 故障注入 `space` 参数：false 分支为 `None`、true 分支为 `reserved_space`（现 2631 vs 2755）；
- `parking_stops` 构造与排空 `debug_assert` 仅存在于 PARKING_ACTIVE 实例（现 2641/2763）；
- first-error 与 scratch 回滚在循环外共享（现 2767-2769）；
- `step_reachable_target_completed` invariant 触发条件保持（现 2711-2717）；
- 停车上下文解析无副作用、重排安全。

备选方案（冻结为否决）：

- 运行时 `has_reserved_parking` 每车分支：在无保留停车热路径引入每车分支与
  重复解析，否决；
- trait object / closure 钩子：动态分发或闭包间接，破坏零开销承诺，否决；
- 维持双份：不收敛，否决。

性能护栏：PR 阶段在同一基准机执行 `cargo +1.96.0 bench -p laneflow-core --bench
core_step --locked`（及 `core_commands`）前后对比；bench 不进 CI（`core_step.rs`
头注约定），以历史基线语义比较。若出现可测回归：先定位根因（纯搬移 vs 收敛）；
根因为收敛时修订收敛设计（如调整 PARKING_ACTIVE 折叠边界）后重新验证；不得
以回退双分支作为可接受结果——回退即放弃 AC1（双分支收敛），必须经显式
G1/AC 变更评审后才可实施。

## 5. CoreError 结论（#381 验收标准 AC4）

- 结论：不纳入本切片，拆为后续 Issue #389（`error.rs` 1,309 行 / 160 variant
  单枚举拆分评估）。
- 依据：`error.rs` 独立于 `world.rs` 结构，拆分与命令域正交；单枚举集中当前
  全部错误模型，保持 crate 内 match 完整性与 Display 文案一致性（`CoreError`
  仅 derive `Clone, Debug, thiserror::Error`，无序列化面；`#[non_exhaustive]`
  已阻止下游 exhaustive match）；公开 variant 路径（`CoreError::Variant`）是
  公共 API 面，拆分将改变该路径——是否保留路径兼容（re-export 别名或接受
  clean break）由 #389 设计判断；本切片不拆分。

## 6. 文档与术语

- `docs/design/core-runtime.md`：在 §3 Core 边界后新增"world 模块结构"小节，
  记录 §3 冻结模块布局，并注明 `tick_*` 子模块是 `CoreWorld` 方法宿主而非领域
  逻辑所有者（领域逻辑仍在 crate 级同名模块）（本切片实施时同步）。
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
  tick_longitudinal → tick_advance）；④ step 双分支收敛；⑤ `tests.rs` 与
  `retained_memory.rs` 搬迁；
  ⑥ `core-runtime.md` 结构小节与 glossary 词条。
- 验收：AC1 拆分 + 双分支收敛；AC2 导出面不变（`pub mod world` +
  `pub use world::CoreWorld`；外部使用点 laneflow-bevy / core-test-support /
  compiler-test-support / benches 全经 crate 根或 `world::CoreWorld` 路径）；
  AC3 `cargo test --workspace --locked` 全量 + bench 同机对比 +
  `check-commit-messages` + `format-md-tables --check`；AC4 见 §5。

## 8. 术语

| 中文规范术语 | 英文辅助名（English Alias） | 精确标识符 / 缩写 | 中文规范含义                                                                                                                                              |
| ------------ | --------------------------- | ----------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 命令域       | command domain              | —                 | 提案中（Proposed；#381）：按领域命令与查询入口划分的 `CoreWorld` 实现组织单位；#381 拆分把 parking、route、vehicle、signal 与 tick 推进各自收敛为子模块。 |