# laneflow-scenario

`laneflow-scenario` 提供可选、引擎无关的 reference scenario catalog 线格式。

当前保留 v0.10 protected-turning signalized-corridor catalog 0.4 的 TOML wire
类型，以及 caller-owned `50..=200` 人口/回流 policy。`validate` 闭合 catalog 0.4；
`bind(catalog, &SharedNetworkRevision)` 用 Identity v1 把编制字符串派生为
`StableId128`，再经已安装修订的 `SharedIdentityIndex` 得到类型化序号，并记下
`NetworkRevisionId`。热路径不查字符串。`CorridorPopulationPrepare` /
`CorridorPopulationController` 在 `TrafficWorld` 上做 Fisher–Yates、portal/lane
抽样、blocked retry 与原子替换调度；不把政策写入 `step`。

测试用 `toml 1.1.4+spec-1.1.0`（crates.io，MIT OR Apache-2.0）解析检入 catalog。
工作区路径依赖 `laneflow-compiler` / `laneflow-runtime` /
`laneflow-static-contract` / `laneflow-static-network` 与本仓库同为 Apache-2.0。

默认 `cargo test` 覆盖 50/200 车短容量 soak，以及独立 world 上同一 per-tick 回流链的确定性对拍（不声称测了 Bevy catch-up 调度）。完整 10,000 次成功 `Replaced`：

```powershell
cargo +1.98.0 test --offline -p laneflow-scenario --test signalized_corridor_population soak_50_cars_10000_replacements -- --ignored --exact
```

`policy_selection` 是 catalog 顶层必填的闭合选择。`pinned` 仅携带规范 policy StableId
文本，`not_required` 不带其他字段。绑定阶段拒绝未知身份以及带 Gate 根上的
`not_required`。生成器明确编制 `protected-entry`，工程法域 `engineering`、
版本 `protected-entry-1`、依据 `repository:corridor/protected-entry-1`；
示例声明只表示受保护信号组准入，不宣称覆盖现实道路法规全集。
调用方把 bind 结果的 `policy_selection` 传入唯一世界安装入口。
