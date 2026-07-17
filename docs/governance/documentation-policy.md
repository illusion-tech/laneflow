# 文档边界政策

**文档状态**: Active  
**最后更新**: 2026-07-17
**适用范围**: LaneFlow 的 GitHub 治理、仓库文档治理和 AI Agent 开发上下文

## 1. 目标

本文定义 LaneFlow 中“哪些信息放在 GitHub，哪些信息放在仓库文档文件中”。

核心原则：

> GitHub 管当前状态和协作流，仓库文档管长期事实和设计依据。

## 2. GitHub 负责什么

GitHub 是当前工作的协作入口，负责动态状态、讨论、评审和发布事实。

### Issues

Issue 用于承载：

- 功能需求
- 缺陷报告
- 设计讨论入口
- Adapter 支持请求
- 技术债
- 版本任务
- AI Agent 开发任务说明
- 验收 checklist

Issue 不作为长期权威设计来源。涉及长期设计、Core API、数据格式或 Adapter 协议的结论，必须回写到仓库文档。

### Pull Requests

PR 用于承载：

- 本次变更范围
- 关联 Issue
- 测试与验证结果
- Core API 影响
- 数据格式影响
- Adapter 协议影响
- 已知风险与例外
- AI Agent 实现说明

PR 是合并闸口，不是设计文档替代品。

PR 合入 `main` 默认使用 **Rebase and merge**，详见 `github-workflow.md` 第 7 节。

### Projects

GitHub Projects 用于承载：

- Backlog
- Ready
- In Progress
- In Review
- Done
- Blocked
- Milestone 视图

Project 管当前优先级和进度，不承载详细架构。

### Milestones

Milestone 用于组织版本目标，例如：

- `v0.1 Core Prototype`
- `v0.2 Lane Graph + Route`
- `v0.3 Vehicle Following`
- `v0.4 Signals`
- `v0.5 Parking`
- `v0.6 Numeric & Spatial Foundation`
- `v0.7 Bevy Reference Adapter`
- `v1.0 Stable Runtime API`

### Discussions

Discussions 可用于尚未进入路线图的想法、用户反馈、生态讨论和问答。讨论形成稳定结论后，应转为 Issue、ADR 或 design 文档。

### Releases

Releases 用于记录发布事实：

- 版本说明
- breaking changes
- Core API 版本
- 数据格式版本
- Adapter 兼容矩阵
- 示例项目状态

### Wiki

LaneFlow 初期默认不使用 GitHub Wiki。长期知识应进入仓库文档，以便通过 PR 审查、版本化和 AI Agent 读取。

## 3. 仓库文档负责什么

仓库文档保存长期事实、正式设计、规范和模板。

- `README.md`：项目入口、定位、非目标、架构概览、文档入口。
- `docs/architecture.md`：长期架构说明和分层职责。
- `docs/roadmap.md`：稳定路线图和版本能力边界。
- `docs/adr/`：高影响、难回退的架构决策。
- `docs/design/`：Core、数据格式、Adapter、运行时规则等具体设计。
- `docs/governance/`：GitHub 流程、开发闸口、AI Agent 开发规范。
- `docs/reference/`：术语、模板、校验矩阵、命名约定。
- `schemas/`：current JSON Schema 事实源与面向消费者的 identifier/distribution 入口；长期决策仍由 ADR 与 design 文档解释。
- `CONTRIBUTING.md`：贡献流程和协作规则。

### Schema 标识与分发文字

JSON Schema `$id` 与 runtime loader 路径必须分开描述。LaneFlow 按 ADR 0011 把 catalog 中的 `$id` 定义为 public canonical retrieval URL；文档只有在 live monitor 证实 HTTP 200 与 byte equality 时才能声称可下载。已发布版本永久保留且不可原地修改，current/历史边界以 `schemas/publication.json` 为准。历史 closure review 只记录当时事实，不能替代当前 `schemas/README.md`、ADR、CI/CD 与实时可用性证据。

## 4. 决策回写规则

以下内容如果只存在于 Issue、PR、Discussion 或聊天记录中，不算正式完成：

- Core API 边界
- 数据格式和 schema
- Adapter 协议
- 运行时 tick 规则
- 路线、车道图、信号灯、停车系统设计
- breaking changes
- 长期非目标
- 重大技术取舍

这些结论必须回写到 `docs/adr/` 或 `docs/design/`。

## 5. AI Agent 读取规则

AI Agent 开工前应优先读取仓库文档，而不是只依赖 Issue 描述。

最低读取顺序：

1. `README.md`
2. 与任务相关的 Issue
3. `docs/governance/agent-development-guide.md`
4. 相关 `docs/design/` 文档
5. 相关 `docs/adr/` 文档

## 6. 语言约定

LaneFlow 的**模板与治理规范**采用中文优先：

- GitHub Issue / PR 模板的标题、字段名、说明文字以中文为主。
- `docs/governance/`、`docs/reference/` 中的治理与参考规范以中文撰写。
- `.agents/skills/` 中的 Agent 工作流以中文撰写；工具专用薄包装（如 `.cursor/skills/`）同样中文优先。
- 技术标识符（切片类型、Gate 名称、分支前缀、commit 字段名等）可保留英文，便于工具解析与跨环境一致。

代码文档注释和开发者可读的模块说明默认使用中文，除非 API 生态、外部规范或工具字段明确需要英文。运行时错误信息、对外 API 命名语言可在后续专门 ADR 或 design 文档中另行约定；当前阶段默认与项目主要协作者语言一致，优先中文说明。
