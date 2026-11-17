+++
schema_version = 1
id = "reference-glossary"
kind = "reference"
status = "active"
revised_date = "2026-11-17"
summary = "按概念统一中文术语、英文辅助名、符号与旧称查询。"
scope = "LaneFlow 跨文档规范术语与概念关系"
related = ["governance-terminology-policy", "reference-terminology-catalog", "reference-wording-guide"]
+++

# LaneFlow 双语术语表

按领域查找规范概念，或用中文名、英文辅助名和代码标识符搜索。一个概念只有一份
定义；其他领域通过链接引用。同名的历史概念列在“停止采用的概念”，旧称列在
“旧称查询”，都不能据此推断现行 API 或能力。
每个领域先列出中英文与标识符的短索引表，点击中文名进入完整定义、适用边界和依据。

词条正本为 `docs/reference/terms/` 中的 TOML，本页正文由 `generate-glossary`
生成。命名、简称、数量与历史豁免见[语言与术语政策][policy]，条件化改写见
[措辞指南][wording]，数据结构与维护命令见[术语目录规范][catalog]。

词条不维护审批或实现进度。定义中的技术细节以所链接责任文档为准；引用文档的
接受状态不代表概念已经实现。当前道路机动车特化、长期执行域与历史证据边界
仍须按各词条适用范围解释。

[policy]: ../governance/terminology-policy.md
[wording]: wording-guide.md
[catalog]: terminology-catalog.md

<!-- generated-terminology:start -->

按领域浏览：

- [治理与文档][domain-governance]
- [编制与编译][domain-compiler]
- [标识与数据布局][domain-identity]
- [静态路网][domain-static-network]
- [制品与验证][domain-artifacts]
- [交通运行时][domain-runtime]
- [宿主与世界管理][domain-host]
- [空间与几何][domain-spatial]
- [引擎适配][domain-adapter]
- [测量与记录][domain-measurement]
- [测试与检查][domain-testing]
- [停止采用的概念][retired-concepts]
- [旧称查询][former-names]

<a id="domain-governance"></a>
## 治理与文档

点击中文规范名查看完整定义；跨领域概念指向同一词条。

| 中文规范名                                             | 英文辅助名                    | 精确标识符                     |
| ------------------------------------------------------ | ----------------------------- | ------------------------------ |
| [领域标签][term-area-label]                            | area label                    | ` area:* `                     |
| [最终裁决职责][term-authority]                         | authority                     | —                              |
| [真实阻塞依赖][term-blocking-dependency]               | blocking dependency           | ` blocked-by / blocking `      |
| [一次性不兼容切换][term-clean-break]                   | clean break                   | —                              |
| [代码语言立项审查][term-code-language-proposal-review] | code language proposal review | —                              |
| [提交执行框架标识][term-commit-harness-identifier]     | commit harness identifier     | ` Harness `                    |
| [当前态][term-current-state]                           | current state                 | ` current `                    |
| [设计评审字段][term-design-gate-field]                 | design gate field             | ` Design gate `                |
| [开发阶段评审][term-development-gate]                  | development gate              | ` G0–G2 `                      |
| [入口形态][term-harness-surface]                       | harness surface               | —                              |
| [集成专用桥][term-integration-only-bridge]             | integration-only bridge       | —                              |
| [Issue 类型][term-issue-type]                          | issue type                    | ` Feature `、` Bug `、` Task ` |
| [合并组][term-merge-group]                             | merge group                   | ` merge_group `                |
| [合并队列][term-merge-queue]                           | merge queue                   | ` GitHub Merge Queue `         |
| [迁移预言机][term-migration-oracle]                    | migration oracle              | —                              |
| [生产切换][term-production-cutover]                    | production cutover            | —                              |
| [生产路径][term-production-path]                       | production path               | —                              |
| [项目状态][term-project-status]                        | project status                | ` Status `                     |
| [唯一事实来源][term-single-source-of-truth]            | single source of truth        | ` SSOT `                       |
| [目标态][term-target-state]                            | target state                  | ` target `                     |
| [任务类别][term-task-category]                         | task category                 | —                              |
| [仅遥测][term-telemetry-only]                          | telemetry-only                | —                              |
| [工作方式标签][term-work-mode-label]                   | work-mode label               | ` work:* `                     |

<a id="term-area-label"></a>
### 领域标签

英文辅助名：area label。

表达 Issue 或 PR 影响领域的 GitHub Label；跨层工作使用多个领域标签，进入 `Ready` 前至少设置一个。

精确标识符：` area:* `（记号）。

依据：[GitHub 工作流][source-governance-github-workflow-36]。

<a id="term-authority"></a>
### 最终裁决职责

英文辅助名：authority。

对某类事实作最终裁决并承担一致性责任的组件职责。

依据：[路网编译器与共享静态路网][source-design-network-compiler-37]。

<a id="term-blocking-dependency"></a>
### 真实阻塞依赖

英文辅助名：blocking dependency。

GitHub 原生 Issue 依赖关系，表示某项工作实际阻止另一项工作前进；父子分解本身不自动形成阻塞。

精确标识符：` blocked-by / blocking `（记号）。

依据：[GitHub 工作流][source-governance-github-workflow-36]。

<a id="term-clean-break"></a>
### 一次性不兼容切换

英文辅助名：clean break。

不保留长期双 API 的显式破坏性切换。

依据：[路网编译器与共享静态路网][source-design-network-compiler-37]。

<a id="term-code-language-proposal-review"></a>
### 代码语言立项审查

英文辅助名：code language proposal review。

新增或修改拟入库非 Rust 代码前，对语言必要性、用途、工具链、验证与维护范围作出明确接受结论的人流程；纯删除不适用。

依据：[代码语言立项审查][source-governance-code-language-policy-38]。

<a id="term-commit-harness-identifier"></a>
### 提交执行框架标识

英文辅助名：commit harness identifier。

提交归属的 AI 执行产品及入口形态的已登记标识，不表示模型身份。

精确标识符：` Harness `（字段）。

依据：[提交规范][source-reference-commit-convention-39]。

<a id="term-current-state"></a>
### 当前态

英文辅助名：current state。

当前已实现、并受现有 ADR/API 约束的唯一可运行生产事实。生产切换完成后由新路径承担，不得再把已拆除路径写成当前态。

精确标识符：` current `（记号）。

依据：[路网编译器与共享静态路网][source-design-network-compiler-37]。

<a id="term-design-gate-field"></a>
### 设计评审字段

英文辅助名：design gate field。

Project 单选字段，取值为 `N/A`、`Required`、`Accepted`；它索引 G1 判断，不替代 ADR、design 或 Issue 中的设计证据。

精确标识符：` Design gate `（记号）。

依据：[GitHub 工作流][source-governance-github-workflow-36]。

<a id="term-development-gate"></a>
### 开发阶段评审

英文辅助名：development gate。

LaneFlow 从立项、设计冻结到开工的人流程；合并由 required checks、GitHub 原生未解决对话阻断与 Merge Queue 承担。

精确标识符：` G0–G2 `（记号）。

依据：[GitHub 工作流][source-governance-github-workflow-36]。

<a id="term-harness-surface"></a>
### 入口形态

英文辅助名：harness surface。

提供任务交互和上下文集成的产品入口类别；不按底层进程或执行位置分类。

依据：[提交规范][source-reference-commit-convention-39]。

<a id="term-integration-only-bridge"></a>
### 集成专用桥

英文辅助名：integration-only bridge。

只服务迁移验证、不属于生产 API，并在切换后删除的桥接实现。

依据：[路网编译器与共享静态路网][source-design-network-compiler-37]。

<a id="term-issue-type"></a>
### Issue 类型

英文辅助名：issue type。

GitHub 原生的工作性质字段：功能、缺陷或任务；不由标题前缀或同义标签重复表达。

精确标识符：` Feature `（记号）、` Bug `（记号）、` Task `（记号）。

依据：[GitHub 工作流][source-governance-github-workflow-36]。

<a id="term-merge-group"></a>
### 合并组

英文辅助名：merge group。

合并队列为最新默认分支、队列前序变更与当前 PR 生成的临时集成候选；其 Head SHA 承载队列级机器检查。

精确标识符：` merge_group `（字段）。

依据：[GitHub 工作流][source-governance-github-workflow-36]。

<a id="term-merge-queue"></a>
### 合并队列

英文辅助名：merge queue。

GitHub 对已满足 PR 级要求的变更按队列顺序创建合并组、验证最新默认分支组合并在成功后执行合并的平台机制。

精确标识符：` GitHub Merge Queue `（记号）。

依据：[GitHub 工作流][source-governance-github-workflow-36]。

<a id="term-migration-oracle"></a>
### 迁移预言机

英文辅助名：migration oracle。

用于对照目标实现语义、行为或字节结果的独立基准实现。

依据：[路网编译器与共享静态路网][source-design-network-compiler-37]。

<a id="term-production-cutover"></a>
### 生产切换

英文辅助名：production cutover。

目标路径替代当前生产路径，成为仓库内唯一可运行世界。不以 current/target 等价对照为前提，也不以后继性能、规模或安全认证通过为前提；那些认证针对已切换路径本身。

依据：[路网编译器与共享静态路网][source-design-network-compiler-37]。

<a id="term-production-path"></a>
### 生产路径

英文辅助名：production path。

被正式产品入口使用、受现行验证与治理约束的执行路径；对外兼容义务按项目发布阶段确定。

依据：[路网编译器与共享静态路网][source-design-network-compiler-37]。

<a id="term-project-status"></a>
### 项目状态

英文辅助名：project status。

表达 Issue 当前生命周期阶段的 Project 字段；阻塞由原生依赖关系表达，不设置 `Blocked` 状态。

精确标识符：` Status `（字段）。

依据：[GitHub 工作流][source-governance-github-workflow-36]。

<a id="term-single-source-of-truth"></a>
### 唯一事实来源

英文辅助名：single source of truth。

同一事实只由一个规范来源拥有，派生物不得反向与正本竞争。

精确标识符：` SSOT `（缩写）。

依据：[路网编译器与共享静态路网][source-design-network-compiler-37]。

<a id="term-target-state"></a>
### 目标态

英文辅助名：target state。

已经由相应 G1 或 ADR 接受、但尚未完成生产切换的设计事实；生产切换完成后成为当前态。

精确标识符：` target `（记号）。

相关概念：[当前态][term-current-state]、[生产切换][term-production-cutover]。

依据：[路网编译器与共享静态路网][source-design-network-compiler-37]。

<a id="term-task-category"></a>
### 任务类别

英文辅助名：task category。

PR 按工作性质归入的验证维度：`docs-only`、`governance`、`core-runtime`、`data-spec`、`adapter`、`authoring-tool`、`example`、`cross-layer`；类别值保留英文原文，与 Issue Type、`area:*` 标签不是同一维度。

依据：[GitHub 工作流][source-governance-github-workflow-36]。

<a id="term-telemetry-only"></a>
### 仅遥测

英文辅助名：telemetry-only。

只记录或观察状态、不发布 required 结论也不更新既有检查结论的运行方式。

依据：[GitHub 工作流][source-governance-github-workflow-36]。

<a id="term-work-mode-label"></a>
### 工作方式标签

英文辅助名：work-mode label。

表达设计、调研、验证或安全等工作方式的 GitHub Label；不表达生命周期状态。

精确标识符：` work:* `（记号）。

依据：[GitHub 工作流][source-governance-github-workflow-36]。

<a id="domain-compiler"></a>
## 编制与编译

点击中文规范名查看完整定义；跨领域概念指向同一词条。

| 中文规范名                                                                               | 英文辅助名                                                | 精确标识符                     |
| ---------------------------------------------------------------------------------------- | --------------------------------------------------------- | ------------------------------ |
| [制品结构统计][term-artifact-shape]                                                      | artifact shape                                            | —                              |
| [数据编制][term-authoring]                                                               | authoring                                                 | —                              |
| [编制命名空间标识][term-authoring-namespace-id]                                          | authoring namespace ID                                    | ` authoringNamespaceId `       |
| [正本来源模块图][term-authoritative-source-module-graph]                                 | authoritative source module graph                         | —                              |
| [后端][term-backend]                                                                     | backend                                                   | —                              |
| [基础构造][term-base-construction]                                                       | base construction                                         | ` HirBase `                    |
| [行为等价][term-behavior-equivalence]                                                    | behavior equivalence                                      | —                              |
| [构建计划][term-build-plan]                                                              | build plan                                                | ` HirBuildPlan `               |
| [构建器][term-builder]                                                                   | builder                                                   | —                              |
| [缓存][term-cache]                                                                       | cache                                                     | —                              |
| [候选道路修订][term-candidate-road-revision]                                             | candidate road revision                                   | —                              |
| [规范路网容量][term-canonical-network-capacity]                                          | canonical network capacity                                | —                              |
| [规范路网规模][term-canonical-network-usage]                                             | canonical network usage                                   | —                              |
| [规范排列][term-canonical-orders]                                                        | canonical orders                                          | ` CanonicalOrders `            |
| [规范关系出现记录][term-canonical-relation-occurrence]                                   | canonical relation occurrence                             | ` CanonicalRelationCount `     |
| [干净编译][term-clean-compile]                                                           | clean compile                                             | —                              |
| [干净单工作线程编译][term-clean-single-thread-compile]                                   | clean single-thread compile                               | —                              |
| [封闭契约][term-closed-contract]                                                         | closed contract                                           | —                              |
| [冷实例编译][term-cold-instance-compile]                                                 | cold-instance compile                                     | —                              |
| [编译观测值][term-compilation-metrics]                                                   | compilation metrics                                       | ` CompilationMetrics `         |
| [编译单元][term-compilation-unit]                                                        | compilation unit                                          | —                              |
| [编译资源上限][term-compile-limits]                                                      | compile limits                                            | ` CompileLimits `              |
| [配置档资格验证][term-compile-limits-profile-qualification]                              | compile limits profile qualification                      | —                              |
| [配置档校验][term-compile-limits-profile-validation]                                     | compile limits profile validation                         | —                              |
| [编译器校准规模][term-compiler-calibration-scale]                                        | compiler calibration scale                                | —                              |
| [编译器控制总存续内存][term-compiler-controlled-total-live-memory]                       | compiler-controlled total live memory                     | —                              |
| [编译发射器][term-compiler-emitter]                                                      | compiler emitter                                          | ` emitter `                    |
| [编译器基础设施][term-compiler-foundation]                                               | compiler foundation                                       | —                              |
| [编译器中间表示][term-compiler-intermediate-representation]                              | compiler intermediate representation                      | ` compiler IR `                |
| [编译器拥有的静态路网][term-compiler-owned-static-network]                               | compiler-owned static network                             | —                              |
| [编译遍][term-compiler-pass]                                                             | compiler pass                                             | ` pass `                       |
| [编译器资源账本][term-compiler-resource-ledger]                                          | compiler resource ledger                                  | —                              |
| [编译器压力规模][term-compiler-stress-scale]                                             | compiler stress scale                                     | —                              |
| [包依赖图][term-crate-dependency-graph]                                                  | crate dependency graph                                    | ` crate DAG `                  |
| [确定性][term-determinism]                                                               | determinism                                               | —                              |
| [确定性合并顺序][term-deterministic-merge-order]                                         | deterministic merge order                                 | —                              |
| [确定性预言机][term-deterministic-oracle]                                                | deterministic oracle                                      | —                              |
| [诊断][term-diagnostic]                                                                  | diagnostic                                                | —                              |
| [领域预算][term-domain-budget]                                                           | domain budget                                             | ` DomainBudget `               |
| [编辑器编制界面][term-editor-authoring-surface]                                          | editor authoring surface                                  | —                              |
| [特性位][term-feature-bits]                                                              | feature bits                                              | —                              |
| [指纹][term-fingerprint]                                                                 | fingerprint                                               | —                              |
| [冻结计划][term-freeze-plan]                                                             | freeze plan                                               | ` LirFreezePlan `              |
| [前端][term-frontend]                                                                    | frontend                                                  | —                              |
| [生成绑定边界][term-generated-binding-boundary]                                          | generated binding boundary                                | ` laneflow-road-editing-wire ` |
| [几何精度配置档][term-geometry-accuracy-profile]                                         | Geometry accuracy profile                                 | ` GeometryAccuracyProfile `    |
| [几何方向配置档][term-geometry-direction-profile]                                        | Geometry direction profile                                | ` GeometryDirectionProfile `   |
| [几何方向配置档适用标记][term-geometry-direction-profile-applicability-flag]             | Geometry direction profile applicability flag             | ` directionProfileApplies `    |
| [句柄][term-handle]                                                                      | handle                                                    | —                              |
| [高层中间表示][term-high-level-intermediate-representation]                              | high-level intermediate representation                    | ` HIR `                        |
| [导入前端][term-import-frontend]                                                         | import frontend                                           | —                              |
| [增量编译][term-incremental-compile]                                                     | incremental compile                                       | —                              |
| [交换格式][term-interchange-format]                                                      | interchange format                                        | —                              |
| [联结][term-join]                                                                        | join                                                      | —                              |
| [布局][term-layout]                                                                      | layout                                                    | —                              |
| [加载器][term-loader]                                                                    | loader                                                    | —                              |
| [局部索引][term-local-index]                                                             | local index                                               | ` localIndex `                 |
| [降阶][term-lowering]                                                                    | lowering                                                  | —                              |
| [内存计划][term-memory-plan]                                                             | memory plan                                               | ` HirMemoryPlan `              |
| [元数据][term-metadata]                                                                  | metadata                                                  | —                              |
| [中层中间表示][term-mid-level-intermediate-representation]                               | mid-level intermediate representation                     | ` MIR `                        |
| [模块资源计数][term-module-resource-counts]                                              | module resource counts                                    | —                              |
| [规范化][term-normalization]                                                             | normalization                                             | —                              |
| [出现记录][term-occurrence]                                                              | occurrence                                                | —                              |
| [官方前端模块][term-official-frontend-module]                                            | official frontend module                                  | —                              |
| [操作资源预算][term-operation-resource-budget]                                           | operation resource budget                                 | —                              |
| [P100 编译资源配置档][term-p100-compile-limits-profile]                                  | P100 compile limits profile                               | ` LF-COMP-P100 `               |
| [并行编译][term-parallel-compile]                                                        | parallel compile                                          | —                              |
| [部件装配][term-parts-finish]                                                            | parts finish                                              | ` HirParts `                   |
| [逐文档来源记录][term-per-document-source-origin-record]                                 | per-document source origin record                         | ` SourceDocumentOrigin `       |
| [来源链][term-provenance]                                                                | provenance                                                | —                              |
| [重绑定][term-rebind]                                                                    | rebind                                                    | —                              |
| [登记表][term-registry]                                                                  | registry                                                  | —                              |
| [解析器][term-resolver]                                                                  | resolver                                                  | —                              |
| [道路走向定义][term-road-alignment-definition]                                           | road alignment definition                                 | —                              |
| [道路编辑前端][term-road-editing-frontend]                                               | road editing frontend                                     | —                              |
| [道路编辑位置上下文][term-road-editing-location-context]                                 | road editing location context                             | ` RoadEditingLocationContext ` |
| [道路编辑属性路径][term-road-editing-property-path]                                      | road editing property path                                | ` RoadEditingPropertyPath `    |
| [道路编辑来源地址][term-road-editing-source-address]                                     | road editing source address                               | ` RoadEditingSourceAddress `   |
| [道路编辑来源缓冲区][term-road-editing-source-buffer]                                    | road editing source buffer                                | ` LF-ROAD-EDITING-SOURCE-v4 `  |
| [道路编辑来源差异][term-road-editing-source-diff]                                        | road-editing source diff                                  | ` RoadEditingSourceDiff `      |
| [道路编辑来源输入][term-road-editing-source-input]                                       | road editing source input                                 | ` RoadEditingModuleInput `     |
| [道路编辑来源根表][term-road-editing-source-root]                                        | road editing source root                                  | ` RoadEditingSource `          |
| [道路编辑状态][term-road-editing-state]                                                  | road editing state                                        | —                              |
| [共同受检模块接入][term-shared-checked-module-admission]                                 | shared checked module admission                           | —                              |
| [百万单路网配置档][term-single-network-million-compile-limits-profile]                   | single-network million compile limits profile             | ` LF-COMP-SINGLE-NETWORK-1M `  |
| [来源集合摘要][term-source-collection-digest]                                            | source collection digest                                  | ` sourceCollectionDigest `     |
| [来源文档计数][term-source-document-count]                                               | source document count                                     | ` SourceDocumentCount `        |
| [来源文档描述符][term-source-document-descriptor]                                        | source document descriptor                                | ` SourceDocumentDescriptor `   |
| [来源文档摘要][term-source-document-digest]                                              | source document digest                                    | ` sourceDocumentDigest `       |
| [来源文档集摘要][term-source-document-set-digest]                                        | source document-set digest                                | ` sourceDocumentSetDigest `    |
| [来源语言][term-source-language]                                                         | source language                                           | —                              |
| [来源位置][term-source-location]                                                         | source location                                           | ` SourceLocation `             |
| [来源模块][term-source-module]                                                           | source module                                             | —                              |
| [来源模块头][term-source-module-header]                                                  | source module header                                      | ` SourceModuleHeader `         |
| [来源处理限额][term-source-processing-limits]                                            | source processing limits                                  | —                              |
| [稳定容量复用编译][term-stable-capacity-reuse-compile]                                   | stable-capacity reuse compile                             | —                              |
| [静态契约][term-static-contract]                                                         | static contract                                           | ` laneflow-static-contract `   |
| [道路里程站位][term-stationing]                                                          | Stationing                                                | —                              |
| [合成领域专用语言前端][term-synthetic-dsl-frontend]                                      | Synthetic DSL frontend                                    | —                              |
| [合成来源模块][term-synthetic-source-module]                                             | synthetic source module                                   | ` SyntheticModule `            |
| [表行][term-table-row]                                                                   | table row；row                                            | —                              |
| [有类型抽象语法树][term-typed-abstract-syntax-tree]                                      | typed abstract syntax tree                                | ` typed AST `                  |
| [有类型道路编辑模型][term-typed-road-editing-model]                                      | typed road editing model                                  | —                              |
| [已验证规范低层中间表示][term-validated-canonical-low-level-intermediate-representation] | validated canonical low-level intermediate representation | ` canonical LIR `              |
| [已验证编译结果][term-validated-compilation-output]                                      | validated compilation output                              | ` CompilationOutput `          |
| [已验证源映射输入][term-validated-source-map-input]                                      | validated source-map input                                | ` ValidatedSourceMapInput `    |
| [视图][term-view]                                                                        | view                                                      | —                              |

<a id="term-artifact-shape"></a>
### 制品结构统计

英文辅助名：artifact shape。

一份物理制品实际包含的行、字段、UTF-8 字节、向量元素、来源位置、路径步、变化行与载荷大小；只作为资源计划的输入，不与任何限额维度比较。

依据：[0035 编译资源合同分四类边界，规范路网容量由生产端与消费端共同约束，受控分配统一经资源账本][source-adr-0035-148]。

<a id="term-authoring"></a>
### 数据编制

英文辅助名：authoring。

创建、导入、编辑或生成静态路网来源的活动。

依据：[编译器基础设施与合成领域专用语言前端][source-design-compiler-foundation-149]。

<a id="term-authoring-namespace-id"></a>
### 编制命名空间标识

英文辅助名：authoring namespace ID。

隔离稳定标识域、且不依赖文件路径或遍历顺序的持久标识。

精确标识符：` authoringNamespaceId `（字段）。

依据：[编译器基础设施与合成领域专用语言前端][source-design-compiler-foundation-149]。

<a id="term-authoritative-source-module-graph"></a>
### 正本来源模块图

英文辅助名：authoritative source module graph。

一个编译单元内唯一可重放的编制事实来源；节点是来源模块，边是显式导入关系。

依据：[编译器基础设施与合成领域专用语言前端][source-design-compiler-foundation-149]。

<a id="term-backend"></a>
### 后端

英文辅助名：backend。

从已验证编译结果生成特定制品的编译器末端；静态语义只能来自 LIR，源映射可另读已验证来源伴随数据。

依据：[编译器基础设施与合成领域专用语言前端][source-design-compiler-foundation-149]。

<a id="term-base-construction"></a>
### 基础构造

英文辅助名：base construction。

编译器构建 HIR 时统一建立模块轴、符号、身份与引用解析索引的私有阶段。

精确标识符：` HirBase `（类型）。

依据：[编译器基础设施与合成领域专用语言前端][source-design-compiler-foundation-149]。

<a id="term-behavior-equivalence"></a>
### 行为等价

英文辅助名：behavior equivalence。

当前态与目标态在冻结场景中产生相同交通状态、事件和安全结果。

<a id="term-build-plan"></a>
### 构建计划

英文辅助名：build plan。

编译器私有结构：统一记录编译单元各静态语义领域的计数与容量规划，供预算聚合和领域降阶使用。

精确标识符：` HirBuildPlan `（类型）。

依据：[编译器资源账本][source-design-compiler-resource-ledger-150]。

<a id="term-builder"></a>
### 构建器

英文辅助名：builder。

按已冻结输入构造目标对象的组件；不得隐式补充未文档化语义。

<a id="term-cache"></a>
### 缓存

英文辅助名：cache。

只影响复用和性能、不改变规范结果的暂存数据。

<a id="term-candidate-road-revision"></a>
### 候选道路修订

英文辅助名：candidate road revision。

尚未提交、必须先完成编译和验证，失败或取消时不得改变当前路网修订的道路编辑候选。

依据：[道路编辑来源与几何编制前端][source-design-road-editing-source-and-geometry-frontend-151]。

<a id="term-canonical-network-capacity"></a>
### 规范路网容量

英文辅助名：canonical network capacity。

编译端与消费端共同约束的最终规范路网规模上限，维度为稳定实体、机动门、等待区、规范关系出现记录与规范几何点；编译端按最终 LIR、消费端按最终 LFCA 字节以同一计数定义各自统计，差异 base 与公开检查入口的 LFCA 在建立索引前准入。

依据：[0035 编译资源合同分四类边界，规范路网容量由生产端与消费端共同约束，受控分配统一经资源账本][source-adr-0035-148]。

<a id="term-canonical-network-usage"></a>
### 规范路网规模

英文辅助名：canonical network usage。

一份最终规范路网实际包含的规范路网容量各维度数量，只与规范路网容量比较。

依据：[0035 编译资源合同分四类边界，规范路网容量由生产端与消费端共同约束，受控分配统一经资源账本][source-adr-0035-148]。

<a id="term-canonical-orders"></a>
### 规范排列

英文辅助名：canonical orders。

按规范标识原像形成的实体 MIR 与 LIR 双向排列；所有者局部排列仍由各领域冻结产生。

精确标识符：` CanonicalOrders `（类型）。

依据：[编译器基础设施与合成领域专用语言前端][source-design-compiler-foundation-149]。

<a id="term-canonical-relation-occurrence"></a>
### 规范关系出现记录

英文辅助名：canonical relation occurrence。

纳入容量计数的规范关联出现位置，由所有者、角色及该角色规定的局部序号确定；包含编译器派生关系，不按端点去重，同一关联的双向表示只计一次。

精确标识符：` CanonicalRelationCount `（类型）。

参与角色集合由规范路网容量合同统一登记；与来源侧关系出现记录分别定义、分别执行。

依据：[0035 编译资源合同分四类边界，规范路网容量由生产端与消费端共同约束，受控分配统一经资源账本][source-adr-0035-148]。

<a id="term-clean-compile"></a>
### 干净编译

英文辅助名：clean compile。

不使用增量缓存、从完整来源重新执行的编译。

<a id="term-clean-single-thread-compile"></a>
### 干净单工作线程编译

英文辅助名：clean single-thread compile。

不使用增量缓存、只由一个工作线程从完整来源执行全部生产编译遍的确定性参考路径；内存 LIR 保持规范语义一致，后继制品存在时保持精确字节一致。

依据：[编译器基础设施与合成领域专用语言前端][source-design-compiler-foundation-149]。

<a id="term-closed-contract"></a>
### 封闭契约

英文辅助名：closed contract。

以登记的闭合集合冻结允许的类型、字段、配置和组合；未知或未登记输入必须失败关闭，调用方不得任意扩展。

依据：[编译器基础设施与合成领域专用语言前端][source-design-compiler-foundation-149]。

<a id="term-cold-instance-compile"></a>
### 冷实例编译

英文辅助名：cold-instance compile。

以新建编译器实例和空暂存容量执行的干净编译；规模相关来源物化是否计时由具名基准协议冻结，进程启动与磁盘读取不计时。

依据：[编译器基础设施与合成领域专用语言前端][source-design-compiler-foundation-149]。

<a id="term-compilation-metrics"></a>
### 编译观测值

英文辅助名：compilation metrics。

只读描述一次成功编译的记录规模、逻辑输出、编译器控制峰值和同版本语义指纹；不等同于进程内存或制品摘要。

精确标识符：` CompilationMetrics `（类型）。

依据：[编译器基础设施与合成领域专用语言前端][source-design-compiler-foundation-149]。

<a id="term-compilation-unit"></a>
### 编译单元

英文辅助名：compilation unit。

由一个正本来源模块图及其显式选项共同构成的原子编译输入。

依据：[编译器基础设施与合成领域专用语言前端][source-design-compiler-foundation-149]。

<a id="term-compile-limits"></a>
### 编译资源上限

英文辅助名：compile limits。

编译管线在分配前执行的来源处理限额、规范路网容量与操作资源预算组合；制品格式边界独立约束。

精确标识符：` CompileLimits `（类型）。

依据：[0035 编译资源合同分四类边界，规范路网容量由生产端与消费端共同约束，受控分配统一经资源账本][source-adr-0035-148]。

<a id="term-compile-limits-profile-qualification"></a>
### 配置档资格验证

英文辅助名：compile limits profile qualification。

验证具名配置档兑现其公开能力，包括可成功规模、规范统计一致性、诊断与账本完整性，以及各类边界的拒绝行为。

依据：[0035 编译资源合同分四类边界，规范路网容量由生产端与消费端共同约束，受控分配统一经资源账本][source-adr-0035-148]。

<a id="term-compile-limits-profile-validation"></a>
### 配置档校验

英文辅助名：compile limits profile validation。

核对配置档的维度归属、单位、值域、相关整数与平台可表示性，以及失败出口最低资源要求；不包含具体对象格式检查或可选静态生成上界证明。

依据：[0035 编译资源合同分四类边界，规范路网容量由生产端与消费端共同约束，受控分配统一经资源账本][source-adr-0035-148]。

<a id="term-compiler-calibration-scale"></a>
### 编译器校准规模

英文辅助名：compiler calibration scale。

用于观察固定成本、复杂度及缓存/内存拐点的合成编制/中间表示规模；不表示真实城市或产品容量。

依据：[编译器基础设施与合成领域专用语言前端][source-design-compiler-foundation-149]。

<a id="term-compiler-controlled-total-live-memory"></a>
### 编译器控制总存续内存

英文辅助名：compiler-controlled total live memory。

编译请求同时拥有的来源、各阶段记录、字符串、诊断、暂存区和输出构造的存续请求字节总量；不等于进程工作集。

依据：[编译器基础设施与合成领域专用语言前端][source-design-compiler-foundation-149]。

<a id="term-compiler-emitter"></a>
### 编译发射器

英文辅助名：compiler emitter。

从已验证 LIR 生成规范制品的编译器后端。

精确标识符：` emitter `（记号）。

<a id="term-compiler-foundation"></a>
### 编译器基础设施

英文辅助名：compiler foundation。

承载表示类型、编译遍驱动器、诊断、区块分配、确定性和编译发射器边界的公共基础。

依据：[编译器基础设施与合成领域专用语言前端][source-design-compiler-foundation-149]。

<a id="term-compiler-intermediate-representation"></a>
### 编译器中间表示

英文辅助名：compiler intermediate representation。

编译器内部从有类型抽象语法树经 HIR、MIR 到 canonical LIR 的有类型阶段表示总称；各阶段的精确职责由专门词条定义。

精确标识符：` compiler IR `（缩写）。

依据：[编译器基础设施与合成领域专用语言前端][source-design-compiler-foundation-149]。

<a id="term-compiler-owned-static-network"></a>
### 编译器拥有的静态路网

英文辅助名：compiler-owned static network。

全部静态标识、拓扑、几何、规则和索引由编译器统一裁决的静态路网职责划分。

<a id="term-compiler-pass"></a>
### 编译遍

英文辅助名：compiler pass。

具有显式输入、输出、诊断和确定性要求的一次编译转换或验证步骤。

精确标识符：` pass `（记号）。

依据：[编译器基础设施与合成领域专用语言前端][source-design-compiler-foundation-149]。

<a id="term-compiler-resource-ledger"></a>
### 编译器资源账本

英文辅助名：compiler resource ledger。

按宿主顶层操作记录受控存储、计划、预留与授予，并在分配前对照操作资源预算的包内记账结构。

计划准入、同时存续的授予守恒、转交与峰值报告以资源账本设计为准。

依据：[编译器资源账本][source-design-compiler-resource-ledger-150]、[0035 编译资源合同分四类边界，规范路网容量由生产端与消费端共同约束，受控分配统一经资源账本][source-adr-0035-148]。

<a id="term-compiler-stress-scale"></a>
### 编译器压力规模

英文辅助名：compiler stress scale。

放大编制/中间表示对象、用于暴露资源增长和失败边界的合成规模；不构成产品 SLA。

依据：[编译器基础设施与合成领域专用语言前端][source-design-compiler-foundation-149]。

<a id="term-crate-dependency-graph"></a>
### 包依赖图

英文辅助名：crate dependency graph。

