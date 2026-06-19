# 0003 Runtime Tick and Determinism

**状态**: Proposed

**日期**: 2026-06-20

**适用范围**: LaneFlow Core runtime 的 tick 输入、时间推进与确定性策略

**关联文档**:

- 上游决策:
  - `0001-project-scope.md`
  - `0002-dependency-and-licensing-constraints.md`
- 相关设计:
  - `../design/core-runtime.md`

## 背景

LaneFlow Core 需要被多个游戏引擎和数字孪生运行环境调用。不同引擎的 frame loop、暂停策略、时间缩放和浮点实现都可能不同。

如果 Core 直接读取 wall clock 或依赖引擎帧时间，将难以测试，也难以保证同一输入下的可重复行为。v0.1 需要先固定最小 runtime tick 语义，后续 vehicle following、signals、parking 才能建立在同一时间模型上。

## 决策

### 1. Core 不读取 wall clock

Core 不读取系统时间、真实时间或引擎生命周期时间。所有时间推进都由调用方通过 tick 输入显式传入。

### 2. Core 使用显式固定步长语义

Core tick 输入包含正整数 `deltaTimeMs`。v0.1 推荐 Adapter 以固定步长调用 Core；variable frame time 应在 Adapter 侧累积，并拆分为一个或多个固定 tick。

### 3. v0.1 确定性范围有限

v0.1 要求同一 Core 版本、同一运行环境、同一初始状态和同一 tick/input 序列得到一致输出。

v0.1 不要求跨语言、跨 CPU、跨浮点实现的 bit-level determinism。

### 4. Core step 不依赖隐藏全局状态

Core runtime step 的语义应等价于：

```text
step(world, input) -> stepResult
```

实现可以为性能选择内部 mutation，但不得依赖隐藏 clock、随机数或引擎全局状态。

## 后果

- Core 测试可以通过固定输入序列稳定复现。
- Adapter 必须负责把引擎 frame loop 转换为 Core tick。
- 暂停、快进、慢放和 variable frame time 属于 Adapter 或上层调度问题。
- v0.1 可以快速建立 deterministic smoke tests，但不承担跨平台 bit-level deterministic math 成本。
- 如果后续需要跨平台 bit-level determinism，应新增 ADR，而不是扩展本 ADR 的默认含义。
