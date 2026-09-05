<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="assets/brand/laneflow-mark-dark.svg">
    <source media="(prefers-color-scheme: light)" srcset="assets/brand/laneflow-mark.svg">
    <img src="assets/brand/laneflow-mark.svg" width="112" alt="LaneFlow 标记">
  </picture>
</p>

<h1 align="center">LaneFlow</h1>

<p align="center">
  可嵌入、引擎无关、确定性的道路交通运行时与工具链
</p>

<p align="center">
  <code>确定性固定步进</code> · <code>共享静态路网</code> · <code>引擎适配</code>
</p>

LaneFlow 把受检静态路网转换为可共享修订，并在宿主控制下推进道路机动车交通。

> [!IMPORTANT]
> **LaneFlow 仍处于 1.0 前。** 当前工作区 crate 均为 `0.0.0` 且
> `publish = false`，仅支持从源码集成；API 与格式可能直接调整。

## 快速开始

需要 Git、[rustup](https://rustup.rs/) 和 Rust 1.98：

```powershell
git clone https://github.com/illusion-tech/laneflow.git
cd laneflow
cargo +1.98.0 test --locked -p laneflow-bevy --test runtime_min_smoke
```

这个无窗口测试会走通 LFCA、`TrafficWorld`、位姿采样与 Bevy `Transform` 同步。

<details>
<summary>运行完整 workspace 检查</summary>

```powershell
cargo +1.98.0 test --workspace --locked
```

</details>

## 当前能力

| 组件               | 已交付能力                                                    |
| ------------------ | ------------------------------------------------------------- |
| 静态路网工具链     | 编译并检查 LFCA，构建可跨世界共享的 `SharedNetworkRevision`   |
| Traffic Runtime    | 固定步进推进跟驰与信号；停车状态转换由宿主命令驱动            |
| Spatial            | 可选地将规范进度采样为引擎无关位姿                            |
| Reference Adapter  | 提供 Bevy 0.19 接入、无窗口测试与信号化走廊集成夹具           |
| Reference Scenario | 提供 catalog 0.4 与 prepare 绑定；要求显式 `policy_selection` |

宿主负责交通需求、出发计划、路线选择策略，以及引擎生命周期与最终表现。

## 架构

```text
Authoring Sources
        ↓
laneflow-compiler
        ↓
LFCA / Format Check
        ↓
SharedNetworkRevision
    ├─→ TrafficWorld ────────────┐
    └─→ SpatialSession（可选）───┴─→ Adapter / Presentation
```

- `TrafficWorld` 与 `SpatialSession` 消费同一份共享路网修订。
- 路线不写入静态制品；每个世界通过 `TrafficWorld::register_route` 注册路线。

详细职责与依赖方向见[架构说明](docs/architecture.md)和
[Traffic Runtime 共享静态路网消费契约](docs/design/traffic-runtime-shared-consumption.md)。

## 当前边界

- 当前唯一可运行特化是道路机动车。
- 当前唯一已交付 Adapter 是 Bevy 0.19 Reference Adapter。
- 原生示例是集成验证壳，不是完成的可视化演示。
- LaneFlow 不内置交通需求、路线选择或专业交通工程分析。

> 交通参与单元与交通执行域是长期抽象方向，不代表已经支持其他执行域，
> 也不构成交付时间承诺。

## 继续阅读

[文档索引](docs/README.md) ·
[架构说明](docs/architecture.md) ·
[`laneflow-bevy` 接入](crates/laneflow-bevy/README.md) ·
[贡献指南](CONTRIBUTING.md) ·
[品牌标记](assets/brand/README.md) ·
[Issue Tracker](https://github.com/illusion-tech/laneflow/issues)

## 许可证

LaneFlow 采用 [Apache License 2.0](LICENSE)。品牌名称与标记的使用边界见
[品牌标记使用说明](assets/brand/README.md)。
