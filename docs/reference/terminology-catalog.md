+++
schema_version = 1
id = "reference-terminology-catalog"
kind = "reference"
status = "active"
revised_date = "2026-11-17"
summary = "定义术语 TOML 正本、概念身份、生成与网站导出。"
scope = "术语、旧称、措辞规则的结构化来源与文档工具"
related = ["reference-glossary", "reference-wording-guide", "governance-terminology-policy", "reference-document-metadata"]
+++

# 术语目录格式与维护

## 1. 唯一事实来源

`docs/reference/terms/catalog.toml` 声明领域与来源文件。所列 TOML 是词条、旧称和
措辞规则的唯一正本；[术语表][glossary]与[措辞指南][wording]的词条正文全部生成。
页面的 TOML 页头、标题和简短导读继续手工维护，生成器不改 `revised_date`。

术语定义说明概念身份与必要区别；算法、字段、单位、数值约束、实现边界和验证合同
由责任 ADR、design 或 reference 拥有。关系字段直接引用文档 ID，不能在定义中复制
审批或交付流水。用词流程由[语言与术语政策][policy]拥有。

按领域维护少量来源文件。每个概念只定义一次，多领域使用通过 `domains` 表达。
第一个领域决定完整词条所在章节，其他领域生成交叉索引。文件位置不构成概念身份。
各领域先生成“中文规范名、英文辅助名、精确标识符”短索引表，中文名链接到完整
词条；定义、条件与责任来源在词条中展开。索引与正文共用同一份 TOML，不重复维护。

## 2. TOML 与文本合同

- UTF-8；支持 LF 与 CRLF，生成输出统一为 LF。
- 每个文件顶层有整数 `schema_version = 1`；未知版本、未知字段、重复键和错误类型拒绝。
- `catalog.toml` 的 `files` 必须完整列出本目录其他普通 TOML 文件；文件名使用小写
  ASCII 连字符标识加 `.toml`，不得使用路径、重复文件或符号链接。目录不接受嵌套目录。
- 来源文件可包含 `terms`、`name_changes`、`wording_rules` 表数组；至少一条记录。
- ID 使用小写 ASCII 字母、数字和连字符，三类记录全局唯一；改名或移动文件不改 ID。
- 可选字段省略，不使用空字符串或占位破折号；可选列表可省略或为空。
- 名称、符号为非空单行文本，前后无空白；符号保留精确大小写与拼写。
- 定义、范围、条件、原因、说明为非空单行文字，可含行内代码；不嵌入标题、链接、
  图片、HTML、列表或其他 Markdown 块。链接用结构化字段，长论证进入责任文档。
- 顶层字段在前、表数组在后，避免 TOML 表作用域误写。数据不执行模板或脚本。

## 3. 领域目录

目录的必填字段为 `schema_version`、`files`、`domains`。每个 `domains` 成员包含
唯一 `id` 与中文 `name`；成员顺序就是阅读页领域顺序。领域名只在这里维护。

## 4. 概念词条

| 字段              | 含义                                                          |
| ----------------- | ------------------------------------------------------------- |
| `id`              | 稳定概念 ID                                                   |
| `name`            | 唯一规范名，通常是中文；登记借词时保留原词                    |
| `english_aliases` | 英文辅助名称列表；非借词至少一个，不承载代码标识符            |
| `domains`         | 非空、无重复的已登记领域 ID 列表                              |
| `context`         | 可选的明确语境 ID，用于同领域同形异义；不得为绕过冲突任意分配 |
| `definition`      | 中文概念定义，不记录审批或实现进度                            |
| `borrowed`        | 借词说明，含 `language = "en"` 和中文 `first_use_gloss`       |
| `short_forms`     | 允许简称，每项含 `name` 与必填 `scope`                        |
| `symbols`         | 精确符号，每项含 `value` 与 `kind`                            |
| `scope_notes`     | 消歧、适用条件与必要例外的文字列表                            |
| `references`      | 责任文档引用，每项含 `document` 与可选 `anchor`               |
| `related`         | 其他概念 ID 列表，不允许自引用；关联可以双向，不视为重定向    |
| `retirement`      | 概念停止采用的原因与可选替代关系；不用于一般改名              |

`symbols.kind` 为 `type`、`crate`、`field`、`function`、`constant`、`acronym`、
`unit` 或 `notation`。代码符号可以服务多个概念，不能强制其全局唯一。

同一明确领域与语境中，规范名、英文辅助名、允许简称和旧称不得属于不同记录。
比较只对 ASCII 字母忽略大小写；不改写符号，不推断中文同义词，也不把不同领域
同形词一律拒绝。语境是否真的独立、名称是否同义由人工审阅。

