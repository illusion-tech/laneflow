# 0003 Runtime Tick and Determinism

**状态**: Accepted（#496 / ADR 0028 Proposed 修订步长合法区间与相位倍数，未 Pass；正文 v0.1「Core」指当时执行层，现行世界是 `TrafficWorld`）
**日期**: 2026-06-20
**最后更新**: 2026-08-24
**适用范围**: LaneFlow 交通运行时的 tick 输入、时间推进与确定性策略
**关联文档**:

- 上游决策:
  - `0001-project-scope.md`
  - `0002-dependency-and-licensing-constraints.md`
- 相关 ADR:
  - `0004-core-implementation-language.md`
  - `0028-integer-millimeter-traffic-geometry.md`
- 相关设计:
  - `../design/traffic-runtime-shared-consumption.md`
  - `../design/traffic-runtime-integer-geometry.md`
  - `../design/core-runtime.md`（Retired）

## 背景

LaneFlow Core 需要被多个游戏引擎和数字孪生运行环境调用。不同引擎的 frame loop、暂停策略、时间缩放和浮点实现都可能不同。

如果 Core 直接读取 wall clock 或依赖引擎帧时间，将难以测试，也难以保证同一输入下的可重复行为。v0.1 需要先固定最小 runtime tick 语义，后续 vehicle following、signals、parking 才能建立在同一时间模型上。

## 决策

### 1. Core 不读取 wall clock

Core 不读取系统时间、真实时间或引擎生命周期时间。所有时间推进都由调用方通过 tick 输入显式传入。

### 2. Core 是 fixed-step runtime

每个交通世界在初始化时确定一个正整数 `fixed_delta_time_ms`。同一 session 运行中不得改变该值。v0.1 未冻范围；#496 / ADR 0028（Proposed，未 Pass）将合法区间冻结为 **`4..=1000` ms**。4 ms 是最细量子，不是默认；面向画面的产品默认建议 16 ms。`>= 100 ms` 合法但不保证跟车观感。不存在全球唯一 Hz。G2 完成前 `main` 仍接受任意正整数步长。

Core step 不接受任意 variable delta。若 `TickInput` 保留 `delta_time_ms` / `deltaTimeMs` 字段，该值必须等于当前 world 的 `fixed_delta_time_ms`；不一致时应返回明确的 validation error，而不是按 variable delta 推进。

Variable frame time 应在 Adapter 或上层 scheduler 侧累积，并拆分为 0 个、1 个或多个固定步进。catch-up、丢弃 backlog、快进和 render interpolation 都不属于运行时 tick 语义。交通运行时 **不提供慢放**：不得用缩小 Δt 或可变 Δt 实现墙钟变慢；Adapter 只能少调用 `step`。**当前**信号相位只需 `durationMs >= fixed_delta_time_ms`。ADR 0028 Proposed 要求每个 `durationMs` 为该世界步长的正整数倍；G2 完成前 `main` 不执行倍数检查。

### 3. v0.1 确定性范围有限

v0.1 要求同一 Core 版本、同一运行环境、同一初始状态和同一 tick/input 序列得到一致输出。

v0.1 不要求跨语言、跨 CPU、跨浮点实现的 bit-level determinism。

### 4. Core step 不依赖隐藏全局状态

Core runtime step 的语义应等价于：

```text
step(world, input) -> stepResult
```

上述形式是概念表达。现行公开入口是 `TrafficWorld::step`；`CoreWorld` 已拆除。

实现可以为性能选择内部 mutation，但不得依赖隐藏 clock、随机数或引擎全局状态。

## 后果

- Core 测试可以通过固定输入序列稳定复现。
- Adapter 必须负责把引擎 frame loop 转换为 Core fixed tick。
- Adapter 必须显式处理 catch-up 上限、drop/backlog 策略和 render interpolation。
- Core 实现必须测试 invalid delta 路径，确保同一 session 内不接受不一致的 tick delta。
- 暂停、快进和 variable frame time 属于 Adapter 或上层调度问题；慢放若发生，也只允许少 `step`，不能改世界步长。
- ADR 0028 不给已提交整数毫米状态补跨 CPU / 跨机器位级承诺。余数输入依赖 IIDM `f32` 与规范弧长 `f32`，本节对浮点位级确定性的拒绝仍然适用。同进程并行仍须与 1-worker 得到相同已提交状态与事件序。
- v0.1 可以快速建立 deterministic smoke tests，但不承担跨平台 bit-level deterministic math 成本。
- 如果后续需要跨平台 bit-level determinism 或联机 lockstep，应新增 ADR，而不是扩展本 ADR 的默认含义。
