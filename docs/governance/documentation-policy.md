+++
schema_version = 1
id = "governance-documentation-policy"
kind = "governance"
status = "active"
revised_date = "2026-11-24"
summary = "规定仓库文档、协作平台与研究资料的职责边界。"
scope = "LaneFlow 的 GitHub 治理、仓库文档治理和 AI Agent 开发上下文"
+++

# 文档边界政策

## 1. 目标

本文定义 LaneFlow 中“哪些信息放在 GitHub，哪些信息放在仓库文档文件中”。

核心原则：

> GitHub 管当前状态和协作流，仓库文档管长期事实和设计依据。

正式文档使用 [TOML 元数据与发布模型][document-metadata]。文档身份、类型、接受或
维护状态、日期与取代关系由类型化页头保存，状态不得附带括号、审批流水或技术合同。
重要适用限制进入正文并提供范围入口；索引状态与反向取代关系由工具生成，不手工复制。
`accepted` 不表示实现或验收完成；文档接受情况与条款是否被取代分别表达。
`revised_date` 记录实质内容修订，接受日期与内容核对日期独立维护；纯格式迁移
不推进修订日期，也不能用 Git 文件修改时间替代它。

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

Issue 的工作性质使用 GitHub 原生 `Feature` / `Bug` / `Task`；影响范围与工作方式使用
`area:*` / `work:*` 标签；阶段使用 Project Status；交付目标使用 Milestone；任务分解
与真实依赖分别使用 parent / sub-issue 和 blocked-by / blocking。标题只写结果，不把
这些字段复制成方括号前缀。字段职责与取值见 `github-workflow.md`。

Issue 不作为长期规范设计来源。涉及长期设计、Traffic Runtime API、数据格式或 Adapter 协议的结论，必须回写到仓库文档。

### Pull Requests

PR 用于承载：

- 本次变更范围
- 关联 Issue
- 测试与验证结果
- Traffic Runtime API 影响
- 数据格式影响
- Adapter 协议影响
- 已知风险与例外
- AI Agent 实现说明

PR 是合并把关环节，不是设计文档替代品。

PR 默认通过合并队列（Merge Queue）进入 `main`，队列最终使用 **Rebase** 保持线性历史；
入队、失效与例外边界详见 `github-workflow.md` 第 7 节。

<a id="doc-state-at-merge"></a>
#### 提审时的文档状态

正式文档送审时，按本 PR 通过并合入后主线应表达的事实填写页头和正文，并在同一 PR 重新生成受影响的索引。
以接受 ADR 或冻结设计为合入条件的 PR，提审版本就写 `status = "accepted"`，并在正文保留尚待实施或验收的范围。
审阅进度在 GitHub 查看。若主线本次交付的就是一份仍待讨论或另行裁决的草案，则按实际用途保留 `draft` 或 `review`。
审阅改变结论时，先更新同一 PR 的文档与索引，再合入。可选的 `decision_date` 只填写有依据的实际裁决日期。

### Projects

GitHub Projects 用于承载：

- Backlog
- Ready
- In Progress
- In Review
- Done
- Cancelled
- Type、Priority、Design gate
- Milestone 视图

Project 管当前优先级和进度，不承载详细架构。阻塞不用 `Blocked` 状态表达，而从原生
blocked-by 关系读取。
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
- Traffic Runtime API 版本
- 数据格式版本
- Adapter 兼容矩阵
- 示例项目状态

### Wiki

LaneFlow 初期默认不使用 GitHub Wiki。长期知识应进入仓库文档，以便通过 PR 审查、版本化和 AI Agent 读取。

## 3. 仓库文档负责什么

仓库文档保存长期事实、正式设计、规范和模板。

- `README.md`：项目入口、公开组件定位、当前能力与边界、架构概览、文档入口。
- `docs/architecture.md`：长期架构说明和分层职责。
- `docs/roadmap.md`：稳定路线图和版本能力边界。
- `docs/adr/`：高影响、难回退的架构决策。
- `docs/design/`：Traffic Runtime、数据格式、Adapter、运行时规则等具体设计。
- `docs/governance/`：GitHub 流程、开发阶段评审、AI Agent 开发规范。
- `docs/guides/`：操作性接入教程，引用真实示例，不替代 design 与 ADR。
- `docs/reference/`：术语、模板、校验矩阵、命名约定，以及仍约束实现的机器可读
  编制种子 / 生产编译基线。不存放里程碑最终审阅、逐任务验证流水账、原始性能轮次
  或 smoke 截图；那些当时证据只保留在 git 历史与 GitHub Issue / PR。
