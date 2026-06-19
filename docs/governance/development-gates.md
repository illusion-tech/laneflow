# 开发闸口

**文档状态**: Active  
**最后更新**: 2026-06-18  
**适用范围**: LaneFlow 的需求、设计、实现、评审与完成治理

## 1. 目标

本文定义 LaneFlow 的轻量开发闸口，避免 Core、数据格式、Adapter 和示例在没有统一输入的情况下各自漂移。

LaneFlow 采用五个闸口：

- `G0`：立项
- `G1`：设计冻结
- `G2`：开工
- `G3`：合并
- `G4`：完成

## 2. 切片类型

每个 Issue 或 PR 应选择最接近的切片类型：

- `docs-only`：只改文档。
- `governance`：流程、模板、CI、项目治理。
- `core-runtime`：LaneFlow Core 运行时逻辑。
- `data-spec`：lane graph、route、signal、parking 等数据格式。
- `adapter`：Unity、Unreal、Godot、O3DE、Web 等引擎适配。
- `authoring-tool`：道路、路线、停车位等编辑或转换工具。
- `example`：示例项目、示例场景或演示数据。
- `cross-layer`：同时影响 Core、数据格式、Adapter 或示例的高风险变更。

## Gate Ledger 增量记录

Gate Ledger 是 Issue 和 PR 上的增量闸口记录，用来说明任务何时通过了 G0-G4。它不是 G4 清场时的一次性补档。

通用规则：

- 每次任务跨过一个 Gate，都应在对应载体留下记录。
- G0、G1、G2 记录在 Issue 中；G3 证据记录在 PR 中，Issue Gate Ledger 只勾选或链接该 PR 判断；G4 回写 Issue。
- 小型 `docs-only` 或 `governance` 任务可以把 G0-G2 合并为一条开工记录，但该记录必须发生在实现或开 PR 之前。
- 如果 G4 阶段才发现 G0-G3 缺失，只能标记为补救记录，并说明流程遗漏原因。
- Agent 不得在缺少当前 Gate 记录时继续推进下一 Gate，除非用户明确接受例外并留下原因、风险和 Cleanup owner。

推荐 Issue Gate Ledger：

```text
- [ ] G0 立项已记录：
- [ ] G1 设计判断已记录：
- [ ] G2 开工判断已记录：
- [ ] G3 合并判断已记录：链接 PR 中的 G3 判断
- [ ] G4 完成判断已记录：回写合并后收口结果
```

## 3. G0 立项闸口

目标：确认是否需要进入开发，以及最小交付边界是什么。

必须明确：

- 背景和使用场景
- 本次目标
- 本次明确不做
- 验收标准
- 影响范围
- 是否需要 ADR 或 design 文档
- 是否需要拆分子 Issue

通过标准：

- 已有 GitHub Issue。
- 任务边界足够小，可以独立评审。
- 验收标准可验证。
- Issue Gate Ledger 中已有 G0 记录。

## 4. G1 设计冻结闸口

目标：确认实现前的正式输入已经足够稳定。

以下变更必须先通过 G1：

- Core API 新增、删除或破坏性变更。
- 数据格式或 schema 变更。
- Adapter 协议变更。
- 运行时 tick、路线、避让、信号灯、停车系统等核心规则变更。
- 会影响多个引擎适配器的设计。

G1 证据可以是：

- ADR
- `docs/design/` 文档
- Issue 中链接到正式文档的冻结评论

通过标准：

- 设计输入清楚。
- 非目标清楚。
- 兼容性影响清楚。
- 测试要求清楚。
- Issue Gate Ledger 中已有 G1 记录；不需要 G1 时，也应记录不适用原因。

## 5. G2 开工闸口

目标：确认实现者不是边做边猜。

开工前应确认：

- Issue 状态为 `Ready` 或等价状态。
- 已阅读相关 ADR 和 design 文档。
- 已知道本次是否影响 Core、data spec、Adapter 或 example。
- 已知道需要补哪些测试和文档。
- Issue Gate Ledger 中已有 G2 记录。

如果实现中发现设计输入不稳定，应暂停扩展实现，回到 G1 补设计或拆子切片。

## 6. G3 合并闸口

目标：确认变更可以合入主干。

所有 PR 必须说明：

- 切片类型
- 关联 Issue
- 本次变更范围
- 本次明确不做
- 文档更新情况
- 测试与验证结果
- 已知风险与例外

按切片类型追加要求：

- `docs-only`：说明无运行时行为变更。
- `governance`：说明影响的流程和模板。
- `core-runtime`：提供单元测试、确定性行为说明或未覆盖原因。
- `data-spec`：说明兼容性、版本影响和示例数据影响。
- `adapter`：说明引擎边界、Core 依赖方向和手工验证结果。
- `example`：说明示例运行方式和覆盖能力。
- `cross-layer`：说明端到端路径、回归风险和是否需要示例 smoke test。

默认阻断条件：

- Adapter 代码把引擎依赖泄漏进 Core。
- 数据格式变化没有文档说明。
- Core API 破坏性变化没有 ADR 或 design 依据。
- PR 声称完成父任务，但实际只完成子范围。
- 必需测试未运行且没有原因。
- 例外没有清理责任或后续 Issue。
- 缺少 G0-G2 Gate Ledger，且没有记录为显式例外或补救。

PR 合入 `main` 默认使用 **Rebase and merge**；若使用 Squash 或 Merge commit，须在 PR 中说明原因。详见 `github-workflow.md` 第 7 节。

G3 记录应写在 PR 描述或 PR 评论中，至少包含 checks、review 状态、验证结果、风险、例外和合并方式。

## 7. G4 完成闸口

目标：确认 `Done` 代表后续任务可以依赖。

Issue 关闭前必须满足：

- 关联 PR 已按默认策略（Rebase and merge）合并，或说明为什么无需 PR / 为什么使用其他合并方式。
- 验收 checklist 已完成。
- 文档已回写，或说明不需要。
- 测试和验证结果已记录。
- 未完成范围已拆出后续 Issue。
- 父 Issue 只在所有子 Issue 完成后关闭。
- G4 记录已回写关联 Issue。
- Project 中关联 Issue 和 PR 均已移动到 `Done`，或说明为什么不适用。
- 本地和远端 PR 分支已清理，或说明保留原因。
- 临时权限、ruleset bypass 或 admin override 已撤回，或说明保留原因、风险和 Cleanup owner。

G4 记录只负责最终闭环；不应在 G4 阶段首次补写 G0-G3。若必须补写，应标记为补救记录。

## 8. 例外治理

允许例外，但必须显式留痕。

例外记录至少包含：

- 原因
- 风险范围
- 临时接受边界
- 后续清理 Issue
- Cleanup owner

不得用“后面再补”替代例外记录。
