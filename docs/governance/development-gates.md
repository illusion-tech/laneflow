# 开发阶段评审

**文档状态**: Active
**最后更新**: 2026-11-05

**适用范围**: LaneFlow 的需求、设计、实现、评审与合并

## 1. 目标

本文定义 LaneFlow 的轻量开发阶段评审。人流程写在 Issue 模板里，机器不解析正文。
合并由 required checks、GitHub 原生未解决对话阻断和 Merge Queue 执行。基础重建见
[ADR 0026][0026-merge-governance-rebuild]，External Review 退役与当前目标态见
[ADR 0027][0027-retire-external-review-check]。

人可读阶段：

- `G0`：立项（要不要做、做什么、不做什么）
- `G1`：设计冻结（高影响变更是否已有 ADR / design）
- `G2`：开工（实现前读过哪些文档）

合并与完成不再使用 G3 Owner comment、Gate Ledger 或 G4 档案。PR 合入且 Issue
因 `Closes #n` 关闭，即该任务完成。

## 2. 任务类别

每个 PR 应选择最接近的验证任务类别；它与 Issue Type、`area:*` 标签不是同一维度：

- `docs-only`：只改文档。
- `governance`：流程、模板、CI、项目治理。
- `core-runtime`：Traffic Runtime 运行时逻辑；该字符串是现有 PR 验证分类标识符。
- `data-spec`：lane graph、route、signal、parking 等数据格式。
- `adapter`：Unity、Unreal、Godot、O3DE、Web 等引擎适配。
- `authoring-tool`：道路、路线、停车位等编辑或转换工具。
- `example`：示例项目、示例场景或演示数据。
- `cross-layer`：同时影响 Traffic Runtime、数据格式、Adapter 或示例的高风险变更。

任务类别写在 PR 模板里给人看，不进 commit 检查。

## 3. G0 立项

拟新增或修改非 Rust 代码（纯删除除外）时，还必须在 G0 完成
[代码语言立项审查][code-language-policy]，取得覆盖该语言、用途和路径的维护者
明确接受结论后才可进入该方案的 G2。研究、测试、CI 与嵌入脚本均无默认豁免；
本项独立于 G1 是否适用。

目标：确认是否需要进入开发，以及最小交付边界是什么。

必须明确：

- 背景和使用场景
- 本次目标
- 本次明确不做
- 验收标准
- 影响范围
- 是否需要 ADR 或 design 文档

通过标准：已有 GitHub Issue，已按 `Feature` / `Bug` / `Task` 设置原生 Type，任务边界
可独立评审，验收标准可验证。Project、Labels 等侧边栏字段直接在 GitHub 上维护，
不要抄进 Issue 正文。G0 完成且 G1 已接受或不适用、至少有一个 `area:*`、没有开放
blocker 时，Project 才能进入 `Ready`。

小型 `docs-only` 或 `governance` 任务可以把 G0–G2 合成一条开工说明，但必须发生在
实现或开 PR 之前。

## 4. G1 设计冻结

以下变更必须先通过 G1：

- Traffic Runtime API 新增、删除或破坏性变更。
- 数据格式或 schema 变更。
- Adapter 协议变更。
- 运行时 tick、路线、避让、信号灯、停车系统等核心规则变更。
- 会影响多个引擎适配器的设计。

证据可以是 ADR、`docs/design/` 文档，或 Issue 中链接到正式文档的说明。不需要 G1
时，在 Issue 里写一句不适用原因即可。公开组件定位或宿主应用/交通职责边界发生实质
变化时，必须回写 ADR、architecture、roadmap、glossary 和相关 Skills，并对当前
exact head 重新取得审阅；不得沿用旧 head 的设计结论。

**1.0 正式发布前**：G1 冻的是当前规范，不是对外兼容承诺。若设计字面会逼出并行实现
或兼容层，改写 ADR/design 并只留一套。态度见
`.agents/skills/laneflow-pre-1-0/SKILL.md`。1.0 之后的对外兼容必须单开 G1。

Project 的 `Design gate` 使用 `N/A`、`Required`、`Accepted`。`Required` 表示尚未完成
设计冻结，不能进入 `Ready`；`Accepted` 必须能链接到本节允许的设计证据。

## 5. G2 开工

开工前应确认：

- Issue 的 Project Status 为 `Ready`。
- Issue 有 assignee，且没有开放的 blocked-by 依赖。
- 已阅读相关 ADR 和 design 文档。
- 已知道本次是否影响 Traffic Runtime、data spec、Adapter 或 example。
- 已知道需要补哪些测试和文档。

记录 G2 后把 Project Status 更新为 `In Progress`。如果实现中发现设计输入不稳定，
应暂停扩展实现，回到 G1 补设计或拆子任务。

### 5.1 负载敏感计量

含计量的任务在正文声明计量类别。类别按结果是否随同机负载变化判断，不按指标名判断：

| 类别     | 判定                                                                        | 要求                                                             |
| -------- | --------------------------------------------------------------------------- | ---------------------------------------------------------------- |
| 不敏感   | 结果不随同机负载变化，如摘要比较、精确字节数、口径隔离正确的分配计数        | 不需要安静测量窗口；计数受辅助线程或调度影响时修正计量口径       |
| 负载敏感 | 结果随同机负载变化，如墙钟、tick 间隔、吞吐、尾延迟、带墙钟期限的 OS 级采样 | 进入验收或正式记录时，须在安静测量窗口内运行                     |
| 探索性   | 只用于开发判断                                                              | 不受窗口约束，不作为结论引用；要进入结论，须在安静测量窗口内重测 |

