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
7. 相关的 `docs/design/` 与 `docs/adr/` 文档
8. 涉及 #291 目标静态路网编译、静态镜像（Static Image）、静态执行约束/分区规划
   提示/运行时执行计划（Static Execution Constraint / Partition Planning Hints /
   Runtime Execution Plan），或 `laneflow-core/CoreWorld` →
   `laneflow-runtime/TrafficWorld` 的 #294 生产切换时，读取
   `docs/adr/0020-compiler-owned-static-network-and-static-image.md` 与
   `docs/design/network-compiler.md`
9. 涉及 #308 编译器工作负载、资源/性能预算校准、研究停止护栏或私有容器候选时，
   读取 `docs/design/compiler-budget-calibration.md`、
   `docs/reference/compiler-calibration-workloads-v1.json` 与
   `docs/reference/compiler-calibration-evidence-v1.schema.json`、
   `docs/reference/compiler-calibration-contract-v1.json`；该研究代码只允许
   在 #308 自身 G2 后进入 `research/` 非生产边界
10. 涉及 #292、#315、#296、#297、`laneflow-static-contract`、
    `laneflow-compiler`、官方前端共同受检模块接入、合成领域专用语言前端
    （Synthetic DSL Frontend）、几何文档前端、current JSON 退役或集成专用
    LIR→当前态投影时，额外读取
   `docs/design/compiler-foundation.md`；准备 #292 G3 或复核生产性能证据时，同时读取
   `docs/reference/v0.10-compiler-foundation-validation.md`
11. 涉及城市模拟游戏范围、Routing、路网修订、存档/回放、并行或 fidelity 时读取
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
7. PR 准备合并时，默认使用 **Rebase and merge**（`gh pr merge <number> --rebase`），除非 PR 中已说明例外。

## 规则

- 不要把引擎相关依赖引入当前 Core 或目标 Traffic Runtime。
- 当前态使用 `LaneFlow Core` / `laneflow-core` / `CoreWorld`；#291 目标态使用中文
  规范名“LaneFlow 交通运行时”及精确标识符 `laneflow-runtime` / `TrafficWorld`。
  在 ADR 0020 完成 Accepted 与阶段 8 生产切换 Issue #294 完成 G4 前，不得把目标态
  写成当前 API。
- 不要在不更新 design 文档的情况下改变数据格式语义。
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
  partition/worker、world seed 或 runtime snapshot 写入共享静态镜像。
- 精确并行路径不得用分区导致的额外一 tick 延迟（Partition-induced Extra-tick
  Delay）换吞吐；Adapter LOD 和多世界吞吐也不能冒充 Traffic Runtime 保真度或
  单世界扩展。
- Rust 数字字面量等仓库级可读性规则只应用于本次触及范围；历史格式问题应单独跟踪。
- 不要在只完成子切片时声称父任务已完成。
- 不要隐瞒未运行的检查；说明未运行项及原因。
- 提交标题使用 Conventional Commits，提交正文保留 `Gate`、`Slice`、`Impact`、`Scope`、`Validation`、`Docs` 与 `Refs/Closes`，详见 `docs/reference/commit-convention.md`。

## 交付说明

完成实现工作后，应汇报：

- 变更摘要
- 对当前态 Core API、目标态 Traffic Runtime API、数据格式、Adapter API 的影响
- 已运行的验证
- 文档是否更新或为何无需更新
- 剩余风险或后续 Issue
- 建议的 PR 合并方式（默认 Rebase and merge）
