# laneflow-scenario

`laneflow-scenario` 提供可选、引擎无关的 reference scenario catalog 线格式。

当前保留 v0.10 protected-turning signalized-corridor catalog 0.2 的 TOML wire
类型，以及 prepare 阶段把 `route_id` / `edge_id` 字符串绑到共享路网修订类型化序号的入口。
热路径不查字符串。50–200 确定性回流见 [#475](https://github.com/illusion-tech/laneflow/issues/475)。
