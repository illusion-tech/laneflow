# laneflow-scenario

`laneflow-scenario` 提供可选、引擎无关的 reference scenario catalog 线格式。

当前保留 v0.10 protected-turning signalized-corridor catalog 0.2 的 TOML wire
类型。`validate` 闭合 catalog 0.2；`bind(catalog, &SharedNetworkRevision)` 用 Identity v1
把编制字符串派生为 `StableId128`，再经已安装修订的 `SharedIdentityIndex` 得到类型化序号，
并记下 `NetworkRevisionId`。热路径不查字符串。50–200 确定性回流见
[#475](https://github.com/illusion-tech/laneflow/issues/475)。

测试用 `toml 1.1.4+spec-1.1.0`（crates.io，MIT OR Apache-2.0）解析检入 catalog；不进入
Runtime 或 Adapter 生产 graph。工作区路径依赖 `laneflow-compiler` /
`laneflow-static-contract` / `laneflow-static-network` 与本仓库同为 Apache-2.0。
