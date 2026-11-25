+++
schema_version = 1
id = "reference-document-metadata"
kind = "reference"
status = "active"
revised_date = "2026-11-17"
summary = "定义正式文档的 TOML 元数据、有效性关系和发布数据模型。"
scope = "docs 中的正式 Markdown、生成索引与后续网站消费入口。"
issues = ["https://github.com/illusion-tech/laneflow/issues/1435"]
related = ["governance-documentation-policy", "reference-documentation-checks"]
+++

# 文档元数据与发布模型

正式文档采用 TOML 页头与普通 Markdown 正文。元数据承载机器需要的身份、分类、
状态及关系；正文解释设计、决策理由、取代范围和实现差异。接受设计不等于实现、
验收或产品认证完成。本规范由 #1435 的维护者接受方案实施。

## 1. 来源与唯一事实来源

- `docs/**/*.md` 是正式文档来源，含目录首页；`fixtures`、`tests`、`target`、
  `node_modules` 和 `.git` 子目录不是正式页面来源。来源中的符号链接拒绝。
- 根 README、贡献指南、组件 README、schema 和品牌说明保持原生格式；普通链接
  检查继续覆盖其既有范围。它们不是自动发布页面；以后导入网站须显式设计导入映射。
- Skill、工具入口、GitHub 模板、测试说明不套用本页头，不能将 Skill 的 YAML
  元数据当作正式文档元数据。
- 文档 ID、状态、摘要和取代关系只在文档页头维护。索引列表和反向取代关系生成，
  不另建可手改的状态清单。带上下文的正文链接可以继续存在。
- Markdown 页头由 Rust 类型模型解析；索引和目录导出共用该结果，网站适配器不得
  从中文字段、括号、审批句式或 GitHub 正文猜测状态。

## 2. 页头语法

页头从文件第一行开始，由独占一行的 `+++` 包围。编码为 UTF-8，不写 BOM；支持
LF 与 CRLF。分隔符属于文档封装，不属于 TOML。字段区间先从 Markdown 解析输入中
屏蔽，保留原行号；页头中的 URL、数组和符号不能成为正文链接或标题。

字符串使用 TOML 字符串，日期一律使用带引号的 `YYYY-MM-DD`，校验真实日历日期。
字段名与枚举值区分大小写。未知字段、重复键、错误类型、空白必填文本、未知
`schema_version` 均拒绝。可选字段缺省时省略；TOML 没有 null 值。

源文件使用 TOML 1.0 已有语法，不依赖 1.1 新增的多行内联表与新转义；解析库版本由
Cargo.lock 钉定。当前模型直接接收该库支持的 TOML；它不声称具有独立的 TOML 1.0
语法门禁。网站端优先消费规范化 JSON，直接读取源页头的适配器须运行同一组互操作
样本，不能以“都支持 TOML”替代兼容验证。

顶层字段写在前，表与表数组写在后。表声明后的空行不重置作用域；关系对象中的
未知字段同样拒绝，避免顶层 `status` 误落进最后一个 `supersedes` 对象。

```text
+++
schema_version = 1
id = "design-example"
kind = "design"
status = "accepted"
revised_date = "2026-11-17"
summary = "定义示例能力的输入、执行规则和失败行为。"
scope = "示例能力的公共接口。"
applicability = "#implementation-boundary"
related = ["adr-0001"]

[[supersedes]]
target = "design-previous"
extent = "partial"
explanation = "#replacement-scope"
+++

# 示例设计
```

示例中的 ID 与锚点仅说明结构；新文档必须改成真实存在的目标。

## 3. 字段合同

| 字段                | 类型       | 要求与语义                                                               |
| ------------------- | ---------- | ------------------------------------------------------------------------ |
| `schema_version`    | 整数       | 必填，当前为 1；是元数据版本，不是产品数据格式版本                       |
| `id`                | 字符串     | 必填且全局唯一；小写 ASCII 字母或数字组成的非空连字符分段                |
| `kind`              | 枚举       | 必填，见下节                                                             |
| `status`            | 枚举       | 必填，按文档类型限制；不能附加括号或其他字符串                           |
| `summary`           | 字符串     | 必填的单行纯文本，用于列表和搜索；不含 Markdown 标记                     |
| `scope`             | 字符串     | 非索引必填，说明约束对象，不镜像任务进度                                 |
| `date`              | 日期字符串 | 可选，文档原有记录日期；不自动解释为批准、创建或实现日期                 |
| `revised_date`      | 日期字符串 | 最近一次实质内容修订的日期；新建或实质修订时填写，存量日期无法确认时省略 |
| `decision_date`     | 日期字符串 | 可选，仅用于 accepted/rejected，必须有可核实的接受或拒绝依据             |
| `verified_date`     | 日期字符串 | 可选，明确执行过内容核对的日期，与 verified_revision 成对                |
| `verified_revision` | 字符串     | 可选，核对依据的 40 位小写 Git 提交 ID；不表示当前 HEAD 已复核           |
| `applicability`     | 字符串     | 可选，本页稳定 ASCII 锚点，如 #implementation-boundary                   |
| `issues`            | 字符串数组 | 可选，唯一的 GitHub Issue/PR 完整 HTTPS URL；不缓存其状态                |
| `related`           | ID 数组    | 可选，存在、不重复且不是自身的正式文档 ID                                |
| `supersedes`        | 表数组     | 可选，每项包含 target、extent、explanation，不允许额外字段               |

