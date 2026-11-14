# 离线文档结构检查

**文档状态**: Active
**最后更新**: 2026-11-11

**适用范围**: `xtask check-docs` 检查的本地路径、标题锚点与关键索引入口
**关联文档**:

- [`validation-matrix.md`][validation-matrix] 第 9 节：这条检查在验证矩阵中的位置
- [`../../xtask/data/doc-checks.toml`][doc-checks]：关键索引入口的 source→target 列表
- [`../governance/documentation-policy.md`][documentation-policy]：文档职责边界；本检查不改写该政策

## 1. 命令

在仓库根目录运行：

```powershell
cargo +1.99.1 run --locked -p xtask -- check-docs [<path...>]
```

- 无参数时扫描第 2 节的默认来源。传入路径只收窄或扩展来源扫描，不关闭关键索引入口检查。
- 只读，没有修复模式。成功退出 0；规则失败或无法读取、越界、显式符号链接等执行失败退出 1。
- 仓库根是进程当前工作目录，不是编译期源码路径。
- 依赖已在本地缓存后，可以用 `cargo +1.99.1 run --offline --locked -p xtask -- check-docs`。已构建的检查器本身不访问网络。`--locked` 不表示首次构建不会下载依赖。

失败行的形式是：

```text
path:line:column: target=<原始目标>: missing-path / missing-anchor / missing-required-entry / inline-form: <中文原因>
```

不能可靠判定、但也不当作锚点失败的语法另起一行：

```text
path:line:column: target=<原始目标>: skip: <中文原因>
```

汇总行给出 source、link、skipped 和 error 计数，并按 `http`、`mailto`、`rustdoc`、`unsupported`、`non-markdown-fragment` 分开。诊断按路径、行、列和类别稳定排序。

## 2. 默认来源

作为来源扫描：

- 根目录 `README.md`、`CONTRIBUTING.md`、`AGENTS.md`
- `docs/**/*.md`，含保留的 Retired 短结论和历史测量说明；只检结构，不检语义
- `.agents/**/*.md`、`.cursor/skills/**/SKILL.md`
- `crates/*/README.md`、`tools/*/README.md`、`schemas/**/*.md`
- `.github/*.md`，不递归其中的 fixture
- `research/README.md`，不递归研究历史

默认不从这些位置收集来源：`research/**`（除首页）、`**/tests/**`、`**/fixtures/**`、`**/evidence/**`、`**/src/generated/**`、`.git/`、`target/`，以及防御性的 `vendor/` 与 `node_modules/`。

这是不从来源目录收集，不是忽略指向 fixture、测试或生成文件的链接。指向真实 fixture、JSON 或 `.rs` 的本地路径仍检查存在性，但不解析非 Markdown 文件里的锚点。`.git` 单独处理：它是仓库元数据，不是已提交内容。链接指向 `.git` 或其下路径时记为 `missing-path`，即使工作区里该路径存在（普通 clone 是目录，链接 worktree 可能是文件）。显式传入路径可以进入 research 文档；仍然不跟随符号链接，也不离开仓库。

## 3. 链接与路径

解析器是 `pulldown-cmark`（CommonMark，并打开表格、删除线和任务列表）。不使用正则扫描 `](...)`。链接和图片使用同一套目标规则。未定义的 shortcut（单独的 `[text]`，且没有链接定义）不是链接。Markdown 事件中的 HTML 由 `html5gum` 按 WHATWG 分词，不是手写的 HTML 子集。

围栏外的行内链接 `[文字](url)`、行内图片 `![说明](url)` 和自动链接（含 `<user@example.com>`）记为 `inline-form` 失败，不再按目标检查，也不访问外部地址。页脚引用 `[文字][标]`、`[文字][]`、已定义的 shortcut `[文字]`，以及 `![说明][标]`，继续按下面的目标规则检查。代码围栏里的示例不是链接。

