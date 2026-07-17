# 架构

**文档状态**: Accepted  
**最后更新**: 2026-07-16  
**适用范围**: LaneFlow 分层、Rust crate 依赖方向、Traffic Data、Signals、planned Parking 与 Core/Adapter 边界

## 1. 架构目标

LaneFlow 是一个引擎无关的轻量 NPC 车流 runtime。

核心架构目标：

- Core 与具体游戏引擎解耦。
- 数据格式可以被工具、示例和多个 Adapter 共享。
- Adapter 负责引擎集成和表现层，不复制 Core 交通规则。
- 示例场景用于验证最小可用闭环。

## 2. 分层

```text
Authoring Layer
  │
  v
Traffic Data Layer (`laneflow-data`)
  │
  v
LaneFlow Core (`laneflow-core`)
  │
  v
Engine Adapter Layer
  │
  v
Presentation Layer
```

当前 Rust crate 依赖方向固定为：

```text
laneflow-data -> laneflow-core
laneflow-core -X-> laneflow-data

Engine Adapter -> laneflow-core
Engine Adapter -> laneflow-data  (按需加载外部数据)
```

外部格式可以依赖 Core domain types 做 normalization；Core 不反向依赖 JSON、Serde、JSON Schema、文件系统或 Adapter。详细决策见 `adr/0007-traffic-data-crate-and-loader-boundary.md`。

## 3. Authoring Layer

Authoring Layer 负责生成或编辑交通数据：

- 道路编辑
- 车道编辑
- 路线编辑
- 红绿灯配置
- 停车位配置
- 示例数据生成

它可以是独立工具、引擎编辑器插件或离线转换脚本。

## 4. Traffic Data Layer

Traffic Data Layer 保存 Core 可消费的数据：

- lane graph
- route
- signal
- parking
- spawn rules
- vehicle profiles

数据格式应尽量保持引擎无关。

当前 Rust workspace 中，Traffic Data Layer 已由 `laneflow-data` 表达。它负责：

- 当前 v0.5 external package、必填版本闸口与旧版/未来版拒绝；
- JSON syntax、wire shape、units 和字段路径诊断；
- external ID 到 Core domain input 的转换；
- 调用 Core constructors 完成 lane graph、route、Vehicle Profile、static Signals 与 static Parking normalization。

`laneflow-data` 不拥有 fixed tick、runtime entity、world lifecycle 或 Engine asset I/O。初始 loader 接收内存 bytes/string，不直接读取文件或创建 `CoreWorld`。

current v0.5 在保持相同依赖方向的前提下包含 StopLine、MovementGate、SignalGroup、fixed-time Controller/Phase，以及 immutable ParkingArea/ParkingSpace、entry/exit anchors 和 edge-relative geometry，并由两个 canonical fixtures 锁定。详细契约见 `design/data-format.md` 与 `design/data-loading.md`。

Traffic Data 只承载 immutable ParkingArea/ParkingSpace、entry/exit anchors 与 edge-relative geometry，不持久化 reservation、occupancy、initial parked vehicles 或 runtime handles。#107 已原子切换 schema、private DTO、loader、fixtures 与 current docs；runtime Parking 继续由 #108/#109 实现。

## 5. LaneFlow Core

LaneFlow Core 负责运行时交通逻辑：

- vehicle state
- route following
- lane graph traversal
- vehicle following
- signal compliance
- intersection rules
- parking behavior

Core 不依赖具体游戏引擎 API。

Rust workspace 中，Core 由 `laneflow-core` 表达。Core 拥有 `InitialTrafficData`、lane graph、route、Vehicle Profile、typed handle、registry/resolver 和全部 domain/runtime invariant。

`InitialTrafficData` 只表示可用于初始化 world 的已验证静态输入，当前包含 lane graph、routes、Vehicle Profiles 与 immutable Signals/Parking registries，不拥有 tick、initial vehicles 或 runtime route generation。初始 route validation 与 runtime route registration 复用同一 Core 规则，包括 route-final-StopLine 约束。

v0.4 Signals 在 Core 内保持四层职责：Controller 产生 indication；MovementGate/StopLine 表达空间准入；compliance policy 解释 signal-layer permission；纵向 constraint、安全投影与 permission-aware traversal 保证结果不可绕过。#94-#97 已交付 static registry/current data、absolute-time fixed-time snapshot、只读 query/events、restrictive yellow/red SignalStop、hard projection、permission-aware route-occurrence traversal，以及 10k/100k matched validation。SignalController 不硬编码国家/转向规则，Adapter 只 query/render。长期分层见 ADR 0009、`design/signal-system.md` 与 `reference/v0.4-closure-review.md`。

Planned v0.5 Parking 由 Core 私有 binding aggregate 持有唯一 runtime authority；`VehicleStatus::Parked` 与 exact Occupied binding 一致，Parked vehicle 保留 live identity但不进入 travel-lane occupancy。ParkingStop、SignalStop、RouteEnd 与 leader/no-overlap 使用共同 fixed-tick constraint/traversal pipeline；Adapter 只消费 immutable registry、snapshot、records/events 和 position authority。详细设计见 ADR 0010 与 `design/parking-system.md`；该段不表示 production runtime 已实现 Parking。

## 6. Engine Adapter Layer

Engine Adapter 负责把 Core 状态映射到具体引擎：

- tick 调用
- actor / entity 生命周期
- transform 同步
- mesh / prefab / scene object 绑定
- debug draw
- UI 面板
- LOD 和性能策略

Adapter 不应把引擎依赖引入 Core。

Adapter 可以按需调用 `laneflow-data` 解析自身 asset pipeline 已读取的内存数据，但不得要求 Core 理解引擎路径、asset handle 或异步加载协议。

## 7. Presentation Layer

Presentation Layer 负责用户可见效果：

- 车辆模型
- 道路表现
- 动画
- 灯光
- 调试可视化
- 示例场景 UI

Presentation 可以因引擎不同而完全不同。
