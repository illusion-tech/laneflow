# laneflow-scenario

可选的、引擎无关的参考场景 catalog。它不是交通运行时的加载器，人口政策也不进入 `step`。

## 职责与边界

当前保留信号走廊 catalog 0.4 的 TOML 类型，以及调用方持有的 `50..=200` 辆车回流政策。`validate` 闭合 catalog。`bind(catalog, &SharedNetworkRevision)` 用 Identity v1 把编制字符串变成这条修订上的序号，并记下 `NetworkRevisionId` 与 `WorldPolicySelection`。

`CorridorPopulationPrepare` / `CorridorPopulationController` 在世界外面做抽样、阻塞重试和原子替换调度。宿主决定要不要使用它们。无窗口示例只使用 `bind`，然后自己 `register_route` 和 `spawn_vehicle`，不启动控制器。

## 最小使用入口

检入资产是 `examples/data/v0.2-signalized-corridor.catalog.toml` 与同名 LFCA。文件名里的 `v0.2` 是场景制品名，不是 catalog 版本，也不是 LFCA 版本。catalog 版本是 0.4。

无窗口程序的完整顺序见[从源码接入无窗口交通运行时][source-integration]。持续车流再使用人口控制器，见下方设计文档。

## 主要约束与限制

- `policy_selection` 是 catalog 顶层必填项。`pinned` 只带策略稳定标识，`not_required` 不带其他字段。
- 整网存在门、冲突区或参与者流时，`not_required` 在 `bind` 阶段被拒绝。走廊必须显式 Pinned。
- 热路径不再查找编制字符串。槽位和路线按 portal 的 spawn slot 对齐，不按边长挑选。
- 控制器校验修订与策略。拒绝时不修改待替换队列。它不证明 Bevy 的 catch-up 调度。
- 测试用 `toml` 只解析检入 catalog，不进入交通运行时的生产依赖。

## 示例与验证

一辆车、一条路线的无窗口路径：

```powershell
cargo +1.99.1 run --locked -p laneflow-runtime --example headless
cargo +1.99.1 test --locked -p laneflow-runtime --test headless_smoke
```

人口控制器的短容量测试随 `cargo test -p laneflow-scenario` 运行。完整一万次成功替换是忽略测试，命令见[人口与回流设计][signalized-corridor-population]。

## 深入阅读

- [从源码接入无窗口交通运行时][source-integration]
- [人口与回流设计][signalized-corridor-population]
- [`laneflow-runtime`][laneflow-runtime-readme]
- [当前能力与限制][capabilities]

[source-integration]: ../../docs/guides/source-integration.md
[signalized-corridor-population]: ../../docs/design/signalized-corridor-population.md
[laneflow-runtime-readme]: ../../crates/laneflow-runtime/README.md
[capabilities]: ../../docs/reference/capabilities.md