Rust 包之间必须保持无环、且不得让静态组件反向依赖动态运行时的依赖关系图。

精确标识符：` crate DAG `（记号）。

<a id="term-determinism"></a>
### 确定性

英文辅助名：determinism。

相同规范输入在冻结条件下产生相同语义、顺序和要求范围内相同字节。

<a id="term-deterministic-merge-order"></a>
### 确定性合并顺序

英文辅助名：deterministic merge order。

并行或分片结果按固定规则合并，使输出不依赖线程调度。

依据：[编译器基础设施与合成领域专用语言前端][source-design-compiler-foundation-149]。

<a id="term-deterministic-oracle"></a>
### 确定性预言机

英文辅助名：deterministic oracle。

其他执行模式必须在冻结的语义或精确字节边界上匹配的参考执行路径。

依据：[编译器基础设施与合成领域专用语言前端][source-design-compiler-foundation-149]。

<a id="term-diagnostic"></a>
### 诊断

英文辅助名：diagnostic。

带稳定代码、严重程度、来源位置、原因和建议的编译或验证反馈。

<a id="term-domain-budget"></a>
### 领域预算

英文辅助名：domain budget。

单个静态语义领域对持久表、查找表与暂存区字节的独立估算；聚合规则由编译器内存设计规定。

精确标识符：` DomainBudget `（类型）。

依据：[编译器资源账本][source-design-compiler-resource-ledger-150]。

<a id="term-editor-authoring-surface"></a>
### 编辑器编制界面

英文辅助名：editor authoring surface。

编辑并持久化来源模块、显示来源诊断的交互界面；它不私有化编译语义。

依据：[编译器基础设施与合成领域专用语言前端][source-design-compiler-foundation-149]。

<a id="term-feature-bits"></a>
### 特性位

英文辅助名：feature bits。

可组合的功能位；不能用任意特性位组合替代合同规定的封闭配置档。

<a id="term-fingerprint"></a>
### 指纹

英文辅助名：fingerprint。

用于快速候选匹配的短摘要；命中后仍须比较完整正本值。

<a id="term-freeze-plan"></a>
### 冻结计划

英文辅助名：freeze plan。

编译器根据 MIR 统一规划 LIR 记录计数、暂存与输出字节、限额观测的私有结构。

精确标识符：` LirFreezePlan `（类型）。

依据：[编译器资源账本][source-design-compiler-resource-ledger-150]。

<a id="term-frontend"></a>
### 前端

英文辅助名：frontend。

只负责特定来源语言的解析、类型化与来源位置，不拥有后续全局静态语义。

依据：[编译器基础设施与合成领域专用语言前端][source-design-compiler-foundation-149]。

<a id="term-generated-binding-boundary"></a>
### 生成绑定边界

英文辅助名：generated binding boundary。

只承载固定 `flatc` Rust 生成物并隔离 generated accessor `unsafe` 的私有、不可发布 package；不成为公共 SDK、第二套领域模型或第三方前端接口。

精确标识符：` laneflow-road-editing-wire `（包）。

依据：[道路编辑来源与几何编制前端][source-design-road-editing-source-and-geometry-frontend-151]。

<a id="term-geometry-accuracy-profile"></a>
### 几何精度配置档

英文辅助名：Geometry accuracy profile。

规定编制曲线转换为规范几何时位置误差工程目标的封闭配置档；固定采样与离线观测不构成连续最大误差保证。

精确标识符：` GeometryAccuracyProfile `（类型）。

依据：[道路编辑来源与几何编制前端][source-design-road-editing-source-and-geometry-frontend-151]。

<a id="term-geometry-direction-profile"></a>
### 几何方向配置档

英文辅助名：Geometry direction profile。

规定折线方向连续性工程目标的封闭配置档；与位置精度配置档正交组合，同一编译单元内一致。

精确标识符：` GeometryDirectionProfile `（类型）。

依据：[道路编辑来源与几何编制前端][source-design-road-editing-source-and-geometry-frontend-151]。

<a id="term-geometry-direction-profile-applicability-flag"></a>
### 几何方向配置档适用标记

英文辅助名：Geometry direction profile applicability flag。

逐规范几何的二值标记，决定哪些内部弦与连接应用同次编译的全局方向配置档；不引入逐行配置档。

精确标识符：` directionProfileApplies `（字段）。

依据：[可移植规范制品与辅助制品格式][source-design-portable-canonical-artifact-152]。

<a id="term-handle"></a>
### 句柄

英文辅助名：handle。

进程或世界内使用的有类型实体引用。

<a id="term-high-level-intermediate-representation"></a>
### 高层中间表示

英文辅助名：high-level intermediate representation。

已完成模块、命名空间、符号、引用、单位和编制语义解析的表示。交通一维存储正本为整数毫米。

精确标识符：` HIR `（缩写）。

依据：[编译器基础设施与合成领域专用语言前端][source-design-compiler-foundation-149]。

<a id="term-import-frontend"></a>
### 导入前端

英文辅助名：import frontend。

把外部来源及其工具、选项和来源链显式转换为来源模块的前端。

依据：[编译器基础设施与合成领域专用语言前端][source-design-compiler-foundation-149]。

<a id="term-incremental-compile"></a>
### 增量编译

英文辅助名：incremental compile。

复用未失效中间结果、但必须与干净编译产生相同规范结果的编译。

<a id="term-interchange-format"></a>
### 交换格式

英文辅助名：interchange format。

可由独立工具跨进程或跨语言读写、具有显式版本与兼容规则的序列化数据契约。

依据：[编译器基础设施与合成领域专用语言前端][source-design-compiler-foundation-149]。

<a id="term-join"></a>
### 联结

英文辅助名：join。

按共同键把两个独立制品或表的记录关联起来。

<a id="term-layout"></a>
### 布局

英文辅助名：layout。

表、字段、偏移量、对齐和节的确定性字节组织方式。

<a id="term-loader"></a>
### 加载器

英文辅助名：loader。

读取并验证输入、构造目标对象或受检视图的边界组件。

<a id="term-local-index"></a>
### 局部索引

英文辅助名：local index。

当前所有者序列快照中的位置；不构成跨编译稳定标识。

精确标识符：` localIndex `（字段）。

<a id="term-lowering"></a>
### 降阶

英文辅助名：lowering。

把较高层表示确定性转换为较低层表示、且不补入后端私有语义的过程。

依据：[编译器基础设施与合成领域专用语言前端][source-design-compiler-foundation-149]。

<a id="term-memory-plan"></a>
### 内存计划

英文辅助名：memory plan。

由构建计划聚合的 HIR 持久表、查找表和互斥暂存峰值预算；不进入公共 API 或制品。

精确标识符：` HirMemoryPlan `（类型）。

依据：[编译器资源账本][source-design-compiler-resource-ledger-150]。

<a id="term-metadata"></a>
### 元数据

英文辅助名：metadata。

可重算或辅助解释、但不单独拥有行为裁决权的数据。

本词条指编译与运行时中的辅助数据；正式文档元数据按其独立规范承载文档身份、状态与关系。

<a id="term-mid-level-intermediate-representation"></a>
### 中层中间表示

英文辅助名：mid-level intermediate representation。

已完成拓扑、几何展开和全局静态语义推导、但尚未绑定目标布局的表示。交通一维存储正本为整数毫米。

精确标识符：` MIR `（缩写）。

依据：[编译器基础设施与合成领域专用语言前端][source-design-compiler-foundation-149]。

<a id="term-module-resource-counts"></a>
### 模块资源计数

英文辅助名：module resource counts。

由官方前端在受检构造期间一次性派生、供共同接入累计检查来源字节、声明、引用、关系、字符串、几何点和存续内存等上限的字段私有计数集合。

依据：[编译器基础设施与合成领域专用语言前端][source-design-compiler-foundation-149]。

<a id="term-normalization"></a>
### 规范化

英文辅助名：normalization。

把输入转换为满足统一单位、默认、顺序和不变量的规范形态。

<a id="term-occurrence"></a>
### 出现记录

英文辅助名：occurrence。

某静态声明在路线（Route）、路径或所有者序列中的具体位置记录。

<a id="term-official-frontend-module"></a>
### 官方前端模块

英文辅助名：official frontend module。

对 LaneFlow 官方前端完成受检构造、以具体字段私有类型绑定来源记录身份、描述符、导入、声明和模块资源计数的模块所作的共同分类；它不是第三方扩展协议。

依据：[编译器基础设施与合成领域专用语言前端][source-design-compiler-foundation-149]。

<a id="term-operation-resource-budget"></a>
### 操作资源预算

英文辅助名：operation resource budget。

宿主为一次编译、发射或公开检查提供的资源，即编译器控制总存续字节与一次编译后允许保留的容量字节，由编译器资源账本在分配前执行；工作量与取消同属操作资源，另行立项。

依据：[0035 编译资源合同分四类边界，规范路网容量由生产端与消费端共同约束，受控分配统一经资源账本][source-adr-0035-148]。

<a id="term-p100-compile-limits-profile"></a>
### P100 编译资源配置档

英文辅助名：P100 compile limits profile。

由宿主显式选择、组合来源处理限额、规范路网容量和操作资源预算的具名生产编译资源配置档。

精确标识符：` LF-COMP-P100 `（常量）。

依据：[0035 编译资源合同分四类边界，规范路网容量由生产端与消费端共同约束，受控分配统一经资源账本][source-adr-0035-148]。

<a id="term-parallel-compile"></a>
### 并行编译

英文辅助名：parallel compile。

并发执行编译遍、并按确定性合并顺序产生结果的编译。

<a id="term-parts-finish"></a>
### 部件装配

英文辅助名：parts finish。

在各领域降阶零错误后原子组装 HIR 的私有边界；失败不提交部分结果。

精确标识符：` HirParts `（类型）。

依据：[编译器基础设施与合成领域专用语言前端][source-design-compiler-foundation-149]。

<a id="term-per-document-source-origin-record"></a>
### 逐文档来源记录

英文辅助名：per-document source origin record。

冷显示/审计元数据；区分文档角色，Traffic/Spatial 另绑定清单制品引用。宿主来源未认证；不证明内容或发布真实性，不进入标识、LIR 语义或文档集摘要。

精确标识符：` SourceDocumentOrigin `（类型）。

依据：[编译器基础设施与合成领域专用语言前端][source-design-compiler-foundation-149]。

<a id="term-provenance"></a>
### 来源链

英文辅助名：provenance。

关联输入、工具、参数、构建、转换与发布过程的可审计来源关系。

依据：[编译器基础设施与合成领域专用语言前端][source-design-compiler-foundation-149]。

<a id="term-rebind"></a>
### 重绑定

英文辅助名：rebind。

在另一个图、世界或制品上下文中重新解析并建立引用。

<a id="term-registry"></a>
### 登记表

英文辅助名：registry。

按稳定规则保存实体、句柄或解析关系的集合。

<a id="term-resolver"></a>
### 解析器

英文辅助名：resolver。

把外部标识、稳定标识或键解析为有类型引用的组件。

<a id="term-road-alignment-definition"></a>
### 道路走向定义

英文辅助名：road alignment definition。

重建当前道路所需的编制描述；来源 v1 只保存 line/cubic Bézier，其他曲线按 B1 工程目标转换并记录观测误差；不属于运行时规范几何。

依据：[道路编辑来源与几何编制前端][source-design-road-editing-source-and-geometry-frontend-151]。

<a id="term-road-editing-frontend"></a>
### 道路编辑前端

英文辅助名：road editing frontend。

把版本化道路编辑状态有界解码为共同有类型道路编辑模型，并产生结构化来源位置的 production 前端。

依据：[道路编辑来源与几何编制前端][source-design-road-editing-source-and-geometry-frontend-151]。

<a id="term-road-editing-location-context"></a>
### 道路编辑位置上下文

英文辅助名：road editing location context。

在失败诊断或成功源映射中拥有 intern 属性路径、画布选择键与显示字节并解析紧凑 ordinal 的字段私有上下文。

精确标识符：` RoadEditingLocationContext `（类型）。

依据：[道路编辑来源与几何编制前端][source-design-road-editing-source-and-geometry-frontend-151]。

<a id="term-road-editing-property-path"></a>
### 道路编辑属性路径

英文辅助名：road editing property path。

由已知 table field、struct member 与 union variant 组成的闭合叶属性 step 序列；不接受任意字符串或向量下标。

精确标识符：` RoadEditingPropertyPath `（类型）。

依据：[道路编辑来源与几何编制前端][source-design-road-editing-source-and-geometry-frontend-151]。

<a id="term-road-editing-source-address"></a>
### 道路编辑来源地址

英文辅助名：road editing source address。

由模块 namespace、实体 kind、完整 owner-key tuple 和 sibling-local key 构成；不同 parent 可复用同名 local key；不参与稳定标识。

精确标识符：` RoadEditingSourceAddress `（类型）。

依据：[道路编辑来源与几何编制前端][source-design-road-editing-source-and-geometry-frontend-151]。

<a id="term-road-editing-source-buffer"></a>
### 道路编辑来源缓冲区

英文辅助名：road editing source buffer。

以 FlatBuffers 编码一个道路编辑模块的版本化来源文档；精确版本、根表与字节要求由道路编辑来源合同规定。

精确标识符：` LF-ROAD-EDITING-SOURCE-v4 `（常量）。

依据：[道路编辑来源与几何编制前端][source-design-road-editing-source-and-geometry-frontend-151]。

<a id="term-road-editing-source-diff"></a>
### 道路编辑来源差异

英文辅助名：road-editing source diff。

比较稳定道路编辑实体、属性和仅编制内容变化的结构化差异；能力与适用边界由道路编辑来源设计规定。

精确标识符：` RoadEditingSourceDiff `（类型）。

依据：[编译器基础设施与合成领域专用语言前端][source-design-compiler-foundation-149]。

<a id="term-road-editing-source-input"></a>
### 道路编辑来源输入

英文辅助名：road editing source input。

借用完整道路编辑来源缓冲区、经编译单元构建器原子完成验证、语义预检、降阶和共同接入的输入。

精确标识符：` RoadEditingModuleInput `（类型）。

依据：[道路编辑来源与几何编制前端][source-design-road-editing-source-and-geometry-frontend-151]。

<a id="term-road-editing-source-root"></a>
### 道路编辑来源根表

英文辅助名：road editing source root。

保存精确格式版本、唯一模块头、道路走向定义及 Identity v1 有类型稳定声明向量；owner-local 值留在 owner table，table offset 不构成领域身份。

精确标识符：` RoadEditingSource `（类型）。

依据：[道路编辑来源与几何编制前端][source-design-road-editing-source-and-geometry-frontend-151]。

<a id="term-road-editing-state"></a>
### 道路编辑状态

英文辅助名：road editing state。

可编辑、可重放的道路编制事实，保存走向、横断面、连接、规则和稳定关系；不包含共享数组、车辆或交互历史。

存档只接受已经进入活动交通运行时的已提交编辑状态；工作中和候选状态由宿主另行管理。

依据：[道路编辑来源与几何编制前端][source-design-road-editing-source-and-geometry-frontend-151]。

<a id="term-shared-checked-module-admission"></a>
### 共同受检模块接入

英文辅助名：shared checked module admission。

把不同官方前端模块原子移入同一编译单元，并统一执行唯一性、资源上限、导入图和规范顺序检查的编译器私有接入路径。

依据：[编译器基础设施与合成领域专用语言前端][source-design-compiler-foundation-149]。

<a id="term-single-network-million-compile-limits-profile"></a>
### 百万单路网配置档

英文辅助名：single-network million compile limits profile。

以一百万个现实混合稳定静态实体为公开规模目标、须通过端到端资格验证的具名大型编译配置档；不提供无限模式或调用方覆写。

精确标识符：` LF-COMP-SINGLE-NETWORK-1M `（常量）。

依据：[0035 编译资源合同分四类边界，规范路网容量由生产端与消费端共同约束，受控分配统一经资源账本][source-adr-0035-148]。

<a id="term-source-collection-digest"></a>
### 来源集合摘要

英文辅助名：source collection digest。

按依赖优先模块顺序对命名空间及每模块来源文档集摘要进行版本化、域分隔聚合所得的 SHA-256；它绑定一次 LFSM 的完整来源模块集合，不替代逐文档摘要，也不进入路网修订标识。

精确标识符：` sourceCollectionDigest `（字段）。

依据：[编译器基础设施与合成领域专用语言前端][source-design-compiler-foundation-149]。

<a id="term-source-document-count"></a>
### 来源文档计数

英文辅助名：source document count。

一个编译单元内独立来源文档描述符的累计数量；它不属于有类型抽象语法树领域记录，不能挤入 `TypedAstRecordCount`。

精确标识符：` SourceDocumentCount `（类型）。

依据：[0035 编译资源合同分四类边界，规范路网容量由生产端与消费端共同约束，受控分配统一经资源账本][source-adr-0035-148]。

<a id="term-source-document-descriptor"></a>
### 来源文档描述符

英文辅助名：source document descriptor。

把一个来源文档的稳定文档键、精确内容摘要、原始字节长度、所属逻辑模块及逐文档来源记录不可分绑定的字段私有值；模块与文档不要求一一对应。

精确标识符：` SourceDocumentDescriptor `（类型）。

依据：[编译器基础设施与合成领域专用语言前端][source-design-compiler-foundation-149]。

<a id="term-source-document-digest"></a>
### 来源文档摘要

英文辅助名：source document digest。

由官方前端对一份版本化规范来源记录的精确原始字节计算一次所得的 SHA-256；它不由调用方自报，也不参与实体稳定标识。

精确标识符：` sourceDocumentDigest `（字段）。

相关概念：[来源文档集摘要][term-source-document-set-digest]、[来源集合摘要][term-source-collection-digest]。

依据：[编译器基础设施与合成领域专用语言前端][source-design-compiler-foundation-149]。

<a id="term-source-document-set-digest"></a>
### 来源文档集摘要

英文辅助名：source document-set digest。

对一个逻辑模块内按文档键规范排序的文档键、精确长度和逐文档摘要进行版本化、域分隔聚合所得的 SHA-256；它用于模块级重放/缓存比较，不替代逐文档身份。

精确标识符：` sourceDocumentSetDigest `（字段）。

依据：[编译器基础设施与合成领域专用语言前端][source-design-compiler-foundation-149]。

<a id="term-source-language"></a>
### 来源语言

英文辅助名：source language。

定义来源模块语法和来源保真的输入语言。

依据：[编译器基础设施与合成领域专用语言前端][source-design-compiler-foundation-149]。

<a id="term-source-location"></a>
### 来源位置

英文辅助名：source location。

诊断和源映射使用的闭合和类型；文本来源使用真实 `SourceSpan` 行列，道路编辑来源使用稳定实体/owner-local subject、schema property 与可选画布选区，只有结构损坏可使用 byte range。

精确标识符：` SourceLocation `（类型）。

依据：[编译器基础设施与合成领域专用语言前端][source-design-compiler-foundation-149]。

<a id="term-source-module"></a>
### 来源模块

英文辅助名：source module。

绑定命名空间、来源语言、工具/选项与来源链的逻辑编译节点；合成前端恰有一份文档，后继官方前端可拥有多份文档并绑定文档集摘要。

依据：[编译器基础设施与合成领域专用语言前端][source-design-compiler-foundation-149]。

<a id="term-source-module-header"></a>
### 来源模块头

英文辅助名：source module header。

调用方提供非内容字段，官方前端派生模块描述符、文档集摘要、逐文档摘要与描述符。

精确标识符：` SourceModuleHeader `（类型）。

依据：[编译器基础设施与合成领域专用语言前端][source-design-compiler-foundation-149]。

<a id="term-source-processing-limits"></a>
### 来源处理限额

英文辅助名：source processing limits。

编译单元准入与编译管线执行的来源与阶段规模上限，如模块、来源字节、声明、各阶段记录、有类型引用、来源关系出现记录、标识字段出现记录、符号、字符串与诊断；字符串与关系维度为来源侧口径，不用于约束外部制品。

依据：[0035 编译资源合同分四类边界，规范路网容量由生产端与消费端共同约束，受控分配统一经资源账本][source-adr-0035-148]。

<a id="term-stable-capacity-reuse-compile"></a>
### 稳定容量复用编译

英文辅助名：stable-capacity reuse compile。

复用编译器的无语义暂存容量、但不复用语义结果的干净编译；它不是增量编译，结果和诊断必须与冷实例编译一致。

依据：[编译器基础设施与合成领域专用语言前端][source-design-compiler-foundation-149]。

<a id="term-static-contract"></a>
### 静态契约

英文辅助名：static contract。

编译器、格式检查、共享静态路网、运行时与空间层共享的目标中立值、登记表和版本契约。

精确标识符：` laneflow-static-contract `（包）。

<a id="term-stationing"></a>
### 道路里程站位

英文辅助名：Stationing。

从道路参考线起点沿配置档无关的编译期 reference station 基表单调累计的距离坐标，用于区段、横断面和连接几何的纵向定位；不属于持久身份，也不由最终输出档位反向改变。

依据：[道路编辑来源与几何编制前端][source-design-road-editing-source-and-geometry-frontend-151]。

<a id="term-synthetic-dsl-frontend"></a>
### 合成领域专用语言前端

英文辅助名：Synthetic DSL frontend。

面向测试、基准、示例和程序化场景的可重放来源前端。

依据：[编译器基础设施与合成领域专用语言前端][source-design-compiler-foundation-149]。

<a id="term-synthetic-source-module"></a>
### 合成来源模块

英文辅助名：synthetic source module。

由 `SyntheticModuleBuilder` 受检构造、承载可重放合成来源的具体编译模块。

精确标识符：` SyntheticModule `（类型）。

依据：[编译器基础设施与合成领域专用语言前端][source-design-compiler-foundation-149]。

<a id="term-table-row"></a>
### 表行

英文辅助名：table row；row。

静态表中的一条记录；LIR 与规范制品的逻辑表行使用有类型逻辑序号。

<a id="term-typed-abstract-syntax-tree"></a>
### 有类型抽象语法树

英文辅助名：typed abstract syntax tree。

保留前端语法、显式键、类型、单位和来源位置的第一层表示。准入后交通一维不再把编制 `f64` 米当存储正本。

精确标识符：` typed AST `（缩写）。

依据：[编译器基础设施与合成领域专用语言前端][source-design-compiler-foundation-149]。

<a id="term-typed-road-editing-model"></a>
### 有类型道路编辑模型

英文辅助名：typed road editing model。

由可视化编辑器、可发布程序化生成器、SDK 与 importer 共享的字段、单位、稳定身份、引用和不变量契约；物理编码不拥有第二套语义。

依据：[道路编辑来源与几何编制前端][source-design-road-editing-source-and-geometry-frontend-151]。

<a id="term-validated-canonical-low-level-intermediate-representation"></a>
### 已验证规范低层中间表示

英文辅助名：validated canonical low-level intermediate representation。

后端唯一输入；包含稳定标识、有类型序号、规范数值、确定性关系和布局无关预计算。交通一维与 LFCA 同一套整数毫米。

精确标识符：` canonical LIR `（缩写）。

依据：[编译器基础设施与合成领域专用语言前端][source-design-compiler-foundation-149]。

<a id="term-validated-compilation-output"></a>
### 已验证编译结果

英文辅助名：validated compilation output。

原子拥有 LIR、已验证源映射输入和非错误级诊断的成功编译结果；错误级诊断存在时不得构造。

精确标识符：` CompilationOutput `（类型）。

依据：[编译器基础设施与合成领域专用语言前端][source-design-compiler-foundation-149]。

<a id="term-validated-source-map-input"></a>
### 已验证源映射输入

英文辅助名：validated source-map input。

与 LIR 同次成功编译冻结、只保存来源模块、来源位置、来源链及 LIR 键关联且不能补充静态语义的伴随数据。

精确标识符：` ValidatedSourceMapInput `（类型）。

依据：[编译器基础设施与合成领域专用语言前端][source-design-compiler-foundation-149]。

<a id="term-view"></a>
### 视图

英文辅助名：view。

借用或共享已验证底层存储、且不复制正本数据的只读访问接口。

<a id="domain-identity"></a>
## 标识与数据布局

点击中文规范名查看完整定义；跨领域概念指向同一词条。

| 中文规范名                                             | 英文辅助名                      | 精确标识符                 |
| ------------------------------------------------------ | ------------------------------- | -------------------------- |
| [区块分配键][term-arena-key]                           | arena key                       | —                          |
| [规范身份表][term-canonical-identity-table]            | canonical identity table        | ` CanonicalIdentityTable ` |
| [规范标识元组][term-canonical-identity-tuple]          | canonical identity tuple        | ` CanonicalIdentity `      |
| [冷数据][term-cold-data]                               | cold data                       | ` cold `                   |
| [完备所有者树][term-complete-owner-tree]               | complete owner tree             | —                          |
| [压缩稀疏行][term-compressed-sparse-row]               | compressed sparse row           | ` CSR `                    |
| [交叉索引][term-cross-index]                           | cross-index                     | —                          |
| [密集句柄][term-dense-handle]                          | dense handle                    | —                          |
| [确定性派生摘要][term-deterministic-derivation-digest] | deterministic derivation digest | —                          |
| [摘要][term-digest]                                    | digest                          | —                          |
| [实体种类][term-entity-kind]                           | entity kind                     | ` entityKind `             |
| [字段标签][term-field-tag]                             | field tag                       | ` field_tag `              |
| [扁平区间][term-flat-range]                            | flat range                      | —                          |
| [热列][term-hot-column]                                | hot column                      | —                          |
| [热数据][term-hot-data]                                | hot data                        | ` hot `                    |
| [热路径][term-hot-path]                                | hot path                        | —                          |
| [标识编码封装][term-identity-encoding-envelope]        | identity encoding envelope      | —                          |
| [内部完整性摘要][term-internal-integrity-digest]       | internal integrity digest       | —                          |
| [互操作摘要][term-interoperable-digest]                | interoperable digest            | —                          |
| [键][term-key]                                         | key                             | —                          |
| [所有者 / 成员][term-owner]                            | owner；member                   | —                          |
| [所有者局部出现记录][term-owner-local-occurrence]      | owner-local occurrence          | —                          |
| [所有者局部关系][term-owner-local-relation]            | owner-local relation            | —                          |
| [持久标识摘要][term-persistent-identity-digest]        | persistent identity digest      | —                          |
| [反向索引][term-reverse-index]                         | reverse index                   | —                          |
| [角色][term-role]                                      | role                            | —                          |
| [稳定锚点][term-stable-anchor]                         | stable anchor                   | —                          |
| [稳定标识][term-stable-identifier]                     | stable identifier               | ` StableId128 `            |
| [数组分列结构][term-structure-of-arrays]               | structure of arrays             | ` SoA `                    |
| [瞬态指纹][term-transient-fingerprint]                 | transient fingerprint           | —                          |
| [有类型逻辑序号][term-typed-logical-ordinal]           | typed logical ordinal           | —                          |
| [唯一所有者关系][term-unique-owner-relation]           | unique owner relation           | —                          |
| [温数据][term-warm-data]                               | warm data                       | ` warm `                   |
| [零拷贝][term-zero-copy]                               | zero-copy                       | —                          |

<a id="term-arena-key"></a>
### 区块分配键

英文辅助名：arena key。

仅在一次编译的有类型区块分配器内有效、不得进入 LIR 或持久制品的私有 `u32` 引用。

依据：[路网编译器与共享静态路网][source-design-network-compiler-37]。

<a id="term-canonical-identity-table"></a>
### 规范身份表

英文辅助名：canonical identity table。

可移植规范制品中保存稳定实体完整标识原像、声明稳定标识与有类型逻辑序号的不可裁剪语义表。

精确标识符：` CanonicalIdentityTable `（类型）。

依据：[路网编译器与共享静态路网][source-design-network-compiler-37]。

<a id="term-canonical-identity-tuple"></a>
### 规范标识元组

英文辅助名：canonical identity tuple。

由编制命名空间、实体种类及稳定父/局部锚点组成的标识正本。

精确标识符：` CanonicalIdentity `（类型）。

依据：[路网编译器与共享静态路网][source-design-network-compiler-37]。

<a id="term-cold-data"></a>
### 冷数据

英文辅助名：cold data。

主要在加载、恢复、修订切换或诊断边界读取的数据；“冷”不表示可以从生产配置档裁剪。

精确标识符：` cold `（记号）。

依据：[共享静态路网][source-design-shared-static-network-187]。

<a id="term-complete-owner-tree"></a>
### 完备所有者树

英文辅助名：complete owner tree。

所有要求归属的成员都具有唯一所有者、且不存在重复归属或孤儿成员的所有者 / 成员结构。

依据：[路网编译器与共享静态路网][source-design-network-compiler-37]。

<a id="term-compressed-sparse-row"></a>
### 压缩稀疏行

英文辅助名：compressed sparse row。

用偏移量/区间表示变长邻接或成员序列的紧凑布局。

精确标识符：` CSR `（缩写）。

依据：[路网编译器与共享静态路网][source-design-network-compiler-37]。

<a id="term-cross-index"></a>
### 交叉索引

英文辅助名：cross-index。

在两个有类型表或数据节之间预计算的定向引用索引。

依据：[路网编译器与共享静态路网][source-design-network-compiler-37]。

<a id="term-dense-handle"></a>
### 密集句柄

英文辅助名：dense handle。

运行时使用的紧凑有类型引用；热路径不携带字符串或稳定标识。

依据：[路网编译器与共享静态路网][source-design-network-compiler-37]。

<a id="term-deterministic-derivation-digest"></a>
### 确定性派生摘要

英文辅助名：deterministic derivation digest。

ADR 0033 摘要角色之一：用哈希将规范原像映射为可重建的排序键、采样值或派生参数；可决定持久结果，但不作为唯一身份或内容相等证明。满足输入信任、碰撞处理与分布要求后默认采用 XXH3-64，算法及字节规则须版本化。

依据：[0033 摘要角色与摘要算法选型裁决权][source-adr-0033-188]。

<a id="term-digest"></a>
### 摘要

英文辅助名：digest。

对完整字节或规范原像计算的固定长度值；用途由 ADR 0033 的角色定义，不天然构成身份或完整性证明。

依据：[0033 摘要角色与摘要算法选型裁决权][source-adr-0033-188]。

<a id="term-entity-kind"></a>
### 实体种类

英文辅助名：entity kind。

标识登记表中具有固定代码、英文短名（slug）和必需字段序列的实体类别。

精确标识符：` entityKind `（字段）。

依据：[路网编译器与共享静态路网][source-design-network-compiler-37]。

<a id="term-field-tag"></a>
### 字段标签

英文辅助名：field tag。

标识编码中用于标明规范字段语义的冻结数字标签。

精确标识符：` field_tag `（字段）。

依据：[路网编译器与共享静态路网][source-design-network-compiler-37]。

<a id="term-flat-range"></a>
### 扁平区间

英文辅助名：flat range。

以起点和长度引用连续表片段、避免逐项指针的数据表示。

依据：[路网编译器与共享静态路网][source-design-network-compiler-37]。

<a id="term-hot-column"></a>
### 热列

英文辅助名：hot column。

按字段分列保存、供热路径高频读取的热数据列，例如共享静态路网的边长与边限速列；描述访问与布局，不另立数据正本、单位或持久格式。

依据：[共享静态路网][source-design-shared-static-network-187]。

<a id="term-hot-data"></a>
### 热数据

英文辅助名：hot data。

固定步进或位姿批次高频读取、必须按连续布局和缓存局部性优化的数据；热度不单独决定配置档必选性。

精确标识符：` hot `（记号）。

依据：[共享静态路网][source-design-shared-static-network-187]。

<a id="term-hot-path"></a>
### 热路径

英文辅助名：hot path。

高频固定步进/位姿执行路径；禁止字符串、哈希查找和持久标识计算。

依据：[共享静态路网][source-design-shared-static-network-187]。

<a id="term-identity-encoding-envelope"></a>
### 标识编码封装

英文辅助名：identity encoding envelope。

冻结魔数、编码版本、实体种类、字段数量和标签/长度/值序列的公共字节外壳。

依据：[路网编译器与共享静态路网][source-design-network-compiler-37]。

<a id="term-internal-integrity-digest"></a>
### 内部完整性摘要

英文辅助名：internal integrity digest。

ADR 0033 摘要角色之一：认证字节或规范值未被改变、只由 LaneFlow 实现产生和重算的摘要；默认 BLAKE3-256。

依据：[0033 摘要角色与摘要算法选型裁决权][source-adr-0033-188]。

<a id="term-interoperable-digest"></a>
### 互操作摘要

英文辅助名：interoperable digest。

ADR 0033 摘要角色之一：须允许 LaneFlow 之外的实现、通用工具或人工流程独立重算的摘要；默认 SHA-256，须写明外部重算方。

依据：[0033 摘要角色与摘要算法选型裁决权][source-adr-0033-188]。

<a id="term-key"></a>
### 键

英文辅助名：key。

在明确作用域内区分声明或关系的值；只有冻结为稳定锚点时才参与持久标识。

依据：[路网编译器与共享静态路网][source-design-network-compiler-37]。

<a id="term-owner"></a>
### 所有者 / 成员

英文辅助名：owner；member。

负责组织事实的实体及其直接从属记录。

依据：[路网编译器与共享静态路网][source-design-network-compiler-37]。

<a id="term-owner-local-occurrence"></a>
### 所有者局部出现记录

英文辅助名：owner-local occurrence。

由 `(ownerOrdinal, role, localIndex)` 定位的当前快照记录；`localIndex` 不是跨编译标识。

依据：[路网编译器与共享静态路网][source-design-network-compiler-37]。

<a id="term-owner-local-relation"></a>
### 所有者局部关系

英文辅助名：owner-local relation。

只在所属实体的当前序列快照内有意义、没有全局稳定标识的关系记录。