| 目标                                                          | 结果                                                                  |
| ------------------------------------------------------------- | --------------------------------------------------------------------- |
| 相对路径、以 `/` 开头的仓库根路径                             | 从源文件目录或仓库根做词法解析，再按目录项的原始大小写检查存在性      |
| Markdown 文件上的 fragment                                    | 必须命中第 4 节的标题 slug，或显式 `<a id>` / `<a name>`              |
| 非 Markdown 文件上的 fragment                                 | 只确认路径存在，锚点记入 `non-markdown-fragment`，不报 missing-anchor |
| 目录，且没有 fragment                                         | 目录存在即可                                                          |
| 目录，且带 fragment                                           | `skip`：不猜测会不会渲染 `README.md`                                  |
| `http` / `https`，含本仓库历史 `blob` / `tree` / `commit` URL | 不访问，只计入 `http`。即使地址指向本仓库，也不改写成当前树的路径     |
| `mailto`                                                      | 不访问，计入 `mailto`                                                 |
| 含 `::`、且没有 `/` 的 rustdoc 符号                           | 不解析，计入 `rustdoc`                                                |
| 查询字符串、非法百分号编码、反斜线、盘符、其他协议            | 可见 `skip`，不算锚点已通过                                           |
| `..` 越出仓库，或路径组件是符号链接                           | `missing-path`                                                        |
| `.git` 及其子路径                                             | `missing-path`：不是已提交内容，即使工作区里存在                      |

百分号编码只解码一次。`%20` 是空格，`+` 不是空格，`%2520` 不会再解成空格。`%2F` 与 `%2f` 不是目录分隔符，出现在本地路径里记为路径失败。`%252F` 只解码一次，得到字面量 `%2F`，既不当成斜线，也不按编码斜线拒绝。以 `//` 开头的协议相对地址和 `http` / `https` 一样不访问，计入 `http`；单个 `/` 仍从仓库根解析。`{#id}` 不作为自定义锚点扩展。不解析任意 HTML 标题，也不解析 HTML 属性里的路径；`<a href="missing.md">` 因此不会被当成 Markdown 链接。

行内代码里的路径默认不当链接。只有第 5 节把 `accept_code_span` 设为 true 的边，才接受整段代码恰好是该路径。代码围栏、缩进代码块和 HTML 注释里的假链接不检查。

## 4. 标题锚点

这是有限的 GitHub 兼容子集，不是 GitHub 渲染器的完整复现。

标题文本取 ATX 与 Setext 的可见文字：强调、加粗、行内代码和链接只保留文字，不保留标记或 URL。然后：

1. 去掉首尾空白。
2. 只把 ASCII `A-Z` 收成小写。非 ASCII 字母保持原样，所以 `É` 仍是 `É`。
3. 保留连字符、下划线、Unicode 字母和数字（含中文）。
4. 每个空白字符各映射为一个 `-`，不合并。标点删掉后两侧空白各留一个 `-`，所以 `A = B`
   与 `a  b` 都得到 `a--b`。
5. 删除其余字符，包括用来排版的标点。
6. 同一文件的标题共享一个 slug 命名空间。重复的 slug 依次尝试 `-1`、`-2`；若该后缀已被占用，则继续往后取。显式 HTML 锚点不参加这个计数，但 fragment 命中任一类即可。

正例：

| 标题文本                                                      | slug                                                    |
| ------------------------------------------------------------- | ------------------------------------------------------- |
| `快速开始`                                                    | `快速开始`                                              |
| `8. 测试与验证`                                               | `8-测试与验证`                                          |
| `6.1 模型目录与登记`                                          | `61-模型目录与登记`                                     |
| `6.2 Required checks`                                         | `62-required-checks`                                    |
| `5. 同拍联合求解与预约约束（裁定 A3/A4 回写，核心）`          | `5-同拍联合求解与预约约束裁定-a3a4-回写核心`            |
| `7.2 LF-ROUTING-CANDIDATE-LFSYNTH-v1 首轮确定性证据（#1267）` | `72-lf-routing-candidate-lfsynth-v1-首轮确定性证据1267` |
| `Foo_Bar-Baz`                                                 | `foo_bar-baz`                                           |
| `A = B`                                                       | `a--b`                                                  |
| `6. 能力检查（裁定 A6 = 选项 3 回写）`                        | `6-能力检查裁定-a6--选项-3-回写`                        |

反例：

