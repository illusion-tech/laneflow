# 设计文档

本目录用于保存 LaneFlow 的具体设计文档，重点回答“系统具体怎么做”。

`docs/adr/` 记录高影响决策原因；`docs/design/` 记录 Core、数据格式、Adapter 和运行时系统的可执行设计。

## 推荐设计文档

初始阶段建议逐步补齐：

- `core-runtime.md`：Core runtime、tick、vehicle state 和系统边界。
- `core-id-handles.md`：Core external ID、typed handle、registry / resolver、动态 lifecycle 和事件 payload 边界。
- `lane-graph.md`：车道图、连接关系、拓扑约束。
- `route-system.md`：路线选择、路径跟随、目标点。
- `vehicle-following.md`：前车避让、速度控制和安全距离。
- `signal-system.md`：红绿灯、路口规则和信号相位。
- `parking-system.md`：停车位、进出停车、占用状态。
- `adapter-api.md`：Core 与 Engine Adapter 的接口边界。
- `data-format.md`：lane graph、route 等外部数据格式、validation 和 loader 边界；Rust crate 所有权见 ADR 0007。
- `data-loading.md`：v0.2/v0.3 Rust loader、严格版本分流、Core normalization、错误与测试边界。
- `example-scenarios.md`：示例场景和验证路径。

## 文档状态

设计文档状态建议使用：

- `Draft`：草稿中，不能直接作为稳定实现输入。
- `Review`：已形成可审阅版本，但仍可能调整。
- `Accepted`：可作为当前阶段实现输入。
- `Active`：持续维护的治理性或索引性文档。
- `Archived`：历史保留，不再作为默认输入。

## 页头约定

正式设计文档建议包含：

```md
# Document Title

**文档状态**: Draft
**最后更新**: YYYY-MM-DD
**适用范围**:
**关联文档**:
```

## 使用规则

- 涉及 Core API、data spec 或 Adapter 协议的实现，应先有相关 design 文档或 ADR。
- PR 中发现设计与实现不一致时，应先回写设计或拆分后续 Issue。
- 设计文档不记录单次 PR 的测试结果。