正文恰有一个非空 H1；显示标题取 H1 的可见文本，不另设 `title`。日期不从合并时间
或文件系统时间推测。

### 3.1 内容修订日期

`revised_date` 表示作者记录的最近一次实质内容修订日期。新建文档时记录初版内容日期；
修改技术合同、决策内容、适用范围、实现边界、操作步骤或治理规则时更新。状态或
取代关系发生实质变化也属于修订。日期使用修订实际发生的日历日期，不在合并或构建时
自动改写；人审负责判断本次变更是否构成实质修订。

仅修改拼写、排版、文件位置、无语义变化的链接，或在保持原意的前提下迁移页头与
重排段落，不更新 `revised_date`。索引生成器只更新生成区，不因被引用页面改变而
推进索引页的日期；人工导读或导航规则发生实质变化时才更新索引页的日期。

存量迁移保留原有“最后更新／最近修订”明确记录的日期，不因本次 TOML 格式迁移
批量改成迁移当天。一个字段含多个日期时，根据修订说明选取最近一次内容修订；
只有“日期／生效日期／最后核对”或无法确认的记录，不自动当作内容修订日期。
历史依据不足时省略该字段，待下一次实质修订时填写；不填占位值，不从 Git 时间猜测。

`revised_date`、`decision_date` 与 `verified_date` 各自独立：修订不自动代表重新
接受或核对，后两者不随修订自动推进。目录导出原样提供 `revised_date`；网站可显示
为“内容修订日期”，缺失时省略。Git 派生的文件最后修改时间须另行标注，不能覆盖
或回填此字段。当前导出不包含 Git 修改时间；后续适配器须取得充分历史，不能用
构建时间代替。

## 4. 类型、状态与有效性

| kind       | 含义                     | 允许的 status                            |
| ---------- | ------------------------ | ---------------------------------------- |
| adr        | 架构决策记录             | draft、review、accepted、rejected        |
| design     | 具体设计与持续维护的合同 | draft、review、accepted、active、retired |
| guide      | 操作指南                 | draft、review、active、retired           |
| reference  | 参考规范与说明           | draft、review、active、retired           |
| governance | 项目治理                 | draft、review、active、retired           |
| index      | 目录首页                 | draft、review、active、retired           |

- `draft`：草案，不能作为稳定实现依据。
- `review`：文档内容待接受；不镜像 Project 列或 PR 审阅状态。
- `accepted`：决策或设计已接受；不表示实现存在、测试通过或产品认证完成。
- `rejected`：该决策未采用；不能使既有决策失效。
- `active`：持续维护的现行说明；不自动补发 G1 接受事实。
- `retired`：停止作为现行依据；保留页面不代表仍推荐采用。

现有 Active 设计不机械改成 Accepted。README.md 是 index，其他文件不能是 index；
ADR 目录中的正式记录必须是 adr。状态不再使用 Superseded 或“部分取代”：接受情况
与条款有效性分开。

目录从生效取代关系计算 `validity`：`unreplaced`、`partially_superseded`、
`superseded`。`unreplaced` 只表示没有登记生效取代边，不是正确性、实现完成或产品
有效性认证。页面展示同时读取 status 和 validity，不能只显示其中一个。

## 5. 取代关系

`supersedes` 只在后继文档维护，反向 `replaced_by` 自动生成：

- `target` 是被取代文档 ID；每个源到同一目标只能登记一次。
- `extent` 仅允许 `full` 或 `partial`。多个部分取代不自动合并成全部取代。
- `explanation` 指向后继文档的稳定锚点；正文明确被替代条款、保留条款与限制。
- accepted/active 来源的边生效；draft/review/rejected 的边仅保留为提议，不改变
  目标有效性。已生效后来源退役，不自动恢复旧合同；retired 来源边继续生效。
