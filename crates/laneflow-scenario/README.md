# laneflow-scenario

`laneflow-scenario` 提供可选、引擎无关的 reference scenario catalog 线格式。

当前只保留 v0.10 protected-turning signalized-corridor catalog 0.2 的 TOML wire
类型。人口 controller、Core batch 与 `CoreWorld` 入口已拆除；迁到 Runtime 是
follow-up。

详细历史契约见 `docs/design/signalized-corridor-population.md` 与 ADR 0016。