依据：[路网编译器与共享静态路网][source-design-network-compiler-37]。

<a id="term-persistent-identity-digest"></a>
### 持久标识摘要

英文辅助名：persistent identity digest。

ADR 0033 摘要角色之一：从规范原像派生、作为实体或对象稳定标识持久保存的摘要；默认 BLAKE3，现行 `StableId128` 为 BLAKE3-128。

依据：[0033 摘要角色与摘要算法选型裁决权][source-adr-0033-188]。

<a id="term-reverse-index"></a>
### 反向索引

英文辅助名：reverse index。

从目标或成员回到所有者、关系或候选集合的预计算索引。

依据：[路网编译器与共享静态路网][source-design-network-compiler-37]。

<a id="term-role"></a>
### 角色

英文辅助名：role。

在所有者局部键中区分同一所有者下不同关系序列的有类型语义。

依据：[路网编译器与共享静态路网][source-design-network-compiler-37]。

<a id="term-stable-anchor"></a>
### 稳定锚点

英文辅助名：stable anchor。

显式持久键或父实体稳定标识；坐标、数组位置和遍历顺序不得作为锚点。

依据：[路网编译器与共享静态路网][source-design-network-compiler-37]。

<a id="term-stable-identifier"></a>
### 稳定标识

英文辅助名：stable identifier。

对规范标识元组执行冻结编码和 BLAKE3-128 后得到的 128 位持久标识。

精确标识符：` StableId128 `（类型）。

依据：[路网编译器与共享静态路网][source-design-network-compiler-37]。

<a id="term-structure-of-arrays"></a>
### 数组分列结构

英文辅助名：structure of arrays。

按字段分列保存同类记录，以提高连续访问和向量化效率的数据布局。

精确标识符：` SoA `（缩写）。

依据：[路网编译器与共享静态路网][source-design-network-compiler-37]。

<a id="term-transient-fingerprint"></a>
### 瞬态指纹

英文辅助名：transient fingerprint。

ADR 0033 摘要角色之一：只在同一进程内用于哈希表、缓存与候选匹配的指纹；命中后须比较完整正本值，不得持久化或影响确定性输出。

依据：[0033 摘要角色与摘要算法选型裁决权][source-adr-0033-188]。

<a id="term-typed-logical-ordinal"></a>
### 有类型逻辑序号

英文辅助名：typed logical ordinal。

在单份 LIR 或规范制品内按逻辑表类型区分的行序号；不是物理分块位置或跨制品稳定身份。

依据：[路网编译器与共享静态路网][source-design-network-compiler-37]。

<a id="term-unique-owner-relation"></a>
### 唯一所有者关系

英文辅助名：unique owner relation。

一个成员必须解析到恰好一个有效所有者；零所有者或多所有者均为结构化错误。

依据：[路网编译器与共享静态路网][source-design-network-compiler-37]。

<a id="term-warm-data"></a>
### 温数据

英文辅助名：warm data。

低于稳态热路径频率、但仍可能由运行时查询或边界操作读取的数据；是否必选由有类型节与封闭配置档裁决。

精确标识符：` warm `（记号）。

依据：[共享静态路网][source-design-shared-static-network-187]。

<a id="term-zero-copy"></a>
### 零拷贝

英文辅助名：zero-copy。

验证后直接借用不可变底层字节建立视图、不复制所指数据的访问方式；不表示整个构建或运行过程无分配。

依据：[可移植规范制品与辅助制品格式][source-design-portable-canonical-artifact-152]。

<a id="domain-static-network"></a>
## 静态路网

点击中文规范名查看完整定义；跨领域概念指向同一词条。

| 中文规范名                                                  | 英文辅助名                         | 精确标识符                          |
| ----------------------------------------------------------- | ---------------------------------- | ----------------------------------- |
| [准入规则][term-access-rule]                                | access rule                        | ` AccessRule `                      |
| [进口道][term-approach]                                     | approach                           | —                                   |
| [编制车道][term-authoring-lane]                             | authoring lane                     | ` AuthoringLane `                   |
| [规范路网来源绑定][term-canonical-network-origin]           | canonical network origin           | ` CanonicalNetworkOrigin `          |
| [规范路网语义载荷][term-canonical-network-semantic-payload] | canonical network semantic payload | ` canonicalNetworkSemanticPayload ` |
| [冲突通行段][term-conflict-passage]                         | conflict passage                   | ` ConflictPassage `                 |
| [路线冲突出现记录][term-conflict-passage-occurrence]        | conflict passage occurrence        | ` ConflictPassageOccurrence `       |
| [冲突区][term-conflict-zone]                                | conflict zone                      | ` ConflictZone `                    |
| [覆盖关系][term-coverage]                                   | coverage                           | —                                   |
| [出口道][term-exit-approach]                                | exit approach                      | —                                   |
| [设施带][term-facility-band]                                | facility band                      | ` FacilityBand `                    |
| [路口][term-junction]                                       | junction                           | ` Junction `                        |
| [路口组][term-junction-group]                               | junction group                     | ` JunctionGroup `                   |
| [车道图边][term-lane-edge]                                  | lane edge                          | ` LaneEdge `                        |
| [车道组][term-lane-group]                                   | lane group                         | ` LaneGroup `                       |
| [机动门][term-maneuver-gate]                                | maneuver gate                      | ` ManeuverGate `                    |
| [机动路径][term-maneuver-path]                              | maneuver path                      | ` ManeuverPath `                    |
| [通行流向][term-movement]                                   | movement                           | ` Movement `                        |
| [路网修订][term-network-revision]                           | network revision                   | —                                   |
| [路网修订标识][term-network-revision-id]                    | network revision ID                | ` NetworkRevisionId `               |
| [停车设施][term-parking-facility]                           | parking facility                   | ` ParkingFacility `                 |
| [停车位][term-parking-space]                                | parking space                      | ` ParkingSpace `                    |
| [参与者类别][term-participant-class]                        | participant class                  | ` ParticipantClass `                |
| [参与者流][term-participant-stream]                         | participant stream                 | ` ParticipantStream `               |
| [路径锚点][term-path-anchor]                                | path anchor                        | ` PathAnchor `                      |
| [道路走廊][term-road-corridor]                              | road corridor                      | ` RoadCorridor `                    |
| [道路区段][term-road-section]                               | road section                       | ` RoadSection `                     |
| [稳定身份索引][term-shared-identity-index]                  | shared identity index              | ` SharedIdentityIndex `             |
| [共享路网修订][term-shared-network-revision]                | shared network revision            | ` SharedNetworkRevision `           |
| [共享空间路网][term-shared-spatial-network]                 | shared spatial network             | ` SharedSpatialNetwork `            |
| [共享静态路网][term-shared-static-network]                  | shared static network              | ` laneflow-static-network `         |
| [共享交通路网][term-shared-traffic-network]                 | shared traffic network             | ` SharedTrafficNetwork `            |
| [信号控制器][term-signal-controller]                        | signal controller                  | ` SignalController `                |
| [信号组][term-signal-group]                                 | signal group                       | ` SignalGroup `                     |
| [信号相位][term-signal-phase]                               | signal phase                       | ` SignalPhase `                     |
| [空间保留构建选项][term-spatial-retention-build-option]     | spatial retention build option     | ` SpatialBuildOption `              |
| [静态路网][term-static-network]                             | static network                     | —                                   |
| [停止线][term-stop-line]                                    | stop line                          | ` StopLine `                        |
| [拓扑][term-topology]                                       | topology                           | —                                   |
| [车辆配置][term-vehicle-profile]                            | vehicle profile                    | ` VehicleProfile `                  |
| [等待区][term-waiting-zone]                                 | waiting zone                       | ` WaitingZone `                     |

<a id="term-access-rule"></a>
### 准入规则

英文辅助名：access rule。

对参与者与目标施加允许、拒绝或约束效果的静态规则。

精确标识符：` AccessRule `（类型）。

依据：[多模式横断面与准入分层][source-design-cross-section-access-230]。

<a id="term-approach"></a>
### 进口道

英文辅助名：approach。

路口中指向入口方向的有向接近关系；中文交通工程标准词，与出口道成对。

依据：[Road / Junction / Maneuver 静态模型][source-design-road-junction-model-231]。

<a id="term-authoring-lane"></a>
### 编制车道

英文辅助名：authoring lane。

来源模块中具有稳定键、可展开为道路车道边的车道声明。

精确标识符：` AuthoringLane `（类型）。

依据：[Road / Junction / Maneuver 静态模型][source-design-road-junction-model-231]。

<a id="term-canonical-network-origin"></a>
### 规范路网来源绑定

英文辅助名：canonical network origin。

共享路网修订保存的 LFCA digest/length、`NetworkRevisionId` 与静态契约版本；用于恢复/切换核对，不是可序列化 descriptor 或发布信任锚。

精确标识符：` CanonicalNetworkOrigin `（类型）。

依据：[共享静态路网][source-design-shared-static-network-187]。

<a id="term-canonical-network-semantic-payload"></a>
### 规范路网语义载荷

英文辅助名：canonical network semantic payload。

排除工具来源、诊断和目标布局，完整冻结运行时可观察静态路网语义及其派生契约版本的目标无关规范字节。

精确标识符：` canonicalNetworkSemanticPayload `（字段）。

依据：[可移植规范制品与辅助制品格式][source-design-portable-canonical-artifact-152]。

<a id="term-conflict-passage"></a>
### 冲突通行段

英文辅助名：conflict passage。

某参与者流穿过一个冲突区的所有者局部 entry/exit 路径区间；不具有独立稳定身份。

精确标识符：` ConflictPassage `（类型）。

依据：[WaitingZone、ConflictZone 与通行权分层][source-design-waiting-zone-conflict-right-of-way-232]。

<a id="term-conflict-passage-occurrence"></a>
### 路线冲突出现记录

英文辅助名：conflict passage occurrence。

某条每世界路线中的一次冲突通行段出现；以参与者流 ordinal 与所有者局部 passage 下标回指共享静态事实。

精确标识符：` ConflictPassageOccurrence `（类型）。

依据：[WaitingZone、ConflictZone 与通行权分层][source-design-waiting-zone-conflict-right-of-way-232]。

<a id="term-conflict-zone"></a>
### 冲突区

英文辅助名：conflict zone。

多个参与者流可能发生空间冲突、需要运行时裁决的区域。

精确标识符：` ConflictZone `（类型）。

依据：[WaitingZone、ConflictZone 与通行权分层][source-design-waiting-zone-conflict-right-of-way-232]。

<a id="term-coverage"></a>
### 覆盖关系

英文辅助名：coverage。

门、等待区、冲突区或规则对路径/出现记录的适用范围。

依据：[Road / Junction / Maneuver 静态模型][source-design-road-junction-model-231]。

<a id="term-exit-approach"></a>
### 出口道

英文辅助名：exit approach。

路口中指向出口方向的有向接近关系；与进口道成对。

依据：[Road / Junction / Maneuver 静态模型][source-design-road-junction-model-231]。

<a id="term-facility-band"></a>
### 设施带

英文辅助名：facility band。

道路走廊内不直接承担机动车遍历拓扑的设施横带。

精确标识符：` FacilityBand `（类型）。

依据：[多模式横断面与准入分层][source-design-cross-section-access-230]。

<a id="term-junction"></a>
### 路口

英文辅助名：junction。

组织通行流向和机动路径的静态路口声明。

精确标识符：` Junction `（类型）。

依据：[Road / Junction / Maneuver 静态模型][source-design-road-junction-model-231]。

<a id="term-junction-group"></a>
### 路口组

英文辅助名：junction group。

组织多个路口的更高层静态组合；尚未进入标识 v1（Identity v1）。

精确标识符：` JunctionGroup `（类型）。

属于候选扩展，不因术语登记进入现行静态标识或制品格式。

依据：[Road / Junction / Maneuver 静态模型][source-design-road-junction-model-231]。

<a id="term-lane-edge"></a>
### 车道图边

英文辅助名：lane edge。

具有显式稳定边键、可独立寻址的基础遍历拓扑实体；道路车道覆盖与路口内部边归属只是关系 / 角色，不参与其稳定身份。

精确标识符：` LaneEdge `（类型）。

依据：[Road / Junction / Maneuver 静态模型][source-design-road-junction-model-231]。

<a id="term-lane-group"></a>
### 车道组

英文辅助名：lane group。

道路区段内组织车道成员的静态分组。

精确标识符：` LaneGroup `（类型）。

依据：[多模式横断面与准入分层][source-design-cross-section-access-230]。

<a id="term-maneuver-gate"></a>
### 机动门

英文辅助名：maneuver gate。

绑定机动路径、用于空间准入和信号约束的静态门。

精确标识符：` ManeuverGate `（类型）。

依据：[Road / Junction / Maneuver 静态模型][source-design-road-junction-model-231]。

<a id="term-maneuver-path"></a>
### 机动路径

英文辅助名：maneuver path。

某通行流向内连接入口边、内部边和出口边的可遍历路径。

精确标识符：` ManeuverPath `（类型）。

依据：[Road / Junction / Maneuver 静态模型][source-design-road-junction-model-231]。

<a id="term-movement"></a>
### 通行流向

英文辅助名：movement。

从进口道到出口道的静态通行意图；转向类别只是元数据。

精确标识符：` Movement `（类型）。

依据：[Road / Junction / Maneuver 静态模型][source-design-road-junction-model-231]。

<a id="term-network-revision"></a>
### 路网修订

英文辅助名：network revision。

在其生命周期内不可变、经编译和检查，并由重算路网修订标识绑定的一个静态路网版本。

依据：[可移植规范制品与辅助制品格式][source-design-portable-canonical-artifact-152]。

<a id="term-network-revision-id"></a>
### 路网修订标识

英文辅助名：network revision ID。

对规范路网语义载荷计算、由后发射检查重算的版本化不透明摘要；发布时由 LFCP/manifest 绑定，只支持相等性比较。

精确标识符：` NetworkRevisionId `（类型）。

依据：[可移植规范制品与辅助制品格式][source-design-portable-canonical-artifact-152]。

<a id="term-parking-facility"></a>
### 停车设施

英文辅助名：parking facility。

组织显式停车位与虚拟停车容量，并可拥有多个虚拟入口和出口的唯一停车设施实体。

精确标识符：` ParkingFacility `（类型）。

相关概念：[停车位][term-parking-space]、[虚拟停车容量][term-virtual-parking-capacity]。

依据：[停车系统设计][source-design-parking-system-234]。

<a id="term-parking-space"></a>
### 停车位

英文辅助名：parking space。

有排他占用、静态几何和 parked pose 的具体位置；可选归属于停车设施，该组织关系不构成停车位身份父锚点。

精确标识符：` ParkingSpace `（类型）。

依据：[停车系统设计][source-design-parking-system-234]。

<a id="term-participant-class"></a>
### 参与者类别

英文辅助名：participant class。

数据声明、可继承的准入分类；不定义交通执行域，也不证明对应运行时行为已经实现。

精确标识符：` ParticipantClass `（类型）。

依据：[多模式横断面与准入分层][source-design-cross-section-access-230]。

<a id="term-participant-stream"></a>
### 参与者流

英文辅助名：participant stream。

进入冲突裁决的有向参与者流。

精确标识符：` ParticipantStream `（类型）。

依据：[WaitingZone、ConflictZone 与通行权分层][source-design-waiting-zone-conflict-right-of-way-232]。

<a id="term-path-anchor"></a>
### 路径锚点

英文辅助名：path anchor。

以机动门、路径边界或边内整数毫米位置精确标识机动路径位置的闭合联合。

精确标识符：` PathAnchor `（类型）。

依据：[Road / Junction / Maneuver 静态模型][source-design-road-junction-model-231]。

<a id="term-road-corridor"></a>
### 道路走廊

英文辅助名：road corridor。

组织方向性道路区段与非遍历设施带的横断面所有者。

精确标识符：` RoadCorridor `（类型）。

依据：[Road / Junction / Maneuver 静态模型][source-design-road-junction-model-231]。

<a id="term-road-section"></a>
### 道路区段

英文辅助名：road section。

道路走廊内具有方向和横断面成员关系的区段。

精确标识符：` RoadSection `（类型）。

依据：[Road / Junction / Maneuver 静态模型][source-design-road-junction-model-231]。

<a id="term-shared-identity-index"></a>
### 稳定身份索引

英文辅助名：shared identity index。

所有共享路网修订都必须包含、在稳定标识与 typed ordinal 之间双向映射的冷数据索引；它不进入 steady tick，也不证明跨修订语义兼容。

精确标识符：` SharedIdentityIndex `（类型）。

依据：[共享静态路网][source-design-shared-static-network-187]。

<a id="term-shared-network-revision"></a>
### 共享路网修订

英文辅助名：shared network revision。

绑定同一路网修订标识的交通、身份、规划提示与可选空间组件的共享对象；各组件不得独立切换。

精确标识符：` SharedNetworkRevision `（类型）。

允许简称“修订”：运行时状态完整性语境，且上下文已经明确所指共享路网修订。

相关概念：[共享静态路网][term-shared-static-network]、[共享交通路网][term-shared-traffic-network]、[共享空间路网][term-shared-spatial-network]。

依据：[共享静态路网][source-design-shared-static-network-187]。

<a id="term-shared-spatial-network"></a>
### 共享空间路网

英文辅助名：shared spatial network。

共享路网修订中可选的规范空间数据；可以只有 facility/profile/frame，非空 LaneEdge geometry 才提供 lane-pose sampling；不得与其他修订的 Traffic component 混用。

精确标识符：` SharedSpatialNetwork `（类型）。

依据：[共享静态路网][source-design-shared-static-network-187]。

<a id="term-shared-static-network"></a>
### 共享静态路网

英文辅助名：shared static network。

从受检规范制品有界构建、不可变且可由多个世界共享的进程内静态数据层；不是文件、ABI、存档或发布对象。

精确标识符：` laneflow-static-network `（包）。

依据：[共享静态路网][source-design-shared-static-network-187]。

<a id="term-shared-traffic-network"></a>
### 共享交通路网

英文辅助名：shared traffic network。

共享路网修订中面向 Traffic Runtime 的必选性能数据；采用类型化密集句柄、连续 hot/cold columns、CSR/range 与预计算 occurrence/constraints。

精确标识符：` SharedTrafficNetwork `（类型）。

依据：[共享静态路网][source-design-shared-static-network-187]。

<a id="term-signal-controller"></a>
### 信号控制器

英文辅助名：signal controller。

产生信号相位和指示时间序列的静态控制程序。

精确标识符：` SignalController `（类型）。

依据：[Signal System 设计][source-design-signal-system-235]。

<a id="term-signal-group"></a>
### 信号组

英文辅助名：signal group。

面向一组门或通行意图输出信号指示的静态组。

精确标识符：` SignalGroup `（类型）。

依据：[Signal System 设计][source-design-signal-system-235]。

<a id="term-signal-phase"></a>
### 信号相位

英文辅助名：signal phase。

信号控制器内具有稳定键的阶段声明。

精确标识符：` SignalPhase `（类型）。

依据：[Signal System 设计][source-design-signal-system-235]。

<a id="term-spatial-retention-build-option"></a>
### 空间保留构建选项

英文辅助名：spatial retention build option。

构建共享静态路网时决定是否保留可用空间组件的选项；不进入制品、存档或发布对象。

精确标识符：` SpatialBuildOption `（类型）。

依据：[共享静态路网][source-design-shared-static-network-187]。

<a id="term-static-network"></a>
### 静态路网

英文辅助名：static network。

不随世界固定步进改变的道路、关系、规则、几何和预计算索引集合。

依据：[Road / Junction / Maneuver 静态模型][source-design-road-junction-model-231]。

<a id="term-stop-line"></a>
### 停止线

英文辅助名：stop line。

表达车辆必须在其前满足通行约束的静态线。

精确标识符：` StopLine `（类型）。

依据：[Road / Junction / Maneuver 静态模型][source-design-road-junction-model-231]。

<a id="term-topology"></a>
### 拓扑

英文辅助名：topology。

边、节点、连接、所有权和可遍历关系。

依据：[Road / Junction / Maneuver 静态模型][source-design-road-junction-model-231]。

<a id="term-vehicle-profile"></a>
### 车辆配置

英文辅助名：vehicle profile。

冻结车辆运动与安全参数的静态配置。

精确标识符：` VehicleProfile `（类型）。

依据：[Vehicle Following 设计][source-design-vehicle-following-236]。

<a id="term-waiting-zone"></a>
### 等待区

英文辅助名：waiting zone。

绑定机动路径并表达等待容量、顺序或位置的静态区域。

精确标识符：` WaitingZone `（类型）。

依据：[Road / Junction / Maneuver 静态模型][source-design-road-junction-model-231]。

<a id="domain-artifacts"></a>
## 制品与验证

点击中文规范名查看完整定义；跨领域概念指向同一词条。

| 中文规范名                                                     | 英文辅助名                          | 精确标识符                                   |
| -------------------------------------------------------------- | ----------------------------------- | -------------------------------------------- |
| [对齐][term-alignment]                                         | alignment                           | —                                            |
| [应用二进制接口][term-application-binary-interface]            | application binary interface        | ` ABI `                                      |
| [有界结构校验器][term-bounded-structural-verifier]             | bounded structural verifier         | —                                            |
| [构建与目标选择器][term-build-and-target-selector]             | build and target selector           | —                                            |
| [字节完全一致][term-byte-for-byte-equality]                    | byte-for-byte equality              | ` exact bytes `                              |
| [字节序][term-byte-order]                                      | byte order；endianness              | —                                            |
| [调用方运行上限][term-caller-runtime-limit]                    | caller runtime limit                | —                                            |
| [规范发布候选][term-canonical-publication-candidate]           | canonical publication candidate     | ` PortablePublicationCandidate `             |
| [规范发布描述符][term-canonical-publication-descriptor]        | canonical publication descriptor    | ` CanonicalPublicationDescriptor `、` LFCP ` |
| [基数][term-cardinality]                                       | cardinality                         | —                                            |
| [受检规范路网输入][term-checked-canonical-network-input]       | checked canonical network input     | ` CheckedCanonicalNetworkInput `             |
| [封闭种类选择器][term-closed-kind-selector]                    | closed kind selector                | —                                            |
| [已关闭暂存对象来源][term-closed-staged-object-source]         | closed staged object source         | ` ClosedStagedObjectSource `                 |
| [编译器语义验证][term-compiler-semantic-validation]            | compiler semantic validation        | —                                            |
| [内容身份与长度绑定][term-content-identity-and-length-binding] | content identity and length binding | —                                            |
| [精确字节摘要][term-exact-bytes-digest]                        | exact-bytes digest                  | —                                            |
| [精确当前版本][term-exact-current]                             | exact-current                       | —                                            |
| [失败关闭][term-fail-closed]                                   | fail closed                         | —                                            |
| [格式硬上限][term-format-hard-limit]                           | format hard limit                   | —                                            |
| [不可变][term-immutable]                                       | immutable                           | —                                            |
| [不可变字节后备存储][term-immutable-byte-backing]              | immutable byte backing              | —                                            |
| [魔数][term-magic]                                             | magic                               | ` "LFID" `                                   |
| [内存映射][term-memory-mapping]                                | memory mapping；mmap                | —                                            |
| [最小发布闭合][term-minimal-publication-closure]               | minimal publication closure         | —                                            |
| [可变][term-mutable]                                           | mutable                             | —                                            |
| [偏移量][term-offset]                                          | offset                              | —                                            |
| [外层编码结构][term-outer-encoding-structure]                  | outer encoding structure            | —                                            |
| [可移植规范制品][term-portable-canonical-artifact]             | portable canonical artifact         | ` LFCA `                                     |
| [后发射检查][term-post-emission-check]                         | post-emission check                 | ` check_post_emission_bundle `               |
| [后发射受检制品组][term-post-emission-checked-bundle]          | post-emission checked bundle        | ` PostEmissionCheckedBundle `                |
| [发布清单][term-publication-manifest]                          | publication manifest                | —                                            |
| [修订—制品绑定][term-revision-artifact-binding]                | revision–artifact binding           | ` RevisionArtifactBindingV1 `                |
| [运行时前置条件][term-runtime-precondition]                    | runtime precondition                | —                                            |
| [节目录][term-section-directory]                               | section directory                   | —                                            |
| [语义差异][term-semantic-diff]                                 | semantic diff                       | —                                            |
| [语义差异封套][term-semantic-diff-envelope]                    | semantic diff envelope              | ` SemanticDiffEnvelope `、` LFSD `           |
| [源映射][term-source-map]                                      | source map                          | —                                            |
| [源映射封套][term-source-map-envelope]                         | source map envelope                 | ` SourceMapEnvelope `、` LFSM `              |
| [目标三元组][term-target-triple]                               | target triple                       | ` targetTriple `                             |
| [信任锚][term-trust-anchor]                                    | trust anchor                        | —                                            |
| [已值域检查规范制品][term-value-checked-canonical-artifact]    | value-checked canonical artifact    | ` ValueCheckedObjectView `                   |
| [版本轴][term-version-axis]                                    | version axis                        | —                                            |
| [包装类型][term-wrapper-type]                                  | wrapper type                        | —                                            |

<a id="term-alignment"></a>
### 对齐

英文辅助名：alignment。

数据地址或偏移量必须满足的倍数约束。

依据：[可移植规范制品与辅助制品格式][source-design-portable-canonical-artifact-152]。

<a id="term-application-binary-interface"></a>
### 应用二进制接口

英文辅助名：application binary interface。

规定二进制组件之间数据布局、对齐、字节序与调用约定的接口契约。

精确标识符：` ABI `（缩写）。

共享静态路网只提供进程内对象，不交付稳定镜像 ABI。

依据：[ADR 0025：受检规范路网与共享静态路网][source-adr-0025-280]。

<a id="term-bounded-structural-verifier"></a>
### 有界结构校验器

英文辅助名：bounded structural verifier。

对不可信制品字节执行偏移量、区间、数值、基数和资源上限检查的校验器。

依据：[可移植规范制品与辅助制品格式][source-design-portable-canonical-artifact-152]。

<a id="term-build-and-target-selector"></a>
### 构建与目标选择器

英文辅助名：build and target selector。

选择构建工具、目标平台或封闭配置档的字段；不是协议版本。

依据：[可移植规范制品与辅助制品格式][source-design-portable-canonical-artifact-152]。

<a id="term-byte-for-byte-equality"></a>
### 字节完全一致

英文辅助名：byte-for-byte equality。

两个目标对象的完整字节序列逐字节相同。

精确标识符：` exact bytes `（记号）。

依据：[可移植规范制品与辅助制品格式][source-design-portable-canonical-artifact-152]。

<a id="term-byte-order"></a>
### 字节序

英文辅助名：byte order；endianness。

多字节整数在制品中的排列规则。

依据：[可移植规范制品与辅助制品格式][source-design-portable-canonical-artifact-152]。

<a id="term-caller-runtime-limit"></a>
### 调用方运行上限

英文辅助名：caller runtime limit。

调用方针对一次读取或发射显式提供、可以低于但不得高于格式硬上限的资源预算；改变它不改变 wire version，也不授权扩大格式安全边界。

依据：[可移植规范制品与辅助制品格式][source-design-portable-canonical-artifact-152]。

<a id="term-canonical-publication-candidate"></a>
### 规范发布候选

英文辅助名：canonical publication candidate。

同次成功编译原子产生并拥有制品、源映射、差异与显式 base binding 的未发布候选；发布入口仍必须执行后发射 bundle 检查。

精确标识符：` PortablePublicationCandidate `（类型）。

依据：[编译器后发射检查与最小发布闭合][source-design-compiler-post-emission-check-and-minimal-publication-closure-281]。

<a id="term-canonical-publication-descriptor"></a>
### 规范发布描述符

英文辅助名：canonical publication descriptor。

绑定规范制品、源映射、来源链与对象键的发布描述；不自行证明对象已发布或来源可信。

精确标识符：` CanonicalPublicationDescriptor `（类型）、` LFCP `（缩写）。

依据：[可移植规范制品与辅助制品格式][source-design-portable-canonical-artifact-152]。

<a id="term-cardinality"></a>
### 基数

英文辅助名：cardinality。

表、节、实体、点或关系集合的记录数量。

依据：[可移植规范制品与辅助制品格式][source-design-portable-canonical-artifact-152]。

<a id="term-checked-canonical-network-input"></a>
### 受检规范路网输入

英文辅助名：checked canonical network input。

由格式检查器构造、绑定不可变受检制品来源、精确摘要与长度、重算路网修订标识的进程内能力，是共享静态路网构建器的唯一输入。

精确标识符：` CheckedCanonicalNetworkInput `（类型）。

依据：[编译器后发射检查与最小发布闭合][source-design-compiler-post-emission-check-and-minimal-publication-closure-281]。

<a id="term-closed-kind-selector"></a>
### 封闭种类选择器

英文辅助名：closed kind selector。

选择版本化封套的封闭种类及必需字段集合；不是协议版本。

依据：[可移植规范制品与辅助制品格式][source-design-portable-canonical-artifact-152]。

<a id="term-closed-staged-object-source"></a>
### 已关闭暂存对象来源

英文辅助名：closed staged object source。

字段私有 staged writer 完成 flush、固定 file identity/exact length 并关闭 LaneFlow 全部写 handle/mapping 后得到的可重读来源；checker 与共享静态构建直接复用，不要求安装到内容仓库。

精确标识符：` ClosedStagedObjectSource `（类型）。

依据：[编译器后发射检查与最小发布闭合][source-design-compiler-post-emission-check-and-minimal-publication-closure-281]。

<a id="term-compiler-semantic-validation"></a>
### 编译器语义验证

英文辅助名：compiler semantic validation。

编译器对来源和 IR 执行的主语义检查。

依据：[编译器基础设施与合成领域专用语言前端][source-design-compiler-foundation-149]。

<a id="term-content-identity-and-length-binding"></a>
### 内容身份与长度绑定

英文辅助名：content identity and length binding。

以摘要和精确字节长度共同认证目标对象、或以语义修订标识认证规范内容的外部绑定字段。

依据：[可移植规范制品与辅助制品格式][source-design-portable-canonical-artifact-152]。

<a id="term-exact-bytes-digest"></a>
### 精确字节摘要

英文辅助名：exact-bytes digest。

对目标对象完整字节序列计算的 SHA-256；目标对象不得嵌入自身摘要。

依据：[可移植规范制品与辅助制品格式][source-design-portable-canonical-artifact-152]。

<a id="term-exact-current"></a>
### 精确当前版本

英文辅助名：exact-current。

只接受当前冻结版本，不在生产路径隐式兼容历史或未来版本。

依据：[可移植规范制品与辅助制品格式][source-design-portable-canonical-artifact-152]。

<a id="term-fail-closed"></a>
### 失败关闭

英文辅助名：fail closed。

发生歧义、缺失、越界、验证失败或外部状态不可确认时拒绝继续。

依据：[可移植规范制品与辅助制品格式][source-design-portable-canonical-artifact-152]。

<a id="term-format-hard-limit"></a>
### 格式硬上限

英文辅助名：format hard limit。

由格式版本冻结、任何读取器都不得提高或禁用的安全天花板；用于在 hash、分配和建立受检视图前限制对象、计数、字符串、向量和累计暂存规模，不是性能目标。

依据：[可移植规范制品与辅助制品格式][source-design-portable-canonical-artifact-152]。

<a id="term-immutable"></a>
### 不可变

英文辅助名：immutable。

构造后不再改变，可由多个世界/会话安全共享。

依据：[可移植规范制品与辅助制品格式][source-design-portable-canonical-artifact-152]。

<a id="term-immutable-byte-backing"></a>
### 不可变字节后备存储

英文辅助名：immutable byte backing。

从分块验证完成到有类型视图释放期间保持同一精确字节、且不存在可写别名的资产或拥有存储。

依据：[编译器后发射检查与最小发布闭合][source-design-compiler-post-emission-check-and-minimal-publication-closure-281]。

<a id="term-magic"></a>
### 魔数

英文辅助名：magic。

用固定字节区分协议或制品类型的标记。

精确标识符：` "LFID" `（常量）。

依据：[可移植规范制品与辅助制品格式][source-design-portable-canonical-artifact-152]。

<a id="term-memory-mapping"></a>
### 内存映射

英文辅助名：memory mapping；mmap。

把文件字节映射到地址空间的访问方式；当前私有制品暂存用途不定义共享静态路网文件 ABI 或跨进程页共享。

依据：[编译器后发射检查与最小发布闭合][source-design-compiler-post-emission-check-and-minimal-publication-closure-281]。

<a id="term-minimal-publication-closure"></a>
### 最小发布闭合

英文辅助名：minimal publication closure。

从受检能力构造发布描述符绑定、由宿主负责持久化、真实性认证与文件事务的发布职责边界。

依据：[编译器后发射检查与最小发布闭合][source-design-compiler-post-emission-check-and-minimal-publication-closure-281]。

<a id="term-mutable"></a>
### 可变

英文辅助名：mutable。

随运行时命令、固定步进或生命周期改变，必须由明确最终裁决职责拥有。

依据：[可移植规范制品与辅助制品格式][source-design-portable-canonical-artifact-152]。

<a id="term-offset"></a>
### 偏移量

英文辅助名：offset。

从容器或节起点到目标数据的字节距离。

依据：[可移植规范制品与辅助制品格式][source-design-portable-canonical-artifact-152]。

<a id="term-outer-encoding-structure"></a>
### 外层编码结构

英文辅助名：outer encoding structure。

在被描述的内容之外添加层级或组帧信息的编码结构；例如限定 Bytes 只包含 RowV1 时，被排除的 TableV1 或其他外层结构。

