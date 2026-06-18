# 0001 Project Scope

**状态**: Accepted  
**日期**: 2026-06-17  
**适用范围**: LaneFlow 项目定位、非目标和核心边界

## 背景

LaneFlow 需要在主流游戏引擎和数字孪生场景中生成可信的 NPC 车辆流动效果。

项目目标不是替代专业交通工程仿真器，也不是实现完整城市经济或出行需求模拟，而是提供一个轻量、可嵌入、引擎无关的交通 runtime。

## 决策

LaneFlow 定位为：

> Engine-Agnostic Traffic Core + Game Engine Adapter + NPC Vehicle Runtime

LaneFlow Core 负责：

- 车辆逻辑
- 车道图
- 路线
- 红绿灯
- 前车避让
- 停车系统
- 引擎无关 runtime 状态更新

Engine Adapter 负责：

- 引擎生命周期对接
- 车辆模型
- 道路表现
- 动画
- LOD
- UI
- 调试可视化
- 示例场景集成

## 非目标

LaneFlow 暂不追求：

- 城市经济模拟
- 市民出行需求模拟
- 专业交通工程仿真
- 城市级 OD 矩阵
- 自动驾驶传感器仿真
- 完整 SUMO-like 系统
- 高精度车辆动力学
- 复杂行人、公交或轨道交通系统

## 后果

- Core 设计应优先保持轻量和可嵌入。
- Adapter 不应反向污染 Core 的抽象。
- 与 SUMO、CARLA、libsumo 等系统的集成可以作为工具链或离线数据来源讨论，但不作为客户端 Core 依赖。
- GitHub Issue 和 PR 应围绕这个范围判断是否属于 LaneFlow 当前目标。

