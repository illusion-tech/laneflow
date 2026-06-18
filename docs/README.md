# LaneFlow 文档

本目录保存 LaneFlow 的长期设计依据、架构决策、治理规则、参考资料和开发约定。

GitHub 用于管理当前任务、协作状态和合并证据；仓库文档用于保存可版本化、可审查、可被 AI Agent 稳定读取的长期事实。

## 目录结构

```text
docs/
  README.md
  architecture.md
  roadmap.md
  adr/
  design/
  governance/
  reference/
```

## 目录职责

- `docs/adr/`：记录高影响、难回退的架构决策，重点回答“为什么这样定”。
- `docs/design/`：记录 Core、数据格式、Adapter、运行时规则等设计，重点回答“具体怎么做”。
- `docs/governance/`：记录 GitHub 工作流、开发闸口、AI Agent 开发规则和文档边界。
- `docs/reference/`：记录术语、模板、校验矩阵和长期复用的辅助规范。
- `.agents/`：记录跨 Agent 的执行工作流；工具专用入口只应薄包装这些工作流。

语言约定：模板与治理规范中文优先，详见 `docs/governance/documentation-policy.md` 第 6 节。

## 推荐阅读顺序

1. `README.md`
2. `docs/architecture.md`
3. `docs/roadmap.md`
4. `AGENTS.md`
5. `.agents/README.md`
6. `docs/governance/documentation-policy.md`
7. `docs/governance/github-workflow.md`
8. `docs/governance/development-gates.md`
9. `docs/governance/agent-development-guide.md`
10. `docs/adr/README.md`
11. `docs/design/README.md`

