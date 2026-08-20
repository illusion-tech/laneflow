# 文档边界政策

**文档状态**: Active  
**最后更新**: 2026-08-20
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

PR 默认通过合并队列（Merge Queue）进入 `main`，队列最终使用 **Rebase** 保持线性历史；
入队、失效与例外边界详见 `github-workflow.md` 第 7 节。

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
长期仓库文档可以保存稳定依赖顺序、实现开工前置条件和有日期的历史完成事实，但不得
镜像当前 Project 列、当前 Milestone 归属或原生 `Blocked by` / `Blocking` 状态；
这些实时元数据必须在使用时从 GitHub 读取。

### Milestones

Milestone 用于组织版本目标，例如：

- `v0.1 Core Prototype`
- `v0.2 Lane Graph + Route`
- `v0.3 Vehicle Following`
- `v0.4 Signals`
- `v0.5 Parking`
- `v0.6 Numeric & Spatial Foundation`
- `v0.7 Bevy Reference Adapter`
- `v0.8 Signalized Corridor MVP`
- `v0.9 Complete Signalized Corridor Example`
- `v1.0 Scope TBD`

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

## 6. 语言与术语约定

LaneFlow 的长期设计、模板与治理规范采用**中文权威、英文辅助**：

- GitHub Issue / PR 模板的标题、字段名、说明文字以中文为主。
- `docs/governance/`、`docs/reference/` 中的治理与参考规范以中文撰写。
- `.agents/skills/` 中的 Agent 工作流以中文撰写；工具专用薄包装（如 `.cursor/skills/`）同样中文优先。
- 技术标识符（切片类型、Gate 名称、分支前缀、commit 字段名等）可保留英文，便于工具解析与跨环境一致。
- ADR、design 和 architecture 中的领域术语必须有中文规范名与英文辅助名；中文定义
  是权威事实，英文别名不得改变语义。
- 新术语先进入 `docs/reference/glossary.md`，首次出现写作“中文术语（English
  Alias）”，后文优先中文。中英文冲突时以 glossary 的中文定义为准。
- 面向读者的数量以**清楚且无歧义**为首要要求。零较多的中文正文建议使用“一千”
  “一万”“十万”“一百万”等规范中文数量，表格、公式和机器输入建议使用
  `1000`、`10000`、`100000`、`1000000` 等完整十进制数字；上下文已经明确数量级、
  单位和计数对象时，`1k`、`10k`、`100k`、`1M` 等缩写或 `1 万` 等写法可以保留，
  不作为自动门禁或合并阻断项。小写 `m` 可能同时被理解为百万或米，使用时应由单位
  和上下文明确消除歧义；审阅发现歧义、错误数量级或前后口径不一致时必须修正，
  其余仅作可读性与一致性建议。代码标识符、测试/基准 ID、文件名、命令参数和外部
  协议字面量始终按原拼写保留并使用代码格式。
- 交通仿真规模不得使用 `Agent` 或“代理”作为计数单位；`Agent` 在项目正文中默认
  指 AI Agent 工作流，引用第三方专有术语时必须同时给出中文解释。#215 Accepted 的
  current 五项车辆计数继续约束现有性能证据；#291 已接受的长期通用规模使用术语表
  定义的交通参与单元，并按交通执行域分别报告个体、活动、意图更新、表现、聚合记录
  和聚合等价六项目标计数。Accepted 目标计数不得暗示目标态实现已经存在，也不得改写
  current 工作负载或历史证据。任何状态下都不得把不同执行域的未分解总数写成可比较
  性能指标。
- 当前 Core、现有 `Vehicle*` API 与历史车辆 workload 应继续使用准确的车辆术语，
  但必须标明 current/车辆特化边界；不得把车辆证据改写成已经支持非机动车、行人或
  轨道交通，也不得让当前车辆特化反向定义目标交通运行时。
- 历史 closure、验证和基准记录保存当时的证据语义、研究归因与能力边界。后续术语
  治理可以做数量或译名等价更新，但不得用新的 Proposed 架构重新定性旧结论；确需
  解释时，应保留原陈述并另行明确标注“按后续提案框架重述”，不能让重述冒充原始
  证据。
- Rust 类型、crate、字段、算法和协议常量等精确标识符使用反引号保留原文，但周边
  正文必须以中文说明其规范语义。

代码文档注释和开发者可读的模块说明默认使用中文，除非 API 生态、外部规范或工具字段明确需要英文。运行时错误信息、对外 API 命名语言可在后续专门 ADR 或 design 文档中另行约定；当前阶段默认与项目主要协作者语言一致，优先中文说明。
