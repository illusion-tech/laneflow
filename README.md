# LaneFlow

LaneFlow 是一个面向主流游戏引擎的轻量 NPC 车流运行时系统，用于在园区、厂区、校园、景区、停车场、道路片区和数字孪生场景中生成可信的车辆流动效果。

它不是完整交通工程仿真器，也不是城市经济模拟系统，而是一个引擎无关的交通 runtime。LaneFlow Core 负责车辆逻辑、车道图、路线、红绿灯、前车避让和停车系统；不同游戏引擎通过 Engine Adapter 接入，并负责车辆模型、道路表现、动画、LOD、UI 和调试可视化。

## 项目定位

LaneFlow 的核心目标是：

- 支持局部路网中的 NPC 车辆流；
- 支持园区、厂区、校园、景区、停车场等中小型场景；
- 支持车辆生成、路线行驶、前车避让、红绿灯、路口规则和停车；
- 核心逻辑不绑定具体游戏引擎；
- 支持对接主流商业游戏引擎和开源游戏引擎；
- 可用于桌面端、Web、移动端和数字孪生项目；
- 商用可控，不把 SUMO / CARLA / libsumo 作为客户端核心依赖。

一句话概括：

> LaneFlow = Engine-Agnostic Traffic Core + Game Engine Adapter + NPC Vehicle Runtime

## 适用场景

- 园区内部道路车流
- 厂区物流车 / 巡检车 / 服务车
- 校园车辆和摆渡车
- 景区观光车
- 停车场进出车辆
- 数字孪生局部道路展示
- 游戏场景中的背景 NPC 车流
- Web / 移动端轻量交通展示

## 非目标

LaneFlow 暂不追求以下能力：

- 城市经济模拟
- 市民出行需求模拟
- 专业交通工程仿真
- 城市级 OD 矩阵
- 自动驾驶传感器仿真
- 完整 SUMO-like 系统
- 高精度车辆动力学
- 复杂行人 / 公交 / 轨道交通系统

## 核心架构

```text
┌─────────────────────────────────────┐
│           Authoring Layer           │
│   道路编辑、路线编辑、停车位配置    │
└──────────────────┬──────────────────┘
                   ↓
┌─────────────────────────────────────┐
│          Traffic Data Layer         │
│ lane graph / route / signal / park  │
└──────────────────┬──────────────────┘
                   ↓
┌─────────────────────────────────────┐
│          LaneFlow Core              │
│ vehicle / route / signal / parking  │
│ engine-agnostic runtime             │
└──────────────────┬──────────────────┘
                   ↓
┌─────────────────────────────────────┐
│          Engine Adapter Layer       │
│ Unreal / Unity / Godot / O3DE / Web │
└──────────────────┬──────────────────┘
                   ↓
┌─────────────────────────────────────┐
│          Presentation Layer         │
│ mesh / actor / entity / LOD / debug │
└─────────────────────────────────────┘
```

## Rust workspace

- `crates/laneflow-bevy`：Bevy 0.19 Reference Adapter；使用最小 modular dependency graph，提供单活动 `LaneFlowSession`、专用 fixed schedule、Vehicle/Entity 部分双射、frame placement、原子 local Transform 同步、可选 Gizmos 与 campus native reference example。
- `crates/laneflow-core`：引擎无关的 Core domain/runtime、typed handles、fixed tick、fixed-time Signals snapshot/query/events、SignalStop 与 permission-aware traversal，以及私有 occupancy/leader、IIDM、safe-speed 与 no-overlap projection pipeline。
- `crates/laneflow-data`：当前 Traffic v0.7 JSON loader、严格版本闸口、per-edge `speedLimit` 与 Core normalization；依赖方向固定为 `laneflow-data -> laneflow-core`。
- `crates/laneflow-spatial`：LaneFlow 自有的有界 `f32` canonical 点、向量、单位方向、稳定 frame ID、immutable edge-binding registry，以及带 placement token、Parking pose 和失败原子性的批量位姿提取；依赖方向固定为 `laneflow-spatial -> laneflow-core`，Core 不反向依赖 Spatial。
- `tools/laneflow-corridor-generator`：v0.8 直行信号化走廊的离线 authoring 工具；读取内部 TOML，确定性生成并校验 Traffic/Spatial/Manifest JSON 与 scenario-local catalog TOML。
- `research/issue-123-spatial-prototype`：#123 G1 使用的研究用工作区成员；不属于生产接口，第三方几何候选只作为开发依赖进行对照。
- `xtask`：Markdown 表格格式化、提交消息和 Gate evidence 等仓库治理工具。

Data crate 只接收调用方提供的内存 bytes/string，不读取引擎路径或直接创建 `CoreWorld`。详细边界见 `docs/design/data-loading.md`、ADR 0007 与 ADR 0008。

## 许可证

LaneFlow 公开仓库采用 [Apache License 2.0](LICENSE)。`laneflow-core`、`laneflow-data` 与本仓库其他自有内容按 Apache-2.0-only 分发；第三方材料仍遵循其各自许可证。

未来高级编辑器、城市级或分布式仿真、优化分析、企业 Adapter、云服务与商业支持可以在独立产品和独立许可证下交付。商业产品可以依赖开放 Core/Data，开放仓库不得依赖商业实现。详细边界与依赖审计规则见 `docs/adr/0002-dependency-and-licensing-constraints.md` 和 `docs/governance/dependency-security.md`。

## 项目治理

LaneFlow 采用 GitHub-first 治理：

- GitHub Issue 负责当前任务、需求、验收标准和依赖。
- GitHub Pull Request 负责合并证据、测试记录和风险说明。
- GitHub Project 负责当前进度、优先级和版本看板。
- 仓库文档负责长期设计事实、架构决策、治理规范和 AI Agent 开发规则。

推荐阅读：

- `docs/README.md`
- `docs/architecture.md`
- `docs/roadmap.md`
- `docs/governance/documentation-policy.md`
- `docs/governance/github-workflow.md`
- `docs/governance/development-gates.md`
- `docs/governance/agent-development-guide.md`
- `CONTRIBUTING.md`