`references.document` 必须指向现存正式文档 ID；`anchor` 不带 `#`，必须在该文档
存在。优先引用明确章节，长期引用使用显式稳定锚点。项目特有合同应提供责任来源；
无需项目特殊解释的通用术语可以没有来源。概念间使用 `related`，不能引用生成术语
页作为自己的技术依据。

```toml
schema_version = 1

[[terms]]
id = "shared-network-revision"
name = "共享路网修订"
english_aliases = ["shared network revision"]
domains = ["static-network"]
definition = "绑定同一路网修订标识的共享组件对象；组件不得独立切换。"
symbols = [{ value = "SharedNetworkRevision", kind = "type" }]
short_forms = [{ name = "修订", scope = "完整性语境中已经明确所指对象。" }]
references = [{ document = "design-shared-static-network" }]
```

示例只展示字段结构，实际定义以术语 TOML 正本为准。

## 5. 旧称、退役与措辞

### 5.1 旧称映射

`name_changes` 每条包含 `id`、非空 `names`、非空 `domains`、可选 `context`、
`scope`、非空 `replacements`、布尔 `include_compounds`，以及可选 `exceptions`
和 `issues`。`issues` 是正整数议题号列表，只保留登记依据，不复制任务状态。

替代项包含 `target` 与 `condition`。目标可以是现行概念或另一条旧称映射，必须
最终通向现行概念；拒绝缺失目标、退役目标、自环与循环。多个替代项表示按条件
选择，不能把它们当作无条件同义词。`include_compounds` 只说明同义复合词的编辑
范围，不授权机械子串替换。

### 5.2 概念停止采用

`terms.retirement` 包含必填 `reason` 和可选 `replacements`。退役概念必须有责任
文档依据；没有等价替代物时不填替代项。它们在单独章节供历史阅读检索，不出现在
现行概念章节。替代关系说明职责对应，不推导接口或字节兼容。

### 5.3 按语境改写

`wording_rules` 包含 `id`、非空 `expressions`、`scope`、`guidance`，以及可选
`alternatives`、`exceptions`、`references` 和 `issues`。`alternatives` 与旧称
替代项使用同一结构；没有单一替换词时只写动作指导。规则生成到[措辞指南][wording]。

## 6. 接受状态、边界与日期

术语记录没有 `implementation_status`、审批状态、完成比例或任务阶段字段。
引用设计的 `status` 与 `validity` 从现有文档目录读取，只表示来源文档状态，
不能据此推断某词条已实现、被全部取代或所有数值均已接受。

提案中的概念允许登记以便阅读，必须以范围说明明确提案语境并引用责任文档。
当前车辆特化、长期执行域和历史测量的区别继续显式保留，不因结构迁移消失。

修订定义、名称资格、范围或实质措辞规则时，更新受影响阅读页的 `revised_date`。
纯文件分拆、排序、生成刷新和排版不推进日期；不为每个词条复制一份修改日期。
日期语义遵循[文档元数据规范][metadata]。

## 7. 生成、校验与导出

```powershell
cargo +1.100.0 run --locked -p xtask -- generate-glossary
cargo +1.100.0 run --locked -p xtask -- generate-doc-indexes
cargo +1.100.0 run --locked -p xtask -- check-docs
cargo +1.100.0 run --locked -p xtask -- export-docs
```

`generate-glossary --check` 只检查漂移。生成区由唯一一对
`generated-terminology:start/end` 标记限定，必须位于页面末尾；生成器先验证两份
页面、全部数据与引用，再开始写入。I/O 失败仍须检查工作区，不能视为文件事务。

词条锚点为 `term-{id}`，旧称为 `name-{id}`，措辞为 `wording-{id}`。生成标题
和显式锚点继续接受原 Markdown 锚点检查。链接使用页脚引用，全部受 `check-docs`
检查；未登记文件、无效数据、引用错误和生成漂移进入现有 `Commit message` 检查。

`export-docs` 的目录格式为 `schema_version = 2`，新增 `terminology` 对象；其内部
`schema_version = 1`，包含 `domains`、`terms`、`name_changes` 和 `wording_rules`。
每条记录额外有仓库相对 `source` 路径与稳定逻辑 `url`；网站可按规范名、英文名、
简称、旧称和符号建立索引，并通过 `references.document` 联结 `pages` 中的状态、
有效性和页面地址。退役与语境信息必须随搜索结果展示，不能只返回名称。

导出不包含本机绝对路径、构建时间或推断出的实现状态。部署子路径仍由网站适配器
添加。词条与三个记录数组按 ID 稳定排序，不依赖目录枚举次序；模型与生成器共用，
不要求网站重新解析 Markdown 长表格。

机器检查确定的结构；中文语义、责任来源是否充分、例外是否适用仍由人审。不开启
全文禁词扫描、词数限制或自动全文替换。

[glossary]: glossary.md
[wording]: wording-guide.md
[policy]: ../governance/terminology-policy.md
[metadata]: document-metadata.md
