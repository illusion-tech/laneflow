---
name: laneflow-development
description: 指导 LaneFlow 的 AI Agent 实现工作。适用于功能实现、缺陷修复、测试更新、当前 Core/目标 Traffic Runtime 代码变更、数据格式修改或准备实现 PR。
---

# LaneFlow 开发

## 先读这些

1. `README.md`
2. `docs/governance/agent-development-guide.md`
3. `docs/governance/development-gates.md`
4. `docs/reference/commit-convention.md`
5. `docs/reference/glossary.md`
6. 修改 Rust 时读取 `docs/reference/rust-code-style.md`
7. 相关的 `docs/design/` 与 `docs/adr/` 文档；治理交付同时读取
   `docs/adr/0027-retire-external-review-check.md`
8. 涉及 #291 目标静态路网编译、#300 共享静态路网、静态执行约束/分区规划
   提示/运行时执行计划（Static Execution Constraint / Partition Planning Hints /
   Runtime Execution Plan），或 `laneflow-runtime/TrafficWorld` 时，读取
   `docs/adr/0020-compiler-owned-static-network-and-static-image.md`、
   `docs/adr/0025-checked-canonical-network-and-shared-static-network.md`、
   `docs/design/network-compiler.md`、`docs/design/shared-static-network.md` 与
   `docs/design/traffic-runtime-shared-consumption.md`
9. 涉及已关闭的 #308 编译器预算校准时，只读
   `docs/design/compiler-budget-calibration.md` 的短结论。查阅当时制品使用 G4
   精确证据提交 `de4cd460a96415cafbd811141568b81f74d73534` 与交付 PR #310。不得把该
   研究重新加回生产 crate 或工作区成员
10. 涉及 #292、#315、#296、#297、`laneflow-static-contract`、
    `laneflow-compiler`、官方前端共同受检模块接入、合成领域专用语言前端
    （Synthetic DSL Frontend）、道路编辑前端、current JSON 退役或集成专用
    LIR→当前态投影时，额外读取
   `docs/design/compiler-foundation.md`；#296 还须读取
   `docs/adr/0023-road-editing-state-and-phased-network-replacement.md` 与
   `docs/design/road-editing-source-and-geometry-frontend.md`；复核生产编译性能时读取
   `docs/reference/v0.10-compiler-production-baseline.md`
11. 涉及 #298、`laneflow-format`、可移植规范制品、源映射封套、语义差异封套、
    规范发布描述符、逐字节确定性或原子发布时，额外读取
    `docs/design/portable-canonical-artifact.md` 与
    `docs/design/compiler-post-emission-check-and-minimal-publication-closure.md`
12. 涉及城市模拟游戏范围、Routing、路网修订、存档/回放、并行或 fidelity 时读取
   `docs/adr/0021-city-simulation-game-traffic-foundation.md`

若任务涉及当前态 Core API、目标态 Traffic Runtime API、数据格式或 Adapter API，
但缺少相关设计输入，应先停止实现并提出 G1 设计缺口。

## 工作流

1. 确认切片类型与影响范围。
2. 修改前先阅读现有代码与测试。
3. 变更范围限定在 Issue 或用户请求内。
4. 若改变长期行为或契约，同步更新文档。
5. 按切片类型运行对应检查。
6. 记录验证结果、文档状态与剩余风险。
7. PR 准备合并时，冻结 current exact head `H_pr` 并完成该 head 的 required checks 与适用 CodeQL；
   未解决 review conversation 由 GitHub 原生规则阻断。全部 success 后才可运行入队命令，不得在 pending 时预先武装 auto-merge。`main` 要求通过
   Merge Queue 入队，使用 `gh pr merge <number> --repo illusion-tech/laneflow --match-head-commit <H_pr>`，
   最终 Rebase 方式由队列规则控制。不得因 `main` 单纯前进手工 rebase；不得使用 `--admin` 绕过队列。
   队列 live Ruleset 必须保留 required checks，并关闭 `strict_required_status_checks_policy`；集成结果由
   GitHub 为最新目标分支组合生成的 `H_mg` 承载。`H_pr` 与真实 `H_mg` 都必须完成
   `Commit message`、`Rust checks`、`Dependency policy`、`Analyze (actions)`、`Analyze (rust)`；
   CodeQL 两项必须来自 GitHub Actions App `integration_id=15368`，不得用原生 CodeQL rule 或其他 SHA 补足。