- 所有边（含提议）必须目标存在、没有自引用和循环。多个直接生效的 full 来源
  拒绝，应按后继顺序串联；同一文档自身修订使用 Git 历史，不建立自引用。
- 扩展、补充说明和实现依赖使用 related 或正文链接，不能误登记为取代。
- 已在原位改写为现行正本的 design 不因旧条款曾被替代就整页标记为失效。历史 ADR
  的部分失效与现行 design 的原位维护是不同职责。

只有人审能判断条款是否真的被替代；图校验验证登记的一致性，不从自然语言推断边。

## 6. 适用与实现边界

重要限制放入正文独立章节，用 `applicability` 指向它。尤其应说明已接受但未实施的
设计、只适用某些章节的变更、历史证据不可外推的条件，以及被替代后仍有效的条款。

不新增 implementation_status、完成百分比、负责人、Milestone、blocker 等字段。
实时进度由 GitHub 管理，能力说明按明确的实现与验证依据维护。清理状态附注必须逐项
保留仍有效的限制；纯审批流水和任务完成记录回到 Issue/PR/Git，不搬成另一个长字段。

## 7. 索引、链接与网站数据

所有 index 页面末尾恰有一组 `generated-doc-index:start/end` HTML 注释标记。
该区域列出同目录文档及直属子目录首页，按稳定 ID 排序，标题、状态、有效性、摘要
及引用定义全部生成。人工导读放在区域之前；不得手改生成列表或复制版本号与进度。

```powershell
cargo +1.100.0 run --locked -p xtask -- generate-doc-indexes
cargo +1.100.0 run --locked -p xtask -- generate-doc-indexes --check
cargo +1.100.0 run --locked -p xtask -- check-docs
cargo +1.100.0 run --locked -p xtask -- export-docs
```

`export-docs` 先执行完整检查，然后写入忽略目录 `target/docs/catalog.json`。它包含
元数据、源路径、H1 标题、稳定逻辑 URL `/docs/{id}/`、有效性、反向关系、锚点集合、
按正文顺序排列的标题/级别/锚点映射、本地链接原文及解析结果和 Markdown 正文。
正文换行统一为 LF；源文件排序和结构输出确定；不含本机绝对路径或
构建时间，不将构建目录作为第二份正本提交。

目录导出格式当前为 `schema_version = 2`；`pages` 保留上述页面模型，新增
`terminology` 承载结构化概念、旧称与措辞规则。该对象与生成术语阅读页共用
TOML 正本及 Rust 模型，字段、稳定词条 URL 和文档联结规则见
[术语目录格式与维护][terminology-catalog]。正式文档页头仍为 schema_version 1，
不得与目录导出的格式版本混用。

正文继续用文件相对链接和页脚引用；导出为正式页面链接提供 page_id 和逻辑 URL，
指向正式目录的链接映射到该目录 README 的 ID；非正式源码/附件链接保留 target、
directory 与 fragment。下游须分别映射页面、资源和源码；
实际部署的 base path 由适配器加到逻辑 URL，不能简单把 .md 替换为 .html。
外部链接保留在正文，不在离线检查时访问。

沿用[文档检查][documentation-checks]的标题锚点算法。需要稳定引用的章节使用
`<a id="stable-name"></a>`；改标题不得随意改变显式锚点。网站渲染器须消费同一算法
或导出锚点映射，不能使用另一套 slug 后声称链接一致。
显式锚点不得与另一元素的显式锚点或标题 slug 冲突；同一 a 元素的 id/name 同值
只算一个目标。重复标题仍按既有算法生成不同后缀。

正式正文保持普通 Markdown。表格、代码围栏、任务列表、图片、Mermaid 和提示块须
在建站适配器中逐类验收；本次导出不编译这些展示内容，也不是已完成网站渲染的证明。
不引入框架专用 JSX/组件语法。网站只允许必要 HTML（包括显式锚点），不得执行正文
脚本；具体 HTML 清理与资源复制在建站任务实现。GitHub/编辑器对 TOML 页头的预览
效果不是本地检查器保证的一部分。

## 8. 校验与维护边界

机器校验语法、类型、唯一 ID、合法日期、关系目标与循环、范围锚点、链接和索引漂移。
人审负责批准事实、取代范围、当前实现边界和技术正确性。禁止解析聊天或 GitHub
正文判断审批，禁止以字数限制或括号黑名单代替语义审阅。

新增字段须同时修改 Rust 类型、验证、测试与本规范；未知字段不能静默接受。
产品未发布阶段只有一套当前模型，不维护旧加粗页头回退解析器或 YAML/TOML 双轨。

[documentation-checks]: documentation-checks.md
[terminology-catalog]: terminology-catalog.md