- `schemas/`：道路编辑 FlatBuffers source；不是对外发布入口，也不是 JSON loader
  契约。长期决策仍由 ADR 与 design 文档解释。
- `CONTRIBUTING.md`：贡献流程和协作规则。

公开入口、ADR、设计文档和 Agent 工作流遵循 ADR 0021 的同一组件定位与宿主边界。
README 保持简洁，以可验证的当前能力为依据；长期方向与尚未完成的研究须明确
标注。地区交通规则和城市工作负载可以作为通用技术验证场景，不写入具体产品计划
或指定第一消费者，也不维护另一套内部产品定位作为架构正本。

### Schema 标识

ADR 0011 的公共 retrieval / 永久保留义务已被 ADR 0026 取代；不得把内部 schema
文件写成对外兼容承诺。current JSON schema 与加载 crate 已随 #301 删除。道路编辑
B1 仍是未发布的内部 source。

### 研究结论与过程数据边界

仓库不设研究资料专用目录（#1378 移除了根 `research/`）。有用的是研究结论，不是
研究过程；研究产出按职责归位：

- **现行结论**：影响现行行为、限制或裁定的结论原位回写所属 `docs/design/`、ADR 或
  现行 reference。结论至少说明研究问题、关键结果、影响结论的主要条件、已知限制，
  以及关联源码或 Issue / PR 入口；结论引用的聚合数字（中位数、范围、计数）属于
  结论本身。
- **任务过程**：计划、阶段记录、测量方法与一次性结论报告由 Issue / PR 承载，不在
  main 继续维护。
- **规范样本与活测试输入**：被现行合同指定的设计样本放在所属设计文档旁的
  `docs/design/fixtures/`；活测试消费的输入放在所属 crate 的 `tests/fixtures/`。
- **过程数据**：逐区间或逐样本原始测量、搜索轨迹、全量结果集、重跑中间态、
  环境转储、二进制和采集原包，一律不进入源码树。本地生成使用已忽略的 `target/`
  或仓库外目录；后续争议可能需要重跑实验，不能假定可恢复当年的完整现场。仅在
  具体任务确有对外交付或原件复核需求时，在该任务中明确交付入口与校验方式。

研究结论不因入仓而成为产品性能或交通质量认证。

不得以「证据保存」「完整回归证据」等名义把过程数据提交进仓库。Agent 不得在研究
任务中自发生产清单、manifest、逐字节一致性声明或 SHA-256 登记等归档物，除该任务
明确要求外不生成；允许入仓不等于要求生成或提交。

冻结研究数据不得被任何代码以 `include_str!` / `include_bytes!` 编译期嵌入
（#1249）。活测试消费的冻结数据放在所属 crate 的 `tests/fixtures/`，新增大体量
fixture 须在 PR 说明其体积为断言必要数据（对拍或字节迁移证据）；确需外部冻结
数据的手动重放实验经环境变量从冻结提交 checkout 运行时读取，不得回落到当前树
（最后一个此类入口 `PINNED_RESEARCH_ROOT` 已随 #1385 退役，有效最小输入已归位
`tests/fixtures/`）。

清理证据时区分：

- **迁移并继续承诺可恢复**：按承诺范围验证实际下载和恢复，再删除当前文件并修复
  文档引用与依赖入口；声称保留原件时，不得用重新序列化或重跑结果替代原始字节。
- **退役、不再保留完整证据**：可通过普通 PR 删除并修复文档引用与依赖入口，说明
  不再保证原始数据恢复，无须先建立外部归档。已提交材料可通过冻结提交查阅，但
  Git 历史不代表未提交的外部原包备份。