- `Title {#custom}` 的 slug 是 `title-custom`。`{#custom}` 不会另建锚点 `custom`。
- `A = B` 的 slug 不是 `a-b`：链接写成 `#a-b` 会报 `missing-anchor`。
- 顺序为 `重复`、`重复`、`重复-1` 时，三个锚点是 `重复`、`重复-1`、`重复-1-1`，不是 `重复-2`。
- 顺序为 `Dup-1`、`Dup`、`Dup` 时，第三个锚点是 `dup-2`，因为 `dup-1` 已被第一个标题占用。
- 注释和 `<abbr id>` 里的值不是锚点。`<a id>` / `<a name>` 由 `html5gum` 读取，因此无引号的 `<a id=raw>` 也会登记。
- 需要稳定显式锚点时，写 `<a id="name"></a>` 或 `<a name="name"></a>`。单引号、双引号和无引号属性都可以。

## 5. 关键索引入口

[`xtask/data/doc-checks.toml`][doc-checks] 是这些边的列表。每条包含来源、目标和原因。它不要求每份 Markdown 都进入索引，也不能加入“忽略这个文件的错误”之类的条目。图片里的路径仍按第 3 节检查是否存在，但不能充当这些导航入口；图片替代文字中的行内代码同样不算。图片替代文字里的 Markdown 链接也不能充当这些入口。其中未定义的引用同样不检查，也不记为路径失败。图片替代文字里的 `<a id>` / `<a name>` 不登记为锚点。带结尾斜杠的行内代码是目录形式，也不能满足指向文件的入口。没有 fragment 的入口只接受同样不带 fragment 的链接或行内代码。`<a id>` / `<a name>` 的属性值由 `html5gum` 按 WHATWG 解码字符引用；重复属性只保留第一个。`<template>` 里的 `<a>` 不登记为锚点。`script` 进入 script data，`style`、`xmp`、`iframe`、`noembed` 与 `noframes` 进入 raw text，`textarea` 与 `title` 进入 RCDATA，其中的 `<a>` 不是锚点；`plaintext` 一直持续到文档结束。`<script/>` 在 HTML 里仍会打开该元素。页脚引用的空目标指向当前文档。行内的空链接和空图片仍是 `inline-form`。`crates/` 与 `tools/` 下名为 `tests`、`fixtures`、`evidence`、`target`、`vendor` 或 `node_modules` 的直接子目录也不作为默认来源。

当前保护的是根 README、文档首页、reference / design / ADR 索引、`AGENTS.md`、`.agents/README.md`、五个 Cursor skill 薄包装，以及 Runtime README 上已经存在的入口。`AGENTS.md`、Agent 工作流首页和 Cursor skill 这些边接受精确行内代码路径，不把它们改写成导航链接，也不扫描代码围栏里的路径。

根 README 已经链到的 crate README 只随普通链接做存在性检查，不要求每份 crate README 都写同一组“深入阅读”。

## 6. 误报与能力边界

可见的 `skip` 表示这项没有完成锚点校验。它不是失败，也不是“锚点已经正确”。HTTP、mailto、rustdoc 和非 Markdown fragment 只出现在汇总计数里，避免把外部地址刷满输出。

不要用配置文件掩盖存量错误。默认全量扫描目前允许有路径或锚点失败；那些结果留给后续文档删并，而不是在这次检查里逐文件忽略。

结构检查通过，只说明所覆盖的路径和锚点对得上。它不判断文档语义，不维护旧词清单，不编译示例，也不代替跨文档一致性复核。它也还不是合并门禁。

已知子集差异：

- 非 ASCII 字母不做 Unicode 小写。
- 特殊空白（如不换行空格）会变成 `-`，而不是从 slug 中删掉。
- HTML 锚点不占用重复标题的后缀计数。
- 不解析 HTML 标题，也不解析 HTML 属性中的 URL。
- 同一份 README 若被 rustdoc `include_str!` 引用，这里仍按仓库里的 Markdown 位置解释相对链接，不模拟 rustdoc 输出目录。

[validation-matrix]: validation-matrix.md
[doc-checks]: ../../xtask/data/doc-checks.toml
[documentation-policy]: ../governance/documentation-policy.md
