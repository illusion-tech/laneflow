<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="assets/brand/laneflow-mark-dark.svg">
    <img src="assets/brand/laneflow-mark.svg" width="144" alt="LaneFlow 标记">
  </picture>
</p>

<h1 align="center">LaneFlow</h1>

<p align="center">
  可嵌入、引擎无关、确定性的道路交通运行时与工具链
</p>

LaneFlow 将静态路网编译、交通运行时、空间位姿采样和引擎表现分离。宿主负责交通
需求、出发计划和路线选择策略；LaneFlow 负责验证静态路网与候选路线，并按固定步进
推进当前道路机动车交通，适合嵌入游戏引擎或实时仿真宿主。

> **开发状态**
>
> LaneFlow 尚未发布 1.0。当前工作区中的包（crate）均为 `0.0.0` 且 `publish = false`，
> 仅支持从源码集成；API 和格式仍可能直接调整，不承诺为开发中旧路径保留兼容层。

## 当前能力

- 从受检编制来源确定性编译可移植规范制品（Portable Canonical Artifact，LFCA），
  经格式检查后构建进程内不可变、可由多个世界共享的 `SharedNetworkRevision`。
- 通过 `laneflow-runtime::TrafficWorld` 安装共享路网修订，在每个世界中注册路线，
  以确定性固定步进执行车辆跟驰、信号约束和停车生命周期。
- 提供运行时快照、路网修订切换、已提交交通观测和宿主路径规划（Routing）接入边界；
  交通需求、动态成本与路线选择算法仍由宿主拥有。
- 通过可选的 `laneflow-spatial::SpatialSession` 将规范进度采样为引擎无关位姿。
- 提供 Bevy 0.19 参考适配器（Reference Adapter），以及无窗口端到端冒烟测试和
  信号化走廊集成夹具。
- 提供 reference scenario catalog 0.4 与 prepare 绑定；catalog 必须显式声明
  `policy_selection`。

### 当前限制

- 可运行实现目前只覆盖道路机动车执行域，不表示已经支持非机动车、行人或轨道交通。
- 当前只交付 Bevy Reference Adapter；其他引擎尚无可用引擎适配器（Adapter）。
- 原生示例是用于验证 Runtime、Spatial 与 Adapter 接线的集成壳，不是完成的可视化演示。
- LaneFlow 不内置交通需求生成或路线选择策略，也不是专业交通工程分析、自动驾驶
  传感器仿真、高精度车辆动力学或完整 SUMO-like 系统。

### 长期方向（非当前能力）

交通参与单元（Traffic Participant Unit）与交通执行域（Traffic Execution Domain）是
已接受的长期抽象方向；当前道路机动车仍是唯一可运行特化，其他执行域是否和何时
交付没有承诺。长期设计见[架构说明](docs/architecture.md)。

## 架构

```text
Authoring Sources
  -> laneflow-compiler
  -> LFCA / laneflow-format checks
  -> SharedNetworkRevision
       |-> laneflow-runtime -> TrafficWorld -----------------|
       `-> laneflow-spatial -> SpatialSession (optional) ----|-> Engine Adapter / Presentation
```

`TrafficWorld` 与可选的 `SpatialSession` 消费同一份共享路网修订，Adapter 只负责宿主
生命周期、变换和表现。路线不属于静态路网制品；宿主通过
`TrafficWorld::register_route` 将候选边序列注册到具体世界。

完整职责与依赖方向见[架构说明](docs/architecture.md)和
[Traffic Runtime 共享静态路网消费契约](docs/design/traffic-runtime-shared-consumption.md)。

## Quick Start

需要 Git、[rustup](https://rustup.rs/) 和 Rust 1.98。下面的无窗口 smoke test 会读取受检
LFCA、构建 `SharedNetworkRevision`、安装 `TrafficWorld`、执行固定步进、采样位姿，
并确认 Bevy `Transform` 发生变化：

```powershell
git clone https://github.com/illusion-tech/laneflow.git
cd laneflow
cargo +1.98.0 test --locked -p laneflow-bevy --test runtime_min_smoke
```

运行完整 workspace 检查：

```powershell
cargo +1.98.0 test --workspace --locked
```

Bevy 接入代码和另一个走廊 smoke 见
[`laneflow-bevy` 使用说明](crates/laneflow-bevy/README.md)。

## 文档与贡献

- [文档索引](docs/README.md)
- [架构说明](docs/architecture.md)
- [设计文档索引](docs/design/README.md)
- [贡献指南](CONTRIBUTING.md)
- [品牌标记使用说明](assets/brand/README.md)
- [Issue Tracker](https://github.com/illusion-tech/laneflow/issues)

## 许可证

LaneFlow 采用 [Apache License 2.0](LICENSE)。仓库自有代码、文档和品牌 SVG 在没有
另行声明时按 Apache-2.0-only 分发；品牌名称和标记的商标使用边界见
[品牌标记使用说明](assets/brand/README.md)。