<a id="term-portable-canonical-artifact"></a>
### 可移植规范制品

英文辅助名：portable canonical artifact。

平台无关、确定性、可发布、可长期审计并可由公共格式检查和后继工具读取的静态路网制品。

精确标识符：` LFCA `（缩写）。

依据：[可移植规范制品与辅助制品格式][source-design-portable-canonical-artifact-152]。

<a id="term-post-emission-check"></a>
### 后发射检查

英文辅助名：post-emission check。

依据最终规范制品、源映射、语义差异及显式差异基线，重算摘要、长度与修订并核对跨对象绑定的检查。

精确标识符：` check_post_emission_bundle `（函数）。

依据：[编译器后发射检查与最小发布闭合][source-design-compiler-post-emission-check-and-minimal-publication-closure-281]。

<a id="term-post-emission-checked-bundle"></a>
### 后发射受检制品组

英文辅助名：post-emission checked bundle。

后发射检查成功后持有三对象不可变受检来源句柄的进程内能力；字段私有且不可序列化，不要求三份完整对象同时驻留，也不表示已发布或可信。

精确标识符：` PostEmissionCheckedBundle `（类型）。

依据：[编译器后发射检查与最小发布闭合][source-design-compiler-post-emission-check-and-minimal-publication-closure-281]。

<a id="term-publication-manifest"></a>
### 发布清单

英文辅助名：publication manifest。

对制品集合、外部描述符、摘要和真实性进行发布级绑定的清单。

依据：[可移植规范制品与辅助制品格式][source-design-portable-canonical-artifact-152]。

<a id="term-revision-artifact-binding"></a>
### 修订—制品绑定

英文辅助名：revision–artifact binding。

以路网修订派生版本、路网修订标识、目标制品精确字节摘要和精确字节长度四项共同标识一份制品；只比较 revision 不能证明 exact bytes 相同，只比较 digest/length 不能证明语义修订声明正确。

精确标识符：` RevisionArtifactBindingV1 `（类型）。

依据：[可移植规范制品与辅助制品格式][source-design-portable-canonical-artifact-152]。

<a id="term-runtime-precondition"></a>
### 运行时前置条件

英文辅助名：runtime precondition。

建立视图或世界前必须由结构校验器证明成立的不变量。

依据：[可移植规范制品与辅助制品格式][source-design-portable-canonical-artifact-152]。

<a id="term-section-directory"></a>
### 节目录

英文辅助名：section directory。

记录制品各数据节种类、位置和长度等结构信息的目录；精确字段由所属格式规定。

依据：[可移植规范制品与辅助制品格式][source-design-portable-canonical-artifact-152]。

<a id="term-semantic-diff"></a>
### 语义差异

英文辅助名：semantic diff。

以稳定标识和所有者局部键比较两个规范制品语义变化的结构化结果；它不能自行决定修订激活，跨修订状态迁移必须由对象外可信切换描述符绑定。

依据：[可移植规范制品与辅助制品格式][source-design-portable-canonical-artifact-152]。

<a id="term-semantic-diff-envelope"></a>
### 语义差异封套

英文辅助名：semantic diff envelope。

版本化绑定旧/新路网修订与规范制品精确字节，并承载规范排序差异记录的派生制品封套。

精确标识符：` SemanticDiffEnvelope `（类型）、` LFSD `（缩写）。

依据：[可移植规范制品与辅助制品格式][source-design-portable-canonical-artifact-152]。

<a id="term-source-map"></a>
### 源映射

英文辅助名：source map。

把规范实体、关系或诊断反向关联到来源模块和来源位置的派生制品。

依据：[可移植规范制品与辅助制品格式][source-design-portable-canonical-artifact-152]。

<a id="term-source-map-envelope"></a>
### 源映射封套

英文辅助名：source map envelope。

版本化绑定精确规范制品、路网修订与编译来源链，并承载源映射记录的派生制品封套。

精确标识符：` SourceMapEnvelope `（类型）、` LFSM `（缩写）。

依据：[可移植规范制品与辅助制品格式][source-design-portable-canonical-artifact-152]。

<a id="term-target-triple"></a>
### 目标三元组

英文辅助名：target triple。

冻结 CPU 架构、供应商、操作系统和 ABI 环境的目标标识。

精确标识符：` targetTriple `（字段）。

依据：[可移植规范制品与辅助制品格式][source-design-portable-canonical-artifact-152]。

<a id="term-trust-anchor"></a>
### 信任锚

英文辅助名：trust anchor。

位于待验证对象之外、由签名、认证资产链或固定摘要提供的可信依据。

依据：[可移植规范制品与辅助制品格式][source-design-portable-canonical-artifact-152]。

<a id="term-value-checked-canonical-artifact"></a>
### 已值域检查规范制品

英文辅助名：value-checked canonical artifact。

通过有界结构、registry 和直接值域检查的借用型可移植对象视图；不证明跨对象 binding、来源真实性或发布状态。

精确标识符：` ValueCheckedObjectView `（类型）。

依据：[可移植规范制品与辅助制品格式][source-design-portable-canonical-artifact-152]。

<a id="term-version-axis"></a>
### 版本轴

英文辅助名：version axis。

只描述一个独立兼容维度、不能与其他维度混用的版本字段。

依据：[可移植规范制品与辅助制品格式][source-design-portable-canonical-artifact-152]。

<a id="term-wrapper-type"></a>
### 包装类型

英文辅助名：wrapper type。

在类型层面包裹已有值的类型，如按值持有 `Vec` 的批次类型；不据此推定存在文件格式或版本化制品。

<a id="domain-runtime"></a>
## 交通运行时

点击中文规范名查看完整定义；跨领域概念指向同一词条。

| 中文规范名                                                                       | 英文辅助名                                        | 精确标识符                      |
| -------------------------------------------------------------------------------- | ------------------------------------------------- | ------------------------------- |
| [活动紧凑位置][term-active-compact-position]                                     | Active Compact Position                           | —                               |
| [活动运动池][term-active-motion-pool]                                            | Active Motion Pool                                | —                               |
| [管理状态][term-administrative-state]                                            | administrative state                              | ` AdministrativeState `         |
| [阿姆达尔瓶颈][term-amdahl-bottleneck]                                           | Amdahl bottleneck                                 | —                               |
| [尝试世代][term-attempt-epoch]                                                   | Attempt Epoch                                     | —                               |
| [认证操作失败][term-authentication-operational-failure]                          | authentication operational failure                | —                               |
| [编制准入量化][term-authoring-admission-quantization]                            | authoring admission quantization                  | —                               |
| [屏障][term-barrier]                                                             | barrier                                           | —                               |
| [分块列式布局][term-block-soa]                                                   | Block SoA                                         | —                               |
| [车身溢出占用][term-body-spill-occupancy]                                        | body spill occupancy                              | —                               |
| [边界交换][term-boundary-exchange]                                               | boundary exchange                                 | —                               |
| [候选][term-candidate]                                                           | candidate                                         | —                               |
| [候选执行依赖图][term-candidate-execution-dependency-graph]                      | Candidate Execution Dependency Graph              | —                               |
| [规范逻辑位置][term-canonical-logical-position]                                  | Canonical Logical Position                        | —                               |
| [规范归约裁决方][term-canonical-reduction-authority]                             | canonical reduction authority                     | —                               |
| [已提交世界状态][term-committed-world-state]                                     | committed world state                             | ` CommittedWorldState `         |
| [完成前沿][term-completion-frontier]                                             | Completion Frontier                               | —                               |
| [凝聚有向无环图][term-condensation-directed-acyclic-graph]                       | condensation directed acyclic graph               | ` condensation DAG `            |
| [冲突滞后基准][term-conflict-lag-reference]                                      | conflict lag reference                            | ` ConflictLagReference `        |
| [冲突通行段持久定位值][term-conflict-passage-locator]                            | conflict passage locator                          | ` ConflictPassageLocator `      |
| [约束][term-constraint]                                                          | constraint                                        | —                               |
| [派生索引][term-derived-indexes]                                                 | derived indexes                                   | ` DerivedIndexes `              |
| [确定性提交计划][term-deterministic-commit-plan]                                 | Deterministic Commit Plan                         | ` CommitPlan `                  |
| [确定性基数/桶排序][term-deterministic-radix-bucket-sorting]                     | deterministic radix/bucket sorting                | —                               |
| [失真][term-distortion]                                                          | distortion                                        | —                               |
| [判因][term-distortion-attribution]                                              | distortion attribution                            | —                               |
| [失真指纹][term-distortion-fingerprint]                                          | distortion fingerprint                            | —                               |
| [耐久修订内容基线][term-durable-revision-content-baseline]                       | durable revision content baseline                 | —                               |
| [耐久修订内容原像][term-durable-revision-content-preimage]                       | durable revision content preimage                 | —                               |
| [动态通行定义][term-dynamic-traversal-definition]                                | dynamic traversal definition                      | —                               |
| [入口 frontier][term-entry-frontier]                                             | entry frontier                                    | —                               |
| [精确执行路径][term-exact-execution-path]                                        | exact execution path                              | —                               |
| [精确参考预言机][term-exact-reference-oracle]                                    | exact reference oracle                            | —                               |
| [执行块][term-execution-block]                                                   | Execution Block                                   | —                               |
| [执行配置][term-execution-configuration]                                         | Execution Configuration                           | ` ExecutionConfig `             |
| [执行资源][term-execution-resources]                                             | Execution Resources                               | —                               |
| [保真度契约][term-fidelity-contract]                                             | fidelity contract                                 | —                               |
| [保真度降级][term-fidelity-degradation]                                          | fidelity degradation                              | —                               |
| [固定步进][term-fixed-tick]                                                      | fixed tick                                        | ` tick `                        |
| [新生成][term-fresh-placement]                                                   | fresh placement                                   | ` spawn_vehicle `               |
| [融合单工作线程执行器][term-fused-single-worker-executor]                        | fused single-worker executor                      | —                               |
| [间隙接受参数][term-gap-acceptance-profile]                                      | gap acceptance profile                            | ` GapAcceptanceProfile `        |
| [门合规规则][term-gate-compliance-rule]                                          | gate compliance rule                              | ` GateComplianceRule `          |
| [过载优雅降级][term-graceful-overload-degradation]                               | graceful overload degradation                     | —                               |
| [授权][term-grant]                                                               | grant                                             | —                               |
| [边界邻域][term-halo]                                                            | halo                                              | —                               |
| [宿主审计重放][term-host-audit-replay]                                           | host audit replay                                 | —                               |
| [整数毫米][term-integer-millimetre]                                              | integer millimetre                                | ` mm `                          |
| [停用][term-integrity-disablement]                                               | integrity disablement                             | —                               |
| [路口规则][term-intersection-rules]                                              | intersection rules                                | —                               |
| [路口观测视图][term-lane-flow-junction-observation]                              | lane flow junction observation                    | ` LaneFlowJunctionObservation ` |
| [车道图][term-lane-graph]                                                        | lane graph                                        | —                               |
| [LaneFlow 交通运行时][term-laneflow-traffic-runtime]                             | LaneFlow Traffic Runtime                          | ` laneflow-runtime `            |
| [生命周期][term-lifecycle]                                                       | lifecycle                                         | —                               |
| [局部意图批次][term-local-intent-batch]                                          | local intent batch                                | ` LocalIntentBatch `            |
| [逻辑检查地址][term-logical-check-address]                                       | Logical Check Address                             | —                               |
| [逻辑摘要原像][term-logical-digest-preimage]                                     | logical digest preimage                           | —                               |
| [逻辑更新位置][term-logical-update-position]                                     | Logical Update Position                           | —                               |
| [机动方向][term-maneuver-direction]                                              | maneuver direction                                | ` ManeuverDirection `           |
| [微米累加器][term-micrometre-accumulator]                                        | micrometre accumulator                            | ` carry_um `                    |
| [毫米每秒][term-millimetres-per-second]                                          | millimetres per second                            | ` mm/s `                        |
| [漏检面][term-missed-detection-surface]                                          | missed-detection surface                          | —                               |
| [互不可见的共横向过渡段][term-mutually-invisible-co-lateral-transition-segments] | mutually invisible co-lateral transition segments | —                               |
| [近门车辆][term-near-gate-vehicle]                                               | near-gate vehicle                                 | —                               |
| [占用][term-occupancy]                                                           | occupancy                                         | —                               |
| [占用桶][term-occupancy-bucket]                                                  | occupancy bucket                                  | ` OccupancyBucketOrdinal `      |
| [占用索引][term-occupancy-index]                                                 | occupancy index                                   | ` OccupancyIndex `              |
| [占用记录][term-occupancy-record]                                                | occupancy record                                  | ` OccupancyRecord `             |
| [开放入口][term-open-entrance]                                                   | open entrance                                     | ` VehicleEntrance `             |
| [页级内存失真][term-page-level-memory-distortion]                                | page-level memory distortion                      | —                               |
| [停车][term-parking]                                                             | parking                                           | —                               |
| [停车到达][term-parking-arrival]                                                 | parking arrival                                   | ` ParkingArrivalObservation `   |
| [停车绑定][term-parking-binding]                                                 | parking binding                                   | ` ParkingBinding `              |
| [停车设施][term-parking-facility]                                                | parking facility                                  | ` ParkingFacility `             |
| [停车目标][term-parking-target]                                                  | parking target                                    | ` ParkingTarget `               |
| [分区][term-partition]                                                           | partition                                         | —                               |
| [分区规划提示][term-partition-planning-hints]                                    | partition planning hints                          | ` PartitionPlanningHints `      |
| [每世界可变状态][term-per-world-mutable-state]                                   | per-world mutable state                           | —                               |
| [物理行][term-physical-row]                                                      | Physical Row                                      | —                               |
| [策略][term-policy]                                                              | policy                                            | —                               |
| [策略局部成员变更][term-policy-local-change]                                     | policy local change                               | ` PolicyLocalChange `           |
| [策略绑定][term-policy-pin]                                                      | policy pin                                        | ` PolicyPin `                   |
| [潜在资源支持图][term-potential-resource-support-graph]                          | Potential Resource Support Graph                  | —                               |
| [提案][term-proposal]                                                            | proposal                                          | —                               |
| [归约跨度][term-reduction-span]                                                  | reduction span                                    | —                               |
| [归约工作量][term-reduction-work]                                                | reduction work                                    | —                               |
| [法规身份][term-regulation-identity]                                             | regulation identity                               | ` RegulationIdentity `          |
| [预约][term-reservation]                                                         | reservation                                       | —                               |
| [裁决批次][term-resolution-batch]                                                | resolution batch                                  | ` ResolutionBatch `             |
| [已解析门策略单元][term-resolved-gate-policy-cell]                               | resolved gate policy cell                         | ` ResolvedGatePolicyCell `      |
| [已解析通行流规则][term-resolved-stream-rule]                                    | resolved stream rule                              | ` ResolvedStreamRule `          |
| [资源声明][term-resource-claim]                                                  | resource claim；claim                             | —                               |
| [资源依赖组件][term-resource-dependency-component]                               | Resource Dependency Component                     | —                               |
| [路权策略集][term-right-of-way-policy-set]                                       | right of way policy set                           | ` RightOfWayPolicySet `         |
| [道路机动车执行域][term-road-motor-vehicle-execution-domain]                     | road motor vehicle execution domain               | ` road_motor_vehicle `          |
| [路线][term-route]                                                               | route                                             | ` Route `                       |
| [路线机动门观测][term-route-gate-observation]                                    | route gate observation                            | ` RouteGateObservation `        |
| [运行时覆盖层][term-runtime-overlay]                                             | runtime overlay                                   | —                               |
| [串行阶段协议][term-serial-phase-protocol]                                       | Serial Phase Protocol                             | —                               |
| [会话][term-session]                                                             | session                                           | —                               |
| [共享路网修订认证][term-shared-network-revision-authentication]                  | shared network revision authentication            | —                               |
| [信号][term-signal]                                                              | signal                                            | —                               |
| [快照][term-snapshot]                                                            | snapshot                                          | —                               |
| [静态执行约束图][term-static-execution-constraint-graph]                         | static execution constraint graph                 | —                               |
| [停驻状态][term-stationary-state]                                                | stationary state                                  | —                               |
| [稳态步进][term-steady-state-tick]                                               | steady-state tick                                 | —                               |
| [步进只读视图][term-step-read-view]                                              | step read view                                    | ` StepReadView `                |
| [通行流路权规则][term-stream-right-of-way-rule]                                  | stream right of way rule                          | ` StreamRightOfWayRule `        |
| [强连通分量][term-strongly-connected-component]                                  | strongly connected component                      | ` SCC `                         |
| [步进工作区][term-tick-workspace]                                                | tick workspace                                    | ` TickWorkspace `               |
| [交通配置][term-traffic-configuration]                                           | Traffic Configuration                             | ` WorldConfig `                 |
| [交通执行域][term-traffic-execution-domain]                                      | traffic execution domain                          | —                               |
| [交通保真度][term-traffic-fidelity]                                              | traffic fidelity                                  | —                               |
| [交通参与者][term-traffic-participant]                                           | traffic participant                               | —                               |
| [交通参与单元][term-traffic-participant-unit]                                    | traffic participant unit                          | —                               |
| [交通转移事件][term-traffic-transition-event]                                    | traffic transition event                          | ` TrafficTransitionEvent `      |
| [交通世界][term-traffic-world]                                                   | traffic world                                     | ` TrafficWorld `                |
| [转移锚点][term-transition-anchor]                                               | transition anchor                                 | ` TrafficTransitionAnchor `     |
| [可信基准][term-trusted-baseline]                                                | trusted baseline                                  | —                               |
| [车辆目录][term-vehicle-directory]                                               | Vehicle Directory                                 | —                               |
| [跟车][term-vehicle-following]                                                   | vehicle following                                 | —                               |
| [跟车前视][term-vehicle-following-front-lookahead]                               | vehicle-following front lookahead                 | ` front_query_horizon `         |
| [车辆运行单元][term-vehicle-runtime-unit]                                        | vehicle runtime unit                              | ` VehicleState `                |
| [车辆状态][term-vehicle-state]                                                   | vehicle state                                     | —                               |
| [虚拟停驻][term-virtual-parked-state]                                            | virtual parked state                              | —                               |
| [虚拟停车容量][term-virtual-parking-capacity]                                    | virtual parking capacity                          | ` virtual_capacity `            |
| [等待依赖视图][term-waiting-dependency-view]                                     | waiting dependency view                           | —                               |
| [工作线程][term-worker]                                                          | worker                                            | —                               |
| [世界绑定状态][term-world-binding-state]                                         | world binding state                               | ` WorldBindingState `           |
| [世界策略选择][term-world-policy-selection]                                      | world policy selection                            | ` WorldPolicySelection `        |

<a id="term-active-compact-position"></a>
### 活动紧凑位置

英文辅助名：Active Compact Position。

Active 工作集中的连续定位下标，必须映射到完整车辆句柄和逻辑更新位置。

依据：[TrafficWorld 单世界精确并行执行][source-design-traffic-runtime-parallel-execution-410]。

<a id="term-active-motion-pool"></a>
### 活动运动池

英文辅助名：Active Motion Pool。

只保存 Active 车辆正本热运动列的存储池；非活动记录不参与常规热扫描。

依据：[交通运行时列式批量执行][source-design-traffic-runtime-columnar-execution-411]。

<a id="term-administrative-state"></a>
### 管理状态

英文辅助名：administrative state。

迁移日志、事务配对世代等每世界管理数据；可参与提交，不能仅因位于管理分区而视为可丢弃缓存。

精确标识符：` AdministrativeState `（类型）。

依据：[TrafficWorld 串行阶段协议与内部状态分区][source-design-traffic-runtime-phase-protocol-412]。

<a id="term-amdahl-bottleneck"></a>
### 阿姆达尔瓶颈

英文辅助名：Amdahl bottleneck。

不可并行的临界路径限制整体加速比、使增加工作线程不再获得相称收益的扩展性瓶颈。

依据：[TrafficWorld 单世界精确并行执行][source-design-traffic-runtime-parallel-execution-410]。

<a id="term-attempt-epoch"></a>
### 尝试世代

英文辅助名：Attempt Epoch。

区分同一已提交 tick 的不同执行尝试，防止旧回报或 capability 污染重试；不持久化。

依据：[TrafficWorld 单世界精确并行执行][source-design-traffic-runtime-parallel-execution-410]。

<a id="term-authentication-operational-failure"></a>
### 认证操作失败

英文辅助名：authentication operational failure。

共享路网修订认证因资源、限额、取消或来源不可得等操作条件而未产生比较结论的结果；不判因、不停用，也不宣称认证通过。

依据：[运行时状态完整性校验与失真恢复][source-design-traffic-runtime-state-integrity-413]。

<a id="term-authoring-admission-quantization"></a>
### 编制准入量化

英文辅助名：authoring admission quantization。

编制 `f64` SI 在官方前端 / 共同接入边界 round-ties-to-even 一次进入整数毫米或受检 `f32` SI；之后 IR 不得再把交通一维存成编制米列。

依据：[交通运行时整数毫米几何][source-design-traffic-runtime-integer-geometry-414]。

<a id="term-barrier"></a>
### 屏障

英文辅助名：barrier。

一个逻辑阶段等待全部必需分区输入就绪后才允许进入规范归约或提交的同步边界。

依据：[TrafficWorld 单世界精确并行执行][source-design-traffic-runtime-parallel-execution-410]。

<a id="term-block-soa"></a>
### 分块列式布局

英文辅助名：Block SoA。

固定行数块内同字段连续存储；块大小属于执行布局，不属于交通身份或持久格式。

依据：[交通运行时列式批量执行][source-design-traffic-runtime-columnar-execution-411]。

<a id="term-body-spill-occupancy"></a>
### 车身溢出占用

英文辅助名：body spill occupancy。

车身仍覆盖、但前保险杠已离开的更早物理边上的占用记录；不是变道双占用。

依据：[Vehicle Following 设计][source-design-vehicle-following-236]。

<a id="term-boundary-exchange"></a>
### 边界交换

英文辅助名：boundary exchange。

同一 tick 内分区间传递规范输入、提案或资源声明的私有运行时过程。

依据：[TrafficWorld 单世界精确并行执行][source-design-traffic-runtime-parallel-execution-410]。

<a id="term-candidate"></a>
### 候选

英文辅助名：candidate。

尚未通过全部约束、验证或裁决的可能结果。

<a id="term-candidate-execution-dependency-graph"></a>
### 候选执行依赖图

英文辅助名：Candidate Execution Dependency Graph。

以不可交换候选操作为节点并按规范先后定向的执行约束图。

依据：[TrafficWorld 单世界精确并行执行][source-design-traffic-runtime-parallel-execution-410]。

<a id="term-canonical-logical-position"></a>
### 规范逻辑位置

英文辅助名：Canonical Logical Position。

独立计算的规范消费位置；车辆级顺序来自逻辑更新位置，车内顺序来自逻辑检查地址。

依据：[TrafficWorld 单世界精确并行执行][source-design-traffic-runtime-parallel-execution-410]。

<a id="term-canonical-reduction-authority"></a>
### 规范归约裁决方

英文辅助名：canonical reduction authority。

对一个连接资源组件按稳定键形成唯一提交结果的逻辑裁决方；不要求由单一物理线程串行执行。

依据：[TrafficWorld 单世界精确并行执行][source-design-traffic-runtime-parallel-execution-410]。

<a id="term-committed-world-state"></a>
### 已提交世界状态

英文辅助名：committed world state。

当前世界的车辆、路线、资源裁决权、时钟和游标，以及成功提交后可查询的发布批次；内部归属不等同于快照保存范围。

精确标识符：` CommittedWorldState `（类型）。

依据：[TrafficWorld 串行阶段协议与内部状态分区][source-design-traffic-runtime-phase-protocol-412]。

<a id="term-completion-frontier"></a>
### 完成前沿

英文辅助名：Completion Frontier。

证明某错误之前的必要逻辑检查已解析的记录；缺失回报不能当作成功或跳过。

依据：[TrafficWorld 单世界精确并行执行][source-design-traffic-runtime-parallel-execution-410]。

<a id="term-condensation-directed-acyclic-graph"></a>
### 凝聚有向无环图

英文辅助名：condensation directed acyclic graph。

把强连通分量收缩为节点后得到、可用于确定性波次调度的有向无环图。

精确标识符：` condensation DAG `（记号）。

依据：[TrafficWorld 单世界精确并行执行][source-design-traffic-runtime-parallel-execution-410]。

<a id="term-conflict-lag-reference"></a>
### 冲突滞后基准

英文辅助名：conflict lag reference。

单元的无历史、实际清空时间或保守切换起点三种状态；后两者参与相同的滞后间隙检查，切换起点不表示真实清空事件。

精确标识符：` ConflictLagReference `（类型）。

依据：[路权策略编译与 ConflictArbiter 实施合同][source-design-traffic-runtime-right-of-way-policy-415]。

<a id="term-conflict-passage-locator"></a>
### 冲突通行段持久定位值

英文辅助名：conflict passage locator。

stream 与 zone 的稳定引用对，用于存档和迁移时定位已有唯一局部关系；不是独立实体或新的运行时地址。

精确标识符：` ConflictPassageLocator `（类型）。

依据：[路权策略编译与 ConflictArbiter 实施合同][source-design-traffic-runtime-right-of-way-policy-415]。

<a id="term-constraint"></a>
### 约束

英文辅助名：constraint。

对有效值、行为候选或运行时结果施加的不可绕过条件。

<a id="term-derived-indexes"></a>
### 派生索引

英文辅助名：derived indexes。

从当前绑定与已提交状态重建的查询、执行顺序和定位结构；不独立授予交通权利，重建不得改变语义顺序。

精确标识符：` DerivedIndexes `（类型）。

依据：[TrafficWorld 串行阶段协议与内部状态分区][source-design-traffic-runtime-phase-protocol-412]。

<a id="term-deterministic-commit-plan"></a>
### 确定性提交计划

英文辅助名：Deterministic Commit Plan。

对同一基线完整验证的车辆、资源、时钟、游标、日志效果及发布结果；一次消费后无可恢复错误地提交，不是世界副本或新的存档对象。

精确标识符：` CommitPlan `（类型）。

依据：[TrafficWorld 串行阶段协议与内部状态分区][source-design-traffic-runtime-phase-protocol-412]。

<a id="term-deterministic-radix-bucket-sorting"></a>
### 确定性基数/桶排序

英文辅助名：deterministic radix/bucket sorting。

对固定宽度规范键采用与输入规模线性相关、且不依赖线程完成顺序的排序策略族。

依据：[TrafficWorld 单世界精确并行执行][source-design-traffic-runtime-parallel-execution-410]。

<a id="term-distortion"></a>
### 失真

英文辅助名：distortion。

运行时状态完整性语境中，数据内容偏离应有内容的现象；该用词本身不确定原因、判因结论或检测覆盖。具体对象与检测、处置边界以完整性设计为准。

依据：[运行时状态完整性校验与失真恢复][source-design-traffic-runtime-state-integrity-413]。

<a id="term-distortion-attribution"></a>
### 判因

英文辅助名：distortion attribution。

对完整性失配原因作出的归属判断；允许无法判因，不承诺唯一根因。

依据：[运行时状态完整性校验与失真恢复][source-design-traffic-runtime-state-integrity-413]。

<a id="term-distortion-fingerprint"></a>
### 失真指纹

英文辅助名：distortion fingerprint。

失配现场的页级形态分类（页边界偏移、整页替换、页整数倍位移、零页、非页对齐），用于环境对照与判因；非页对齐提示转代码嫌疑。共享路网修订认证只报告逻辑位置，不输出页级指纹。

依据：[运行时状态完整性校验与失真恢复][source-design-traffic-runtime-state-integrity-413]。

<a id="term-durable-revision-content-baseline"></a>
### 耐久修订内容基线

英文辅助名：durable revision content baseline。

由构建方建立、宿主随来源持久保存或重建的修订内容依据，是共享路网修订认证的期望值来源之一；发布来源与编辑来源的保存方式不同。

依据：[运行时状态完整性校验与失真恢复][source-design-traffic-runtime-state-integrity-413]。

<a id="term-durable-revision-content-preimage"></a>
### 耐久修订内容原像

英文辅助名：durable revision content preimage。

与路网修订标识共用的规范路网语义载荷原像；不包含布局与派生表，也不另设第二套编码。

依据：[运行时状态完整性校验与失真恢复][source-design-traffic-runtime-state-integrity-413]。

<a id="term-dynamic-traversal-definition"></a>
### 动态通行定义

英文辅助名：dynamic traversal definition。

目标交通运行时中描述参与单元运行时通行路径及进度语义的通用概念；当前道路机动车执行域的具体投影是 `Route`。

当前交付为道路机动车特化；长期通用概念不表示其他执行域已经实现，也不改写历史车辆证据。

依据：[ADR 0021：交通基础设施定位与宿主边界][source-adr-0021-416]。

<a id="term-entry-frontier"></a>
### 入口 frontier

英文辅助名：entry frontier。

为让行查询维护的冲突接近记录。只为近门车辆打开路线，失效车辆整段重走，其余按反向索引复用冲突距离。

依据：[路权策略编译与 ConflictArbiter 实施合同][source-design-traffic-runtime-right-of-way-policy-415]。

<a id="term-exact-execution-path"></a>
### 精确执行路径

英文辅助名：exact execution path。

不降低交通保真度、且结果不依赖工作线程数或分区计划的执行路径。

依据：[TrafficWorld 单世界精确并行执行][source-design-traffic-runtime-parallel-execution-410]。

<a id="term-exact-reference-oracle"></a>
### 精确参考预言机

英文辅助名：exact reference oracle。

以直观集中式算法产生规范结果、用于证明优化执行路径语义等价而不要求成为生产热路径的实现。

依据：[TrafficWorld 单世界精确并行执行][source-design-traffic-runtime-parallel-execution-410]。

<a id="term-execution-block"></a>
### 执行块

英文辅助名：Execution Block。

独占任务输出范围的物理调度分块；不创建新的交通身份或公开子世界。

依据：[TrafficWorld 单世界精确并行执行][source-design-traffic-runtime-parallel-execution-410]。

<a id="term-execution-configuration"></a>
### 执行配置

英文辅助名：Execution Configuration。

安装和恢复时由宿主显式提供的执行能力要求；不进入交通快照、摘要或身份。

精确标识符：` ExecutionConfig `（类型）。

依据：[TrafficWorld 执行配置、资源与快照边界][source-design-traffic-runtime-execution-config-417]。

<a id="term-execution-resources"></a>
### 执行资源

英文辅助名：Execution Resources。

活动世界拥有的线程与调度资源，和可重建的目标执行计划分开管理。

依据：[TrafficWorld 执行配置、资源与快照边界][source-design-traffic-runtime-execution-config-417]。

<a id="term-fidelity-contract"></a>
### 保真度契约

英文辅助名：fidelity contract。

明确允许的个体、时间、空间与行为语义损失及其验证边界的版本化契约。

依据：[TrafficWorld 单世界精确并行执行][source-design-traffic-runtime-parallel-execution-410]。

<a id="term-fidelity-degradation"></a>
### 保真度降级

英文辅助名：fidelity degradation。

显式减少个体、时间、空间或行为语义的目标能力变化；必须由独立保真度契约和 G1/ADR 授权，不能伪装成调度或表现优化。

依据：[TrafficWorld 单世界精确并行执行][source-design-traffic-runtime-parallel-execution-410]。

<a id="term-fixed-tick"></a>
### 固定步进

英文辅助名：fixed tick。

以确定输入和顺序推进交通状态的一次运行时更新。合法步长 `4..=1000` ms，4 ms 为最细量子，不是默认。

精确标识符：` tick `（函数）。

依据：[交通运行时整数毫米几何][source-design-traffic-runtime-integer-geometry-414]。

<a id="term-fresh-placement"></a>
### 新生成

英文辅助名：fresh placement。

在当前提交时点把一辆尚未处于该运动状态的车放入世界的操作；不意味着从路线起点以零速出发。

精确标识符：` spawn_vehicle `（函数）。

运动安全、开放入口与显式历史起点的可达性检查各有适用条件，以车辆放置设计为准。

依据：[新生成的运动安全准入][source-design-traffic-runtime-vehicle-placement-418]。

<a id="term-fused-single-worker-executor"></a>
### 融合单工作线程执行器

英文辅助名：fused single-worker executor。

保留精确路径可观察语义、但不物化无并发价值的队列、任务图或真实屏障的单工作线程实现。

依据：[TrafficWorld 单世界精确并行执行][source-design-traffic-runtime-parallel-execution-410]。

<a id="term-gap-acceptance-profile"></a>
### 间隙接受参数

英文辅助名：gap acceptance profile。

具名且版本化的最小领先间隙、最小滞后间隙和清空缓冲；属于策略集局部成员。

精确标识符：` GapAcceptanceProfile `（类型）。

依据：[路权策略编译与 ConflictArbiter 实施合同][source-design-traffic-runtime-right-of-way-policy-415]。

<a id="term-gate-compliance-rule"></a>
### 门合规规则

英文辅助名：gate compliance rule。

针对机动门及参与者类别，把适用灯态与禁令解释为停止或候选的封闭规则，不直接授予通行权。

精确标识符：` GateComplianceRule `（类型）。

依据：[路权策略编译与 ConflictArbiter 实施合同][source-design-traffic-runtime-right-of-way-policy-415]。

<a id="term-graceful-overload-degradation"></a>
### 过载优雅降级

英文辅助名：graceful overload degradation。

宿主在过载时通过暂停、慢放或统一调整时间推进保持全部逻辑步进、事件和交通语义的显式处置；不等同于保真度降级。

依据：[TrafficWorld 单世界精确并行执行][source-design-traffic-runtime-parallel-execution-410]。

