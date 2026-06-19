# 0003 Runtime Tick and Determinism

**状态**: Accepted
**日期**: 2026-06-20
**适用范围**: LaneFlow Core runtime 的 tick 输入、时间推进与确定性策略
**关联文档**:

- 上游决策:
  - `0001-project-scope.md`
  - `0002-dependency-and-licensing-constraints.md`
- 相关 ADR:
  - `0004-core-implementation-language.md`
- 相关设计:
  - `../design/core-runtime.md`

## 背景

LaneFlow Core 需要被多个游戏引擎和数字孪生运行环境调用。不同引擎的 frame loop、暂停策略、时间缩放和浮点实现都可能不同。

如果 Core 直接读取 wall clock 或依赖引擎帧时间，将难以测试，也难以保证同一输入下的可重复行为。v0.1 需要先固定最小 runtime tick 语义，后续 vehicle following、signals、parking 才能建立在同一时间模型上。

## 决策

### 1. Core 不读取 wall clock

Core 不读取系统时间、真实时间或引擎生命周期时间。所有时间推进都由调用方通过 tick 输入显式传入。

### 2. Core 是 fixed-step runtime

每个 Core world / simulation session 在初始化时确定一个正整数 `fixed_delta_time_ms`。v0.1 不冻结全局唯一 tick 数值；不同 session 可以选择不同固定步长，但同一 session 运行中不得改变该值。

Core step 不接受任意 variable delta。若 `TickInput` 保留 `delta_time_ms` / `deltaTimeMs` 字段，该值必须等于当前 world 的 `fixed_delta_time_ms`；不一致时应返回明确的 validation error，而不是按 variable delta 推进。

Variable frame time 应在 Adapter 或上层 scheduler 侧累积，并拆分为 0 个、1 个或多个 fixed tick 调用 Core。catch-up、丢弃 backlog、慢放、快进和 render interpolation 都不属于 Core runtime tick 语义。

### 3. v0.1 确定性范围有限

v0.1 要求同一 Core 版本、同一运行环境、同一初始状态和同一 tick/input 序列得到一致输出。

v0.1 不要求跨语言、跨 CPU、跨浮点实现的 bit-level determinism。

### 4. Core step 不依赖隐藏全局状态

Core runtime step 的语义应等价于：

```text
step(world, input) -> stepResult
```

上述形式是概念表达。Rust public API 可以通过 `&mut CoreWorld` 写回状态，具体形态由 `0004-core-implementation-language.md` 和 `../design/core-runtime.md` 约束。

实现可以为性能选择内部 mutation，但不得依赖隐藏 clock、随机数或引擎全局状态。

## 后果

- Core 测试可以通过固定输入序列稳定复现。
- Adapter 必须负责把引擎 frame loop 转换为 Core fixed tick。
- Adapter 必须显式处理 catch-up 上限、drop/backlog 策略和 render interpolation。
- Core 实现必须测试 invalid delta 路径，确保同一 session 内不接受不一致的 tick delta。
- 暂停、快进、慢放和 variable frame time 属于 Adapter 或上层调度问题。
- v0.1 可以快速建立 deterministic smoke tests，但不承担跨平台 bit-level deterministic math 成本。
- 如果后续需要跨平台 bit-level determinism，应新增 ADR，而不是扩展本 ADR 的默认含义。
