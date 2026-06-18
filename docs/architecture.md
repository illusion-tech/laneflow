# 架构

**文档状态**: Draft  
**最后更新**: 2026-06-17  
**适用范围**: LaneFlow 初始架构说明

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
  ↓
Traffic Data Layer
  ↓
LaneFlow Core
  ↓
Engine Adapter Layer
  ↓
Presentation Layer
```

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

## 7. Presentation Layer

Presentation Layer 负责用户可见效果：

- 车辆模型
- 道路表现
- 动画
- 灯光
- 调试可视化
- 示例场景 UI

Presentation 可以因引擎不同而完全不同。