<a id="term-grant"></a>
### 授权

英文辅助名：grant。

运行时对特定车辆在当前条件下通过冲突资源的许可。

<a id="term-halo"></a>
### 边界邻域

英文辅助名：halo。

分区为同一 tick 依赖读取的相邻状态范围；不能通过过期一 tick 数据改变精确语义。

依据：[TrafficWorld 单世界精确并行执行][source-design-traffic-runtime-parallel-execution-410]。

<a id="term-host-audit-replay"></a>
### 宿主审计重放

英文辅助名：host audit replay。

宿主使用独立世界重放较早检查点后的输入并比较检查点状态的审计方法；未完成区间不计入覆盖。

被审计世界仍运行时，独立世界须位于独立进程或另一台机器；结果区分完整匹配、分歧与未完成，管理边界限制见完整性设计。

依据：[运行时状态完整性校验与失真恢复][source-design-traffic-runtime-state-integrity-413]。

<a id="term-integer-millimetre"></a>
### 整数毫米

英文辅助名：integer millimetre。

已提交交通一维几何的正本长度量子；边长、边内进度、车长、间距与停车锚点使用整数毫米。准入后编译器 Typed AST / HIR / MIR / LIR 的交通一维存储正本同为整数毫米。

精确标识符：` mm `（单位）。

依据：[交通运行时整数毫米几何][source-design-traffic-runtime-integer-geometry-414]。

<a id="term-integrity-disablement"></a>
### 停用

英文辅助名：integrity disablement。

检测判定失真对象失配后停止使用的语义；不自动纠偏、回滚、重演或重启，已发布结果不撤销，由宿主执行。

交通运行时不持有失效状态；停止使用的触发条件和边界由完整性设计规定。

依据：[运行时状态完整性校验与失真恢复][source-design-traffic-runtime-state-integrity-413]。

<a id="term-intersection-rules"></a>
### 路口规则

英文辅助名：intersection rules。

对冲突流、准入、授权、预约和通行顺序进行运行时裁决的规则集合。

依据：[WaitingZone、ConflictZone 与通行权分层][source-design-waiting-zone-conflict-right-of-way-232]。

<a id="term-lane-flow-junction-observation"></a>
### 路口观测视图

英文辅助名：lane flow junction observation。

从活动 Bevy Session 借出的只读领域视图，组合当前状态与分别标明语义的最近成功步进记录，不拥有动态交通正本。

精确标识符：` LaneFlowJunctionObservation `（类型）。

依据：[复杂路口只读观测与跨层验证][source-design-junction-observation-and-validation-419]。

<a id="term-lane-graph"></a>
### 车道图

英文辅助名：lane graph。

供运行时遍历和路线跟随使用的有向车道拓扑。

依据：[交通运行时共享静态路网消费][source-design-traffic-runtime-shared-consumption-420]。

<a id="term-laneflow-traffic-runtime"></a>
### LaneFlow 交通运行时

英文辅助名：LaneFlow Traffic Runtime。

当前可运行动态执行层。拥有固定步进、已实现执行域的交通参与单元、动态通行定义和每世界可变交通状态。

精确标识符：` laneflow-runtime `（包）。

允许简称“交通运行时”：上下文已明确属于 LaneFlow。

允许简称“运行时”：与空间层、适配器并列，或明确修饰步进与交通指标。

允许简称“Traffic Runtime”：英文辅助表述中已明确属于 LaneFlow。

允许简称“Runtime”：英文辅助表述中与 Spatial、Adapter 并列，或明确修饰 step、tick、p95 等交通运行时指标。

当前交付为 TrafficWorld 道路机动车特化；长期职责不表示其他执行域已经实现。

依据：[交通运行时共享静态路网消费][source-design-traffic-runtime-shared-consumption-420]。

<a id="term-lifecycle"></a>
### 生命周期

英文辅助名：lifecycle。

交通参与单元、动态通行定义、世界或会话从创建、活动到释放的状态变化规则；当前投影包括车辆与路线。

<a id="term-local-intent-batch"></a>
### 局部意图批次

英文辅助名：local intent batch。

车辆局部运动预览、求值前沿和候选输入的逻辑集合；不是最终位移或已取得的通行权，也不要求独立分配容器。

精确标识符：` LocalIntentBatch `（类型）。

依据：[TrafficWorld 串行阶段协议与内部状态分区][source-design-traffic-runtime-phase-protocol-412]。

<a id="term-logical-check-address"></a>
### 逻辑检查地址

英文辅助名：Logical Check Address。

由串行调用路径、规范循环位置与分支构成的检查位置，用于保持首错及故障注入对应。

依据：[TrafficWorld 单世界精确并行执行][source-design-traffic-runtime-parallel-execution-410]。

<a id="term-logical-digest-preimage"></a>
### 逻辑摘要原像

英文辅助名：logical digest preimage。

对已提交运行时逻辑状态规范化所得、与物理布局无关的摘要输入。

依据：[运行时状态完整性校验与失真恢复][source-design-traffic-runtime-state-integrity-413]。

<a id="term-logical-update-position"></a>
### 逻辑更新位置

英文辅助名：Logical Update Position。

当前拍 live 序列中的规范位置，用于保留业务次序；不等于槽位或线程编号。

依据：[TrafficWorld 单世界精确并行执行][source-design-traffic-runtime-parallel-execution-410]。

<a id="term-maneuver-direction"></a>
### 机动方向

英文辅助名：maneuver direction。

Movement 的显式直行、左转、右转或掉头声明；是非身份属性，未声明不能推断为某一方向。

精确标识符：` ManeuverDirection `（类型）。

依据：[路权策略编译与 ConflictArbiter 实施合同][source-design-traffic-runtime-right-of-way-policy-415]。

<a id="term-micrometre-accumulator"></a>
### 微米累加器

英文辅助名：micrometre accumulator。

每车 `0..=999` 的亚毫米余数。整数硬约束剩余空间为正时才累加；硬停清零；跨边保留；完成/停车/替换清零。

精确标识符：` carry_um `（字段）。

依据：[交通运行时整数毫米几何][source-design-traffic-runtime-integer-geometry-414]。

<a id="term-millimetres-per-second"></a>
### 毫米每秒

英文辅助名：millimetres per second。

已提交车速、边限速与期望车速的整数量子；IIDM 仍用瞬时 SI。不另设速度余数。

精确标识符：` mm/s `（单位）。

依据：[交通运行时整数毫米几何][source-design-traffic-runtime-integer-geometry-414]。

<a id="term-missed-detection-surface"></a>
### 漏检面

英文辅助名：missed-detection surface。

校验方法明确不能覆盖的对象或条件集合；须以对应完整性设计的穷尽边界为准。

依据：[运行时状态完整性校验与失真恢复][source-design-traffic-runtime-state-integrity-413]。

<a id="term-mutually-invisible-co-lateral-transition-segments"></a>
### 互不可见的共横向过渡段

英文辅助名：mutually invisible co-lateral transition segments。

同一走廊中横向跨度相交、且现行运动与仲裁证明无法证明允许路线分支互相覆盖的两条横向过渡段。

现行固定预制过渡对这种组合存在能力限制；允许或拒绝条件以最小换道合同的覆盖证明为准。

依据：[最小换道合同（Lane Change Minimal Contract）][source-design-lane-change-minimal-contract-421]。

<a id="term-near-gate-vehicle"></a>
### 近门车辆

英文辅助名：near-gate vehicle。

下一拍 frontier 将读到的状态下，换边时记下的门距仍落在一拍最大加速度可达上界内的活动车辆。不是全部 Active，也不包含没有下一道 Gate 的车。

依据：[路权策略编译与 ConflictArbiter 实施合同][source-design-traffic-runtime-right-of-way-policy-415]。

<a id="term-occupancy"></a>
### 占用

英文辅助名：occupancy。

实体当前物理占据道路、停车位或冲突资源的事实。

<a id="term-occupancy-bucket"></a>
### 占用桶

英文辅助名：occupancy bucket。

占用索引的分桶键；当前与 `LaneEdgeOrdinal` 1:1。

精确标识符：` OccupancyBucketOrdinal `（类型）。

依据：[Vehicle Following 设计][source-design-vehicle-following-236]。

<a id="term-occupancy-index"></a>
### 占用索引

英文辅助名：occupancy index。

固定步进求解前由 snapshot(T) 重建的私有车道边占用表；供该拍跟车，step 提交后不代表 T+D 观测。

精确标识符：` OccupancyIndex `（类型）。

依据：[Vehicle Following 设计][source-design-vehicle-following-236]。

<a id="term-occupancy-record"></a>
### 占用记录

英文辅助名：occupancy record。

占用索引中一条物理边上的占用片段（车辆、桶、`lo_mm`/`hi_mm`）；不是资源声明。

精确标识符：` OccupancyRecord `（类型）。

依据：[Vehicle Following 设计][source-design-vehicle-following-236]。

<a id="term-open-entrance"></a>
### 开放入口

英文辅助名：open entrance。

由调用方在本次车辆生成或替换中显式绑定的路线起点进入边界；合法绑定决定范围外车尾是否可以不占路，不由拓扑或画面边缘推断。

精确标识符：` VehicleEntrance `（类型）。

依据：[新生成的运动安全准入][source-design-traffic-runtime-vehicle-placement-418]。

<a id="term-page-level-memory-distortion"></a>
### 页级内存失真

英文辅助名：page-level memory distortion。

以整页替换、置零或按页整数倍位移等形态出现的内存内容异常；适用环境、证据和检测边界由完整性设计规定。

依据：[运行时状态完整性校验与失真恢复][source-design-traffic-runtime-state-integrity-413]。

<a id="term-parking"></a>
### 停车

英文辅助名：parking。

对停车设施、显式停车位、虚拟容量、预约、进入、占用和离开进行管理的领域行为。

依据：[停车系统设计][source-design-parking-system-234]。

<a id="term-parking-arrival"></a>
### 停车到达

英文辅助名：parking arrival。

预留车辆在指定路线出现记录和停车锚点上静止、且亚毫米余数为零的已提交事实；到达观测的触发规则由停车合同规定。

精确标识符：` ParkingArrivalObservation `（类型）。

依据：[停车系统设计][source-design-parking-system-234]。

<a id="term-parking-binding"></a>
### 停车绑定

英文辅助名：parking binding。

以完整车辆身份绑定其停车预留或占用及精确目标的运行时私有关系，是停车资源计数与车辆状态关联的动态事实来源。

精确标识符：` ParkingBinding `（类型）。

依据：[停车系统设计][source-design-parking-system-234]。

<a id="term-parking-target"></a>
### 停车目标

英文辅助名：parking target。

caller 精确选择的 tagged 资源：具体 `ExplicitSpace` 或设施 `VirtualPool`。Runtime 不在二者之间自动选位。

精确标识符：` ParkingTarget `（类型）。

依据：[停车系统设计][source-design-parking-system-234]。

<a id="term-partition"></a>
### 分区

英文辅助名：partition。

运行时执行计划中的私有物理所有权/调度单元；不构成公共身份或静态语义。

依据：[TrafficWorld 单世界精确并行执行][source-design-traffic-runtime-parallel-execution-410]。

<a id="term-partition-planning-hints"></a>
### 分区规划提示

英文辅助名：partition planning hints。

LFCA 不保存的非语义性能提示；由 `laneflow-static-network` 按显式 derivation version 从受检规范关系确定性派生，可安全忽略或重建。

精确标识符：` PartitionPlanningHints `（类型）。

依据：[TrafficWorld 单世界精确并行执行][source-design-traffic-runtime-parallel-execution-410]。

<a id="term-per-world-mutable-state"></a>
### 每世界可变状态

英文辅助名：per-world mutable state。

交通参与单元、通行定义、控制器时钟、预约、占用和缓冲区等不能进入共享静态路网的状态；当前投影包括车辆、路线和停车。

依据：[交通运行时共享静态路网消费][source-design-traffic-runtime-shared-consumption-420]。

<a id="term-physical-row"></a>
### 物理行

英文辅助名：Physical Row。

车辆在活动列块中的存储/独占计算位置，必须映射回完整句柄与逻辑更新位置。

依据：[交通运行时列式批量执行][source-design-traffic-runtime-columnar-execution-411]。

<a id="term-policy"></a>
### 策略

英文辅助名：policy。

在既定事实和安全约束内解释候选或选择行为的可版本化规则。

<a id="term-policy-local-change"></a>
### 策略局部成员变更

英文辅助名：policy local change。

LFSD 4 按策略稳定标识、成员种类和原始 key 定位的完整增删改记录；成员无独立 StableId，载荷以稳定引用表达。

精确标识符：` PolicyLocalChange `（类型）。

依据：[路权策略编译与 ConflictArbiter 实施合同][source-design-traffic-runtime-right-of-way-policy-415]。

<a id="term-policy-pin"></a>
### 策略绑定

英文辅助名：policy pin。

世界显式选择的策略稳定身份；策略正文和版本由安装路网修订固定，不绑定真实公历。

精确标识符：` PolicyPin `（类型）。

依据：[路权策略编译与 ConflictArbiter 实施合同][source-design-traffic-runtime-right-of-way-policy-415]。

<a id="term-potential-resource-support-graph"></a>
### 潜在资源支持图

英文辅助名：Potential Resource Support Graph。

包含当前和可能激活的资源关联、休眠依赖及 watcher 的图，用于识别条件交互。

依据：[TrafficWorld 单世界精确并行执行][source-design-traffic-runtime-parallel-execution-410]。

<a id="term-proposal"></a>
### 提案

英文辅助名：proposal。

并行求值阶段生成、尚未取得提交裁决的候选状态变化。

依据：[TrafficWorld 单世界精确并行执行][source-design-traffic-runtime-parallel-execution-410]。

<a id="term-reduction-span"></a>
### 归约跨度

英文辅助名：reduction span。

在无限并行资源下仍不可消除的规范依赖临界路径成本。

依据：[TrafficWorld 单世界精确并行执行][source-design-traffic-runtime-parallel-execution-410]。

<a id="term-reduction-work"></a>
### 归约工作量

英文辅助名：reduction work。

完成一次规范归约需要执行的总操作量，用于区别总成本与临界路径。

依据：[TrafficWorld 单世界精确并行执行][source-design-traffic-runtime-parallel-execution-410]。

<a id="term-regulation-identity"></a>
### 法规身份

英文辅助名：regulation identity。

法域、法规版本及可选来源的共同值；来源信息本身不授予车辆通行权。

精确标识符：` RegulationIdentity `（类型）。

依据：[路权策略编译与 ConflictArbiter 实施合同][source-design-traffic-runtime-right-of-way-policy-415]。

<a id="term-reservation"></a>
### 预约

英文辅助名：reservation。

对停车位、冲突资源或其他有界资源的持有关系。

<a id="term-resolution-batch"></a>
### 裁决批次

英文辅助名：resolution batch。

按规范顺序取得或拒绝完整组合资源后的暂存结果；grant 必须经实际 crossing 才能转成已提交资源裁决权。

精确标识符：` ResolutionBatch `（类型）。

依据：[TrafficWorld 串行阶段协议与内部状态分区][source-design-traffic-runtime-phase-protocol-412]。

<a id="term-resolved-gate-policy-cell"></a>
### 已解析门策略单元

英文辅助名：resolved gate policy cell。

编译后对应一份策略、一个门与实际使用的可准入参与者类别的只读解释结果；同类车辆配置共享，并保留稳定规则归因。

精确标识符：` ResolvedGatePolicyCell `（类型）。

依据：[路权策略编译与 ConflictArbiter 实施合同][source-design-traffic-runtime-right-of-way-policy-415]。

<a id="term-resolved-stream-rule"></a>
### 已解析通行流规则

英文辅助名：resolved stream rule。

编译后对应一份策略、一个流与实际使用的可准入参与者类别的唯一规则及让行目标范围；同类车辆配置共享。

精确标识符：` ResolvedStreamRule `（类型）。

依据：[路权策略编译与 ConflictArbiter 实施合同][source-design-traffic-runtime-right-of-way-policy-415]。

<a id="term-resource-claim"></a>
### 资源声明

英文辅助名：resource claim；claim。

对共享交通资源提出的确定性占用请求；必须经规范归约后才能提交。

依据：[TrafficWorld 单世界精确并行执行][source-design-traffic-runtime-parallel-execution-410]。

<a id="term-resource-dependency-component"></a>
### 资源依赖组件

英文辅助名：Resource Dependency Component。

本拍候选及潜在资源交互形成的逻辑闭包；不等同于地理块或单一物理线程。

依据：[TrafficWorld 单世界精确并行执行][source-design-traffic-runtime-parallel-execution-410]。

<a id="term-right-of-way-policy-set"></a>
### 路权策略集

英文辅助名：right of way policy set。

由一份法规身份固定含义的规则、依据和间隙参数集合；具有独立稳定身份，由宿主显式选择，不按日期筛选。

精确标识符：` RightOfWayPolicySet `（类型）。

依据：[路权策略编译与 ConflictArbiter 实施合同][source-design-traffic-runtime-right-of-way-policy-415]。

<a id="term-road-motor-vehicle-execution-domain"></a>
### 道路机动车执行域

英文辅助名：road motor vehicle execution domain。

当前 `TrafficWorld` 与既有车辆工作负载的报告域；该标记不是生产数据枚举，也不冻结其他执行域的类型体系。

精确标识符：` road_motor_vehicle `（记号）。

依据：[ADR 0021：交通基础设施定位与宿主边界][source-adr-0021-416]。

<a id="term-route"></a>
### 路线

英文辅助名：route。

当前道路机动车执行域中由有序遍历边和相关出现记录组成的车辆行驶计划；唯一生产入口是 `register_route`，得到代际感知句柄，不获得持久 `StableId128`。

精确标识符：` Route `（类型）。

依据：[Route System 设计][source-design-route-system-422]。

<a id="term-route-gate-observation"></a>
### 路线机动门观测

英文辅助名：route gate observation。

当前世界已注册路线某个 hop 的机动门及整数毫米位置，只用于精确定位，不授予通行权。

精确标识符：` RouteGateObservation `（类型）。

依据：[复杂路口只读观测与跨层验证][source-design-junction-observation-and-validation-419]。

<a id="term-runtime-overlay"></a>
### 运行时覆盖层

英文辅助名：runtime overlay。

不改变静态身份或拓扑、按显式命令暂时修改运行时约束的版本化动态状态。

依据：[运行时快照][source-design-traffic-runtime-snapshot-423]。

<a id="term-serial-phase-protocol"></a>
### 串行阶段协议

英文辅助名：Serial Phase Protocol。

一次固定步进的逻辑阶段、数据依赖、读写权限、确定性顺序与失败边界；允许单工作线程融合执行，不要求通用任务图或物理屏障。

依据：[TrafficWorld 串行阶段协议与内部状态分区][source-design-traffic-runtime-phase-protocol-412]。

<a id="term-session"></a>
### 会话

英文辅助名：session。

宿主或空间层中具有明确生命周期和资源所有权的一次活动上下文。

<a id="term-shared-network-revision-authentication"></a>
### 共享路网修订认证

英文辅助名：shared network revision authentication。

在共享路网修订投入使用前或审计时，对照独立来源核对修订内容的只读操作；不进入固定步进路径，确认与处置由宿主负责。

允许简称“修订认证”：全文首次给出完整规范名后，且认证对象明确。

来源准备、修订比较、确认与处置的阶段边界，以及比较算子、操作失败和判因规则，均由完整性设计维护。

依据：[运行时状态完整性校验与失真恢复][source-design-traffic-runtime-state-integrity-413]。

<a id="term-signal"></a>
### 信号

英文辅助名：signal。

由信号控制器和信号组产生、供车辆解释通行约束的运行时指示。

依据：[Signal System 设计][source-design-signal-system-235]。

<a id="term-snapshot"></a>
### 快照

英文辅助名：snapshot。

在明确时点冻结、供只读消费的运行时状态视图。

依据：[运行时快照][source-design-traffic-runtime-snapshot-423]。

<a id="term-static-execution-constraint-graph"></a>
### 静态执行约束图

英文辅助名：static execution constraint graph。

编译器从静态资源依赖派生的工作线程数无关约束、可切分边界与规范合并顺序。

依据：[TrafficWorld 单世界精确并行执行][source-design-traffic-runtime-parallel-execution-410]。

<a id="term-stationary-state"></a>
### 停驻状态

英文辅助名：stationary state。

参与单元暂不沿通行路径运动、但仍保留身份和领域占用语义的运行时状态；当前道路机动车投影包括停车状态。

当前交付为道路机动车特化；长期通用概念不表示其他执行域已经实现，也不改写历史车辆证据。

依据：[ADR 0021：交通基础设施定位与宿主边界][source-adr-0021-416]。

<a id="term-steady-state-tick"></a>
### 稳态步进

英文辅助名：steady-state tick。

初始化和容量稳定后的高频固定步进路径。

依据：[交通运行时整数毫米几何][source-design-traffic-runtime-integer-geometry-414]。

<a id="term-step-read-view"></a>
### 步进只读视图

英文辅助名：step read view。

绑定同一拍初世界及必要派生查询的内部借用视图；不克隆整个世界，不允许领域函数借此修改已提交状态。

精确标识符：` StepReadView `（类型）。

依据：[TrafficWorld 串行阶段协议与内部状态分区][source-design-traffic-runtime-phase-protocol-412]。

<a id="term-stream-right-of-way-rule"></a>
### 通行流路权规则

英文辅助名：stream right of way rule。

针对参与者流及类别，选择法规优先级、让行目标和间隙参数的规则；不覆盖本地物理先后顺序。

精确标识符：` StreamRightOfWayRule `（类型）。

依据：[路权策略编译与 ConflictArbiter 实施合同][source-design-traffic-runtime-right-of-way-policy-415]。

<a id="term-strongly-connected-component"></a>
### 强连通分量

英文辅助名：strongly connected component。

资源依赖图中任意两节点互相可达、需要作为循环依赖分析单元的最大子图。

精确标识符：` SCC `（缩写）。

依据：[TrafficWorld 单世界精确并行执行][source-design-traffic-runtime-parallel-execution-410]。

<a id="term-tick-workspace"></a>
### 步进工作区

英文辅助名：tick workspace。

单次步进的意图、资源暂存、下一状态和待发布结果及其复用存储；失败丢弃本次结果，不清空上次成功发布的批次。

精确标识符：` TickWorkspace `（类型）。

依据：[TrafficWorld 串行阶段协议与内部状态分区][source-design-traffic-runtime-phase-protocol-412]。

<a id="term-traffic-configuration"></a>
### 交通配置

英文辅助名：Traffic Configuration。

影响交通准入与行为的四类容量及固定步长；不含执行参数。

精确标识符：` WorldConfig `（类型）。

依据：[TrafficWorld 执行配置、资源与快照边界][source-design-traffic-runtime-execution-config-417]。

<a id="term-traffic-execution-domain"></a>
### 交通执行域

英文辅助名：traffic execution domain。

共享网络、运动/安全求解和生命周期契约的一类交通参与单元；与准入用 `ParticipantClass` 正交。

当前交付为道路机动车特化；长期通用概念不表示其他执行域已经实现，也不改写历史车辆证据。

依据：[ADR 0021：交通基础设施定位与宿主边界][source-adr-0021-416]。

<a id="term-traffic-fidelity"></a>
### 交通保真度

英文辅助名：traffic fidelity。

交通个体、时间、空间、规则与事件语义相对精确路径的保留程度。

依据：[TrafficWorld 单世界精确并行执行][source-design-traffic-runtime-parallel-execution-410]。

<a id="term-traffic-participant"></a>
### 交通参与者

英文辅助名：traffic participant。

直接使用交通网络或占用交通空间的动态参与者；不等同于城市居民、乘客、出行需求或 AI Agent。

依据：[ADR 0021：交通基础设施定位与宿主边界][source-adr-0021-416]。

<a id="term-traffic-participant-unit"></a>
### 交通参与单元

英文辅助名：traffic participant unit。

交通运行时中可独立保留身份的计数原子；机动车、骑行者组合、行人或轨道运营编组的具体原子由对应执行域 G1 冻结。

当前交付为道路机动车特化；长期通用概念不表示其他执行域已经实现，也不改写历史车辆证据。

相关概念：[交通执行域][term-traffic-execution-domain]、[车辆运行单元][term-vehicle-runtime-unit]。

依据：[ADR 0021：交通基础设施定位与宿主边界][source-adr-0021-416]。

<a id="term-traffic-transition-event"></a>
### 交通转移事件

英文辅助名：traffic transition event。

一次成功固定步进已经提交的机动门、等待区、冲突区或机动完成变化。

精确标识符：` TrafficTransitionEvent `（类型）。

依据：[WaitingZone、ConflictZone 与通行权分层][source-design-waiting-zone-conflict-right-of-way-232]。

<a id="term-traffic-world"></a>
### 交通世界

英文辅助名：traffic world。

安装共享静态路网并持有每世界可变状态的运行时实例。

精确标识符：` TrafficWorld `（类型）。

依据：[交通运行时共享静态路网消费][source-design-traffic-runtime-shared-consumption-420]。

<a id="term-transition-anchor"></a>
### 转移锚点

英文辅助名：transition anchor。

同时标识交通转移事件的路线出现记录和实际触发位置的值；车尾事件按净空时所需的前保险杠位置排序。

精确标识符：` TrafficTransitionAnchor `（类型）。

依据：[WaitingZone、ConflictZone 与通行权分层][source-design-waiting-zone-conflict-right-of-way-232]。

<a id="term-trusted-baseline"></a>
### 可信基准

英文辅助名：trusted baseline。

经独立核验成立的期望值集合及其绑定闭合；期望值不得取自被校验对象自身；无法建立时不得宣称检测或恢复成功。

依据：[运行时状态完整性校验与失真恢复][source-design-traffic-runtime-state-integrity-413]。

<a id="term-vehicle-directory"></a>
### 车辆目录

英文辅助名：Vehicle Directory。

按完整句柄的索引与代次定位存储类别和物理行的登记表；不拥有第二份运动状态。

依据：[交通运行时列式批量执行][source-design-traffic-runtime-columnar-execution-411]。

<a id="term-vehicle-following"></a>
### 跟车

英文辅助名：vehicle following。

根据同一路径上的前车状态施加安全间距和速度约束的运行时行为。

依据：[Vehicle Following 设计][source-design-vehicle-following-236]。

<a id="term-vehicle-following-front-lookahead"></a>
### 跟车前视

英文辅助名：vehicle-following front lookahead。

根据当前速度与车辆配置推导的每车每拍占用查询行走范围；不是目视距离，也不改变占用索引的全体活动车辆重建范围。

精确标识符：` front_query_horizon `（字段）。

依据：[Vehicle Following 设计][source-design-vehicle-following-236]。

<a id="term-vehicle-runtime-unit"></a>
### 车辆运行单元

英文辅助名：vehicle runtime unit。

当前道路机动车执行域的车辆特化交通参与单元；不表示交通运行时只允许车辆。

精确标识符：` VehicleState `（类型）。

依据：[ADR 0021：交通基础设施定位与宿主边界][source-adr-0021-416]。

<a id="term-vehicle-state"></a>
### 车辆状态

英文辅助名：vehicle state。

当前车辆运行单元在某一固定步进边界上的运动、路线与行为状态。

依据：[ADR 0021：交通基础设施定位与宿主边界][source-adr-0021-416]。

<a id="term-virtual-parked-state"></a>
### 虚拟停驻

英文辅助名：virtual parked state。

车辆仍保留 live identity、route 和设施 binding，但不进入道路运动/占用，也不产生 committed pose 的 Parked 状态；不等于 despawn、聚合车辆或隐藏路网。

依据：[停车系统设计][source-design-parking-system-234]。

<a id="term-virtual-parking-capacity"></a>
### 虚拟停车容量

英文辅助名：virtual parking capacity。

停车设施中不展开为具体泊位、内部路网或 parked pose 的计数容量。Runtime 虚拟池只保留 reserved/occupied 计数，不按该容量建立 binding 槽位。

精确标识符：` virtual_capacity `（字段）。

依据：[停车系统设计][source-design-parking-system-234]。

<a id="term-waiting-dependency-view"></a>
### 等待依赖视图

英文辅助名：waiting dependency view。

表达资源持有及容量不足导致的等待关系的可重建执行数据；不是持久化正本。

依据：[WaitingZone、ConflictZone 与通行权分层][source-design-waiting-zone-conflict-right-of-way-232]。

<a id="term-worker"></a>
### 工作线程

英文辅助名：worker。

执行一个或多个运行时任务的计算资源；数量和完成顺序不得改变精确结果。

依据：[TrafficWorld 单世界精确并行执行][source-design-traffic-runtime-parallel-execution-410]。

<a id="term-world-binding-state"></a>
### 世界绑定状态

英文辅助名：world binding state。

活动共享路网修订、已提交来源、世界身份与世代、配置及策略选择的统一绑定；固定步进只读，安装、恢复或切换按各自事务更新。

精确标识符：` WorldBindingState `（类型）。

依据：[TrafficWorld 串行阶段协议与内部状态分区][source-design-traffic-runtime-phase-protocol-412]。

<a id="term-world-policy-selection"></a>
### 世界策略选择

英文辅助名：world policy selection。

显式选择策略绑定，或在整个共享静态路网没有门与冲突输入时声明不需要策略；不存在隐式默认策略。

精确标识符：` WorldPolicySelection `（类型）。

依据：[路权策略编译与 ConflictArbiter 实施合同][source-design-traffic-runtime-right-of-way-policy-415]。

<a id="domain-host"></a>
## 宿主与世界管理

点击中文规范名查看完整定义；跨领域概念指向同一词条。

| 中文规范名                                                        | 英文辅助名                               | 精确标识符                           |
| ----------------------------------------------------------------- | ---------------------------------------- | ------------------------------------ |
| [已准入路线注册命令][term-admitted-route-registration-command]    | admitted route registration command      | ` AdmittedRouteRegisterInput `       |
| [候选路线][term-candidate-route]                                  | candidate route                          | ` CandidateRouteInput `              |
| [检查点][term-checkpoint]                                         | checkpoint                               | —                                    |
| [已提交变更流][term-committed-mutation-stream]                    | committed mutation stream                | —                                    |
| [已提交路网来源][term-committed-network-source]                   | committed network source                 | ` CommittedNetworkSource `           |
| [已提交道路路网][term-committed-road-network]                     | committed road network                   | ` CommittedRoadNetwork `             |
| [已提交交通观测快照][term-committed-traffic-observation-snapshot] | committed traffic observation snapshot   | —                                    |
| [成本模型标识][term-cost-model-identifier]                        | cost model identifier                    | ` CostModelId `                      |
| [切换事件批次][term-cutover-event-batch]                          | cutover event batch                      | —                                    |
| [增量追赶][term-delta-catch-up]                                   | delta catch-up                           | —                                    |
| [失同步诊断制品][term-desynchronization-diagnostic-artifact]      | desynchronization diagnostic artifact    | —                                    |
| [确定性状态摘要][term-deterministic-state-digest]                 | deterministic state digest               | ` StateDigest `                      |
| [动态成本快照][term-dynamic-cost-snapshot]                        | dynamic cost snapshot                    | —                                    |
| [动态成本快照绑定][term-dynamic-cost-snapshot-binding]            | dynamic cost snapshot binding            | ` DynamicCostSnapshotBinding `       |
| [可编辑差异基线][term-editable-diff-base]                         | editable diff base                       | ` EditableDiffBase `                 |
| [等价证明][term-equivalence-proof]                                | equivalence proof                        | —                                    |
| [完整/增量导出][term-full-delta-export]                           | full/delta export                        | —                                    |
| [宿主应用层][term-host-application-layer]                         | host application layer                   | —                                    |
| [输入命令游标][term-input-command-cursor]                         | input command cursor                     | —                                    |
| [输入命令序列][term-input-command-sequence]                       | input command sequence                   | —                                    |
| [迁移增量日志][term-migration-delta-journal]                      | migration delta journal                  | —                                    |
| [出行与交通编排层][term-mobility-and-traffic-orchestration-layer] | mobility and traffic orchestration layer | —                                    |
| [路网修订切换描述符][term-network-revision-cutover-descriptor]    | network revision cutover descriptor      | ` NetworkRevisionCutoverDescriptor ` |
| [路网修订切换事务][term-network-revision-cutover-transaction]     | network revision cutover transaction     | —                                    |
| [观测导出节奏][term-observation-export-cadence]                   | observation export cadence               | —                                    |
| [观测导出会话][term-observation-export-session]                   | observation export session               | ` ObservationExportSession `         |
| [观测选择][term-observation-selection]                            | observation selection                    | —                                    |
| [观测选择摘要][term-observation-selection-digest]                 | observation selection digest             | ` ObservationSelectionDigest `       |
| [观测输入集合摘要][term-observation-set-digest]                   | observation set digest                   | ` ObservationSetDigest `             |
| [观测状态序号][term-observation-state-sequence]                   | observation state sequence               | ` ObservationStateSequence `         |
| [观测 stream 绑定][term-observation-stream-binding]               | observation stream binding               | ` ObservationStreamBinding `         |
| [维护暂停模式][term-paused-maintenance-mode]                      | paused maintenance mode                  | —                                    |
| [已发布 LFCA 引用][term-published-lfca-reference]                 | published LFCA reference                 | ` PublishedLfcaReference `           |
| [静默提交窗口][term-quiescent-commit-window]                      | quiescent commit window                  | —                                    |
| [道路编辑状态摘要][term-road-editing-state-digest]                | road editing state digest                | —                                    |
| [路线边出现记录容量][term-route-edge-occurrence-capacity]         | route edge occurrence capacity           | ` route_edge_occurrence_capacity `   |
| [Routing 准入会话][term-routing-admission-session]                | routing admission session                | ` RoutingAdmissionSession `          |
| [路径规划服务][term-routing-service]                              | routing service                          | —                                    |
| [运行时执行计划][term-runtime-execution-plan]                     | runtime execution plan                   | —                                    |
| [运行时快照][term-runtime-snapshot]                               | runtime snapshot                         | —                                    |
| [存档清单][term-save-manifest]                                    | save manifest                            | —                                    |
| [快照局部标识][term-snapshot-local-identity]                      | snapshot-local identity                  | —                                    |
| [交通基础设施][term-traffic-infrastructure]                       | traffic infrastructure                   | —                                    |
| [出行需求][term-travel-demand]                                    | travel demand                            | —                                    |
| [出行编排层][term-trip-orchestration-layer]                       | trip orchestration layer                 | —                                    |
| [世界][term-world]                                                | world                                    | —                                    |
| [世界世代][term-world-generation]                                 | world generation                         | ` WorldGeneration `                  |
| [世界身份][term-world-identity]                                   | world identity                           | ` world_id `                         |

