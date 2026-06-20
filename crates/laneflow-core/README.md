# laneflow_core

引擎无关的 LaneFlow Core runtime crate。

本 crate 由 issue #9 初始化，作为 v0.1 Core 的实现边界。当前仅包含 Cargo package、模块骨架和测试入口；runtime 行为由后续 v0.1 子 issue 增量实现。

当前工具链策略：

- Rust edition: 2024
- MSRV: 1.96
- nightly-only 能力：不允许
