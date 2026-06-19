# 0004 Core Implementation Language

**状态**: Accepted
**日期**: 2026-06-20
**适用范围**: LaneFlow Core 的主要实现语言、crate 边界与后续 Adapter 绑定策略
**关联文档**:

- 上游决策:
  - `0001-project-scope.md`
  - `0002-dependency-and-licensing-constraints.md`
  - `0003-runtime-tick-and-determinism.md`
- 相关设计:
  - `../design/core-runtime.md`

## 背景

LaneFlow Core 需要作为引擎无关 runtime 被 Unity、Unreal、Godot、O3DE、Web 或其他数字孪生运行环境复用。

Core 需要满足：

- 与具体游戏引擎 API 解耦；
- 支持小而可测的运行时原语；
- 支持显式 fixed tick 和确定性测试；
- 能够在后续通过 Adapter 暴露给多种宿主环境；
- 保持商用可控，不把重型交通仿真器作为客户端运行时依赖。

如果 Core 实现语言不明确，后续 issue 会在 crate/package 边界、API 命名、FFI、WASM、数值类型和测试工具链上各自假设，导致早期设计漂移。

## 决策

LaneFlow v0.1 Core 的初始实现目标为 Rust library crate。

Rust API 是 v0.1 的首要实现边界。C ABI、WASM、Unity / Unreal / Godot / O3DE Adapter 绑定和稳定 Adapter API 不在本 ADR 中冻结，应在后续 Adapter 或 API 设计中单独处理。

初始仓库边界采用 Cargo workspace，并把 Core crate 放在 `crates/laneflow-core`；crate 名称使用 `laneflow_core`。若后续需要调整 workspace 布局，应在对应实现 PR 中说明原因和影响。

Core crate 不得依赖 Unity、Unreal、Godot、O3DE、DOM、WebGL 或其他 presentation / engine API。

Rust edition 和 MSRV 不由本 ADR 冻结。初始化 Rust Core crate 的实现 issue 必须在 `Cargo.toml`、CI 或后续工具链文档中记录实际选择，并避免依赖 nightly-only 能力，除非另有 ADR 或显式例外。

## 实现约束

- 时间推进使用显式 fixed tick，不读取 wall clock。
- Tick 与时间累计字段使用整数类型，例如 `fixed_delta_time_ms: u64`、`tick_index: u64`、`time_ms: u64`。
- v0.1 public Rust API 可接近：

```rust
fn step(world: &mut CoreWorld, input: TickInput) -> Result<StepResult, CoreError>
```

- 实现可以使用内部 mutation，但不得依赖隐藏全局状态、随机数或宿主引擎状态。
- 确定性测试不得依赖 `HashMap` 等无稳定迭代顺序集合的输出顺序；事件、车辆更新和 edge traversal 输出应有稳定顺序。
- v0.1 可以使用 `f64` 表达 speed、distance 和 edge progress，但这只满足同一实现和同一运行环境内的可重复性，不代表跨平台 bit-level determinism。
- 距离、速度和 progress 若跨模块或 public API 暴露，应优先使用 Rust newtype 包装，而不是在 API 边界散落裸 `f64`。
- 测试应对 `tick_index`、`time_ms`、vehicle status、route edge index 和 event order 使用精确断言。
- 测试应对 speed、distance 和 edge progress 等连续浮点值使用明确 epsilon；epsilon 应是 Core 中的命名常量或测试 helper，不应散落 magic number。
- Edge boundary、route completion 等离散行为必须有稳定规则；接近 boundary 的浮点结果应通过明确 epsilon / snap 规则转换为精确事件和状态断言。
- 如果后续需要跨平台 bit-level determinism，应新增 ADR 评估 fixed-point 或 deterministic math 策略。

## 替代方案

- C++ 更贴近部分 native engine 生态，但会增加内存安全、ABI、构建矩阵和跨平台绑定成本。
- TypeScript / JavaScript 有利于 Web 原型，但不适合作为 Unity、Unreal、Godot、O3DE 共享的 native Core 默认实现。
- C# 对 Unity 友好，但会把 Core 的默认生态偏向单一引擎。
- Zig 等语言仍可作为后续研究项，但当前生态、团队熟悉度和绑定工具链不如 Rust 稳妥。
- Fixed-point 可以增强跨平台可重放能力，但 v0.1 会过早冻结单位、scale、舍入、overflow 和几何转换策略；当前阶段先用 `f64` 建立 Core runtime 闭环。

## 后果

- v0.1 Core 实现 issue 应初始化 Rust crate 和 `cargo test` 骨架。
- Core API 设计应优先使用 Rust 类型、错误模型和测试习惯表达，再由后续 Adapter 设计决定绑定层形态。
- Adapter 集成会多一个绑定层，但可以避免引擎依赖污染 Core。
- Web 支持应通过后续 WASM 设计处理，而不是把 DOM / WebGL 依赖放入 Core。
- v0.1 deterministic tests 的断言口径被拆分为离散状态精确断言和连续浮点 epsilon 断言。
- 未来若改用其他 Core 实现语言，应新增 superseding ADR，而不是静默修改本 ADR。
