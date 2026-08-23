# 设计文档

本目录回答“系统具体怎么做”。`docs/adr/` 回答“为什么这样定”。GitHub 记录当前
任务与评审；这里只保存仍约束实现的长期设计。已关闭切片的收口流水账、验证轮次和
截图不进本目录，见 git 历史。

## 当前可运行路径

- `traffic-runtime-shared-consumption.md`：`TrafficWorld` 安装共享静态路网，
  Spatial 只 bind 同一根 `Arc`，current Core/JSON 入口已拆除。
- `shared-static-network.md`：从受检 LFCA 构建 `SharedNetworkRevision`。
- `adapter-api.md`：Runtime / Spatial 与引擎适配器的只读快照、位姿和权威边界。
- `portable-canonical-artifact.md`：LFCA / LFSM / LFSD / LFCP 格式与发布对象。
- `compiler-post-emission-check-and-minimal-publication-closure.md`：后发射检查与
  LFCP v2 最小发布闭合。
- `compiler-foundation.md`：编译器 crate、Typed AST → HIR → MIR → Canonical LIR、
  合成 DSL 与官方前端接入。
- `network-compiler.md`：#291 目标静态编译架构。
- `road-editing-source-and-geometry-frontend.md`：道路编辑 FlatBuffers B1 生产入口。

## 领域规则

这些文档约束 Runtime 仍实现的道路机动车行为，不表示 JSON 或 `CoreWorld` 仍存在。

- `core-id-handles.md`：external ID、typed handle、lifecycle。
- `lane-graph.md`：车道图与连接。
- `route-system.md`：路线定义与跟随。
- `vehicle-following.md`：跟车、安全距离与投影。
- `signal-system.md`：固定时制信号、门控与车辆合规。
- `parking-system.md`：停车占用、生命周期与 ParkingStop。
- `road-junction-model.md`：Junction / Movement / ManeuverPath / ManeuverGate。
- `waiting-zone-conflict-right-of-way.md`：待行区、冲突与通行权（Accepted 设计；
  运行时生产化按独立 Issue）。
- `cross-section-access.md`：横断面与准入 overlay。
- `numeric-representation.md`：数值权威、误差预算与 current `f64` 生产裁决。
- `spatial-geometry.md`：有界 canonical `f32` 几何与位姿。

## 场景、规模与 Adapter

- `example-scenarios.md`、`signalized-corridor-protected-turning.md`、
  `signalized-corridor-population.md`：信号化走廊几何、转向 profile 与人口策略。
- `bevy-reference-adapter.md`：Bevy 0.19 Reference Adapter。
- `core-runtime-performance-baseline.md`：一万 / 十万产品目标与一百万研究包络。
- `real-road-workloads.md`：LuST 真实路网契约。
- `lust-bevy-population-control.md`：LuST / Bevy 示例层人口调节。
- `core-runtime-scalability-audit.md`：城市级可扩展性前置约束；不实现生产分区。

## 已退役（只保留结论）

- `core-runtime.md`：v0.1 `CoreWorld` 基线；现行路径见
  `traffic-runtime-shared-consumption.md`。
- `data-format.md` / `data-loading.md` / `current-package-import.md`：current JSON
  与加载 crate 已删除。
- `compiler-budget-calibration.md`：#308 一次性研究已关闭，证据在 git 历史。

## 文档状态

- `Draft`：不能直接作为稳定实现输入。
- `Review`：可审阅，仍可能调整。
- `Accepted`：当前阶段实现输入。
- `Active`：持续维护的治理或索引。
- `Retired`：结论仍有效，正文已收缩；细节见 git 历史。

正式设计文档页头建议包含状态、最后更新、适用范围和关联文档。涉及 Runtime API、
数据格式或 Adapter 协议的实现应先有相关 design 或 ADR。设计文档不记录单次 PR
的测试结果。
