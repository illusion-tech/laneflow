---
name: laneflow-development
description: 指导 LaneFlow 的 AI Agent 实现工作。适用于功能实现、缺陷修复、测试更新、运行时代码变更、数据格式修改或准备实现 PR。
---

# LaneFlow 开发

## 先读这些

1. `README.md`
2. `docs/governance/agent-development-guide.md`
3. `docs/governance/development-gates.md`
4. `docs/reference/commit-convention.md`
5. 相关的 `docs/design/` 与 `docs/adr/` 文档

若任务涉及 Core API、数据格式或 Adapter API，但缺少相关设计输入，应先停止实现并提出 G1 设计缺口。

## 工作流

1. 确认切片类型与影响范围。
2. 修改前先阅读现有代码与测试。
3. 变更范围限定在 Issue 或用户请求内。
4. 若改变长期行为或契约，同步更新文档。
5. 按切片类型运行对应检查。
6. 记录验证结果、文档状态与剩余风险。
7. PR 准备合并时，默认使用 **Rebase and merge**（`gh pr merge <number> --rebase`），除非 PR 中已说明例外。

## 规则

- 不要把引擎相关依赖引入 Core。
- 不要在不更新 design 文档的情况下改变数据格式语义。
- 不要把无关重构与功能开发混在同一 PR。
- 不要在只完成子切片时声称父任务已完成。
- 不要隐瞒未运行的检查；说明未运行项及原因。

## 交付说明

完成实现工作后，应汇报：

- 变更摘要
- 对 Core API、数据格式、Adapter API 的影响
- 已运行的验证
- 文档是否更新或为何无需更新
- 剩余风险或后续 Issue
- 建议的 PR 合并方式（默认 Rebase and merge）