<a id="term-admitted-route-registration-command"></a>
### 已准入路线注册命令

英文辅助名：admitted route registration command。

候选路线准入成功后供宿主耐久记录和重放的规范化注册输入；不重放观测或成本计算过程。

精确标识符：` AdmittedRouteRegisterInput `（类型）。

依据：[交通观测与 Routing 接入][source-design-traffic-observation-and-routing-integration-472]。

<a id="term-candidate-route"></a>
### 候选路线

英文辅助名：candidate route。

Routing 提交的 LaneEdge 稳定标识序列；经 Runtime 来源与内容验证后成为普通路线。

精确标识符：` CandidateRouteInput `（类型）。

依据：[交通观测与 Routing 接入][source-design-traffic-observation-and-routing-integration-472]。

<a id="term-checkpoint"></a>
### 检查点

英文辅助名：checkpoint。

在回放序列中周期保存、用于缩短恢复和失同步定位区间的版本化状态锚点。

依据：[运行时快照][source-design-traffic-runtime-snapshot-423]。

<a id="term-committed-mutation-stream"></a>
### 已提交变更流

英文辅助名：committed mutation stream。

旧世界在原子提交后产生、供候选迁移状态重解释且不会重复执行输入或发布重复旧世界事件的规范动态状态变化序列。

依据：[修订切换事务][source-design-traffic-runtime-revision-cutover-473]。

<a id="term-committed-network-source"></a>
### 已提交路网来源

英文辅助名：committed network source。

已经进入活动交通运行时修订、可用于重建该修订的来源；编辑来源与发布制品引用具有不同恢复合同。

精确标识符：` CommittedNetworkSource `（类型）。

依据：[已提交道路编辑状态与编辑来源][source-design-committed-road-editing-state-474]。

<a id="term-committed-road-network"></a>
### 已提交道路路网

英文辅助名：committed road network。

已提交路网来源与对应共享路网修订的逻辑聚合；工作中与候选编辑状态不属于该对象。

精确标识符：` CommittedRoadNetwork `（类型）。

依据：[已提交道路编辑状态与编辑来源][source-design-committed-road-editing-state-474]。

<a id="term-committed-traffic-observation-snapshot"></a>
### 已提交交通观测快照

英文辅助名：committed traffic observation snapshot。

从同一已提交交通状态提取的只读观测集合；具体计数、单位与覆盖范围由观测合同规定。

依据：[交通观测与 Routing 接入][source-design-traffic-observation-and-routing-integration-472]。

<a id="term-cost-model-identifier"></a>
### 成本模型标识

英文辅助名：cost model identifier。

宿主所选成本模型身份的 32 字节不透明标识；Runtime 只做精确相等比较，不规定产生算法，不属于 LaneFlow 摘要。

精确标识符：` CostModelId `（类型）。

依据：[交通观测与 Routing 接入][source-design-traffic-observation-and-routing-integration-472]。

<a id="term-cutover-event-batch"></a>
### 切换事件批次

英文辅助名：cutover event batch。

迁移函数生成、在准备期保持不可见，并只与新共享修订/状态绑定原子提交一次的规范排序事件集合；切换放弃时零发布。

依据：[修订切换事务][source-design-traffic-runtime-revision-cutover-473]。

<a id="term-delta-catch-up"></a>
### 增量追赶

英文辅助名：delta catch-up。

候选世界在后台按规范顺序应用基准状态之后的迁移增量，逐步追近运行中旧世界提交点的阶段。

依据：[修订切换事务][source-design-traffic-runtime-revision-cutover-473]。

<a id="term-desynchronization-diagnostic-artifact"></a>
### 失同步诊断制品

英文辅助名：desynchronization diagnostic artifact。

结合冷标识与源映射记录首个分歧 tick、phase、实体和资源组件的调试制品。

依据：[运行时状态完整性校验与失真恢复][source-design-traffic-runtime-state-integrity-413]。

<a id="term-deterministic-state-digest"></a>
### 确定性状态摘要

英文辅助名：deterministic state digest。

对运行时已提交逻辑状态按规范排序计算、与容器编码无关的稳定摘要；用于回放校验、对拍和定位首个分歧区间，不承担发布身份。

精确标识符：` StateDigest `（类型）。

算法与原像版本由运行时快照和摘要角色合同维护，不能从词条登记推断摘要字节兼容。

依据：[运行时快照][source-design-traffic-runtime-snapshot-423]。

<a id="term-dynamic-cost-snapshot"></a>
### 动态成本快照

英文辅助名：dynamic cost snapshot。

对象由宿主 Routing 拥有，Runtime 不解释成本 payload。

依据：[交通观测与 Routing 接入][source-design-traffic-observation-and-routing-integration-472]。

<a id="term-dynamic-cost-snapshot-binding"></a>
### 动态成本快照绑定

英文辅助名：dynamic cost snapshot binding。

绑定世界、修订、观测时点、成本模型、有效窗与载荷摘要的候选路线来源描述；运行时不接收成本载荷。

精确标识符：` DynamicCostSnapshotBinding `（类型）。

依据：[交通观测与 Routing 接入][source-design-traffic-observation-and-routing-integration-472]。

<a id="term-editable-diff-base"></a>
### 可编辑差异基线

英文辅助名：editable diff base。

与当前编辑修订的规范制品精确字节对应、供下一次语义差异计算使用的基线；随来源提交原子更替，不进入存档。

精确标识符：` EditableDiffBase `（类型）。

依据：[已提交道路编辑状态与编辑来源][source-design-committed-road-editing-state-474]。

<a id="term-equivalence-proof"></a>
### 等价证明

英文辅助名：equivalence proof。

切换提交前证明候选状态与切换事件批次等价于确定性迁移函数作用于旧世界提交状态的证据义务；证明失败即放弃候选。

依据：[修订切换事务][source-design-traffic-runtime-revision-cutover-473]。

<a id="term-full-delta-export"></a>
### 完整/增量导出

英文辅助名：full/delta export。

v1 用含全零行的 full 和严格引用上一成功批次的 delta 表达观测。

依据：[交通观测与 Routing 接入][source-design-traffic-observation-and-routing-integration-472]。

<a id="term-host-application-layer"></a>
### 宿主应用层

英文辅助名：host application layer。

拥有应用业务、需求与规则的宿主上层；人口、经济、土地利用、建筑、工作/居住、物流任务与游戏规则是可能的业务内容，不归交通运行时裁决。

依据：[ADR 0021：交通基础设施定位与宿主边界][source-adr-0021-416]。

<a id="term-input-command-cursor"></a>
### 输入命令游标

英文辅助名：input command cursor。

已成功应用的生命周期命令计数，是快照与切换基线比对的命令侧锚点；计入范围、耗尽与失败行为由生命周期合同规定。

依据：[运行时快照][source-design-traffic-runtime-snapshot-423]。

<a id="term-input-command-sequence"></a>
### 输入命令序列

英文辅助名：input command sequence。

按规范顺序驱动世界、可与检查点共同重放的显式外部命令流。

依据：[运行时快照][source-design-traffic-runtime-snapshot-423]。

<a id="term-migration-delta-journal"></a>
### 迁移增量日志

英文辅助名：migration delta journal。

在线迁移准备期间有界记录已提交动态状态/生命周期变化及命令/事件游标、供候选世界迁移追赶的版本化日志。

依据：[修订切换事务][source-design-traffic-runtime-revision-cutover-473]。

<a id="term-mobility-and-traffic-orchestration-layer"></a>
### 出行与交通编排层

英文辅助名：mobility and traffic orchestration layer。

拥有出行需求、出发时刻、交通参与单元生成、目的地、人口生命周期与路线选择策略的上层。

依据：[ADR 0021：交通基础设施定位与宿主边界][source-adr-0021-416]。

<a id="term-network-revision-cutover-descriptor"></a>
### 路网修订切换描述符

英文辅助名：network revision cutover descriptor。

在待验证对象之外可信绑定两侧修订、规范制品来源、语义差异与迁移策略的切换输入。

精确标识符：` NetworkRevisionCutoverDescriptor `（类型）。

依据：[修订切换事务][source-design-traffic-runtime-revision-cutover-473]。

<a id="term-network-revision-cutover-transaction"></a>
### 路网修订切换事务

英文辅助名：network revision cutover transaction。

在固定步进安全边界原子切换共享修订、动态状态和 committed 道路状态的失败关闭事务；“镜像切换”仅为历史名。

依据：[修订切换事务][source-design-traffic-runtime-revision-cutover-473]。

<a id="term-observation-export-cadence"></a>
### 观测导出节奏

英文辅助名：observation export cadence。

上层或宿主明确选择的观测完整基线/增量导出频率；不等同于每个固定步进全网复制。

依据：[交通观测与 Routing 接入][source-design-traffic-observation-and-routing-integration-472]。

<a id="term-observation-export-session"></a>
### 观测导出会话

英文辅助名：observation export session。

Runtime 签发、字段私有且无公共构造器，绑定世界/世代、修订、选择及上一成功批次的调用方能力；不进快照。

精确标识符：` ObservationExportSession `（类型）。

依据：[交通观测与 Routing 接入][source-design-traffic-observation-and-routing-integration-472]。

<a id="term-observation-selection"></a>
### 观测选择

英文辅助名：observation selection。

`AllLaneEdges` 或按 `StableId128` 规范排序的 LaneEdge 集合；不是执行计划分区。

依据：[交通观测与 Routing 接入][source-design-traffic-observation-and-routing-integration-472]。

<a id="term-observation-selection-digest"></a>
### 观测选择摘要

英文辅助名：observation selection digest。

观测选择解析后内容的 BLAKE3-256 内部完整性摘要（ADR 0033）；内容相同的两种选择表达得到相同值，不进快照。

精确标识符：` ObservationSelectionDigest `（类型）。

依据：[交通观测与 Routing 接入][source-design-traffic-observation-and-routing-integration-472]。

<a id="term-observation-set-digest"></a>
### 观测输入集合摘要

英文辅助名：observation set digest。

一组同状态观测批次规范排列后的 BLAKE3-256 内部完整性摘要（ADR 0033），由 `bind_observation_set` 产生，作为成本快照摘要的输入。

精确标识符：` ObservationSetDigest `（类型）。

依据：[交通观测与 Routing 接入][source-design-traffic-observation-and-routing-integration-472]。

<a id="term-observation-state-sequence"></a>
### 观测状态序号

英文辅助名：observation state sequence。

字段私有的严格 `u64`：同一世界世代/stream 内从 `0` 开始，排序成功 step 与影响 v1 行的已提交生命周期变化；checked 耗尽失败关闭，不进快照。

精确标识符：` ObservationStateSequence `（类型）。

依据：[交通观测与 Routing 接入][source-design-traffic-observation-and-routing-integration-472]。

<a id="term-observation-stream-binding"></a>
### 观测 stream 绑定

英文辅助名：observation stream binding。

Runtime 签发的 `(world_id, WorldGeneration)` 进程内能力；每个活动世界世代只有一个观测 stream，不另维护可能漂移的重复计数器。

精确标识符：` ObservationStreamBinding `（类型）。

依据：[交通观测与 Routing 接入][source-design-traffic-observation-and-routing-integration-472]。

<a id="term-paused-maintenance-mode"></a>
### 维护暂停模式

英文辅助名：paused maintenance mode。

由宿主显式声明世界已暂停、允许一次性完成迁移且单独预算完整停顿的切换模式。

依据：[修订切换事务][source-design-traffic-runtime-revision-cutover-473]。

<a id="term-published-lfca-reference"></a>
### 已发布 LFCA 引用

英文辅助名：published LFCA reference。

runtime-only 存档中的不透明宿主持久 asset key 与 LFCA digest/length/revision 绑定；不复制 LFCA，也不替代加载时的资产认证。

精确标识符：` PublishedLfcaReference `（类型）。

依据：[已提交道路编辑状态与编辑来源][source-design-committed-road-editing-state-474]。

<a id="term-quiescent-commit-window"></a>
### 静默提交窗口

英文辅助名：quiescent commit window。

固定步进安全边界上短暂冻结旧世界输入、排空日志尾并原子切换绑定的有界时间窗口。

依据：[修订切换事务][source-design-traffic-runtime-revision-cutover-473]。

<a id="term-road-editing-state-digest"></a>
### 道路编辑状态摘要

英文辅助名：road editing state digest。

对已提交道路编辑状态的规范来源字节计算的内部完整性摘要；不替代来源集合摘要，也不参与路网修订标识。

依据：[已提交道路编辑状态与编辑来源][source-design-committed-road-editing-state-474]。

<a id="term-route-edge-occurrence-capacity"></a>
### 路线边出现记录容量

英文辅助名：route edge occurrence capacity。

`WorldConfig` 对全部存活动态路线边序列 occurrence 总数的语义容量；重复边重复计数，所有路线入口共用，不等于共享静态路网物理边数。

精确标识符：` route_edge_occurrence_capacity `（字段）。

依据：[交通观测与 Routing 接入][source-design-traffic-observation-and-routing-integration-472]。

<a id="term-routing-admission-session"></a>
### Routing 准入会话

英文辅助名：routing admission session。

Runtime 无分配签发、字段私有的调用方能力，精确绑定当前世界/世代/修订与 `CostModelKey`；不进快照。

精确标识符：` RoutingAdmissionSession `（类型）。

依据：[交通观测与 Routing 接入][source-design-traffic-observation-and-routing-integration-472]。

<a id="term-routing-service"></a>
### 路径规划服务

英文辅助名：routing service。

从静态路网和已提交动态成本快照生成候选路径、但不拥有交通参与单元固定步进的服务。

依据：[交通观测与 Routing 接入][source-design-traffic-observation-and-routing-integration-472]。

<a id="term-runtime-execution-plan"></a>
### 运行时执行计划

英文辅助名：runtime execution plan。

每世界依据静态约束、硬件与动态负载建立的分区、工作线程、边界交换和迁移计划。

依据：[TrafficWorld 单世界精确并行执行][source-design-traffic-runtime-parallel-execution-410]。

<a id="term-runtime-snapshot"></a>
### 运行时快照

英文辅助名：runtime snapshot。

绑定版本化路网修订、LFCA origin 与静态契约版本，并可借助共享稳定身份索引在重建的共享修订上精确恢复的每世界可变状态存档制品。

相关概念：[快照局部标识][term-snapshot-local-identity]、[存档清单][term-save-manifest]。

依据：[运行时快照][source-design-traffic-runtime-snapshot-423]。

<a id="term-save-manifest"></a>
### 存档清单

英文辅助名：save manifest。

原子绑定宿主应用、出行编排和 LaneFlow 各版本化存档制品及摘要的上层清单。

依据：[运行时快照][source-design-traffic-runtime-snapshot-423]。

<a id="term-snapshot-local-identity"></a>
### 快照局部标识

英文辅助名：snapshot-local identity。

只在一个运行时快照内稳定、用于重建动态实体引用且不复用原进程句柄的持久键。

依据：[运行时快照][source-design-traffic-runtime-snapshot-423]。

<a id="term-traffic-infrastructure"></a>
### 交通基础设施

英文辅助名：traffic infrastructure。

可嵌入、引擎无关、确定性的道路交通运行时与工具链；当前提供道路机动车能力，并为规模与执行域演进保留明确边界。人口经济、出行需求与路线选择策略由宿主拥有。

依据：[ADR 0021：交通基础设施定位与宿主边界][source-adr-0021-416]。

<a id="term-travel-demand"></a>
### 出行需求

英文辅助名：travel demand。

描述谁在何时为何出发的上层输入；不由交通运行时隐藏生成。

依据：[交通观测与 Routing 接入][source-design-traffic-observation-and-routing-integration-472]。

<a id="term-trip-orchestration-layer"></a>
### 出行编排层

英文辅助名：trip orchestration layer。

拥有出行需求、候选路线选择、收费/政策偏好和调用方随机流，并以显式边界驱动 LaneFlow 的上层系统。

依据：[交通观测与 Routing 接入][source-design-traffic-observation-and-routing-integration-472]。

<a id="term-world"></a>
### 世界

英文辅助名：world。

共享静态数据并持有一份独立可变运行时状态的实例。

依据：[修订切换事务][source-design-traffic-runtime-revision-cutover-473]。

<a id="term-world-generation"></a>
### 世界世代

英文辅助名：world generation。

Runtime 拥有的活动聚合失效轴；新安装世界从 `0` 开始，每次成功切换或原位恢复 checked 递增，失败/放弃不变。它与世界身份共同绑定切换、观测与 Routing 会话，不进入 Runtime Snapshot。

精确标识符：` WorldGeneration `（类型）。

依据：[修订切换事务][source-design-traffic-runtime-revision-cutover-473]。

<a id="term-world-identity"></a>
### 世界身份

英文辅助名：world identity。

宿主指定的每世界唯一身份，安装时作为 `install` 必填参数进入运行时；切换描述符 `worldBinding` 以它做错误世界守卫。同一宿主进程内不得对多个世界复用；编码与生命周期由宿主拥有，运行时不解释。

精确标识符：` world_id `（字段）。

依据：[修订切换事务][source-design-traffic-runtime-revision-cutover-473]。

<a id="domain-spatial"></a>
## 空间与几何

点击中文规范名查看完整定义；跨领域概念指向同一词条。

| 中文规范名                                                  | 英文辅助名                   | 精确标识符             |
| ----------------------------------------------------------- | ---------------------------- | ---------------------- |
| [规范坐标框架][term-canonical-frame]                        | canonical frame              | ` CanonicalFrame `     |
| [路口中心至道路外端距离][term-center-to-outer-end-distance] | center-to-outer-end distance | —                      |
| [冲突区空间区域][term-conflict-zone-region]                 | conflict zone region         | ` ConflictZoneRegion ` |
| [路口外部道路][term-external-road-segment]                  | external road segment        | —                      |
| [几何][term-geometry]                                       | geometry                     | —                      |
| [相位状态位置][term-phase-state-position]                   | phase state position         | —                      |
| [放置令牌][term-placement-token]                            | placement token              | —                      |
| [位姿批次][term-pose-batch]                                 | pose batch                   | —                      |
| [空间层][term-spatial-layer]                                | Spatial layer                | ` laneflow-spatial `   |
| [空间会话][term-spatial-session]                            | spatial session              | ` SpatialSession `     |

<a id="term-canonical-frame"></a>
### 规范坐标框架

英文辅助名：canonical frame。

空间几何和位姿共享的稳定局部坐标框架。

精确标识符：` CanonicalFrame `（类型）。

依据：[空间几何设计][source-design-spatial-geometry-485]。

<a id="term-center-to-outer-end-distance"></a>
### 路口中心至道路外端距离

英文辅助名：center-to-outer-end distance。

路口生成器的 `arm_length_meters` 参数；其值从路口中心计量，不能当作扣除路口半径后的外部道路长度。代码与配置键保持原文。

依据：[示例场景设计][source-design-example-scenarios-486]。

<a id="term-conflict-zone-region"></a>
### 冲突区空间区域

英文辅助名：conflict zone region。

与冲突区配对的可选 2.5D 简单 XZ 多边形及高度范围；只服务验证、调试和表现，不拥有冲突行为。

精确标识符：` ConflictZoneRegion `（类型）。

依据：[空间几何设计][source-design-spatial-geometry-485]。

<a id="term-external-road-segment"></a>
### 路口外部道路

英文辅助名：external road segment。

生成场景中位于路口内部连接之外、通向相邻路口或外围环路的道路部分；上下文明确后可简称“外部道路”。数量须区分有向道路、双向道路和车道，不代替“进口道/出口道”的有向接近关系。

允许简称“外部道路”：生成场景道路几何语境已明确。

依据：[示例场景设计][source-design-example-scenarios-486]。

<a id="term-geometry"></a>
### 几何

英文辅助名：geometry。

坐标框架、曲线、折线、弧长和采样数据。

依据：[空间几何设计][source-design-spatial-geometry-485]。

<a id="term-phase-state-position"></a>
### 相位状态位置

英文辅助名：phase state position。

信号相位状态向量中按 `linkIndex` 关联受控连接的位置；“未认领位置”不是一条道路或一条进口道。转换器是否接受及如何报告，以现有信号源健康检查合同为准。

依据：[示例场景设计][source-design-example-scenarios-486]。

<a id="term-placement-token"></a>
### 放置令牌

英文辅助名：placement token。

绑定规范坐标框架与宿主放置状态、防止过期位姿写入的令牌。

依据：[空间几何设计][source-design-spatial-geometry-485]。

<a id="term-pose-batch"></a>
### 位姿批次

英文辅助名：pose batch。

空间层按稳定顺序批量产生的位置和朝向结果。

依据：[空间几何设计][source-design-spatial-geometry-485]。

<a id="term-spatial-layer"></a>
### 空间层

英文辅助名：Spatial layer。

拥有规范几何采样和位姿语义、但不拥有交通规则的引擎无关组件。

精确标识符：` laneflow-spatial `（包）。

依据：[空间几何设计][source-design-spatial-geometry-485]。

<a id="term-spatial-session"></a>
### 空间会话

英文辅助名：spatial session。

绑定共享路网修订、消费共享空间路网进行规范几何采样的会话。

精确标识符：` SpatialSession `（类型）。

依据：[空间几何设计][source-design-spatial-geometry-485]。

<a id="domain-adapter"></a>
## 引擎适配

点击中文规范名查看完整定义；跨领域概念指向同一词条。

| 中文规范名                        | 英文辅助名              | 精确标识符    |
| --------------------------------- | ----------------------- | ------------- |
| [引擎适配器][term-engine-adapter] | engine adapter；Adapter | —             |
| [宿主变换][term-host-transform]   | host transform          | ` Transform ` |
| [细节层次][term-level-of-detail]  | level of detail         | ` LOD `       |

<a id="term-engine-adapter"></a>
### 引擎适配器

英文辅助名：engine adapter；Adapter。

把运行时快照和空间位姿映射到宿主引擎生命周期与表现对象的组件。

依据：[适配器应用程序接口（API）][source-design-adapter-api-490]。

<a id="term-host-transform"></a>
### 宿主变换

英文辅助名：host transform。

宿主引擎中的位置、旋转和缩放表示；不得反写为交通进度正本。

精确标识符：` Transform `（类型）。

依据：[适配器应用程序接口（API）][source-design-adapter-api-490]。

<a id="term-level-of-detail"></a>
### 细节层次

英文辅助名：level of detail。

按距离、预算或表现需求选择渲染/调试细节的适配器策略。

精确标识符：` LOD `（缩写）。

依据：[适配器应用程序接口（API）][source-design-adapter-api-490]。

<a id="domain-measurement"></a>
## 测量与记录

点击中文规范名查看完整定义；跨领域概念指向同一词条。

| 中文规范名                                                                 | 英文辅助名                                      | 精确标识符                          |
| -------------------------------------------------------------------------- | ----------------------------------------------- | ----------------------------------- |
| [活动交通参与单元数][term-active-traffic-participant-count]                | active traffic participant count                | ` N_active[d] `                     |
| [聚合等价参与单元数][term-aggregate-equivalent-participant-count]          | aggregate-equivalent participant count          | ` N_aggregate_equivalent[d] `       |
| [聚合交通量][term-aggregate-traffic-population]                            | aggregate traffic population                    | ` N_aggregate `                     |
| [聚合交通记录数][term-aggregate-traffic-record-count]                      | aggregate traffic record count                  | ` N_aggregate_records[d] `          |
| [中国特色城市工作负载][term-chinese-style-city-workload]                   | Chinese-style city workload                     | —                                   |
| [诊断记录][term-diagnostic-record]                                         | diagnostic record                               | —                                   |
| [环境记录][term-environment-record]                                        | environment record                              | —                                   |
| [执行路径][term-execution-path]                                            | execution path                                  | —                                   |
| [实验方案][term-experimental-variant]                                      | experimental variant                            | —                                   |
| [文件摘要清单][term-file-digest-manifest]                                  | file digest manifest                            | —                                   |
| [单次运行][term-individual-run]                                            | individual run                                  | —                                   |
| [个体交通参与单元数][term-individual-traffic-participant-count]            | individual traffic participant count            | ` N_individual[d] `                 |
| [个体车辆数][term-individual-vehicle-count]                                | individual vehicle count                        | ` N_individual `                    |
| [意图更新参与单元数][term-intent-update-participant-count]                 | intent-update participant count                 | ` N_intent[d] `                     |
| [意图更新车辆数][term-intent-update-vehicle-count]                         | intent-update vehicle count                     | ` N_intent `                        |
| [测量记录][term-measurement-record]                                        | measurement record                              | —                                   |
| [多世界集合][term-multi-world-ensemble]                                    | multi-world ensemble；ensemble                  | —                                   |
| [P100 首轮认证实测基线][term-p100-initial-attestation-production-baseline] | P100 initial attestation production baseline    | ` LF-ATTEST-P100-PRODUCTION-R0-v1 ` |
| [P100 首轮生产编译基线][term-p100-initial-production-compiler-baseline]    | P100 initial production compiler baseline       | ` LF-COMP-P100-PRODUCTION-R0-v1 `   |
| [峰值分配][term-peak-allocation]                                           | peak allocation                                 | —                                   |
| [逐拍计时日志][term-per-tick-timing-log]                                   | per-tick timing log                             | —                                   |
| [表现交通参与单元数][term-presented-traffic-participant-count]             | presented traffic participant count             | ` N_presented[d] `                  |
| [表现车辆数][term-presented-vehicle-count]                                 | presented vehicle count                         | ` N_presented `                     |
| [安静测量窗口][term-quiet-measurement-window]                              | quiet measurement window                        | —                                   |
| [参考机资产标识][term-reference-machine-asset-identifier]                  | reference-machine asset identifier              | ` referenceMachineId `              |
| [参考机硬件身份指纹][term-reference-machine-hardware-identity-fingerprint] | reference-machine hardware identity fingerprint | ` hardwareIdentitySha256 `          |
| [保留内存][term-retained-memory]                                           | retained memory                                 | —                                   |
| [道路交通活动车辆数][term-road-traffic-active-vehicle-count]               | road-traffic active vehicle count               | ` N_traffic_active `                |
| [运行配置][term-run-configuration]                                         | run configuration                               | —                                   |
| [运行记录][term-run-records]                                               | run records                                     | —                                   |
| [目标产品推荐参考机型][term-target-product-recommended-reference-machine]  | target product recommended reference machine    | ` P100 `                            |
| [测试分组][term-test-case-group]                                           | test case group                                 | —                                   |
| [工作负载][term-workload]                                                  | workload                                        | —                                   |

<a id="term-active-traffic-participant-count"></a>
### 活动交通参与单元数

英文辅助名：active traffic participant count。

执行域 `d` 中当前参与该域运动、安全或占用求解的个体交通参与单元数。

精确标识符：` N_active[d] `（记号）。

用于长期按执行域分解的规模报告；不替代当前车辆基准或历史证据，也不表示非机动车、行人或轨道执行域已经实现。

依据：[ADR 0021：交通基础设施定位与宿主边界][source-adr-0021-416]、[Traffic Runtime 产品性能基线][source-design-core-runtime-performance-baseline-524]。

<a id="term-aggregate-equivalent-participant-count"></a>
### 聚合等价参与单元数

英文辅助名：aggregate-equivalent participant count。

执行域 `d` 中聚合表示所代表的交通参与单元数，用于描述保真度与覆盖规模；不能替代聚合交通记录数。

精确标识符：` N_aggregate_equivalent[d] `（记号）。

用于长期按执行域分解的规模报告；不替代当前车辆基准或历史证据，也不表示非机动车、行人或轨道执行域已经实现。

依据：[ADR 0021：交通基础设施定位与宿主边界][source-adr-0021-416]、[Traffic Runtime 产品性能基线][source-design-core-runtime-performance-baseline-524]。

<a id="term-aggregate-traffic-population"></a>
### 聚合交通量

英文辅助名：aggregate traffic population。

只以流、包或计数存在、没有完整逐车身份的交通量。

精确标识符：` N_aggregate `（记号）。

用于道路机动车特化报告；登记计数名称不证明对应能力或容量已经实现。

依据：[ADR 0021：交通基础设施定位与宿主边界][source-adr-0021-416]、[Traffic Runtime 产品性能基线][source-design-core-runtime-performance-baseline-524]。

<a id="term-aggregate-traffic-record-count"></a>
### 聚合交通记录数

英文辅助名：aggregate traffic record count。

执行域 `d` 中由运行时实际存储、调度或更新的聚合流、包、单元格等记录数，用于衡量计算与内存成本。

精确标识符：` N_aggregate_records[d] `（记号）。

用于长期按执行域分解的规模报告；不替代当前车辆基准或历史证据，也不表示非机动车、行人或轨道执行域已经实现。

依据：[ADR 0021：交通基础设施定位与宿主边界][source-adr-0021-416]、[Traffic Runtime 产品性能基线][source-design-core-runtime-performance-baseline-524]。

<a id="term-chinese-style-city-workload"></a>
### 中国特色城市工作负载

英文辅助名：Chinese-style city workload。

用版本化拓扑、需求与运行时指标表达中国特色交通压力的代表性工作负载族。

依据：[ADR 0021：交通基础设施定位与宿主边界][source-adr-0021-416]。

<a id="term-diagnostic-record"></a>
### 诊断记录

英文辅助名：diagnostic record。

harness 的 `diagnostics.json` 所保存的诊断选择、实际 worker 数、计时等运行信息；其文件摘要独立核对，和测量记录的交叉检查按现有合同执行。

依据：[LF-SYNTH-v2 表现与测量接线][source-design-lf-synth-measurement-525]。

<a id="term-environment-record"></a>
### 环境记录

英文辅助名：environment record。

测量前后采集的本机环境、工具链、源码状态、执行者信息、机器指纹与进程快照；不替代测量记录的构建协议字段。

依据：[测量环境与进程观测][source-design-host-observe-526]。

<a id="term-execution-path"></a>
### 执行路径

英文辅助名：execution path。

在给定条件下实际经过的代码路径，例如融合、分发或回退路径；区别于配置名称，实际路径由既有路径计数等证据确认。

<a id="term-experimental-variant"></a>
### 实验方案

英文辅助名：experimental variant。

与参考实现比较的候选实现或方法；同一方案可按冻结配置重复运行，方案内波动与方案间差异分开判断。

<a id="term-file-digest-manifest"></a>
### 文件摘要清单

英文辅助名：file digest manifest。

`result.files` 中文件名到摘要的映射，用于核对同一次运行的文件完整性；摘要和文件在同一可写目录，不提供执行或来源真实性证明。

依据：[LF-SYNTH-v2 表现与测量接线][source-design-lf-synth-measurement-525]。

<a id="term-individual-run"></a>
### 单次运行

英文辅助名：individual run。

测试或测量的一次执行及其记录；上下文明确后可称“一次运行”或“运行”。起止拍号、是否续跑或分段、完成状态与证据资格按各自合同判断，不由这一称呼推定。

允许简称“一次运行”：当前段落已经明确是一次测试或测量执行。

允许简称“运行”：当前段落已经明确是一次测试或测量执行。

<a id="term-individual-traffic-participant-count"></a>
### 个体交通参与单元数

英文辅助名：individual traffic participant count。

执行域 `d` 中仍存在并保留完整身份与生命周期状态的交通参与单元数。

精确标识符：` N_individual[d] `（记号）。

用于长期按执行域分解的规模报告；不替代当前车辆基准或历史证据，也不表示非机动车、行人或轨道执行域已经实现。

依据：[ADR 0021：交通基础设施定位与宿主边界][source-adr-0021-416]、[Traffic Runtime 产品性能基线][source-design-core-runtime-performance-baseline-524]。

<a id="term-individual-vehicle-count"></a>
### 个体车辆数

英文辅助名：individual vehicle count。

当前 `TrafficWorld` 中仍存在并保留完整身份、路线/进度、停车与生命周期状态的车辆数。

精确标识符：` N_individual `（记号）。

用于道路机动车特化报告；登记计数名称不证明对应能力或容量已经实现。

依据：[ADR 0021：交通基础设施定位与宿主边界][source-adr-0021-416]、[Traffic Runtime 产品性能基线][source-design-core-runtime-performance-baseline-524]。

<a id="term-intent-update-participant-count"></a>
### 意图更新参与单元数

英文辅助名：intent-update participant count。

当前固定步进在执行域 `d` 中实际重新计算昂贵行为或控制意图的个体交通参与单元数。

精确标识符：` N_intent[d] `（记号）。

用于长期按执行域分解的规模报告；不替代当前车辆基准或历史证据，也不表示非机动车、行人或轨道执行域已经实现。

依据：[ADR 0021：交通基础设施定位与宿主边界][source-adr-0021-416]、[Traffic Runtime 产品性能基线][source-design-core-runtime-performance-baseline-524]。