文档、索引、脚本默认值和提交的工具输出不写采集机的本机布局路径（项目 checkout、
工作树、仓库外证据目录、用户目录与 Cargo 主目录），统一用 `<evidence-root>`、
`<checkout>`、`<worktree>`、`<cargo-home>` 占位。本机路径或其占位不能充当团队下载
入口；选择外部交付时应提供实际可访问入口。系统工具链的标准安装位置（如
`Program Files` 下的 MSVC 与 Windows Kits）不反映本机布局，可以保留；不得提交
密钥等敏感信息。

生产契约、规范输入及直接约束现有测试的 fixture 按其语义保留，不因 JSON 后缀一律
迁移；独立的产品、供应链安全校验和任务专项要求不因本节自动取消。证据清理默认走
普通 PR，保留旧提交历史；重写 Git 历史另行取得明确授权。本规则适用于后续研究和
今后的清理，不追补历史档案。#1378 之前入仓的研究资料通过固定提交的 URL 引用，
不另建历史总索引；引用时说明报告保存提交与被测实现提交，不把旧数字当作新 main 的
测量。

## 4. 决策回写规则

以下内容如果只存在于 Issue、PR、Discussion 或聊天记录中，不算正式完成：

- Traffic Runtime API 边界
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

长期设计、模板、治理规范与 Agent 工作流以中文为准。术语收录、首次出现、英文
辅助名、精确标识符、简称、旧称、数量表达和历史豁免统一由
[语言与术语政策][terminology-policy]规定，本政策不复制细则。

[术语表][glossary]是生成的阅读入口，词条正本位于 `docs/reference/terms/`。
新增或修订词条按[术语目录格式][terminology-catalog]维护 TOML 并生成阅读页。
措辞需按机制选择时查阅[措辞指南][wording-guide]；检查器不扫描正文推断禁词。

代码文档注释和开发者可读的模块说明默认使用中文，除非 API 生态、外部规范或工具字段明确需要英文。运行时错误信息、对外 API 命名语言可在后续专门 ADR 或 design 文档中另行约定；当前阶段默认与项目主要协作者语言一致，优先中文说明。注释的功能分层与 rustdoc 契约段落（`# Errors` / `# Panics`）规范见 `docs/reference/rust-code-style.md` §5，本政策不复制细节。

## 7. 受检 Markdown 的链接写法

默认文档检查范围内的 Markdown 必须使用页脚引用定义。同一文件内指向同一目标的引用共用一条定义，定义放在该文件末尾。

默认范围与 `xtask check-docs` 不带路径参数时的来源一致：

- 仓库根目录的 `README.md`、`CONTRIBUTING.md`、`AGENTS.md`
- `docs/**/*.md`
- `.agents/**/*.md`
- `crates/*/README.md`
- `tools/*/README.md`
- `schemas/**/*.md`
- `.github/*.md`（只含该目录下一层的 Markdown，不递归其中的 fixture）

行内链接、行内图片和自动链接都不允许。`[文字][标]`、`[文字][]` 和已经定义的 shortcut `[文字]` 都可以；图片写作 `![说明][标]`。代码围栏里的示例只用于说明写法，不视为正文中的链接。

标签必须是简短语义名。本地文件用文件名主干，如 `documentation-policy`；crate README 用短名，如 `laneflow-bevy-readme`；ADR 用短号，如 `adr-0021`；章节用短区分词，如 `contributing-tests`；外部页面用页面短名，如 `github-issues`、`rustup` 或 `code-scanning-advanced-setup`。禁止把完整 URL、commit SHA 或完整标题 slug 化成标签。同一文件内标签不得重复，冲突时只加最短区分后缀。

允许：

```markdown
见 [文档边界政策][documentation-policy]。
见 [测试与验证][contributing-tests]。

[documentation-policy]: ../governance/documentation-policy.md
[contributing-tests]: ../../CONTRIBUTING.md#8-测试与验证
```

不允许：

```markdown
[文字](../governance/documentation-policy.md)
![说明](image.png)
<https://example.com>
[证据][github-com-example-blob-0123abcd-docs-reference-calibration-evidence-json]
[测试与验证][contributing-8-测试与验证]
```

[document-metadata]: ../reference/document-metadata.md

[terminology-policy]: terminology-policy.md
[glossary]: ../reference/glossary.md
[terminology-catalog]: ../reference/terminology-catalog.md
[wording-guide]: ../reference/wording-guide.md
