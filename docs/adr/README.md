# 架构决策记录

本目录用于记录 LaneFlow 的架构决策记录。

ADR 关注“为什么这样定”，不替代详细设计文档。涉及高影响、难回退、会影响多个模块或长期兼容性的决策，应优先进入 ADR。

## 适用范围

优先写 ADR 的议题：

- LaneFlow Core 与 Engine Adapter 的边界。
- Core 不依赖具体游戏引擎或外部重型交通仿真器。
- Runtime tick、确定性、时间步长策略。
- lane graph、route、signal、parking 等核心数据模型。
- 数据格式版本策略。
- Adapter API 稳定性策略。
- 破坏性变更和兼容性策略。

不适合写 ADR 的内容：

- 普通字段补充。
- 单个示例项目配置。
- 一次 PR 的测试结果。
- 尚未形成结论的开放讨论。

## 当前 ADR 列表

- `0001`: 项目定位与范围边界
- `0002`: 依赖与许可证约束（不依赖 SUMO / CARLA / libsumo）
- `0003`: Runtime tick 与确定性策略
- `0004`: Core 实现语言（Rust）
- `0005`: Core identity、handle 与 lifecycle 模型
- `0006`: Vehicle Following 控制、安全与扩展边界
- `0007`: Traffic Data crate、loader 与 Core normalization 边界
- `0008`: 1.0 前单一当前数据格式与迁移兼容策略
- `0009`: Signal indication、MovementGate/StopLine、法规策略与 Core safety 分层
- `0010`: Parking binding、vehicle lifecycle/position authority 与 Core/Adapter 分层

## 命名规则

文件命名使用：

```text
NNNN-short-title.md
```

示例：

- `0001-project-scope.md`
- `0002-dependency-and-licensing-constraints.md`
- `0003-runtime-tick-and-determinism.md`
- `0004-core-implementation-language.md`

## 状态

ADR 状态建议使用：

- `Proposed`
- `Accepted`
- `Deprecated`
- `Superseded`

若决策被替代，应新增后续 ADR，不要静默改写历史决策。