<a id="term-intent-update-vehicle-count"></a>
### 意图更新车辆数

英文辅助名：intent-update vehicle count。

当前固定步进实际重新计算昂贵控制意图的车辆数。

精确标识符：` N_intent `（记号）。

用于道路机动车特化报告；登记计数名称不证明对应能力或容量已经实现。

依据：[ADR 0021：交通基础设施定位与宿主边界][source-adr-0021-416]、[Traffic Runtime 产品性能基线][source-design-core-runtime-performance-baseline-524]。

<a id="term-measurement-record"></a>
### 测量记录

英文辅助名：measurement record。

harness 的 `measurements.toml` 所保存的测量载荷，包括协议版本、来源、运行配置与样本；不因记录存在就证明达到产品预算。

依据：[LF-SYNTH-v2 表现与测量接线][source-design-lf-synth-measurement-525]。

<a id="term-multi-world-ensemble"></a>
### 多世界集合

英文辅助名：multi-world ensemble；ensemble。

共享静态数据、但各自持有独立可变状态的一组世界，用于回放、探索或吞吐测量。

依据：[ADR 0021：交通基础设施定位与宿主边界][source-adr-0021-416]。

<a id="term-p100-initial-attestation-production-baseline"></a>
### P100 首轮认证实测基线

英文辅助名：P100 initial attestation production baseline。

共享路网修订认证与直算摘要的具名历史实测基线；结论受被测源码、工具链、环境及工作负载约束，不代表后续主干性能认证。

精确标识符：` LF-ATTEST-P100-PRODUCTION-R0-v1 `（常量）。

依据：[运行时状态完整性校验与失真恢复][source-design-traffic-runtime-state-integrity-413]。

<a id="term-p100-initial-production-compiler-baseline"></a>
### P100 首轮生产编译基线

英文辅助名：P100 initial production compiler baseline。

真实合法产品场景在具名参考机上形成的首轮描述性编译基线；用于同机回退对照，不是产品 SLA 或城市容量。

精确标识符：` LF-COMP-P100-PRODUCTION-R0-v1 `（常量）。

依据：[编译器基础设施与合成领域专用语言前端][source-design-compiler-foundation-149]。

<a id="term-peak-allocation"></a>
### 峰值分配

英文辅助名：peak allocation。

某操作期间同时存在的最大动态分配量。

依据：[Traffic Runtime 产品性能基线][source-design-core-runtime-performance-baseline-524]。

<a id="term-per-tick-timing-log"></a>
### 逐拍计时日志

英文辅助名：per-tick timing log。

harness 的 `timings.jsonl` 按完成拍号记录的计时数据；文件存在性、摘要和计时一致性仍须校验，非确定性耗时不参与跨运行语义相等比较。

依据：[LF-SYNTH-v2 表现与测量接线][source-design-lf-synth-measurement-525]。

<a id="term-presented-traffic-participant-count"></a>
### 表现交通参与单元数

英文辅助名：presented traffic participant count。

当前外层帧在执行域 `d` 中由适配器或表现层按个体身份实例化、提取或提交的交通参与单元数。

精确标识符：` N_presented[d] `（记号）。

用于长期按执行域分解的规模报告；不替代当前车辆基准或历史证据，也不表示非机动车、行人或轨道执行域已经实现。

依据：[ADR 0021：交通基础设施定位与宿主边界][source-adr-0021-416]、[Traffic Runtime 产品性能基线][source-design-core-runtime-performance-baseline-524]。

<a id="term-presented-vehicle-count"></a>
### 表现车辆数

英文辅助名：presented vehicle count。

当前外层帧由适配器或表现层实例化、提取或提交的车辆数。

精确标识符：` N_presented `（记号）。

用于道路机动车特化报告；登记计数名称不证明对应能力或容量已经实现。

依据：[ADR 0021：交通基础设施定位与宿主边界][source-adr-0021-416]、[Traffic Runtime 产品性能基线][source-design-core-runtime-performance-baseline-524]。

<a id="term-quiet-measurement-window"></a>
### 安静测量窗口

英文辅助名：quiet measurement window。

维护者批准的一段独占机器的时间，用于运行结果随同机负载变化的正式测量；共享 CI runner 不构成此窗口。

依据：[开发阶段评审][source-governance-development-gates-527]。

<a id="term-reference-machine-asset-identifier"></a>
### 参考机资产标识

英文辅助名：reference-machine asset identifier。

仓库为一台选定物理参考机分配的持久逻辑标识；设备更换不得沿用，硬件维修是否保留须由硬件身份指纹和变更记录共同裁决。

精确标识符：` referenceMachineId `（字段）。

依据：[Traffic Runtime 产品性能基线][source-design-core-runtime-performance-baseline-524]。

<a id="term-reference-machine-hardware-identity-fingerprint"></a>
### 参考机硬件身份指纹

英文辅助名：reference-machine hardware identity fingerprint。

按版本化规范从本机 SMBIOS 身份字段计算的 SHA-256；用于确认正式运行是否发生在同一物理参考机，不公开原始序列号，也不单独证明运行环境或产品通过。

精确标识符：` hardwareIdentitySha256 `（字段）。

依据：[Traffic Runtime 产品性能基线][source-design-core-runtime-performance-baseline-524]。

<a id="term-retained-memory"></a>
### 保留内存

英文辅助名：retained memory。

初始化完成后仍由组件持有的内存。

依据：[Traffic Runtime 产品性能基线][source-design-core-runtime-performance-baseline-524]。

<a id="term-road-traffic-active-vehicle-count"></a>
### 道路交通活动车辆数

英文辅助名：road-traffic active vehicle count。

当前处于道路交通系统、每个 TrafficWorld 固定步进参与运动、安全或占用求解的车辆数。

精确标识符：` N_traffic_active `（记号）。

用于道路机动车特化报告；登记计数名称不证明对应能力或容量已经实现。

依据：[ADR 0021：交通基础设施定位与宿主边界][source-adr-0021-416]、[Traffic Runtime 产品性能基线][source-design-core-runtime-performance-baseline-524]。

<a id="term-run-configuration"></a>
### 运行配置

英文辅助名：run configuration。

一次测试或测量采用的参数组合，如 worker 数、分区参数与分发开关；上下文明确后可简称“配置”。同一配置可以重复运行，配置本身不等于一次执行。

允许简称“配置”：当前段落已经明确是测试或测量参数组合。

<a id="term-run-records"></a>
### 运行记录

英文辅助名：run records。

单次运行产生的回执、测量记录、诊断记录、日志等信息的统称；具体校验和比较须说明文件及字段，不默认所有记录均参与语义相等比较。

依据：[LF-SYNTH-v2 表现与测量接线][source-design-lf-synth-measurement-525]。

<a id="term-target-product-recommended-reference-machine"></a>
### 目标产品推荐参考机型

英文辅助名：target product recommended reference machine。

由产品负责人选定、用于目标产品性能预算与最终认证的具名物理硬件；它由参考机资产标识和硬件身份指纹共同绑定，不是只凭系列或配置继承的机型类别。它不是最低产品机，选定该角色不等于产品通过，等价或替代机型仍须单独验证。

精确标识符：` P100 `（记号）。

依据：[Traffic Runtime 产品性能基线][source-design-core-runtime-performance-baseline-524]。

<a id="term-test-case-group"></a>
### 测试分组

英文辅助名：test case group。

为某组输入、故障或边界条件组织的测试用例；不表示运行时分支或交通分组。

<a id="term-workload"></a>
### 工作负载

英文辅助名：workload。

冻结输入、规模、拓扑、行为组合和测量协议的可复现实验场景。

依据：[ADR 0021：交通基础设施定位与宿主边界][source-adr-0021-416]。

<a id="domain-testing"></a>
## 测试与检查

点击中文规范名查看完整定义；跨领域概念指向同一词条。

| 中文规范名                                          | 英文辅助名                     | 精确标识符 |
| --------------------------------------------------- | ------------------------------ | ---------- |
| [补充检查][term-additional-check]                   | additional check               | —          |
| [备用测试方式][term-alternative-test-execution]     | alternative test execution     | —          |
| [编译失败测试][term-compile-fail-test]              | compile-fail test              | —          |
| [默认规则][term-default-rule]                       | default rule                   | —          |
| [差分测试][term-differential-testing]               | differential testing           | —          |
| [回退路径][term-fallback-path]                      | fallback path                  | —          |
| [故障注入][term-fault-injection]                    | fault injection                | —          |
| [最终排序键][term-final-sort-key]                   | final sort key                 | —          |
| [fixture][term-fixture]                             | —                              | —          |
| [模糊测试][term-fuzz-testing]                       | fuzz testing；fuzz             | —          |
| [仪器探针边界][term-instrumentation-probe-boundary] | instrumentation probe boundary | —          |
| [已知向量][term-known-vector]                       | known vector                   | —          |
| [变形测试][term-metamorphic-test]                   | metamorphic test               | —          |
| [性能归因计时][term-performance-attribution-timing] | performance attribution timing | —          |
| [占位输出][term-placeholder-output]                 | placeholder output             | —          |
| [属性测试][term-property-testing]                   | property testing               | —          |
| [研究/测试仪器][term-research-test-instrumentation] | research/test instrumentation  | —          |
| [保留内存账本][term-retained-memory-ledger]         | retained memory ledger         | —          |
| [语义研究原型][term-semantic-research-prototype]    | semantic research prototype    | —          |
| [语义验证][term-semantic-validation]                | semantic validation            | —          |
| [结构验证][term-structural-verification]            | structural verification        | —          |
| [测试支持接缝][term-test-support-seam]              | test-support seam              | —          |

<a id="term-additional-check"></a>
### 补充检查

英文辅助名：additional check。

在已有检查之外核对另一条件或粒度；须明示检查对象、执行阶段以及当前未检查的范围。

<a id="term-alternative-test-execution"></a>
### 备用测试方式

英文辅助名：alternative test execution。

可选用的另一种测试执行方式；libtest 与 nextest 的线程模型和计时结果不可混同。

<a id="term-compile-fail-test"></a>
### 编译失败测试

英文辅助名：compile-fail test。

以预期无法通过类型检查的调用证明非法公共接口组合在编译期不可表达的测试。

<a id="term-default-rule"></a>
### 默认规则

英文辅助名：default rule。

未指定可选过滤条件时采用的规则；策略声明中的 rank 0 低于显式命中的类别选择器，不等于“全部类别”过滤器。

依据：[路权策略编译与 ConflictArbiter 实施合同][source-design-traffic-runtime-right-of-way-policy-415]。

<a id="term-differential-testing"></a>
### 差分测试

英文辅助名：differential testing。

对相同输入比较两个独立实现的语义、诊断、标识或字节结果。

<a id="term-fallback-path"></a>
### 回退路径

英文辅助名：fallback path。

当前路径的条件不成立时转入的既有路径；例如申请者名单退回全量重建，不表示可忽略错误。

<a id="term-fault-injection"></a>
### 故障注入

英文辅助名：fault injection。

在受检点强制返回受控失败以验证失败原子性等硬不变量的测试机制；注入检查点不得进入默认发布构建。

依据：[Traffic Runtime 仿真与管理模块边界][source-design-traffic-runtime-module-boundary-550]。

<a id="term-final-sort-key"></a>
### 最终排序键

英文辅助名：final sort key。

前序字段全部相同时才比较的键；受控连接排序中的 linkIndex 属此类，不是失败后的替代路径。

<a id="term-fixture"></a>
### fixture

借词；首次出现释义：固定测试环境。

测试与证据链中固定不变的受控输入环境与样本。

<a id="term-fuzz-testing"></a>
### 模糊测试

英文辅助名：fuzz testing；fuzz。

以大量生成或变异输入检查崩溃、越界和失败关闭行为。

<a id="term-instrumentation-probe-boundary"></a>
### 仪器探针边界

英文辅助名：instrumentation probe boundary。

生产路径持有空操作默认探针、研究态注入测量实现的显式接缝；默认发布构建必须与无仪器构建等价。

依据：[Traffic Runtime 仿真与管理模块边界][source-design-traffic-runtime-module-boundary-550]。

<a id="term-known-vector"></a>
### 已知向量

英文辅助名：known vector。

冻结输入及其精确字节/标识/摘要预期值的跨实现测试向量。

<a id="term-metamorphic-test"></a>
### 变形测试

英文辅助名：metamorphic test。

通过不应改变结果的输入变换验证稳定性契约的测试。

<a id="term-performance-attribution-timing"></a>
### 性能归因计时

英文辅助名：performance attribution timing。

在生产步进路径的具名阶段边界采集、只用于机制归因的耗时测量；不构成产品延迟取证。

依据：[Traffic Runtime 仿真与管理模块边界][source-design-traffic-runtime-module-boundary-550]。

<a id="term-placeholder-output"></a>
### 占位输出

英文辅助名：placeholder output。

为继续诊断保留结构或计数的替代内容；转换器的诊断用占位曲线不进入 compiler 验收，也不据此交付 LFCA。

<a id="term-property-testing"></a>
### 属性测试

英文辅助名：property testing。

生成满足约束的输入并检查不变量、往返或关系性质，而不是只断言少量固定样例的测试。

<a id="term-research-test-instrumentation"></a>
### 研究/测试仪器

英文辅助名：research/test instrumentation。

为研究取证或测试检查目的嵌入或接入运行时执行路径的测量、观察与注入机制的总称；不是生产语义的一部分。

依据：[Traffic Runtime 仿真与管理模块边界][source-design-traffic-runtime-module-boundary-550]。

<a id="term-retained-memory-ledger"></a>
### 保留内存账本

英文辅助名：retained memory ledger。

按所有权穷尽枚举组件的保留内存核算结构；新增字段必须触发编译失败以强制分类。

依据：[Traffic Runtime 仿真与管理模块边界][source-design-traffic-runtime-module-boundary-550]。

<a id="term-semantic-research-prototype"></a>
### 语义研究原型

英文辅助名：semantic research prototype。

以单线程生产执行为精确预言机、验证候选架构精确等价或在显式保真度预算内有界等价的非生产研究执行器；结论可登记为研究证据，不自动成为生产架构。

依据：[Traffic Runtime 仿真与管理模块边界][source-design-traffic-runtime-module-boundary-550]。

<a id="term-semantic-validation"></a>
### 语义验证

英文辅助名：semantic validation。

检查来源、实体、关系和领域规则是否满足规范语义的验证。

<a id="term-structural-verification"></a>
### 结构验证

英文辅助名：structural verification。

在不重跑完整领域语义的前提下，检查数据布局、引用边界和消费前置条件的验证。

<a id="term-test-support-seam"></a>
### 测试支持接缝

英文辅助名：test-support seam。

以编译期门控向测试与研究暴露非生产能力的正式边界；`#[doc(hidden)]` 只控制文档可见性，不构成构建排除。

依据：[Traffic Runtime 仿真与管理模块边界][source-design-traffic-runtime-module-boundary-550]。

<a id="retired-concepts"></a>
## 停止采用的概念

本节用于识别旧文档中的概念，不作为现行 API、格式或能力说明。

<a id="term-background-verification"></a>
### 后台验证

英文辅助名：background verification。

旧目标专用静态镜像方案中，由后台任务提前完成尚未请求节检查的策略。

停止采用：独立静态镜像及其描述符、分块验证和发布审计方案已取消；本条不表示现行可移植制品或共享静态路网提供同名能力。

依据：[ADR 0025：受检规范路网与共享静态路网][source-adr-0025-551]。

<a id="term-core-world"></a>
### 核心世界

英文辅助名：core world。

旧核心运行时中的世界类型。

精确标识符：` CoreWorld `（类型）。

停止采用：旧组件及其世界类型已拆除。

- [交通世界][term-traffic-world]：现行职责对应；不表示 API 或字节兼容。

依据：[交通运行时共享静态路网消费][source-design-traffic-runtime-shared-consumption-420]。

<a id="term-current-core-object-graph"></a>
### 当前核心对象图

英文辅助名：current Core object graph。

旧动态执行层的对象、句柄与登记表构造。

停止采用：旧核心运行时已拆除，不作为可运行世界或编译器中间表示。

依据：[交通运行时共享静态路网消费][source-design-traffic-runtime-shared-consumption-420]。

<a id="term-current-json"></a>
### 当前 JSON

英文辅助名：current JSON。

已删除的 Traffic、SpatialPackage 与 ScenarioManifest JSON 生产入口的历史合称。

停止采用：生产编制与消费已转为编译器、可移植规范制品与共享静态路网。

依据：[交通运行时共享静态路网消费][source-design-traffic-runtime-shared-consumption-420]。

<a id="term-eager-verification"></a>
### 预先验证

英文辅助名：eager verification。

旧目标专用静态镜像方案中，在暴露视图前主动完成全部所需分块与结构检查的策略。

停止采用：独立静态镜像及其描述符、分块验证和发布审计方案已取消；本条不表示现行可移植制品或共享静态路网提供同名能力。

依据：[ADR 0025：受检规范路网与共享静态路网][source-adr-0025-551]。

<a id="term-full-audit"></a>
### 完整审计

英文辅助名：full audit。

旧目标专用静态镜像方案中读取并核对完整镜像摘要的独立审计路径。

停止采用：独立静态镜像及其描述符、分块验证和发布审计方案已取消；本条不表示现行可移植制品或共享静态路网提供同名能力。

依据：[ADR 0025：受检规范路网与共享静态路网][source-adr-0025-551]。

<a id="term-geometry-document-frontend"></a>
### 几何文档前端

英文辅助名：Geometry document frontend。

严格 JSON 几何文档方案中的编制前端。

停止采用：道路编辑来源已采用 FlatBuffers，该方案不再作为实现输入。

- [道路编辑前端][term-road-editing-frontend]：现行职责对应；不表示 API 或字节兼容。

依据：[道路编辑来源与几何编制前端][source-design-road-editing-source-and-geometry-frontend-151]。

<a id="term-geometry-source-module"></a>
### 几何来源模块

英文辅助名：Geometry source module。

旧 JSON 几何方案拟公开的预构造模块类型。

精确标识符：` GeometryModule `（类型）。

停止采用：道路编辑前端使用借用来源输入和共同受检接入，不公开该模块类型。

- [道路编辑来源输入][term-road-editing-source-input]：现行职责对应；不表示 API 或字节兼容。

依据：[编译器基础设施与合成领域专用语言前端][source-design-compiler-foundation-149]。

<a id="term-image-header"></a>
### 镜像头

英文辅助名：image header。

旧目标专用静态镜像方案中声明布局、目标、配置档与来源链的起始结构。

停止采用：独立静态镜像及其描述符、分块验证和发布审计方案已取消；本条不表示现行可移植制品或共享静态路网提供同名能力。

依据：[ADR 0025：受检规范路网与共享静态路网][source-adr-0025-551]。

<a id="term-laneflow-core"></a>
### LaneFlow 核心

英文辅助名：LaneFlow Core。

已拆除的前动态执行层组件。

精确标识符：` laneflow-core `（包）。

停止采用：由 LaneFlow 交通运行时一次性替代，不保留可运行旧入口。

- [LaneFlow 交通运行时][term-laneflow-traffic-runtime]：现行职责对应；不表示 API 或字节兼容。

依据：[交通运行时共享静态路网消费][source-design-traffic-runtime-shared-consumption-420]。

<a id="term-lazy-verification"></a>
### 延迟验证

英文辅助名：lazy verification。

旧目标专用静态镜像方案中，在首次请求某节或能力时才完成对应检查的策略。

停止采用：独立静态镜像及其描述符、分块验证和发布审计方案已取消；本条不表示现行可移植制品或共享静态路网提供同名能力。

依据：[ADR 0025：受检规范路网与共享静态路网][source-adr-0025-551]。

<a id="term-p100-initial-compiler-performance-gate"></a>
### P100 首轮编译性能门槛

英文辅助名：P100 initial compiler performance gate。

原编译器研究使用的首轮抽象工作负载门槛。

精确标识符：` LF-COMP-P100-R0-v1 `（常量）。

停止采用：抽象工作负载不能无损映射生产语义，已降为容量估算与实现选型输入，不构成产品验收。

依据：[编译器基础设施与合成领域专用语言前端][source-design-compiler-foundation-149]。

<a id="term-static-route"></a>
### 静态路线

英文辅助名：static route。

旧方案中拟预编译到静态制品的路线实体。

精确标识符：` StaticRoute `（类型）。

停止采用：路线改由每世界注册，不属于现行标识或制品登记。

- [路线][term-route]：现行职责对应；不表示 API 或字节兼容。

依据：[0029 退役预编译静态路线，路线入口只留 register\_route][source-adr-0029-552]。

<a id="term-structurally-verified-image"></a>
### 已结构验证镜像

英文辅助名：structurally verified image。

旧静态镜像分层方案中的结构检查结果类型名。

精确标识符：` StructurallyVerifiedImage `（类型）。

停止采用：现行制品检查采用受检对象视图与规范路网输入，该镜像类型名不表示现行 API。

- [已值域检查规范制品][term-value-checked-canonical-artifact]：现行职责对应；不表示 API 或字节兼容。

依据：[ADR 0025：受检规范路网与共享静态路网][source-adr-0025-280]。

<a id="term-trusted-descriptor"></a>
### 可信描述符

英文辅助名：trusted descriptor。

旧目标专用静态镜像方案中经对象外信任锚认证并绑定目标与配置档的描述符。

停止采用：独立静态镜像及其描述符、分块验证和发布审计方案已取消；本条不表示现行可移植制品或共享静态路网提供同名能力。

依据：[ADR 0025：受检规范路网与共享静态路网][source-adr-0025-551]。

<a id="term-unverified-image-bytes"></a>
### 未验证镜像字节

英文辅助名：unverified image bytes。

旧静态镜像分层方案中的未受检字节类型名。

精确标识符：` UnverifiedImageBytes `（类型）。

停止采用：现行制品检查与共享构建采用受检规范路网输入，该镜像类型名不表示现行 API。

依据：[ADR 0025：受检规范路网与共享静态路网][source-adr-0025-280]。

<a id="term-validated-current-source-bundle"></a>
### 已验证当前态来源包

英文辅助名：validated current-source bundle。

旧 JSON 加载链路中的历史内部来源 DTO。

精确标识符：` ValidatedCurrentSourceBundle `（类型）。

停止采用：所属加载组件已拆除，不是现行编译器输入。

依据：[交通运行时共享静态路网消费][source-design-traffic-runtime-shared-consumption-420]。

<a id="term-validation-receipt-envelope"></a>
### 验证收据封套

英文辅助名：validation receipt envelope；validation receipt。

原方案拟独立交付的验证收据制品。

精确标识符：` ValidationReceiptEnvelope `（类型）。

允许简称“验证收据”：仅在讨论已经取消的历史收据方案时使用。

停止采用：后发射检查与最小发布合同已取消独立验证收据，不保留同义现行制品。

依据：[ADR 0024：编译器后发射检查与最小发布闭合][source-adr-0024-553]。

<a id="former-names"></a>
## 旧称查询

按下面的适用范围和例外选择规范名；条件化替换不能退化为全文字符串替换。

<a id="name-artifact-capacity"></a>
### 制品容量

外部可移植字节准入语境。

- [规范路网容量][term-canonical-network-capacity]：约束 LFCA 表达的规范路网规模。
- [制品结构统计][term-artifact-shape]：描述物理结构并作为资源计划输入。

适用范围包含同义的派生复合词。