- **安静测量窗口（quiet measurement window）**：维护者批准的一段独占机器的时间。窗口内
  没有其他 Agent、构建或测试任务，并记录后台进程。负载阈值由对应测量协议规定；超出时
  按协议废弃并重跑，维护者决定继续的如实记录。共享 CI runner 不构成安静测量窗口。
- **批准**：运行前说明负载、时长和运行顺序，由维护者批准。Agent 不得自行占用。
- **标签**：运行时须占用窗口的任务使用 `needs:quiet-window`。
- **拆分**：结果属于负载敏感类、进入验收或正式记录，且须绑定晚于实现合入的提交时，
  独立成测量任务。实现任务交付入口和计量方案；测量任务只运行和发布记录，不改代码。
  入口缺陷回到实现任务修复，修复后按新提交重跑。不满足这些条件的计量留在原任务。
- **冻结**：每个版本立一个冻结验收提交任务。绑定验收提交的测量任务以它为 blocker，
  冻结前留在 `Backlog`。
- **隔离**：负载敏感测量不与计数分配器或其他插桩同进程运行。
- **合跑**：一个窗口可串行运行多个测量，各自单独记录，不并发。

本节不追溯已关闭的任务。产品测量协议以
[`core-runtime-performance-baseline.md`][core-runtime-performance-baseline]
为准。

## 6. 合并检查

语言选型属于 [代码语言立项审查][code-language-policy] 规定的 G0/G2 人流程，
不增加机器合并检查输入。审阅者发现实际 diff 不符合接受范围时，以普通 review
conversation 处理，继续适用下面的原生对话解决规则。

目标：确认变更可以合入主干。

所有 PR 应说明：

- 任务类别
- 关联 Issue（body 使用 `Closes #<issue>`）
- 本次变更范围与明确不做
- 文档更新情况
- 测试与验证结果
- 已知风险

按任务类别追加：

- `docs-only`：说明无运行时行为变更。
- `governance`：说明影响的流程和模板。
- `core-runtime`：提供单元测试、确定性行为说明或未覆盖原因。
- `data-spec`：说明兼容性、版本影响和示例数据影响。
- `adapter`：说明引擎边界、Traffic Runtime 依赖方向和手工验证结果。
- `example`：说明示例运行方式和覆盖能力。
- `cross-layer`：说明端到端路径、回归风险和是否需要示例 smoke test。

安全设置、扫描 workflow、依赖策略或公开发布相关变更还必须按
`security-scanning.md` 记录适用扫描状态；涉及许可证、Cargo 依赖、cargo-deny 或
Dependabot 时还必须满足 `dependency-security.md`。

### 6.1 Review conversation

普通 GitHub Review 继续用于协作，但不由自定义 Check、受信名单或 reaction 计数。
当前 Ruleset 不要求固定批准数或 CODEOWNERS review；以后若要强制独立批准，必须先
通过治理 Issue 选择 GitHub 原生 required approvals / CODEOWNERS。

未解决的 review conversation 由 GitHub Ruleset
`required_review_thread_resolution: true` 阻止入队。不得在 workflow 或 `xtask` 中
复制 thread 计数状态机。fork / cross-repository PR 必须把最终 patchset 迁到同仓 PR。

### 6.2 Required checks

PR 与 `merge_group` 使用相同的九个 required check 名称：

1. `Commit message`
2. `Rust checks`
3. `Evidence fixture tests`
4. `Wire toolchain audit`
5. `Placement fixture tests`
6. `Urban harness tests`
7. `Dependency policy`
8. `Analyze (actions)`
9. `Analyze (rust)`
`Markdown tables` 只警告。Schema publication 不参与合并检查。

### 6.3 入队

`main` 使用 Merge Queue，最终 **Rebase**。入队前冻结 current exact head `H_pr`：

```powershell
gh pr merge <number> --repo illusion-tech/laneflow --match-head-commit <H_pr>
```

不得在 checks pending 时预先武装 auto-merge。禁止日常 `--admin`。`H_pr` 变了必须
重跑适用检查；`main` 前进只重建 `H_mg` 并重跑机器检查。队列在 `H_mg` 上需要同名
九项绿。owner bypass 的终态由 #493 独立治理。

## 7. 完成

默认一个 Issue 一个完成它的 PR。合并后 GitHub 按 `Closes #<issue>` 关闭 Issue。
未完成的验收拆 follow-up Issue，不要留着父 Issue 等第二份档案。负载敏感计量的拆分条件
见 §5.1。

Project 将以 `Completed` 关闭的 Issue 移到 `Done`；以 `Not planned` 或 Duplicate
关闭的 Issue 移到 `Cancelled`。不要求手写 G4 JSON，也不运行 `check-gate-evidence`。

## 8. 例外

内容等价 rebase、provider / 平台故障、安全或紧急 hotfix 只能走显式、有期限的
仓库设置例外，并记录原因、风险、到期和 Cleanup owner。不得把例外扩展成日常
bypass。确认的检查缺陷另开 Issue 修复，不在评论里发明第二套协议。

[0026-merge-governance-rebuild]: ../adr/0026-merge-governance-rebuild.md
[0027-retire-external-review-check]: ../adr/0027-retire-external-review-check.md
[code-language-policy]: code-language-policy.md
[core-runtime-performance-baseline]: ../design/core-runtime-performance-baseline.md
