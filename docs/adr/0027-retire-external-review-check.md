# 0027 退役 External Review 自定义 Check，收敛五项机器门禁

**状态**: Accepted
**日期**: 2026-08-24
**适用范围**: LaneFlow 合并门禁、GitHub Ruleset / Merge Queue、Review conversation、
GitHub Actions 与 `xtask` 治理工具
**部分取代**: ADR 0026 第 5–9 节及其关于 External Review、六项 required checks、
受信审阅者名单、`H_mg` 盖章和撤销 bypass 启用顺序的后果
**关联 Issue**: [#492](https://github.com/illusion-tech/laneflow/issues/492)、
[#468](https://github.com/illusion-tech/laneflow/issues/468)、
[#493](https://github.com/illusion-tech/laneflow/issues/493)

## 背景

ADR 0026 删除 G3/G4 自然语言门禁后，引入了名为 `External Review` 的自定义 Check。
它从默认分支读取受信审阅者名单，接受非作者的 `APPROVED`、`COMMENTED` 或 PR 正文
👍，再通过 `pull_request_target`、`workflow_run` 与 `merge_group` 在 `H_pr` / `H_mg`
上发布同名 Check。

截至 2026-08-24，live `main` Ruleset 从未把 `External Review` 设为 required。实际
required status checks 已经稳定为五项，Merge Queue 与
`required_review_thread_resolution: true` 也已启用。自定义 Check 因而只是持续运行的
非阻断信号，并不等价于 GitHub 原生 required approval：`COMMENTED` 或正文 👍 也能
通过它。

该信号需要两个 workflow、受信名单、`xtask` evaluator / publisher、CI 路径分流、
模板、治理文档、Skills 与术语共同维护。其维护面和误导成本高于当前收益。

## 决策

### 1. 完整退役自定义 External Review 链路

删除：

- `.github/workflows/external-review.yml`；
- `.github/workflows/external-review-signal.yml`；
- `.github/trusted-reviewers.json`；
- `xtask` 的 `check-external-review`、`publish-external-review-check` 及实现和测试；
- CI paths-filter、PR 模板、现行治理文档、Skills、术语表与验证矩阵中的对应契约。

历史 Check Runs 和 ADR 0026 的历史决策文本保留，不回写或删除 GitHub 历史事实。

### 2. Required status checks 固定为五项

PR head `H_pr` 与 Merge Queue 的合并组 head `H_mg` 使用相同名称：

1. `Commit message`
2. `Rust checks`
3. `Dependency policy`
4. `Analyze (actions)`
5. `Analyze (rust)`

五项 expected source 继续绑定 GitHub Actions App `integration_id=15368`。
`Markdown tables` 继续只警告。原生 CodeQL rule 不替代 `H_mg` 上的两个 `Analyze`
status checks。

### 3. Review 使用 GitHub 原生状态，不自建计数 Check

普通 GitHub Review 继续用于协作，但项目不再用名单、Review state、reaction 或自定义
Check 判定“已有外部审阅”。Ruleset 的 required approval count 与 CODEOWNERS 要求保持
当前值 `0` / disabled；以后若要强制独立批准，必须另开治理 Issue 评估 GitHub 原生
required approvals 或 CODEOWNERS，不再恢复自定义审阅 Check。

未解决的 review conversation 继续由
`required_review_thread_resolution: true` 原生阻止入队或合并。不得在 workflow 或
`xtask` 中复制 thread 计数状态机。

### 4. Merge Queue 与身份边界

`H_pr` 是 PR 补丁身份，承载五项机器检查和 GitHub 原生协作状态；`H_mg` 是与最新
`main` 组合后的集成候选，必须重新完成同名五项机器检查。`main` 前进只重建
`H_mg`，不要求自定义人审盖章。

入队仍使用 current exact head：

```powershell
gh pr merge <number> --repo illusion-tech/laneflow --match-head-commit <H_pr>
```

不得在 checks pending 时预先武装 auto-merge，也不得日常使用 `--admin`。

### 5. Ruleset 与 bypass 边界

`External Review` 从未进入 live Ruleset required contexts，因此退役 workflow 不需要
先改 Ruleset，也不会制造缺少 required context 的锁死窗口。#492 交付前后都必须重新
读取完整 Ruleset，确认五项 required checks、Merge Queue 与
`required_review_thread_resolution: true` 未漂移。

ADR 0026 将撤销 owner bypass 与启用 `External Review` 绑定；该启用顺序由本文取代。
现有 `bypass_mode: always` 不由 #492 修改，其终态由独立 #493 评估。永久 bypass 不得
被解释为 checks 已通过，也不得成为日常合并路径。

### 6. #468 收口

#492 是 #468 的最终治理修订：#468 已交付的 G3/G4、Schema Publication、commit
message、模板和五项机器门禁成果继续有效；其未完成的 `External Review required +
撤 bypass` 目标由本文分别改为“退役自定义 Check”和“#493 独立治理”。#492 的交付 PR
同时关闭 #492 与 #468。

## 后果

- 合并门禁收敛为五项机器 Check、Merge Queue 与 GitHub 原生未解决对话阻断。
- 不再维护受信审阅者名单、reaction/review 聚合、trusted-ref publisher 或 `H_mg`
  人审盖章。
- required approvals / CODEOWNERS 没有被本决策暗中启用；未来变化需单独设计和验证。
- owner bypass 风险转入 #493，不阻塞自定义 Check 的退役。
- 产品 crate、Core API、Traffic Runtime API、数据格式和 Adapter API 均不受影响。