登记依据：[\#1296][issue-1296]。

<a id="name-byte-backing-label"></a>
### 不可变字节背板 / 字节背板

byte backing 存储语境。

- [不可变字节后备存储][term-immutable-byte-backing]：描述受检字节的存储所有权。

适用范围包含同义的派生复合词。

例外：信号灯硬件背板不适用。

登记依据：[\#1229][issue-1229]。

<a id="name-compile-capacity-dimension"></a>
### 容量维度

仅指编译资源配置档的维度分类。

- [来源处理限额][term-source-processing-limits]：约束来源准入或编译中间阶段规模。
- [规范路网容量][term-canonical-network-capacity]：约束最终规范路网规模。

适用范围包含同义的派生复合词。

例外：运行时快照等其他语境中的容量维度不适用。

登记依据：[\#1296][issue-1296]。

<a id="name-durable-root-baseline"></a>
### 耐久根内容基线

修订完整性期望值语境。

- [耐久修订内容基线][term-durable-revision-content-baseline]：描述修订内容依据。

适用范围包含同义的派生复合词。

登记依据：[\#985][issue-985]。

<a id="name-durable-root-preimage"></a>
### 耐久根内容前像

修订完整性摘要原像语境。

- [耐久修订内容原像][term-durable-revision-content-preimage]：描述规范修订内容。

适用范围包含同义的派生复合词。

登记依据：[\#985][issue-985]。

<a id="name-fixture-label"></a>
### 夹具

测试与证据链语境。

- [fixture][term-fixture]：首次出现夹注固定测试环境。

适用范围包含同义的派生复合词。

例外：路径、required check 名和代码标识符保留原文。

登记依据：[\#1229][issue-1229]。

<a id="name-fresh-placement-label"></a>
### 新鲜摆放

fresh placement 操作语境。

- [新生成][term-fresh-placement]：把车辆放入世界的操作。

适用范围包含同义的派生复合词。

例外：spawn_vehicle 等标识符保留。

登记依据：[\#1229][issue-1229]。

<a id="name-generator-arm-length"></a>
### 臂长

路口生成器 arm_length_meters 参数；不可与扣除路口半径后的道路长度混同。

- [路口中心至道路外端距离][term-center-to-outer-end-distance]：描述该参数含义。

适用范围包含同义的派生复合词。

例外：配置键不改名。

登记依据：[\#1012][issue-1012]。

<a id="name-host-budget-dimension"></a>
### 宿主预算维度

编译资源配置档的宿主操作资源语境。

- [操作资源预算][term-operation-resource-budget]：约束一次操作控制的存续字节与保留容量。

适用范围包含同义的派生复合词。

登记依据：[\#1296][issue-1296]。

<a id="name-measurement-envelope"></a>
### 测量封套

harness 的 measurements.toml 输出。

- [测量记录][term-measurement-record]：明确该文件载荷。

适用范围包含同义的派生复合词。

登记依据：[\#1012][issue-1012]。

<a id="name-occurrence-label"></a>
### 出现项

occurrence 概念及其同义派生复合词。

- [出现记录][term-occurrence]：规范出现位置记录。

适用范围包含同义的派生复合词。

登记依据：[\#1229][issue-1229]。

<a id="name-profile-self-consistency"></a>
### 配置档自洽检查

编译资源配置档语境。

- [配置档校验][term-compile-limits-profile-validation]：核对配置档值域、单位与平台可表示性。
- [配置档资格验证][term-compile-limits-profile-qualification]：验证具名配置档兑现公开能力。

适用范围包含同义的派生复合词。

登记依据：[\#1296][issue-1296]。

<a id="name-provenance-label"></a>
### 来源沿袭 / 沿袭

provenance 关系语境。

- [来源链][term-provenance]：描述输入、工具与处理过程的来源关系。

适用范围包含同义的派生复合词。

例外：动词“沿袭”表示沿用旧制时不适用；代码和协议标识符保留原文。

登记依据：[\#1229][issue-1229]。

<a id="name-shared-root"></a>
### 共享根 / 共享静态根 / shared root / shared-root

曾用 Rust 所有权视角泛指共享静态层或某份共享路网修订。

- [共享静态路网][term-shared-static-network]：描述层、内容、布局或内存度量。
- [共享路网修订][term-shared-network-revision]：描述实例、身份、生命周期、绑定、比较或处置。

适用范围包含同义的派生复合词。

例外：根对象、根 Arc、根借用与根更替等对所有权结构的明确描述，以及格式根表、分类树根、Merkle 根和文件系统根不属于旧称。

例外：公开 API 和外部协议标识符保留精确拼写；历史证据中的旧键不改写。

登记依据：[\#985][issue-985]。

<a id="name-shared-root-authentication"></a>
### 共享根认证 / 根认证

共享路网修订完整性认证语境。

- [共享路网修订认证][term-shared-network-revision-authentication]：核对某份共享路网修订的内容。

适用范围包含同义的派生复合词。

登记依据：[\#985][issue-985]。

<a id="name-signal-declaration-arm"></a>
### 声明臂

信号源健康检查中未被受控连接认领的状态向量位置。

- [相位状态位置][term-phase-state-position]：描述位置及是否被认领。

适用范围包含同义的派生复合词。

例外：保留 UnclaimedSignalArm 等代码标识符和原健康检查规则。

登记依据：[\#1012][issue-1012]。

<a id="name-spatial-session-label"></a>
### 当前空间会话

规范几何采样会话；名称不再携带迁移阶段。

- [空间会话][term-spatial-session]：统一使用空间会话。

适用范围包含同义的派生复合词。


[domain-adapter]: #domain-adapter
[domain-artifacts]: #domain-artifacts
[domain-compiler]: #domain-compiler
[domain-governance]: #domain-governance
[domain-host]: #domain-host
[domain-identity]: #domain-identity
[domain-measurement]: #domain-measurement
[domain-runtime]: #domain-runtime
[domain-spatial]: #domain-spatial
[domain-static-network]: #domain-static-network
[domain-testing]: #domain-testing
[former-names]: #former-names
[issue-1012]: https://github.com/illusion-tech/laneflow/issues/1012
[issue-1229]: https://github.com/illusion-tech/laneflow/issues/1229
[issue-1296]: https://github.com/illusion-tech/laneflow/issues/1296
[issue-985]: https://github.com/illusion-tech/laneflow/issues/985
[retired-concepts]: #retired-concepts
[source-adr-0021-416]: ../adr/0021-traffic-infrastructure-and-host-boundary.md
[source-adr-0024-553]: ../adr/0024-compiler-post-emission-check-and-minimal-publication-closure.md
[source-adr-0025-280]: ../adr/0025-checked-canonical-network-and-shared-static-network.md
[source-adr-0025-551]: ../adr/0025-checked-canonical-network-and-shared-static-network.md#1-不交付独立静态镜像制品或-abi
[source-adr-0029-552]: ../adr/0029-retire-precompiled-static-route.md
[source-adr-0033-188]: ../adr/0033-digest-role-and-algorithm-selection-authority.md
[source-adr-0035-148]: ../adr/0035-compile-limits-and-resource-ledger.md
[source-design-adapter-api-490]: ../design/adapter-api.md
[source-design-committed-road-editing-state-474]: ../design/committed-road-editing-state.md
[source-design-compiler-foundation-149]: ../design/compiler-foundation.md
[source-design-compiler-post-emission-check-and-minimal-publication-closure-281]: ../design/compiler-post-emission-check-and-minimal-publication-closure.md
[source-design-compiler-resource-ledger-150]: ../design/compiler-resource-ledger.md
[source-design-core-runtime-performance-baseline-524]: ../design/core-runtime-performance-baseline.md
[source-design-cross-section-access-230]: ../design/cross-section-access.md
[source-design-example-scenarios-486]: ../design/example-scenarios.md
[source-design-host-observe-526]: ../design/host-observe.md
[source-design-junction-observation-and-validation-419]: ../design/junction-observation-and-validation.md
[source-design-lane-change-minimal-contract-421]: ../design/lane-change-minimal-contract.md
[source-design-lf-synth-measurement-525]: ../design/lf-synth-measurement.md
[source-design-network-compiler-37]: ../design/network-compiler.md
[source-design-parking-system-234]: ../design/parking-system.md
[source-design-portable-canonical-artifact-152]: ../design/portable-canonical-artifact.md
[source-design-road-editing-source-and-geometry-frontend-151]: ../design/road-editing-source-and-geometry-frontend.md
[source-design-road-junction-model-231]: ../design/road-junction-model.md
[source-design-route-system-422]: ../design/route-system.md
[source-design-shared-static-network-187]: ../design/shared-static-network.md
[source-design-signal-system-235]: ../design/signal-system.md
[source-design-spatial-geometry-485]: ../design/spatial-geometry.md
[source-design-traffic-observation-and-routing-integration-472]: ../design/traffic-observation-and-routing-integration.md
[source-design-traffic-runtime-columnar-execution-411]: ../design/traffic-runtime-columnar-execution.md
[source-design-traffic-runtime-execution-config-417]: ../design/traffic-runtime-execution-config.md
[source-design-traffic-runtime-integer-geometry-414]: ../design/traffic-runtime-integer-geometry.md
[source-design-traffic-runtime-module-boundary-550]: ../design/traffic-runtime-module-boundary.md
[source-design-traffic-runtime-parallel-execution-410]: ../design/traffic-runtime-parallel-execution.md
[source-design-traffic-runtime-phase-protocol-412]: ../design/traffic-runtime-phase-protocol.md
[source-design-traffic-runtime-revision-cutover-473]: ../design/traffic-runtime-revision-cutover.md
[source-design-traffic-runtime-right-of-way-policy-415]: ../design/traffic-runtime-right-of-way-policy.md
[source-design-traffic-runtime-shared-consumption-420]: ../design/traffic-runtime-shared-consumption.md
[source-design-traffic-runtime-snapshot-423]: ../design/traffic-runtime-snapshot.md
[source-design-traffic-runtime-state-integrity-413]: ../design/traffic-runtime-state-integrity.md
[source-design-traffic-runtime-vehicle-placement-418]: ../design/traffic-runtime-vehicle-placement.md
[source-design-vehicle-following-236]: ../design/vehicle-following.md
[source-design-waiting-zone-conflict-right-of-way-232]: ../design/waiting-zone-conflict-right-of-way.md
[source-governance-code-language-policy-38]: ../governance/code-language-policy.md
[source-governance-development-gates-527]: ../governance/development-gates.md#51-负载敏感计量
[source-governance-github-workflow-36]: ../governance/github-workflow.md
[source-reference-commit-convention-39]: ../reference/commit-convention.md#62-执行框架目录与入口登记
[term-access-rule]: glossary.md#term-access-rule
[term-active-compact-position]: glossary.md#term-active-compact-position
[term-active-motion-pool]: glossary.md#term-active-motion-pool
[term-active-traffic-participant-count]: glossary.md#term-active-traffic-participant-count
[term-additional-check]: glossary.md#term-additional-check
[term-administrative-state]: glossary.md#term-administrative-state
[term-admitted-route-registration-command]: glossary.md#term-admitted-route-registration-command
[term-aggregate-equivalent-participant-count]: glossary.md#term-aggregate-equivalent-participant-count
[term-aggregate-traffic-population]: glossary.md#term-aggregate-traffic-population
[term-aggregate-traffic-record-count]: glossary.md#term-aggregate-traffic-record-count
[term-alignment]: glossary.md#term-alignment
[term-alternative-test-execution]: glossary.md#term-alternative-test-execution
[term-amdahl-bottleneck]: glossary.md#term-amdahl-bottleneck
[term-application-binary-interface]: glossary.md#term-application-binary-interface
[term-approach]: glossary.md#term-approach
[term-area-label]: glossary.md#term-area-label
[term-arena-key]: glossary.md#term-arena-key
[term-artifact-shape]: glossary.md#term-artifact-shape
[term-attempt-epoch]: glossary.md#term-attempt-epoch
[term-authentication-operational-failure]: glossary.md#term-authentication-operational-failure
[term-authoring]: glossary.md#term-authoring
[term-authoring-admission-quantization]: glossary.md#term-authoring-admission-quantization
[term-authoring-lane]: glossary.md#term-authoring-lane
[term-authoring-namespace-id]: glossary.md#term-authoring-namespace-id
[term-authoritative-source-module-graph]: glossary.md#term-authoritative-source-module-graph
[term-authority]: glossary.md#term-authority
[term-backend]: glossary.md#term-backend
[term-barrier]: glossary.md#term-barrier
[term-base-construction]: glossary.md#term-base-construction
[term-behavior-equivalence]: glossary.md#term-behavior-equivalence
[term-block-soa]: glossary.md#term-block-soa
[term-blocking-dependency]: glossary.md#term-blocking-dependency
[term-body-spill-occupancy]: glossary.md#term-body-spill-occupancy
[term-boundary-exchange]: glossary.md#term-boundary-exchange
[term-bounded-structural-verifier]: glossary.md#term-bounded-structural-verifier
[term-build-and-target-selector]: glossary.md#term-build-and-target-selector
[term-build-plan]: glossary.md#term-build-plan
[term-builder]: glossary.md#term-builder
[term-byte-for-byte-equality]: glossary.md#term-byte-for-byte-equality
[term-byte-order]: glossary.md#term-byte-order
[term-cache]: glossary.md#term-cache
[term-caller-runtime-limit]: glossary.md#term-caller-runtime-limit
[term-candidate]: glossary.md#term-candidate
[term-candidate-execution-dependency-graph]: glossary.md#term-candidate-execution-dependency-graph
[term-candidate-road-revision]: glossary.md#term-candidate-road-revision
[term-candidate-route]: glossary.md#term-candidate-route
[term-canonical-frame]: glossary.md#term-canonical-frame
[term-canonical-identity-table]: glossary.md#term-canonical-identity-table
[term-canonical-identity-tuple]: glossary.md#term-canonical-identity-tuple
[term-canonical-logical-position]: glossary.md#term-canonical-logical-position
[term-canonical-network-capacity]: glossary.md#term-canonical-network-capacity
[term-canonical-network-origin]: glossary.md#term-canonical-network-origin
[term-canonical-network-semantic-payload]: glossary.md#term-canonical-network-semantic-payload
[term-canonical-network-usage]: glossary.md#term-canonical-network-usage
[term-canonical-orders]: glossary.md#term-canonical-orders
[term-canonical-publication-candidate]: glossary.md#term-canonical-publication-candidate
[term-canonical-publication-descriptor]: glossary.md#term-canonical-publication-descriptor
[term-canonical-reduction-authority]: glossary.md#term-canonical-reduction-authority
[term-canonical-relation-occurrence]: glossary.md#term-canonical-relation-occurrence
[term-cardinality]: glossary.md#term-cardinality
[term-center-to-outer-end-distance]: glossary.md#term-center-to-outer-end-distance
[term-checked-canonical-network-input]: glossary.md#term-checked-canonical-network-input
[term-checkpoint]: glossary.md#term-checkpoint
[term-chinese-style-city-workload]: glossary.md#term-chinese-style-city-workload
[term-clean-break]: glossary.md#term-clean-break
[term-clean-compile]: glossary.md#term-clean-compile
[term-clean-single-thread-compile]: glossary.md#term-clean-single-thread-compile
[term-closed-contract]: glossary.md#term-closed-contract
[term-closed-kind-selector]: glossary.md#term-closed-kind-selector
[term-closed-staged-object-source]: glossary.md#term-closed-staged-object-source
[term-code-language-proposal-review]: glossary.md#term-code-language-proposal-review
[term-cold-data]: glossary.md#term-cold-data
[term-cold-instance-compile]: glossary.md#term-cold-instance-compile
[term-commit-harness-identifier]: glossary.md#term-commit-harness-identifier
[term-committed-mutation-stream]: glossary.md#term-committed-mutation-stream
[term-committed-network-source]: glossary.md#term-committed-network-source
[term-committed-road-network]: glossary.md#term-committed-road-network
[term-committed-traffic-observation-snapshot]: glossary.md#term-committed-traffic-observation-snapshot
[term-committed-world-state]: glossary.md#term-committed-world-state
[term-compilation-metrics]: glossary.md#term-compilation-metrics
[term-compilation-unit]: glossary.md#term-compilation-unit
[term-compile-fail-test]: glossary.md#term-compile-fail-test
[term-compile-limits]: glossary.md#term-compile-limits
[term-compile-limits-profile-qualification]: glossary.md#term-compile-limits-profile-qualification
[term-compile-limits-profile-validation]: glossary.md#term-compile-limits-profile-validation
[term-compiler-calibration-scale]: glossary.md#term-compiler-calibration-scale
[term-compiler-controlled-total-live-memory]: glossary.md#term-compiler-controlled-total-live-memory
[term-compiler-emitter]: glossary.md#term-compiler-emitter
[term-compiler-foundation]: glossary.md#term-compiler-foundation
[term-compiler-intermediate-representation]: glossary.md#term-compiler-intermediate-representation
[term-compiler-owned-static-network]: glossary.md#term-compiler-owned-static-network
[term-compiler-pass]: glossary.md#term-compiler-pass
[term-compiler-resource-ledger]: glossary.md#term-compiler-resource-ledger
[term-compiler-semantic-validation]: glossary.md#term-compiler-semantic-validation
[term-compiler-stress-scale]: glossary.md#term-compiler-stress-scale
[term-complete-owner-tree]: glossary.md#term-complete-owner-tree
[term-completion-frontier]: glossary.md#term-completion-frontier
[term-compressed-sparse-row]: glossary.md#term-compressed-sparse-row
[term-condensation-directed-acyclic-graph]: glossary.md#term-condensation-directed-acyclic-graph
[term-conflict-lag-reference]: glossary.md#term-conflict-lag-reference
[term-conflict-passage]: glossary.md#term-conflict-passage
[term-conflict-passage-locator]: glossary.md#term-conflict-passage-locator
[term-conflict-passage-occurrence]: glossary.md#term-conflict-passage-occurrence
[term-conflict-zone]: glossary.md#term-conflict-zone
[term-conflict-zone-region]: glossary.md#term-conflict-zone-region
[term-constraint]: glossary.md#term-constraint
[term-content-identity-and-length-binding]: glossary.md#term-content-identity-and-length-binding
[term-cost-model-identifier]: glossary.md#term-cost-model-identifier
[term-coverage]: glossary.md#term-coverage
[term-crate-dependency-graph]: glossary.md#term-crate-dependency-graph
[term-cross-index]: glossary.md#term-cross-index
[term-current-state]: glossary.md#term-current-state
[term-cutover-event-batch]: glossary.md#term-cutover-event-batch
[term-default-rule]: glossary.md#term-default-rule
[term-delta-catch-up]: glossary.md#term-delta-catch-up
[term-dense-handle]: glossary.md#term-dense-handle
[term-derived-indexes]: glossary.md#term-derived-indexes
[term-design-gate-field]: glossary.md#term-design-gate-field
[term-desynchronization-diagnostic-artifact]: glossary.md#term-desynchronization-diagnostic-artifact
[term-determinism]: glossary.md#term-determinism
[term-deterministic-commit-plan]: glossary.md#term-deterministic-commit-plan
[term-deterministic-derivation-digest]: glossary.md#term-deterministic-derivation-digest
[term-deterministic-merge-order]: glossary.md#term-deterministic-merge-order
[term-deterministic-oracle]: glossary.md#term-deterministic-oracle
[term-deterministic-radix-bucket-sorting]: glossary.md#term-deterministic-radix-bucket-sorting
[term-deterministic-state-digest]: glossary.md#term-deterministic-state-digest
[term-development-gate]: glossary.md#term-development-gate
[term-diagnostic]: glossary.md#term-diagnostic
[term-diagnostic-record]: glossary.md#term-diagnostic-record
[term-differential-testing]: glossary.md#term-differential-testing
[term-digest]: glossary.md#term-digest
[term-distortion]: glossary.md#term-distortion
[term-distortion-attribution]: glossary.md#term-distortion-attribution
[term-distortion-fingerprint]: glossary.md#term-distortion-fingerprint
[term-domain-budget]: glossary.md#term-domain-budget
[term-durable-revision-content-baseline]: glossary.md#term-durable-revision-content-baseline
[term-durable-revision-content-preimage]: glossary.md#term-durable-revision-content-preimage
[term-dynamic-cost-snapshot]: glossary.md#term-dynamic-cost-snapshot
[term-dynamic-cost-snapshot-binding]: glossary.md#term-dynamic-cost-snapshot-binding
[term-dynamic-traversal-definition]: glossary.md#term-dynamic-traversal-definition
[term-editable-diff-base]: glossary.md#term-editable-diff-base
[term-editor-authoring-surface]: glossary.md#term-editor-authoring-surface
[term-engine-adapter]: glossary.md#term-engine-adapter
[term-entity-kind]: glossary.md#term-entity-kind
[term-entry-frontier]: glossary.md#term-entry-frontier
[term-environment-record]: glossary.md#term-environment-record
[term-equivalence-proof]: glossary.md#term-equivalence-proof
[term-exact-bytes-digest]: glossary.md#term-exact-bytes-digest
[term-exact-current]: glossary.md#term-exact-current
[term-exact-execution-path]: glossary.md#term-exact-execution-path
[term-exact-reference-oracle]: glossary.md#term-exact-reference-oracle
[term-execution-block]: glossary.md#term-execution-block
[term-execution-configuration]: glossary.md#term-execution-configuration
[term-execution-path]: glossary.md#term-execution-path
[term-execution-resources]: glossary.md#term-execution-resources
[term-exit-approach]: glossary.md#term-exit-approach
[term-experimental-variant]: glossary.md#term-experimental-variant
[term-external-road-segment]: glossary.md#term-external-road-segment
[term-facility-band]: glossary.md#term-facility-band
[term-fail-closed]: glossary.md#term-fail-closed
[term-fallback-path]: glossary.md#term-fallback-path
[term-fault-injection]: glossary.md#term-fault-injection
[term-feature-bits]: glossary.md#term-feature-bits
[term-fidelity-contract]: glossary.md#term-fidelity-contract
[term-fidelity-degradation]: glossary.md#term-fidelity-degradation
[term-field-tag]: glossary.md#term-field-tag
[term-file-digest-manifest]: glossary.md#term-file-digest-manifest
[term-final-sort-key]: glossary.md#term-final-sort-key
[term-fingerprint]: glossary.md#term-fingerprint
[term-fixed-tick]: glossary.md#term-fixed-tick
[term-fixture]: glossary.md#term-fixture
[term-flat-range]: glossary.md#term-flat-range
[term-format-hard-limit]: glossary.md#term-format-hard-limit
[term-freeze-plan]: glossary.md#term-freeze-plan
[term-fresh-placement]: glossary.md#term-fresh-placement
[term-frontend]: glossary.md#term-frontend
[term-full-delta-export]: glossary.md#term-full-delta-export
[term-fused-single-worker-executor]: glossary.md#term-fused-single-worker-executor
[term-fuzz-testing]: glossary.md#term-fuzz-testing
[term-gap-acceptance-profile]: glossary.md#term-gap-acceptance-profile
[term-gate-compliance-rule]: glossary.md#term-gate-compliance-rule
[term-generated-binding-boundary]: glossary.md#term-generated-binding-boundary
[term-geometry]: glossary.md#term-geometry
[term-geometry-accuracy-profile]: glossary.md#term-geometry-accuracy-profile
[term-geometry-direction-profile]: glossary.md#term-geometry-direction-profile
[term-geometry-direction-profile-applicability-flag]: glossary.md#term-geometry-direction-profile-applicability-flag
[term-graceful-overload-degradation]: glossary.md#term-graceful-overload-degradation
[term-grant]: glossary.md#term-grant
[term-halo]: glossary.md#term-halo
[term-handle]: glossary.md#term-handle
[term-harness-surface]: glossary.md#term-harness-surface
[term-high-level-intermediate-representation]: glossary.md#term-high-level-intermediate-representation
[term-host-application-layer]: glossary.md#term-host-application-layer
[term-host-audit-replay]: glossary.md#term-host-audit-replay
[term-host-transform]: glossary.md#term-host-transform
[term-hot-column]: glossary.md#term-hot-column
[term-hot-data]: glossary.md#term-hot-data
[term-hot-path]: glossary.md#term-hot-path
[term-identity-encoding-envelope]: glossary.md#term-identity-encoding-envelope
[term-immutable]: glossary.md#term-immutable
[term-immutable-byte-backing]: glossary.md#term-immutable-byte-backing
[term-import-frontend]: glossary.md#term-import-frontend
[term-incremental-compile]: glossary.md#term-incremental-compile
[term-individual-run]: glossary.md#term-individual-run
[term-individual-traffic-participant-count]: glossary.md#term-individual-traffic-participant-count
[term-individual-vehicle-count]: glossary.md#term-individual-vehicle-count
[term-input-command-cursor]: glossary.md#term-input-command-cursor
[term-input-command-sequence]: glossary.md#term-input-command-sequence
[term-instrumentation-probe-boundary]: glossary.md#term-instrumentation-probe-boundary
[term-integer-millimetre]: glossary.md#term-integer-millimetre
[term-integration-only-bridge]: glossary.md#term-integration-only-bridge
[term-integrity-disablement]: glossary.md#term-integrity-disablement
[term-intent-update-participant-count]: glossary.md#term-intent-update-participant-count
[term-intent-update-vehicle-count]: glossary.md#term-intent-update-vehicle-count
[term-interchange-format]: glossary.md#term-interchange-format
[term-internal-integrity-digest]: glossary.md#term-internal-integrity-digest
[term-interoperable-digest]: glossary.md#term-interoperable-digest
[term-intersection-rules]: glossary.md#term-intersection-rules
[term-issue-type]: glossary.md#term-issue-type
[term-join]: glossary.md#term-join
[term-junction]: glossary.md#term-junction
[term-junction-group]: glossary.md#term-junction-group
[term-key]: glossary.md#term-key
[term-known-vector]: glossary.md#term-known-vector
[term-lane-edge]: glossary.md#term-lane-edge
[term-lane-flow-junction-observation]: glossary.md#term-lane-flow-junction-observation
[term-lane-graph]: glossary.md#term-lane-graph
[term-lane-group]: glossary.md#term-lane-group
[term-laneflow-traffic-runtime]: glossary.md#term-laneflow-traffic-runtime
[term-layout]: glossary.md#term-layout
[term-level-of-detail]: glossary.md#term-level-of-detail
[term-lifecycle]: glossary.md#term-lifecycle
[term-loader]: glossary.md#term-loader
[term-local-index]: glossary.md#term-local-index
[term-local-intent-batch]: glossary.md#term-local-intent-batch
[term-logical-check-address]: glossary.md#term-logical-check-address
[term-logical-digest-preimage]: glossary.md#term-logical-digest-preimage
[term-logical-update-position]: glossary.md#term-logical-update-position
[term-lowering]: glossary.md#term-lowering
[term-magic]: glossary.md#term-magic
[term-maneuver-direction]: glossary.md#term-maneuver-direction
[term-maneuver-gate]: glossary.md#term-maneuver-gate
[term-maneuver-path]: glossary.md#term-maneuver-path
[term-measurement-record]: glossary.md#term-measurement-record
[term-memory-mapping]: glossary.md#term-memory-mapping
[term-memory-plan]: glossary.md#term-memory-plan
[term-merge-group]: glossary.md#term-merge-group
[term-merge-queue]: glossary.md#term-merge-queue
[term-metadata]: glossary.md#term-metadata
[term-metamorphic-test]: glossary.md#term-metamorphic-test
[term-micrometre-accumulator]: glossary.md#term-micrometre-accumulator
[term-mid-level-intermediate-representation]: glossary.md#term-mid-level-intermediate-representation
[term-migration-delta-journal]: glossary.md#term-migration-delta-journal
[term-migration-oracle]: glossary.md#term-migration-oracle
[term-millimetres-per-second]: glossary.md#term-millimetres-per-second
[term-minimal-publication-closure]: glossary.md#term-minimal-publication-closure
[term-missed-detection-surface]: glossary.md#term-missed-detection-surface
[term-mobility-and-traffic-orchestration-layer]: glossary.md#term-mobility-and-traffic-orchestration-layer
[term-module-resource-counts]: glossary.md#term-module-resource-counts
[term-movement]: glossary.md#term-movement
[term-multi-world-ensemble]: glossary.md#term-multi-world-ensemble
[term-mutable]: glossary.md#term-mutable
[term-mutually-invisible-co-lateral-transition-segments]: glossary.md#term-mutually-invisible-co-lateral-transition-segments
[term-near-gate-vehicle]: glossary.md#term-near-gate-vehicle
[term-network-revision]: glossary.md#term-network-revision
[term-network-revision-cutover-descriptor]: glossary.md#term-network-revision-cutover-descriptor
[term-network-revision-cutover-transaction]: glossary.md#term-network-revision-cutover-transaction
[term-network-revision-id]: glossary.md#term-network-revision-id
[term-normalization]: glossary.md#term-normalization
[term-observation-export-cadence]: glossary.md#term-observation-export-cadence
[term-observation-export-session]: glossary.md#term-observation-export-session
[term-observation-selection]: glossary.md#term-observation-selection
[term-observation-selection-digest]: glossary.md#term-observation-selection-digest
[term-observation-set-digest]: glossary.md#term-observation-set-digest
[term-observation-state-sequence]: glossary.md#term-observation-state-sequence
[term-observation-stream-binding]: glossary.md#term-observation-stream-binding
[term-occupancy]: glossary.md#term-occupancy
[term-occupancy-bucket]: glossary.md#term-occupancy-bucket
[term-occupancy-index]: glossary.md#term-occupancy-index
[term-occupancy-record]: glossary.md#term-occupancy-record
[term-occurrence]: glossary.md#term-occurrence
[term-official-frontend-module]: glossary.md#term-official-frontend-module
[term-offset]: glossary.md#term-offset
[term-open-entrance]: glossary.md#term-open-entrance
[term-operation-resource-budget]: glossary.md#term-operation-resource-budget
[term-outer-encoding-structure]: glossary.md#term-outer-encoding-structure
[term-owner]: glossary.md#term-owner
[term-owner-local-occurrence]: glossary.md#term-owner-local-occurrence
[term-owner-local-relation]: glossary.md#term-owner-local-relation
[term-p100-compile-limits-profile]: glossary.md#term-p100-compile-limits-profile
[term-p100-initial-attestation-production-baseline]: glossary.md#term-p100-initial-attestation-production-baseline
[term-p100-initial-production-compiler-baseline]: glossary.md#term-p100-initial-production-compiler-baseline
[term-page-level-memory-distortion]: glossary.md#term-page-level-memory-distortion
[term-parallel-compile]: glossary.md#term-parallel-compile
[term-parking]: glossary.md#term-parking
[term-parking-arrival]: glossary.md#term-parking-arrival
[term-parking-binding]: glossary.md#term-parking-binding
[term-parking-facility]: glossary.md#term-parking-facility
[term-parking-space]: glossary.md#term-parking-space
[term-parking-target]: glossary.md#term-parking-target
[term-participant-class]: glossary.md#term-participant-class
[term-participant-stream]: glossary.md#term-participant-stream
[term-partition]: glossary.md#term-partition
[term-partition-planning-hints]: glossary.md#term-partition-planning-hints
[term-parts-finish]: glossary.md#term-parts-finish
[term-path-anchor]: glossary.md#term-path-anchor
[term-paused-maintenance-mode]: glossary.md#term-paused-maintenance-mode
[term-peak-allocation]: glossary.md#term-peak-allocation
[term-per-document-source-origin-record]: glossary.md#term-per-document-source-origin-record
[term-per-tick-timing-log]: glossary.md#term-per-tick-timing-log
[term-per-world-mutable-state]: glossary.md#term-per-world-mutable-state
[term-performance-attribution-timing]: glossary.md#term-performance-attribution-timing
[term-persistent-identity-digest]: glossary.md#term-persistent-identity-digest
[term-phase-state-position]: glossary.md#term-phase-state-position
[term-physical-row]: glossary.md#term-physical-row
[term-placeholder-output]: glossary.md#term-placeholder-output
[term-placement-token]: glossary.md#term-placement-token
[term-policy]: glossary.md#term-policy
[term-policy-local-change]: glossary.md#term-policy-local-change
[term-policy-pin]: glossary.md#term-policy-pin
[term-portable-canonical-artifact]: glossary.md#term-portable-canonical-artifact
[term-pose-batch]: glossary.md#term-pose-batch
[term-post-emission-check]: glossary.md#term-post-emission-check
[term-post-emission-checked-bundle]: glossary.md#term-post-emission-checked-bundle
[term-potential-resource-support-graph]: glossary.md#term-potential-resource-support-graph
[term-presented-traffic-participant-count]: glossary.md#term-presented-traffic-participant-count
[term-presented-vehicle-count]: glossary.md#term-presented-vehicle-count
[term-production-cutover]: glossary.md#term-production-cutover
[term-production-path]: glossary.md#term-production-path
[term-project-status]: glossary.md#term-project-status
[term-property-testing]: glossary.md#term-property-testing
[term-proposal]: glossary.md#term-proposal
[term-provenance]: glossary.md#term-provenance
[term-publication-manifest]: glossary.md#term-publication-manifest
[term-published-lfca-reference]: glossary.md#term-published-lfca-reference
[term-quiescent-commit-window]: glossary.md#term-quiescent-commit-window
[term-quiet-measurement-window]: glossary.md#term-quiet-measurement-window
[term-rebind]: glossary.md#term-rebind
[term-reduction-span]: glossary.md#term-reduction-span
[term-reduction-work]: glossary.md#term-reduction-work
[term-reference-machine-asset-identifier]: glossary.md#term-reference-machine-asset-identifier
[term-reference-machine-hardware-identity-fingerprint]: glossary.md#term-reference-machine-hardware-identity-fingerprint
[term-registry]: glossary.md#term-registry
[term-regulation-identity]: glossary.md#term-regulation-identity
[term-research-test-instrumentation]: glossary.md#term-research-test-instrumentation
[term-reservation]: glossary.md#term-reservation
[term-resolution-batch]: glossary.md#term-resolution-batch
[term-resolved-gate-policy-cell]: glossary.md#term-resolved-gate-policy-cell
[term-resolved-stream-rule]: glossary.md#term-resolved-stream-rule
[term-resolver]: glossary.md#term-resolver
[term-resource-claim]: glossary.md#term-resource-claim
[term-resource-dependency-component]: glossary.md#term-resource-dependency-component
[term-retained-memory]: glossary.md#term-retained-memory
[term-retained-memory-ledger]: glossary.md#term-retained-memory-ledger
[term-reverse-index]: glossary.md#term-reverse-index
[term-revision-artifact-binding]: glossary.md#term-revision-artifact-binding
[term-right-of-way-policy-set]: glossary.md#term-right-of-way-policy-set
[term-road-alignment-definition]: glossary.md#term-road-alignment-definition
[term-road-corridor]: glossary.md#term-road-corridor
[term-road-editing-frontend]: glossary.md#term-road-editing-frontend
[term-road-editing-location-context]: glossary.md#term-road-editing-location-context
[term-road-editing-property-path]: glossary.md#term-road-editing-property-path
[term-road-editing-source-address]: glossary.md#term-road-editing-source-address
[term-road-editing-source-buffer]: glossary.md#term-road-editing-source-buffer
[term-road-editing-source-diff]: glossary.md#term-road-editing-source-diff
[term-road-editing-source-input]: glossary.md#term-road-editing-source-input
[term-road-editing-source-root]: glossary.md#term-road-editing-source-root
[term-road-editing-state]: glossary.md#term-road-editing-state
[term-road-editing-state-digest]: glossary.md#term-road-editing-state-digest
[term-road-motor-vehicle-execution-domain]: glossary.md#term-road-motor-vehicle-execution-domain
[term-road-section]: glossary.md#term-road-section
[term-road-traffic-active-vehicle-count]: glossary.md#term-road-traffic-active-vehicle-count
[term-role]: glossary.md#term-role
[term-route]: glossary.md#term-route
[term-route-edge-occurrence-capacity]: glossary.md#term-route-edge-occurrence-capacity
[term-route-gate-observation]: glossary.md#term-route-gate-observation
[term-routing-admission-session]: glossary.md#term-routing-admission-session
[term-routing-service]: glossary.md#term-routing-service
[term-run-configuration]: glossary.md#term-run-configuration
[term-run-records]: glossary.md#term-run-records
[term-runtime-execution-plan]: glossary.md#term-runtime-execution-plan
[term-runtime-overlay]: glossary.md#term-runtime-overlay
[term-runtime-precondition]: glossary.md#term-runtime-precondition
[term-runtime-snapshot]: glossary.md#term-runtime-snapshot
[term-save-manifest]: glossary.md#term-save-manifest
[term-section-directory]: glossary.md#term-section-directory
[term-semantic-diff]: glossary.md#term-semantic-diff
[term-semantic-diff-envelope]: glossary.md#term-semantic-diff-envelope
[term-semantic-research-prototype]: glossary.md#term-semantic-research-prototype
[term-semantic-validation]: glossary.md#term-semantic-validation
[term-serial-phase-protocol]: glossary.md#term-serial-phase-protocol
[term-session]: glossary.md#term-session
[term-shared-checked-module-admission]: glossary.md#term-shared-checked-module-admission
[term-shared-identity-index]: glossary.md#term-shared-identity-index
[term-shared-network-revision]: glossary.md#term-shared-network-revision
[term-shared-network-revision-authentication]: glossary.md#term-shared-network-revision-authentication
[term-shared-spatial-network]: glossary.md#term-shared-spatial-network
[term-shared-static-network]: glossary.md#term-shared-static-network
[term-shared-traffic-network]: glossary.md#term-shared-traffic-network
[term-signal]: glossary.md#term-signal
[term-signal-controller]: glossary.md#term-signal-controller
[term-signal-group]: glossary.md#term-signal-group
[term-signal-phase]: glossary.md#term-signal-phase
[term-single-network-million-compile-limits-profile]: glossary.md#term-single-network-million-compile-limits-profile
[term-single-source-of-truth]: glossary.md#term-single-source-of-truth
[term-snapshot]: glossary.md#term-snapshot
[term-snapshot-local-identity]: glossary.md#term-snapshot-local-identity
[term-source-collection-digest]: glossary.md#term-source-collection-digest
[term-source-document-count]: glossary.md#term-source-document-count
[term-source-document-descriptor]: glossary.md#term-source-document-descriptor
[term-source-document-digest]: glossary.md#term-source-document-digest
[term-source-document-set-digest]: glossary.md#term-source-document-set-digest
[term-source-language]: glossary.md#term-source-language
[term-source-location]: glossary.md#term-source-location
[term-source-map]: glossary.md#term-source-map
[term-source-map-envelope]: glossary.md#term-source-map-envelope
[term-source-module]: glossary.md#term-source-module
[term-source-module-header]: glossary.md#term-source-module-header
[term-source-processing-limits]: glossary.md#term-source-processing-limits
[term-spatial-layer]: glossary.md#term-spatial-layer
[term-spatial-retention-build-option]: glossary.md#term-spatial-retention-build-option
[term-spatial-session]: glossary.md#term-spatial-session
[term-stable-anchor]: glossary.md#term-stable-anchor
[term-stable-capacity-reuse-compile]: glossary.md#term-stable-capacity-reuse-compile
[term-stable-identifier]: glossary.md#term-stable-identifier
[term-static-contract]: glossary.md#term-static-contract
[term-static-execution-constraint-graph]: glossary.md#term-static-execution-constraint-graph
[term-static-network]: glossary.md#term-static-network
[term-stationary-state]: glossary.md#term-stationary-state
[term-stationing]: glossary.md#term-stationing
[term-steady-state-tick]: glossary.md#term-steady-state-tick
[term-step-read-view]: glossary.md#term-step-read-view
[term-stop-line]: glossary.md#term-stop-line
[term-stream-right-of-way-rule]: glossary.md#term-stream-right-of-way-rule
[term-strongly-connected-component]: glossary.md#term-strongly-connected-component
[term-structural-verification]: glossary.md#term-structural-verification
[term-structure-of-arrays]: glossary.md#term-structure-of-arrays
[term-synthetic-dsl-frontend]: glossary.md#term-synthetic-dsl-frontend
[term-synthetic-source-module]: glossary.md#term-synthetic-source-module
[term-table-row]: glossary.md#term-table-row
[term-target-product-recommended-reference-machine]: glossary.md#term-target-product-recommended-reference-machine
[term-target-state]: glossary.md#term-target-state
[term-target-triple]: glossary.md#term-target-triple
[term-task-category]: glossary.md#term-task-category
[term-telemetry-only]: glossary.md#term-telemetry-only
[term-test-case-group]: glossary.md#term-test-case-group
[term-test-support-seam]: glossary.md#term-test-support-seam
[term-tick-workspace]: glossary.md#term-tick-workspace
[term-topology]: glossary.md#term-topology
[term-traffic-configuration]: glossary.md#term-traffic-configuration
[term-traffic-execution-domain]: glossary.md#term-traffic-execution-domain
[term-traffic-fidelity]: glossary.md#term-traffic-fidelity
[term-traffic-infrastructure]: glossary.md#term-traffic-infrastructure
[term-traffic-participant]: glossary.md#term-traffic-participant
[term-traffic-participant-unit]: glossary.md#term-traffic-participant-unit
[term-traffic-transition-event]: glossary.md#term-traffic-transition-event
[term-traffic-world]: glossary.md#term-traffic-world
[term-transient-fingerprint]: glossary.md#term-transient-fingerprint
[term-transition-anchor]: glossary.md#term-transition-anchor
[term-travel-demand]: glossary.md#term-travel-demand
[term-trip-orchestration-layer]: glossary.md#term-trip-orchestration-layer
[term-trust-anchor]: glossary.md#term-trust-anchor
[term-trusted-baseline]: glossary.md#term-trusted-baseline
[term-typed-abstract-syntax-tree]: glossary.md#term-typed-abstract-syntax-tree
[term-typed-logical-ordinal]: glossary.md#term-typed-logical-ordinal
[term-typed-road-editing-model]: glossary.md#term-typed-road-editing-model
[term-unique-owner-relation]: glossary.md#term-unique-owner-relation
[term-validated-canonical-low-level-intermediate-representation]: glossary.md#term-validated-canonical-low-level-intermediate-representation
[term-validated-compilation-output]: glossary.md#term-validated-compilation-output
[term-validated-source-map-input]: glossary.md#term-validated-source-map-input
[term-value-checked-canonical-artifact]: glossary.md#term-value-checked-canonical-artifact
[term-vehicle-directory]: glossary.md#term-vehicle-directory
[term-vehicle-following]: glossary.md#term-vehicle-following
[term-vehicle-following-front-lookahead]: glossary.md#term-vehicle-following-front-lookahead
[term-vehicle-profile]: glossary.md#term-vehicle-profile
[term-vehicle-runtime-unit]: glossary.md#term-vehicle-runtime-unit
[term-vehicle-state]: glossary.md#term-vehicle-state
[term-version-axis]: glossary.md#term-version-axis
[term-view]: glossary.md#term-view
[term-virtual-parked-state]: glossary.md#term-virtual-parked-state
[term-virtual-parking-capacity]: glossary.md#term-virtual-parking-capacity
[term-waiting-dependency-view]: glossary.md#term-waiting-dependency-view
[term-waiting-zone]: glossary.md#term-waiting-zone
[term-warm-data]: glossary.md#term-warm-data
[term-work-mode-label]: glossary.md#term-work-mode-label
[term-worker]: glossary.md#term-worker
[term-workload]: glossary.md#term-workload
[term-world]: glossary.md#term-world
[term-world-binding-state]: glossary.md#term-world-binding-state
[term-world-generation]: glossary.md#term-world-generation
[term-world-identity]: glossary.md#term-world-identity
[term-world-policy-selection]: glossary.md#term-world-policy-selection
[term-wrapper-type]: glossary.md#term-wrapper-type
[term-zero-copy]: glossary.md#term-zero-copy

<!-- generated-terminology:end -->