### Rust 语义导航

修改 Rust 前，按需使用下列方式缩小源码阅读范围：

- 可用时，先使用基于 `rust-analyzer` 的只读语义工具定位定义、活动配置引用、类型、
  符号与诊断；使用 mcpls 时读取 `docs/reference/mcpls.md`。
- 使用 mcpls 处理当前工作区前，先通过工作区自有的已知符号确认 server 指向当前
  worktree；标准库、工具链或依赖的合法定义路径可以位于 worktree 外。
- 冷启动、空结果或错误不得解释为“没有定义/引用”；等待已知工作区符号可返回，
  否则回退到 `rg` 与源码阅读。
- 继续使用 `rg` 覆盖字面量、配置、注释、未启用 feature 和其他文本搜索。
- 分别使用 `cargo check` 与 `cargo test` 判断最终的编译与测试正确性；语义诊断只作
  快速反馈。

## 规则

- 1.0 正式发布前遵守 `.agents/skills/laneflow-pre-1.0/SKILL.md`。
- 不要把引擎相关依赖引入当前 Core 或目标 Traffic Runtime。
- 可运行世界使用中文规范名“LaneFlow 交通运行时”及精确标识符
  `laneflow-runtime` / `TrafficWorld`。`laneflow-core` / `CoreWorld` 与 current
  JSON 入口已拆除，不得再当作正式入口。契约见
  `docs/design/traffic-runtime-shared-consumption.md`。
- 不要在不更新 design 文档的情况下改变数据格式语义。静态数据现行路径是 LFCA /
  共享静态路网，不是已删除的 current JSON。
- 长期文档中的领域术语必须中文权威、英文辅助；新术语先补
  `docs/reference/glossary.md`，代码和协议标识符保留精确原文。
- 面向读者的数量必须让数量级、单位和计数对象清楚且无歧义；零较多的中文正文建议
  使用规范中文数量，表格、公式和机器输入建议使用完整十进制数字。上下文明确时
  `k`/`M` 等缩写或 `1 万` 等写法可以保留，仅作可读性与一致性建议，不设自动门禁；
  小写 `m` 应特别避免与米混淆。交通仿真规模不使用 `Agent` 或“代理”计数。长期
  通用规模使用交通参与单元并按交通执行域分解；当前 `Vehicle*` API、车辆 workload
  与历史证据保持车辆特化标注，不得冒充其他执行域。不可改的代码、测试、文件和
  协议标识符保持原文并用代码格式标明。
- 不要把无关重构与功能开发混在同一 PR。
- 不要把城市经济、出行需求或路线选择策略隐藏进 Traffic Runtime；不要把最终
  partition/worker、world seed、runtime snapshot 或 working/candidate 道路编辑状态
  写入共享静态路网。共享静态路网不是文件 ABI、磁盘缓存或存档对象。
- 精确并行路径不得用分区导致的额外一 tick 延迟（Partition-induced Extra-tick
  Delay）换吞吐；Adapter LOD 和多世界吞吐也不能冒充 Traffic Runtime 保真度或
  单世界扩展。
- Rust 数字字面量等仓库级可读性规则只应用于本次触及范围；历史格式问题应单独跟踪。
- 不要在只完成子切片时声称父任务已完成。
- 不要隐瞒未运行的检查；说明未运行项及原因。
- 提交标题使用 Conventional Commits；scope 可省略，使用时表示主要受影响的组件或职责域，不等同于 PR 切片类型。footer 使用 `Refs` / `Closes`；标题带 `!` 时必须有 `BREAKING CHANGE:`。详见 `docs/reference/commit-convention.md`。

## 交付说明

完成实现工作后，应汇报：

- 变更摘要
- 对当前态 Core API、目标态 Traffic Runtime API、数据格式、Adapter API 的影响
- 已运行的验证
- 文档是否更新或为何无需更新
- 剩余风险或后续 Issue
- PR 合并路径（默认 Merge Queue，队列最终 Rebase）与 `H_pr/H_mg/H_main` 状态
