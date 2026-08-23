# LaneFlow

LaneFlow 当前是一个面向主流游戏引擎、可嵌入的轻量 NPC 交通运行时，用于在园区、
厂区、校园、景区、停车场、道路片区和数字孪生等局部道路场景中生成可信的车辆流动
效果。#291 G1 已接受编译器拥有的静态路网与交通运行时长期设计；Accepted ADR 0025 /
#300 G1 已进一步冻结以受检 LFCA 构建进程内共享静态路网、且不交付独立静态镜像文件/
ABI 的修订。共享静态路网与 `TrafficWorld` 已由 #300 / #301 交付为当前可运行路径。
`laneflow-runtime` 是唯一可运行交通世界；current JSON 不再安装可运行入口。
详见 `docs/design/traffic-runtime-shared-consumption.md`。

Accepted ADR 0021 把“为未来的中国特色城市模拟游戏提供交通基础”定义为 LaneFlow
的第一长期产品目标。该目标不是完整交通工程仿真器，也不拥有城市经济、市民出行
需求或游戏规则，而是让这些上层系统通过显式命令、快照、事件和路径规划边界驱动
一个引擎无关的交通运行时。当前 `TrafficWorld` 只实现道路机动车车辆特化，负责
车辆逻辑、车道图、路线、红绿灯、前车避让和停车占用；已接受的目标态交通运行时
（Target Traffic Runtime）以交通参与单元（Traffic Participant Unit）和交通执行域
（Traffic Execution Domain）承载长期多模式扩展，当前车辆能力不排除未来的非机动车、
行人或轨道交通。不同游戏引擎通过引擎适配器（Engine Adapter）接入，并负责已实现
执行域的模型、道路/设施表现、动画、细节层次（LOD）、UI 和调试可视化。

## 项目定位

LaneFlow 的当前能力与 #291 已接受长期目标共同关注：

- 长期目标：面向中国特色城市模拟游戏，提供可嵌入、确定性、可扩展的交通基础；
- 支持局部路网中的 NPC 车辆流；
- 支持园区、厂区、校园、景区、停车场等中小型场景；
- 支持车辆生成、路线行驶、前车避让、红绿灯、路口规则和停车；
- 核心逻辑不绑定具体游戏引擎；
- 支持对接主流商业游戏引擎和开源游戏引擎；
- 可用于桌面端、Web、移动端和数字孪生项目；
- 商用可控，不把 SUMO / CARLA / libsumo 作为客户端核心依赖。

#291 已接受目标态的一句话概括（对应实现尚未交付）：

> LaneFlow = Static Network Compiler + Engine-Agnostic Traffic Runtime + Game Engine Adapter

## 适用场景

- 长期目标：中国特色城市模拟游戏中的交通基础
- 园区内部道路车流
- 厂区物流车 / 巡检车 / 服务车
- 校园车辆和摆渡车
- 景区观光车
- 停车场进出车辆
- 数字孪生局部道路展示
- 游戏场景中的背景 NPC 车流
- Web / 移动端轻量交通展示

## 非目标

下列能力不由 LaneFlow 交通基础自身拥有；未来城市模拟游戏、出行编排服务或独立专业产品可以在 LaneFlow 之上实现：

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
│     LaneFlow Traffic Runtime        │
│ TrafficWorld / route / signal / park│
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

- `crates/laneflow-bevy`：Bevy 0.19 Reference Adapter；使用最小 modular dependency graph，提供单活动 `LaneFlowSession`、专用 fixed schedule，以及 `TrafficWorld` + 可选 `SpatialSession` 的最小示例。campus / 走廊 Core 入口已拆除。
- `crates/laneflow-runtime`：引擎无关的交通运行时。`TrafficWorld` 安装 `Arc<SharedNetworkRevision>`，拥有 1-worker 固定步进、动态 Route、车辆、停车占用与信号 snapshot。
- `crates/laneflow-scenario`：可选、引擎无关的 reference scenario catalog 线格式；走廊人口迁到 Runtime 见 [#472](https://github.com/illusion-tech/laneflow/issues/472)。
- `crates/laneflow-spatial`：LaneFlow 自有的有界 `f32` canonical 点、向量、单位方向、稳定 frame ID，以及绑定共享根 `Arc` 的 `SpatialSession` 位姿采样；不依赖 Runtime。
- `tools/laneflow-corridor-generator`：受保护转向走廊的离线生成工具；读取内部 TOML，确定性写出 scenario-local catalog 0.2 TOML。内存里仍可构造遗留 Traffic/Spatial/Manifest JSON，但不落盘、没有生产 schema 或加载 crate；走廊人口迁到 Runtime 见 [#472](https://github.com/illusion-tech/laneflow/issues/472)。
- `research/issue-123-spatial-prototype`：#123 G1 使用的研究用工作区成员；不属于生产接口，第三方几何候选只作为开发依赖进行对照。
- `xtask`：Markdown 表格格式化、提交消息和 External Review Check 等仓库治理工具。

可运行交通世界只从共享静态路网修订安装，不从 current JSON 创建。详细边界见 `docs/design/traffic-runtime-shared-consumption.md`。

## 许可证

LaneFlow 公开仓库采用 [Apache License 2.0](LICENSE)。`laneflow-runtime` 与本仓库其他自有内容按 Apache-2.0-only 分发；第三方材料仍遵循其各自许可证。

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
- `docs/adr/0021-city-simulation-game-traffic-foundation.md`
- `docs/governance/documentation-policy.md`
- `docs/governance/github-workflow.md`
- `docs/governance/development-gates.md`
- `docs/governance/agent-development-guide.md`
- `CONTRIBUTING.md`
