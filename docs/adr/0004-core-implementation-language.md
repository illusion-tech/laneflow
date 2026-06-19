# 0004 Core Implementation Language

**状态**: Proposed
**日期**: 2026-06-20
**适用范围**: LaneFlow Core 的主要实现语言、crate 边界与后续 Adapter 绑定策略

## 背景

LaneFlow Core 需要作为引擎无关 runtime 被 Unity、Unreal、Godot、O3DE、Web 或其他数字孪生运行环境复用。

Core 需要满足：

- 与具体游戏引擎 API 解耦；
- 支持小而可测的运行时原语；
- 支持显式 tick 和确定性测试；
- 能够在后续通过 Adapter 暴露给多种宿主环境；
- 保持商用可控，不把重型交通仿真器作为客户端运行时依赖。

如果 Core 实现语言不明确，后续 issue 会在 crate/package 边界、API 命名、FFI、WASM、数值类型和测试工具链上各自假设，导致早期设计漂移。

## 决策

LaneFlow v0.1 Core 的初始实现目标为 Rust library crate。

Rust API 是 v0.1 的首要实现边界。C ABI、WASM、Unity / Unreal / Godot / O3DE Adapter 绑定和稳定 Adapter API 不在本 ADR 中冻结，应在后续 Adapter 或 API 设计中单独处理。

Core crate 不得依赖 Unity、Unreal、Godot、O3DE、DOM、WebGL 或其他 presentation / engine API。

## 实现约束

- 时间推进使用显式输入，不读取 wall clock。
- Tick 与时间累计字段优先使用整数类型，例如 `delta_time_ms: u64`、`tick_index: u64`、`time_ms: u64`。
- v0.1 public Rust API 可接近：

```rust
fn step(world: &mut CoreWorld, input: TickInput) -> Result<StepResult, CoreError>
```

- 实现可以使用内部 mutation，但不得依赖隐藏全局状态、随机数或宿主引擎状态。
- 确定性测试不得依赖 `HashMap` 等无稳定迭代顺序集合的输出顺序；事件、车辆更新和 edge traversal 输出应有稳定顺序。
- v0.1 可以使用 `f64` 表达 speed / distance，但这只满足同一实现和同一运行环境内的可重复性，不代表跨平台 bit-level determinism。
- 如果后续需要跨平台 bit-level determinism，应新增 ADR 评估 fixed-point 或 deterministic math 策略。

## 替代方案

- C++ 更贴近部分 native engine 生态，但会增加内存安全、ABI、构建矩阵和跨平台绑定成本。
- TypeScript / JavaScript 有利于 Web 原型，但不适合作为 Unity、Unreal、Godot、O3DE 共享的 native Core 默认实现。
- C# 对 Unity 友好，但会把 Core 的默认生态偏向单一引擎。
- Zig 等语言仍可作为后续研究项，但当前生态、团队熟悉度和绑定工具链不如 Rust 稳妥。

## 后果

- v0.1 Core 实现 issue 应初始化 Rust crate 和 `cargo test` 骨架。
- Core API 设计应优先使用 Rust 类型、错误模型和测试习惯表达，再由后续 Adapter 设计决定绑定层形态。
- Adapter 集成会多一个绑定层，但可以避免引擎依赖污染 Core。
- Web 支持应通过后续 WASM 设计处理，而不是把 DOM / WebGL 依赖放入 Core。
- 未来若改用其他 Core 实现语言，应新增 superseding ADR，而不是静默修改本 ADR。